//! Local vanilla world owner joining world-global SavedData to resident dimensions.
//!
//! This is not the dedicated server engine: callers still drive simulation ticks
//! and choose chunk lifecycle/readiness. Each dimension owns the complete opaque
//! NBT payloads for active and manually parked chunks; periodic candidate
//! discovery, generation, tick scheduling and automatic persistence remain
//! future engine responsibilities.
//! The owner centralizes local SavedData, per-dimension borders/clocks and cached
//! region files without carrying client or remote-world state.
use crate::{
    chunk::ChunkData,
    compression::Compression,
    gamerules::GameRules,
    level::{LevelAccess, ResidentDimension},
    nbt::Limits,
    region::RegionFile,
    scoreboard::Scoreboard,
    weather::Weather,
    world_border::WorldBorder,
    world_saved_data::{WorldClocks, WorldSavedData},
};
use horizon_core::{position::ChunkPos, ticks::TickTypeId};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
};

struct DimensionRegionStore {
    directory: PathBuf,
    opened: HashMap<ChunkPos, RegionFile>,
}

impl DimensionRegionStore {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            opened: HashMap::new(),
        }
    }

    fn region_mut(
        &mut self,
        position: ChunkPos,
        create: bool,
    ) -> io::Result<Option<&mut RegionFile>> {
        if !self.opened.contains_key(&position) {
            let path = self
                .directory
                .join(format!("r.{}.{}.mca", position.x, position.z));
            if !create && !path.is_file() {
                return Ok(None);
            }
            fs::create_dir_all(&self.directory)?;
            let region = RegionFile::open_writable(&path, create)?;
            self.opened.insert(position, region);
        }
        Ok(self.opened.get_mut(&position))
    }

    fn sync_all(&self) -> io::Result<()> {
        for region in self.opened.values() {
            region.sync()?;
        }
        Ok(())
    }
}

/// Runtime state for one registered dimension in a local vanilla world.
pub struct WorldDimension<'registry> {
    resident: ResidentDimension<'registry>,
    border: WorldBorder,
    regions: DimensionRegionStore,
    /// Opaque NBT payloads for chunks whose block/tick state is resident.
    active: HashMap<ChunkPos, ChunkData>,
    /// Complete opaque NBT payloads retained while their block/tick state is parked.
    parked: HashMap<ChunkPos, ChunkData>,
    /// The per-dimension day clock persisted in `world_clocks.dat`.
    total_ticks: i64,
}

/// Counts the per-candidate actions performed by a caller-driven ticket sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TicketLifecycleReport {
    pub loaded: usize,
    pub activated: usize,
    pub parked: usize,
    pub missing: usize,
    pub deferred_dirty: usize,
}

/// Result of one caller-triggered dirty-chunk save sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyChunkSaveReport {
    /// Dirty snapshots acknowledged after their region sync succeeded.
    pub saved: usize,
    /// Dirty resident chunks left for a later caller-triggered sweep.
    pub remaining_dirty: usize,
}

impl<'registry> WorldDimension<'registry> {
    pub fn resident(&self) -> &ResidentDimension<'registry> {
        &self.resident
    }

    pub fn resident_mut(&mut self) -> &mut ResidentDimension<'registry> {
        &mut self.resident
    }

    pub fn border(&self) -> &WorldBorder {
        &self.border
    }

    pub fn border_mut(&mut self) -> &mut WorldBorder {
        &mut self.border
    }

    pub fn total_ticks(&self) -> i64 {
        self.total_ticks
    }

    /// The dimension's persisted vanilla clock (`world_clocks.dat`).
    pub fn day_time(&self) -> i64 {
        self.total_ticks
    }

    /// Set this dimension's day clock without changing the shared game clock.
    pub fn set_day_time(&mut self, day_time: i64) {
        self.total_ticks = day_time;
        let game_time = self.resident.level().game_time();
        self.resident.set_times(game_time, day_time);
    }

    /// Legacy spelling for setting this dimension's saved day clock; this does
    /// not change the world's shared game-time counter.
    #[deprecated(note = "use set_day_time; game time is owned by LocalWorldRuntime")]
    pub fn set_total_ticks(&mut self, ticks: i64) {
        self.set_day_time(ticks);
    }

    /// Park one clean resident chunk after coordinated unload succeeds. Dirty
    /// snapshots must be saved and synced first; the dimension retains its NBT.
    pub fn park_chunk(&mut self, position: ChunkPos) -> bool {
        if !self.active.contains_key(&position)
            || self.parked.contains_key(&position)
            || self.resident.chunk_save_revision(position).is_none()
            || self.resident.is_chunk_dirty(position)
            || !self.resident.unload_chunk(position)
        {
            return false;
        }
        let chunk_data = self
            .active
            .remove(&position)
            .expect("validated active chunk payload");
        let replaced = self.parked.insert(position, chunk_data);
        debug_assert!(replaced.is_none(), "duplicate parked chunk was rejected");
        true
    }

    /// Restore a parked payload to the resident block/tick owners. The complete
    /// NBT moves back to active ownership only after both owners load successfully.
    pub fn activate_parked_chunk(
        &mut self,
        position: ChunkPos,
        game_time: i64,
        resolve_block: impl FnMut(&str) -> Option<TickTypeId>,
        resolve_fluid: impl FnMut(&str) -> Option<TickTypeId>,
    ) -> io::Result<bool> {
        let Some(data) = self.parked.get(&position) else {
            return Ok(false);
        };
        if self.active.contains_key(&position)
            || self.resident.chunk_save_revision(position).is_some()
        {
            return Ok(false);
        }
        if !self
            .resident
            .load_chunk_data(data, game_time, resolve_block, resolve_fluid)?
        {
            return Ok(false);
        }
        let data = self
            .parked
            .remove(&position)
            .expect("loaded parked chunk payload remains present");
        let replaced = self.active.insert(position, data);
        debug_assert!(replaced.is_none(), "active chunk payload was checked");
        Ok(true)
    }

    pub fn is_chunk_parked(&self, position: ChunkPos) -> bool {
        self.parked.contains_key(&position)
    }

    pub fn parked_chunk_count(&self) -> usize {
        self.parked.len()
    }

    /// Read-only access to a chunk's complete opaque NBT while active.
    pub fn active_chunk_data(&self, position: ChunkPos) -> Option<&ChunkData> {
        self.active.get(&position)
    }

    /// Load one region chunk into resident block/tick state and retain its full
    /// NBT payload in this dimension. Returns false if the region record is absent
    /// or the chunk is already active/parked.
    pub fn load_chunk_from_region(
        &mut self,
        position: ChunkPos,
        limits: Limits,
        game_time: i64,
        resolve_block: impl FnMut(&str) -> Option<TickTypeId>,
        resolve_fluid: impl FnMut(&str) -> Option<TickTypeId>,
    ) -> io::Result<bool> {
        if self.active.contains_key(&position) || self.parked.contains_key(&position) {
            return Ok(false);
        }
        let Some(region) = self.regions.region_mut(position.region(), false)? else {
            return Ok(false);
        };
        let Some(record) = region.read(position)? else {
            return Ok(false);
        };
        let data = ChunkData::new(record.nbt(limits)?)?;
        if data.position() != position {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk coordinates do not match the region slot",
            ));
        }
        if !self
            .resident
            .load_chunk_data(&data, game_time, resolve_block, resolve_fluid)?
        {
            return Ok(false);
        }
        let replaced = self.active.insert(position, data);
        debug_assert!(replaced.is_none(), "active chunk payload was checked");
        Ok(true)
    }

    /// Reconcile only caller-supplied candidates against the current `FULL`
    /// ticket status. Persisted records are loaded, parked records reactivated,
    /// and clean active chunks outside the ticketed set parked. Missing records
    /// are reported but never generated; dirty active chunks stay resident.
    /// Work completed for earlier candidates is not rolled back after an error.
    pub fn reconcile_ticketed_chunks(
        &mut self,
        candidates: impl IntoIterator<Item = ChunkPos>,
        limits: Limits,
        game_time: i64,
        mut resolve_block: impl FnMut(&str) -> Option<TickTypeId>,
        mut resolve_fluid: impl FnMut(&str) -> Option<TickTypeId>,
    ) -> io::Result<TicketLifecycleReport> {
        let levels = self.resident.chunk_tickets().levels();
        let mut seen = HashSet::new();
        let mut report = TicketLifecycleReport::default();
        for position in candidates {
            if !seen.insert(position) {
                continue;
            }
            if levels.fully_loaded(position.x, position.z) {
                if self.parked.contains_key(&position) {
                    if self.activate_parked_chunk(
                        position,
                        game_time,
                        &mut resolve_block,
                        &mut resolve_fluid,
                    )? {
                        report.activated += 1;
                    }
                } else if self.active.contains_key(&position)
                    || self.resident.chunk_save_revision(position).is_some()
                {
                    continue;
                } else if self.load_chunk_from_region(
                    position,
                    limits,
                    game_time,
                    &mut resolve_block,
                    &mut resolve_fluid,
                )? {
                    report.loaded += 1;
                } else {
                    report.missing += 1;
                }
            } else if self.active.contains_key(&position) {
                if self.resident.is_chunk_dirty(position) {
                    report.deferred_dirty += 1;
                } else if self.park_chunk(position) {
                    report.parked += 1;
                }
            }
        }
        Ok(report)
    }

    /// Write one complete active chunk snapshot and acknowledge it only after
    /// the owning region has synced successfully. The dimension retains its
    /// opaque NBT fields across saves.
    pub fn save_chunk_to_region(
        &mut self,
        position: ChunkPos,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
    ) -> io::Result<bool> {
        self.save_owned_chunk_to_region_inner(
            position,
            compression,
            timestamp,
            limits,
            game_time,
            block_name,
            fluid_name,
            RegionFile::sync,
        )
    }

    #[cfg(test)]
    pub(crate) fn save_chunk_to_region_with_sync_for_test(
        &mut self,
        position: ChunkPos,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
        sync: impl FnOnce(&RegionFile) -> io::Result<()>,
    ) -> io::Result<bool> {
        self.save_owned_chunk_to_region_inner(
            position,
            compression,
            timestamp,
            limits,
            game_time,
            block_name,
            fluid_name,
            sync,
        )
    }

    /// Save up to `budget` dirty resident chunks in deterministic coordinate
    /// order. This is an explicit sweep, not a timer or autosave scheduler;
    /// each chunk is acknowledged only after its region sync succeeds. If an
    /// error occurs, earlier chunks may already be durable and later chunks
    /// remain dirty for a retry.
    pub fn save_dirty_chunks(
        &mut self,
        budget: usize,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
    ) -> io::Result<DirtyChunkSaveReport> {
        self.save_dirty_chunks_inner(
            budget,
            compression,
            timestamp,
            limits,
            game_time,
            block_name,
            fluid_name,
            RegionFile::sync,
        )
    }

    #[cfg(test)]
    pub(crate) fn save_dirty_chunks_with_sync_for_test(
        &mut self,
        budget: usize,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
        sync: impl FnMut(&RegionFile) -> io::Result<()>,
    ) -> io::Result<DirtyChunkSaveReport> {
        self.save_dirty_chunks_inner(
            budget,
            compression,
            timestamp,
            limits,
            game_time,
            block_name,
            fluid_name,
            sync,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn save_dirty_chunks_inner(
        &mut self,
        budget: usize,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        mut block_name: impl FnMut(TickTypeId) -> Option<String>,
        mut fluid_name: impl FnMut(TickTypeId) -> Option<String>,
        mut sync: impl FnMut(&RegionFile) -> io::Result<()>,
    ) -> io::Result<DirtyChunkSaveReport> {
        let dirty = self.resident.dirty_chunks();
        let mut saved = 0;
        for position in dirty.into_iter().take(budget) {
            if self.save_owned_chunk_to_region_inner(
                position,
                compression,
                timestamp,
                limits,
                game_time,
                &mut block_name,
                &mut fluid_name,
                |region| sync(region),
            )? {
                saved += 1;
            }
        }
        Ok(DirtyChunkSaveReport {
            saved,
            remaining_dirty: self.resident.dirty_chunks().len(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn save_owned_chunk_to_region_inner(
        &mut self,
        position: ChunkPos,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
        sync: impl FnOnce(&RegionFile) -> io::Result<()>,
    ) -> io::Result<bool> {
        let mut data = self
            .active
            .remove(&position)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "chunk is not active"))?;
        let result = self.save_chunk_data_to_region_inner(
            &mut data,
            compression,
            timestamp,
            limits,
            game_time,
            block_name,
            fluid_name,
            sync,
        );
        let replaced = self.active.insert(position, data);
        debug_assert!(
            replaced.is_none(),
            "active payload was removed before saving"
        );
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn save_chunk_data_to_region_inner(
        &mut self,
        data: &mut ChunkData,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
        sync: impl FnOnce(&RegionFile) -> io::Result<()>,
    ) -> io::Result<bool> {
        let position = data.position();
        let revision = self
            .resident
            .chunk_save_revision(position)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "chunk is not resident"))?;
        let region = self
            .regions
            .region_mut(position.region(), true)?
            .expect("created region must be open");
        self.resident.save_region_chunk(
            region,
            data,
            compression,
            timestamp,
            limits,
            game_time,
            block_name,
            fluid_name,
        )?;
        sync(region)?;
        Ok(self.resident.mark_chunk_snapshot_saved(position, &revision))
    }

    /// Flush every region file opened for this dimension.
    pub fn sync_regions(&self) -> io::Result<()> {
        self.regions.sync_all()
    }
}

/// World-global vanilla state and the dimensions currently owned by this process.
pub struct LocalWorldRuntime<'registry> {
    saved: WorldSavedData,
    scoreboard: Scoreboard,
    game_rules: GameRules,
    weather: Weather,
    clocks: WorldClocks,
    game_time: i64,
    dimensions: BTreeMap<String, WorldDimension<'registry>>,
}
impl<'registry> LocalWorldRuntime<'registry> {
    /// Open the local SavedData files. The path is always treated as local storage;
    /// this runtime has no remote-world persistence mode.
    pub fn open(root: impl AsRef<Path>, enabled_features: &[String]) -> io::Result<Self> {
        let saved = WorldSavedData::new(root.as_ref());
        Ok(Self {
            game_time: saved.load_game_time()?,
            scoreboard: saved.load_scoreboard()?,
            game_rules: saved.load_game_rules(enabled_features)?,
            weather: saved.load_weather()?,
            clocks: saved.load_world_clocks()?,
            saved,
            dimensions: BTreeMap::new(),
        })
    }

    pub fn scoreboard(&self) -> &Scoreboard {
        &self.scoreboard
    }

    pub fn scoreboard_mut(&mut self) -> &mut Scoreboard {
        &mut self.scoreboard
    }

    pub fn game_rules(&self) -> &GameRules {
        &self.game_rules
    }

    pub fn game_rules_mut(&mut self) -> &mut GameRules {
        &mut self.game_rules
    }

    pub fn weather(&self) -> &Weather {
        &self.weather
    }

    pub fn weather_mut(&mut self) -> &mut Weather {
        &mut self.weather
    }

    /// The shared vanilla world game-time counter (`level.dat` `Data.Time`).
    pub fn game_time(&self) -> i64 {
        self.game_time
    }

    /// Set the shared game clock on all current dimensions without changing
    /// their individually restored day clocks.
    pub fn set_game_time(&mut self, game_time: i64) {
        self.game_time = game_time;
        for dimension in self.dimensions.values_mut() {
            let day_time = dimension.resident.level().day_time();
            dimension.resident.set_times(game_time, day_time);
        }
    }

    /// Advance the shared world game-time clock once and apply it to every
    /// resident dimension. Dimension day clocks remain unchanged.
    pub fn advance_game_time(&mut self) -> i64 {
        let game_time = self.game_time().wrapping_add(1);
        self.set_game_time(game_time);
        game_time
    }

    /// Advance one dimension's day clock according to its owning game-rule
    /// decision. Returns `None` if the dimension is not registered.
    pub fn advance_dimension_day_time(&mut self, key: &str, advance: bool) -> Option<i64> {
        let dimension = self.dimensions.get_mut(key)?;
        let day_time = dimension.day_time().wrapping_add(i64::from(advance));
        dimension.set_day_time(day_time);
        Some(day_time)
    }

    pub fn dimension(&self, key: &str) -> Option<&WorldDimension<'registry>> {
        self.dimensions.get(key)
    }

    pub fn dimension_mut(&mut self, key: &str) -> Option<&mut WorldDimension<'registry>> {
        self.dimensions.get_mut(key)
    }

    pub fn dimensions(&self) -> impl Iterator<Item = (&str, &WorldDimension<'registry>)> {
        self.dimensions
            .iter()
            .map(|(key, dimension)| (key.as_str(), dimension))
    }

    /// Register an empty resident dimension before loading its chunks, so restored
    /// clocks are applied before tick deadlines are unpacked. Loads border and
    /// clock from SavedData; duplicate keys leave the runtime unchanged.
    pub fn add_dimension(&mut self, resident: ResidentDimension<'registry>) -> io::Result<bool> {
        let key = resident.key().to_owned();
        if self.dimensions.contains_key(&key) {
            return Ok(false);
        }
        if resident.level().loaded_chunk_count() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "register the empty dimension before loading resident chunks",
            ));
        }
        let border = self.saved.load_world_border(&key)?;
        let region_directory = self.saved.region_directory(&key)?;
        let total_ticks = self.clocks.total_ticks(&key).unwrap_or(self.game_time);
        let mut resident = resident;
        resident.set_times(self.game_time, total_ticks);
        self.dimensions.insert(
            key,
            WorldDimension {
                resident,
                border,
                regions: DimensionRegionStore::new(region_directory),
                active: HashMap::new(),
                parked: HashMap::new(),
                total_ticks,
            },
        );
        Ok(true)
    }

    /// Persist world-global SavedData and the borders/clocks of registered
    /// dimensions. Dirty chunks use the explicit chunk-level save contract and
    /// `sync_regions` provides an aggregate shutdown flush.
    pub fn save_world_data(&mut self) -> io::Result<()> {
        let game_time = self.game_time();
        self.saved.save_game_time(game_time)?;
        for (key, dimension) in &self.dimensions {
            self.clocks
                .set_total_ticks(key.clone(), dimension.total_ticks);
        }
        self.saved.save_scoreboard(&self.scoreboard)?;
        self.saved.save_game_rules(&self.game_rules)?;
        self.saved.save_weather(&self.weather)?;
        self.saved.save_world_clocks(&self.clocks)?;
        for (key, dimension) in &self.dimensions {
            self.saved.save_world_border(key, &dimension.border)?;
        }
        Ok(())
    }

    /// Flush every region file opened by the registered dimensions. Per-chunk
    /// saves already sync before acknowledging dirty state; this is also a
    /// convenient, explicit durability boundary for world shutdown.
    pub fn sync_regions(&self) -> io::Result<()> {
        for dimension in self.dimensions.values() {
            dimension.sync_regions()?;
        }
        Ok(())
    }
}

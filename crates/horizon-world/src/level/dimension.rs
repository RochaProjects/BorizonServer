//! Server-owned per-dimension coordinator for resident block columns and tick queues.
//!
//! This is a lifecycle boundary, not a complete running server level: generation,
//! entities, lighting, region-file ownership and durability policy remain with the
//! caller. It makes it harder for the resident block store and scheduled tick owner
//! to disagree about which chunks are loaded.
use super::{
    Behaviours, InteractionResult, LevelAccess, PlaceContext, PlaceOutcome, PlayerContext,
    ResidentLevel, Ticks,
};
use crate::{
    chunk::ChunkData,
    compression::Compression,
    nbt::Limits,
    region::RegionFile,
    ticket_levels::{ChunkTickets, Sources},
};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::{BlockPos, ChunkPos},
    ticks::{SavedTick, TickTypeId},
};
use std::{collections::HashMap, io};

#[derive(Clone, Debug, PartialEq, Eq)]
struct TickSnapshot {
    blocks: Vec<SavedTick>,
    fluids: Vec<SavedTick>,
}

/// Revision of one complete block-state and scheduled-tick save snapshot.
/// The opaque tick contents make acknowledgements safe even if queues change
/// after a staged or asynchronous save began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentChunkRevision {
    block_revision: u128,
    tick_snapshot: TickSnapshot,
}

/// Coordinates a dimension's resident block columns and scheduled block/fluid ticks.
pub struct ResidentDimension<'registry> {
    key: String,
    level: ResidentLevel<'registry>,
    ticks: Ticks,
    chunk_tickets: ChunkTickets,
    saved_ticks: HashMap<ChunkPos, TickSnapshot>,
}
impl<'registry> ResidentDimension<'registry> {
    /// Pair an initialized resident level with its tick owner and vanilla dimension ID.
    pub fn new(key: impl Into<String>, level: ResidentLevel<'registry>, ticks: Ticks) -> Self {
        let mut chunk_tickets = ChunkTickets::default();
        // A server dimension with no sources has no block-ticking chunks. The
        // `None` source mode remains available for headless/test levels that
        // intentionally tick everywhere.
        chunk_tickets.update(Some(Sources::default()));
        let mut level = level;
        level.set_chunk_ticking_levels(chunk_tickets.levels());
        Self {
            key: key.into(),
            level,
            ticks,
            chunk_tickets,
            saved_ticks: HashMap::new(),
        }
    }

    /// Vanilla dimension identifier, such as `minecraft:overworld`.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn level(&self) -> &ResidentLevel<'registry> {
        &self.level
    }

    pub fn level_mut(&mut self) -> &mut ResidentLevel<'registry> {
        &mut self.level
    }

    /// Drain block mutations produced by this dimension since its last
    /// publication pass.
    pub fn take_block_changes(&mut self) -> Vec<(BlockPos, crate::registry::BlockStateId)> {
        self.level.take_block_changes()
    }

    /// Drain simulation side effects from this dimension.
    pub fn take_events(&mut self) -> Vec<super::LevelEvent> {
        self.level.take_events()
    }

    /// Run a block's vanilla empty-hand interaction against this dimension's
    /// paired level and scheduled-tick owner.
    pub fn use_without_item(
        &mut self,
        behaviours: &Behaviours,
        pos: BlockPos,
        face: Direction,
        hit: Vec3,
        player: PlayerContext,
    ) -> InteractionResult {
        behaviours.use_without_item(&mut self.level, &mut self.ticks, pos, face, hit, player)
    }

    /// Run vanilla `BlockItem.place` against this dimension's paired resident
    /// level and scheduled-tick owner.
    pub fn place_block(&mut self, behaviours: &Behaviours, context: &PlaceContext) -> PlaceOutcome {
        behaviours.place_block(&mut self.level, &mut self.ticks, context)
    }

    pub fn ticks(&self) -> &Ticks {
        &self.ticks
    }

    pub fn ticks_mut(&mut self) -> &mut Ticks {
        &mut self.ticks
    }

    /// Current vanilla chunk tickets for this dimension.
    pub fn chunk_tickets(&self) -> &ChunkTickets {
        &self.chunk_tickets
    }

    /// Rebuild this dimension's ticket levels and immediately apply their
    /// block-ticking readiness to the resident level. Loading/unloading chunks
    /// remains the job of the caller's chunk lifecycle.
    pub fn update_chunk_tickets(&mut self, sources: Option<Sources>) {
        self.chunk_tickets.update(sources);
        self.level
            .set_chunk_ticking_levels(self.chunk_tickets.levels());
    }

    /// Set the level's game/day clocks together so simulation reads the same
    /// values used by the dimension's scheduled-tick lifecycle.
    pub(crate) fn set_times(&mut self, game_time: i64, day_time: i64) {
        self.level.set_times(game_time, day_time);
    }

    /// Run vanilla scheduled block and fluid ticks for the current game time.
    /// Random ticks, block events, block entities and entity ticks remain explicit
    /// later phases for the owning engine.
    pub fn tick_scheduled(&mut self, behaviours: &Behaviours) {
        behaviours.tick_scheduled(&mut self.level, &mut self.ticks);
    }

    /// Run random ticks in loaded chunks selected by the dimension's random
    /// simulation ticket level. Section discovery uses the resident palettes.
    pub fn tick_random_chunks(&mut self, behaviours: &Behaviours) {
        let chunks = self
            .level
            .loaded_chunks()
            .into_iter()
            .filter(|chunk| {
                self.chunk_tickets.levels().random_ticking(chunk.x, chunk.z)
                    && self.level.is_ticking_chunk(*chunk)
            })
            .collect::<Vec<_>>();
        for chunk in chunks {
            let (level, ticks) = (&mut self.level, &mut self.ticks);
            behaviours.tick_chunk_random(level, ticks, chunk, |level, from| {
                level.next_random_ticking_section(chunk, from, |state| {
                    behaviours.random_tick_candidate(state)
                })
            });
        }
    }

    /// Run queued vanilla block events after scheduled and random ticks.
    pub fn run_block_events(&mut self, behaviours: &Behaviours) {
        behaviours.run_block_events(&mut self.level, &mut self.ticks);
    }

    /// Advance moving piston block entities for one server tick.
    ///
    /// Vanilla runs handled block events before this phase, so a piston that
    /// starts moving from an event receives its first movement tick in the
    /// same level tick. Other block entity tickers are owned by higher-level
    /// game systems and are not represented by this dimension yet.
    pub fn tick_block_entities(&mut self, behaviours: &Behaviours) {
        behaviours.tick_block_entities(&mut self.level, &mut self.ticks);
    }

    /// Apply player-driven destruction using this dimension's paired resident
    /// level and tick owners. This mirrors the existing `Behaviours` contract
    /// while keeping the borrow boundary server-owned and atomic.
    pub fn destroy_block_by_player(
        &mut self,
        behaviours: &Behaviours,
        pos: BlockPos,
        creative: bool,
        correct_tool: bool,
    ) -> bool {
        behaviours.player_destroy_block(
            &mut self.level,
            &mut self.ticks,
            pos,
            creative,
            correct_tool,
        )
    }

    /// Load a persisted chunk into both owners. If either tick list cannot be
    /// decoded/registered, the just-loaded block column is rolled back.
    /// Returns false if the block column was already resident.
    pub fn load_chunk_data(
        &mut self,
        data: &ChunkData,
        game_time: i64,
        resolve_block: impl FnMut(&str) -> Option<TickTypeId>,
        resolve_fluid: impl FnMut(&str) -> Option<TickTypeId>,
    ) -> io::Result<bool> {
        let position = data.position();
        if !self.level.load_chunk_data(data)? {
            return Ok(false);
        }
        if let Err(error) = self.ticks.load_saved_chunk_ticks(
            position,
            data,
            game_time,
            resolve_block,
            resolve_fluid,
        ) {
            let rolled_back = self.level.unload_chunk(position);
            debug_assert!(rolled_back, "newly loaded clean chunk must roll back");
            return Err(error);
        }
        self.record_tick_snapshot(position);
        Ok(true)
    }

    /// Install a newly generated proto/full record into both resident owners.
    /// Unlike loading disk state, its initial snapshot is dirty and eligible for
    /// the normal save sweep. Tick decode failures retain the existing rollback.
    pub fn load_generated_chunk_data(
        &mut self,
        data: &ChunkData,
        game_time: i64,
        resolve_block: impl FnMut(&str) -> Option<TickTypeId>,
        resolve_fluid: impl FnMut(&str) -> Option<TickTypeId>,
    ) -> io::Result<bool> {
        let position = data.position();
        if !self.load_chunk_data(data, game_time, resolve_block, resolve_fluid)? {
            return Ok(false);
        }
        let marked = self.level.mark_chunk_dirty(position);
        debug_assert!(marked, "generated chunk was just loaded into the level");
        Ok(marked)
    }

    /// Read and load one chunk from an already-open region. Region ownership and
    /// sync remain with the caller; chunk block/tick lifecycle is coordinated here.
    pub fn load_region_chunk(
        &mut self,
        region: &mut RegionFile,
        position: ChunkPos,
        limits: Limits,
        game_time: i64,
        resolve_block: impl FnMut(&str) -> Option<TickTypeId>,
        resolve_fluid: impl FnMut(&str) -> Option<TickTypeId>,
    ) -> io::Result<bool> {
        if position.region() != region.position() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chunk does not belong to this region",
            ));
        }
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
        self.load_chunk_data(&data, game_time, resolve_block, resolve_fluid)
    }

    /// Add a newly generated empty chunk and register empty block/fluid queues.
    /// A duplicate tick registration rolls the column back without dropping it.
    pub fn load_empty_chunk(&mut self, position: ChunkPos) -> io::Result<bool> {
        if !self.level.load_empty_chunk(position) {
            return Ok(false);
        }
        if let Err(error) = self.ticks.add_empty_chunk(position) {
            let discarded = self.level.discard_chunk(position);
            debug_assert!(discarded, "newly created chunk must roll back");
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                error.to_string(),
            ));
        }
        self.record_tick_snapshot(position);
        Ok(true)
    }

    /// Save block states and both scheduled tick lists to a chunk snapshot.
    /// This does not acknowledge dirty state: call `mark_chunk_saved` only after
    /// the caller's region write and durability boundary succeed.
    pub fn save_chunk_data(
        &self,
        data: &mut ChunkData,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
    ) -> io::Result<()> {
        let mut staged = data.clone();
        self.level.save_chunk_data(&mut staged)?;
        self.ticks.save_chunk_ticks(
            staged.position(),
            &mut staged,
            game_time,
            block_name,
            fluid_name,
        )?;
        *data = staged;
        Ok(())
    }

    /// Encode both block palettes and scheduled ticks and write one Anvil record.
    /// Call `mark_chunk_saved` only after the caller successfully syncs the region.
    pub fn save_region_chunk(
        &self,
        region: &mut RegionFile,
        data: &mut ChunkData,
        compression: Compression,
        timestamp: u32,
        limits: Limits,
        game_time: i64,
        block_name: impl FnMut(TickTypeId) -> Option<String>,
        fluid_name: impl FnMut(TickTypeId) -> Option<String>,
    ) -> io::Result<()> {
        let position = data.position();
        if position.region() != region.position() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chunk does not belong to this region",
            ));
        }
        let mut staged = data.clone();
        self.level.save_chunk_data(&mut staged)?;
        self.ticks
            .save_chunk_ticks(position, &mut staged, game_time, block_name, fluid_name)?;
        let record =
            crate::region::ChunkRecord::from_nbt(staged.root(), compression, timestamp, limits)?;
        region.write(position, &record)?;
        *data = staged;
        Ok(())
    }

    /// Capture the block and tick state that a save will persist.
    pub fn chunk_save_revision(&self, position: ChunkPos) -> Option<ResidentChunkRevision> {
        Some(ResidentChunkRevision {
            block_revision: self.level.chunk_revision(position)?,
            tick_snapshot: self.current_tick_snapshot(position)?,
        })
    }

    /// Acknowledge a complete chunk snapshot after its storage commit succeeds.
    /// Returns false if either blocks or scheduled ticks changed since capture.
    pub fn mark_chunk_snapshot_saved(
        &mut self,
        position: ChunkPos,
        revision: &ResidentChunkRevision,
    ) -> bool {
        if self.level.chunk_revision(position) != Some(revision.block_revision)
            || self.current_tick_snapshot(position).as_ref() != Some(&revision.tick_snapshot)
        {
            return false;
        }
        if !self
            .level
            .mark_chunk_saved(position, revision.block_revision)
        {
            return false;
        }
        self.saved_ticks
            .insert(position, revision.tick_snapshot.clone());
        true
    }

    /// Acknowledge block-state-only callers. Tick changes remain dirty until
    /// acknowledged through `mark_chunk_snapshot_saved` with a full snapshot.
    pub fn mark_chunk_saved(&mut self, position: ChunkPos, revision: u128) -> bool {
        self.level.mark_chunk_saved(position, revision)
    }

    fn current_tick_snapshot(&self, position: ChunkPos) -> Option<TickSnapshot> {
        Some(TickSnapshot {
            blocks: self.ticks.blocks.chunk(position)?.pack(0),
            fluids: self.ticks.fluids.chunk(position)?.pack(0),
        })
    }

    fn record_tick_snapshot(&mut self, position: ChunkPos) {
        if let Some(snapshot) = self.current_tick_snapshot(position) {
            self.saved_ticks.insert(position, snapshot);
        }
    }

    /// Whether block state or scheduled block/fluid ticks have changed since
    /// their last acknowledged durable snapshot.
    pub fn is_chunk_dirty(&self, position: ChunkPos) -> bool {
        if self.level.is_chunk_dirty(position) {
            return true;
        }
        match (
            self.current_tick_snapshot(position),
            self.saved_ticks.get(&position),
        ) {
            (Some(current), Some(saved)) => current != *saved,
            // A resident block column must have both vanilla tick queues and
            // a baseline. Missing either indicates external owner mutation;
            // fail closed rather than allowing a possibly lossy unload.
            _ => self.level.chunk_revision(position).is_some(),
        }
    }

    /// Resident chunks with either block-state or scheduled-tick changes.
    pub fn dirty_chunks(&self) -> Vec<ChunkPos> {
        let mut chunks = self
            .level
            .loaded_chunks()
            .into_iter()
            .filter(|&position| self.is_chunk_dirty(position))
            .collect::<Vec<_>>();
        chunks.sort_by_key(|position| (position.x, position.z));
        chunks
    }

    /// Unload a clean chunk from both owners. Dirty block or tick snapshots are retained.
    pub fn unload_chunk(&mut self, position: ChunkPos) -> bool {
        if self.is_chunk_dirty(position) || !self.level.unload_chunk(position) {
            return false;
        }
        self.ticks.remove_chunk(position);
        self.saved_ticks.remove(&position);
        true
    }

    /// Explicitly discard a chunk and its tick queues, including unsaved edits.
    pub fn discard_chunk(&mut self, position: ChunkPos) -> bool {
        let discarded = self.level.discard_chunk(position);
        if discarded {
            self.ticks.remove_chunk(position);
            self.saved_ticks.remove(&position);
        }
        discarded
    }

    /// Whether both vanilla tick queues are registered for this resident chunk.
    pub fn has_chunk_ticks(&self, position: ChunkPos) -> bool {
        self.ticks.blocks.chunk(position).is_some() && self.ticks.fluids.chunk(position).is_some()
    }
}

//! Server-owned per-dimension coordinator for resident block columns and tick queues.
//!
//! This is a lifecycle boundary, not a complete running server level: generation,
//! entities, lighting, region-file ownership and durability policy remain with the
//! caller. It makes it harder for the resident block store and scheduled tick owner
//! to disagree about which chunks are loaded.
use super::{Behaviours, LevelAccess, ResidentLevel, Ticks};
use crate::{chunk::ChunkData, compression::Compression, nbt::Limits, region::RegionFile};
use horizon_core::{position::ChunkPos, ticks::TickTypeId};
use std::io;

/// Coordinates a dimension's resident block columns and scheduled block/fluid ticks.
pub struct ResidentDimension<'registry> {
    key: String,
    level: ResidentLevel<'registry>,
    ticks: Ticks,
}
impl<'registry> ResidentDimension<'registry> {
    /// Pair an initialized resident level with its tick owner and vanilla dimension ID.
    pub fn new(key: impl Into<String>, level: ResidentLevel<'registry>, ticks: Ticks) -> Self {
        Self {
            key: key.into(),
            level,
            ticks,
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

    pub fn ticks(&self) -> &Ticks {
        &self.ticks
    }

    pub fn ticks_mut(&mut self) -> &mut Ticks {
        &mut self.ticks
    }

    /// Set the level's game/day clocks together so simulation reads the same
    /// values used by the dimension's scheduled-tick lifecycle.
    pub fn set_times(&mut self, game_time: i64, day_time: i64) {
        self.level.set_times(game_time, day_time);
    }

    /// Advance one dimension game tick; day time advances only when the caller's
    /// `advance_time` game rule permits it. Scheduled queues observe this clock.
    pub fn advance_time(&mut self, advance_day_time: bool) -> (i64, i64) {
        let game_time = self.level.game_time().wrapping_add(1);
        let day_time = self
            .level
            .day_time()
            .wrapping_add(i64::from(advance_day_time));
        self.level.set_times(game_time, day_time);
        (game_time, day_time)
    }

    /// Run vanilla scheduled block and fluid ticks for the current game time.
    /// Random ticks, block events, block entities and entity ticks remain explicit
    /// later phases for the owning engine.
    pub fn tick_scheduled(&mut self, behaviours: &Behaviours) {
        behaviours.tick_scheduled(&mut self.level, &mut self.ticks);
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
        Ok(true)
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

    /// Acknowledge a chunk snapshot after its storage commit succeeds.
    pub fn mark_chunk_saved(&mut self, position: ChunkPos, revision: u128) -> bool {
        self.level.mark_chunk_saved(position, revision)
    }

    /// Unload a clean chunk from both owners. Dirty chunks are retained.
    pub fn unload_chunk(&mut self, position: ChunkPos) -> bool {
        if !self.level.unload_chunk(position) {
            return false;
        }
        self.ticks.remove_chunk(position);
        true
    }

    /// Explicitly discard a chunk and its tick queues, including unsaved edits.
    pub fn discard_chunk(&mut self, position: ChunkPos) -> bool {
        let discarded = self.level.discard_chunk(position);
        if discarded {
            self.ticks.remove_chunk(position);
        }
        discarded
    }

    /// Whether both vanilla tick queues are registered for this resident chunk.
    pub fn has_chunk_ticks(&self, position: ChunkPos) -> bool {
        self.ticks.blocks.chunk(position).is_some() && self.ticks.fluids.chunk(position).is_some()
    }
}

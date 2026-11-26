//! Server-side vanilla block behaviour: `Level.setBlock` with neighbour and
//! shape-update propagation, scheduled block/fluid ticks, random ticks,
//! placement (`getStateForPlacement`) and interaction (`useWithoutItem`).
//!
//! The library is client-agnostic. A game owns its chunk storage and exposes
//! it through [`LevelAccess`]; it also owns one [`Ticks`] per level (scheduled
//! tick queues, the in-flight neighbour-update queue and the random-tick LCG).
//! [`Behaviours`] holds immutable tables loaded once from the pinned client
//! resources (`block-behaviours.json`, `gameplay-facts.json`,
//! `block-tags.json`, `cached-collision.nbt`; see
//! `crates/horizon-prepare`).
//!
//! # Driving a level
//!
//! Per server tick, in vanilla order (`ServerLevel.tick`):
//! 1. [`Behaviours::tick_scheduled`] runs due block ticks then fluid ticks
//!    (`blockTicks.tick(time, 65536, ...)`, then `fluidTicks`).
//! 2. For every ticking chunk, [`Behaviours::tick_chunk_random`] performs
//!    `ServerLevel.tickChunk`'s random ticks: for each non-empty section,
//!    `randomTickSpeed` (game rule, default 3) positions are drawn with
//!    `Level.getBlockRandomPos` and randomly-ticking block and fluid states are
//!    ticked. The caller says which sections are randomly ticking when each is
//!    reached (vanilla counts randomly-ticking states per section);
//!    [`Behaviours::random_tick`] ticks a single position.
//! 3. [`Behaviours::run_block_events`] runs queued block events (pistons),
//!    then (after entities) [`Behaviours::tick_block_entities`] advances
//!    moving pistons ([`Ticks::moving_pistons`]).
//! 4. Player actions call [`Behaviours::place_block`] /
//!    [`Behaviours::use_without_item`] / [`Behaviours::destroy_block`].
//!
//! Every block mutation goes through [`Behaviours::set_block`] with vanilla's
//! update flags ([`flags`]). The library writes storage only through
//! [`LevelAccess::set_raw`]; the caller updates heightmaps, light, block
//! entities and render meshes for the written position.
//!
//! Side effects the library cannot perform (sounds, particles, item drops,
//! spawning falling-block or TNT entities, growing trees) are reported as
//! [`LevelEvent`]s through [`LevelAccess::emit`], in vanilla call order.
//!
//! Randomness is drawn from [`LevelAccess::random`] (vanilla `level.random`, a
//! `LegacyRandomSource`) in the same order as vanilla.
//!
//! # Integration sketch
//!
//! ```ignore
//! use horizon_world::level::{self, Behaviours, LevelAccess, LevelEvent, PlaceContext, Ticks, flags};
//!
//! struct GameLevel { chunks: Chunks, random: LegacyRandom, time: i64, events: Vec<LevelEvent> }
//! impl LevelAccess for GameLevel {
//!     fn block_state(&self, pos: BlockPos) -> BlockStateId { self.chunks.get(pos) }
//!     fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
//!         let old = self.chunks.set(pos, state)?; // also mark light/heightmap/mesh dirty
//!         Some(old)
//!     }
//!     fn is_loaded(&self, pos: BlockPos) -> bool { self.chunks.contains(pos.chunk()) }
//!     fn random(&mut self) -> &mut LegacyRandom { &mut self.random }
//!     fn game_time(&self) -> i64 { self.time }
//!     fn block_light(&self, pos: BlockPos) -> i32 { self.chunks.block_light(pos) }
//!     fn sky_light(&self, pos: BlockPos) -> i32 { self.chunks.sky_light(pos) }
//!     fn emit(&mut self, event: LevelEvent) { self.events.push(event) }
//! }
//!
//! let registry = level::data::load_registry(&resources)?;
//! let behaviours = Behaviours::load(&registry, &resources)?;
//! let mut ticks = Ticks::new(seed as i32);
//! ticks.add_empty_chunk(chunk_pos)?;          // when a chunk loads (remove_chunk on unload)
//!
//! // Every server tick:
//! level.time += 1;
//! behaviours.tick_scheduled(&mut level, &mut ticks);
//! for chunk in ticking_chunks {
//!     behaviours.tick_chunk_random(&mut level, &mut ticks, chunk, |level, from| level.next_ticking_section(chunk, from));
//! }
//! behaviours.run_block_events(&mut level, &mut ticks);
//! /* tick entities */
//! behaviours.tick_block_entities(&mut level, &mut ticks);
//! for event in level.events.drain(..) { /* play sounds, spawn drops/falling blocks, grow trees */ }
//!
//! // Player actions:
//! behaviours.place_block(&mut level, &mut ticks, &PlaceContext { clicked_pos, face, hit, inside: false,
//!     yaw, pitch, sneaking, item: "minecraft:oak_stairs".into(), can_use_game_master_blocks: false });
//! behaviours.use_without_item(&mut level, &mut ticks, pos, face, hit, player_context);
//! behaviours.player_destroy_block(&mut level, &mut ticks, pos, creative, correct_tool);
//! // Falling-block entity landed:
//! behaviours.falling_block_landed(&mut level, &mut ticks, landing_pos, carried_state, false);
//! ```
use crate::ticket_levels::Levels;
use crate::{
    chunk::BlockSection,
    dimension_type::DimensionFacts,
    fluid::{FluidBlockBindings, FluidKind, FluidState},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    LevelKey,
    geometry::{Aabb, Direction, VoxelShape},
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use std::{collections::HashMap, io, path::Path, sync::Arc, sync::RwLock};

pub mod classes;
pub mod data;
pub mod dimension;
pub mod states;
pub mod ticks;

mod bonemeal;
mod buckets;
mod creaking;
mod falling;
mod feature_world;
mod fluids;
mod growth;
mod hooks;
mod interact;
pub mod pistons;
mod place;
pub mod portal;
pub mod portal_travel;
mod query;
pub(crate) use query::data_2d;
pub mod orientation;
mod rails;
mod redstone;
pub mod removal;
pub mod sculk;
mod shapes;
mod shelf;
mod signals;
mod speleothem;
mod survive;
mod tick;
mod timeline;
mod update;
mod weather;

#[cfg(test)]
mod growth_tests;
#[cfg(test)]
mod tests;

pub use bonemeal::TemplateSettings;
pub use buckets::{BucketContent, is_lava_bucket};
pub use data::{BlockData, ItemBinding, ItemKind, Named, Props, PushReaction, StateFacts, Tags};
pub use dimension::{ResidentChunkRevision, ResidentDimension};
pub use falling::Landing;
pub use interact::{InteractionResult, PlayerContext, PlayerEffect, UseBranch, hit_slot};
pub use pistons::{BlockEventData, MovingPiston};
pub use place::{PlaceContext, PlaceOutcome, PlacementContext};
pub use states::{BlockId, PropKey, StateTable};
pub use tick::{Precipitation, PrecipitationKind, RandomTickMask};
pub use ticks::Ticks;

/// `Block.UPDATE_*` flags accepted by [`Behaviours::set_block`].
pub mod flags {
    pub const UPDATE_NEIGHBORS: i32 = 1;
    pub const UPDATE_CLIENTS: i32 = 2;
    pub const UPDATE_INVISIBLE: i32 = 4;
    /// Read only by a client's level, which rebuilds the section at once.
    pub const UPDATE_IMMEDIATE: i32 = 8;
    pub const UPDATE_KNOWN_SHAPE: i32 = 16;
    pub const UPDATE_SUPPRESS_DROPS: i32 = 32;
    pub const UPDATE_MOVE_BY_PISTON: i32 = 64;
    pub const UPDATE_SKIP_SHAPE_UPDATE_ON_WIRE: i32 = 128;
    pub const UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS: i32 = 256;
    pub const UPDATE_SKIP_ON_PLACE: i32 = 512;
    pub const UPDATE_NONE: i32 = 260;
    pub const UPDATE_ALL: i32 = 3;
    pub const UPDATE_ALL_IMMEDIATE: i32 = 11;
    pub const UPDATE_SKIP_ALL_SIDEEFFECTS: i32 = 816;
    /// `Block.UPDATE_LIMIT`, the recursion budget of shape updates.
    pub const UPDATE_LIMIT: i32 = 512;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
    Custom(LevelKey),
}

impl Dimension {
    /// The level key of this dimension.
    pub fn key(self) -> LevelKey {
        match self {
            Self::Overworld => LevelKey::OVERWORLD,
            Self::Nether => LevelKey::NETHER,
            Self::End => LevelKey::END,
            Self::Custom(key) => key,
        }
    }

    /// The dimension of a level key, retaining custom registry ids.
    pub fn of_level(key: LevelKey) -> Self {
        match key {
            LevelKey::NETHER => Self::Nether,
            LevelKey::END => Self::End,
            LevelKey::OVERWORLD => Self::Overworld,
            other => Self::Custom(other),
        }
    }
}

/// The sea level of a built-in level (a custom level gets the Overworld's).
pub fn vanilla_sea_level(level: LevelKey) -> i32 {
    match level {
        LevelKey::NETHER => 32,
        LevelKey::END => 0,
        _ => 63,
    }
}

/// The sea level each level's generator builds with (`ChunkGenerator.getSeaLevel`), by level key.
/// Filled when a world opens from the noise settings its data packs replace (`set_sea_levels`),
/// and by each level's generator as it opens (`register_sea_level`).
static SEA_LEVELS: std::sync::LazyLock<RwLock<HashMap<LevelKey, i32>>> =
    std::sync::LazyLock::new(Default::default);

/// Replaces every registered sea level, as a world opens.
pub fn set_sea_levels(levels: Vec<(LevelKey, i32)>) {
    *SEA_LEVELS.write().unwrap_or_else(|e| e.into_inner()) = levels.into_iter().collect();
}

/// Registers one level's sea level: its generator's, or the one a server sent
/// (`CommonPlayerSpawnInfo.seaLevel`).
pub fn register_sea_level(level: LevelKey, sea_level: i32) {
    SEA_LEVELS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(level, sea_level);
}

/// A level's sea level (`Level.getSeaLevel`): the registered one, else `vanilla_sea_level`.
pub fn sea_level_of(level: LevelKey) -> i32 {
    SEA_LEVELS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&level)
        .copied()
        .unwrap_or_else(|| vanilla_sea_level(level))
}

/// Game rules consulted by block behaviour (vanilla defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameRules {
    pub water_source_conversion: bool,
    pub lava_source_conversion: bool,
    pub random_tick_speed: i32,
    pub block_drops: bool,
    pub mob_griefing: bool,
    pub max_snow_accumulation_height: i32,
    pub spawn_mobs: bool,
    pub spawn_monsters: bool,
    /// `fire_spread_radius_around_player`: -1 spreads everywhere.
    pub fire_spread_radius_around_player: i32,
    pub spread_vines: bool,
}
impl Default for GameRules {
    fn default() -> Self {
        Self::from(&crate::gamerules::GameRules::default())
    }
}

impl From<&crate::gamerules::GameRules> for GameRules {
    fn from(rules: &crate::gamerules::GameRules) -> Self {
        Self {
            water_source_conversion: rules.bool("water_source_conversion"),
            lava_source_conversion: rules.bool("lava_source_conversion"),
            random_tick_speed: rules.int("random_tick_speed"),
            block_drops: rules.bool("block_drops"),
            mob_griefing: rules.bool("mob_griefing"),
            max_snow_accumulation_height: rules.int("max_snow_accumulation_height"),
            spawn_mobs: rules.bool("spawn_mobs"),
            spawn_monsters: rules.bool("spawn_monsters"),
            fire_spread_radius_around_player: rules.int("fire_spread_radius_around_player"),
            spread_vines: rules.bool("spread_vines"),
        }
    }
}

/// Entity filters for the entity-sensing blocks (pressure plates, tripwire).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityQuery {
    /// Any entity that does not ignore block triggers (wooden plates, tripwire).
    Everything,
    /// Living entities only (stone plates).
    Mobs,
    /// Minecarts only (detector rails).
    Minecart,
    /// Arrows only (`AbstractArrow`, thrown tridents included): the entities
    /// a wooden button senses.
    Arrows,
}

/// Side effects reported to the caller, in vanilla call order.
#[derive(Clone, Debug, PartialEq)]
pub enum LevelEvent {
    /// `Level.levelEvent(type, pos, data)`, using vanilla's numeric event IDs
    /// (e.g. 2001 block destroyed with `data` = state ID, 1501 lava
    /// extinguished, 1009 fire extinguished, 1502 redstone torch burnout,
    /// 2009 dripping water/sponge, 1033/1034 ... per `LevelEvent.java`).
    LevelEvent { id: i32, pos: BlockPos, data: i32 },
    /// `Level.playSound(null, pos, sound, BLOCKS, volume, pitch)`.
    Sound {
        pos: BlockPos,
        sound: Arc<str>,
        volume: f32,
        pitch: f32,
    },
    /// A placed template's container whose data has a `LootTable`
    /// (`RandomizableContainer.tryLoadLootTable`): the caller keeps the table
    /// and rolls it, as for a container a command gives one.
    ContainerLootTable { pos: BlockPos, tag: crate::nbt::Tag },
    /// `Block.dropResources(state, level, pos)`: the caller evaluates the
    /// block's loot table (no tool, no breaker) and pops the items.
    DropResources { pos: BlockPos, state: BlockStateId },
    /// `FallingBlockEntity.fall`: the block at `pos` was already replaced by its
    /// fluid's legacy block; the caller spawns and simulates a falling-block
    /// entity carrying `state` and calls [`Behaviours::falling_block_landed`]
    /// (or drops the item) when it lands.
    FallingBlock {
        pos: BlockPos,
        state: BlockStateId,
        /// Tip damage configured by `SpeleothemBlock.spawnFallingStalactite`.
        hurt_per_distance: Option<f32>,
    },
    /// A sapling reached stage 1 and tried to grow (`TreeGrower.growTree`).
    /// The caller runs the grower's configured feature and, on success,
    /// replaces the sapling (vanilla leaves the sapling on failure).
    GrowTree {
        pos: BlockPos,
        state: BlockStateId,
        grower: Arc<str>,
    },
    /// A configured feature placement request from a bonemealed or random-ticked
    /// block (huge mushrooms, fungi). The block was already removed when vanilla
    /// does so before placing; on failure the caller restores `state`.
    PlaceFeature {
        pos: BlockPos,
        state: BlockStateId,
        feature: Arc<str>,
    },
    /// `TntBlock.prime`: spawn primed TNT pushed along `rot` (radians, drawn
    /// from the level random by `new PrimedTnt`). The primed sound and game
    /// event follow as their own events; the caller removes the block.
    PrimeTnt { pos: BlockPos, rot: f64 },
    /// Block-entity or entity side effects the caller implements: `"dispense"`
    /// (dispenser/dropper/crafter scheduled tick), `"brushable"` (a suspicious
    /// block's scheduled tick: `checkReset`), `"hatch"` (frogspawn
    /// hatched into tadpoles; the block is already destroyed), `"portal_spawn"`
    /// (a nether portal won its zombified piglin roll; the caller checks
    /// `anyPlayerCloseEnoughForSpawning` and spawns below the portal),
    /// `"golem"` (a carved pumpkin was placed; the caller builds the golem
    /// its blocks complete).
    BlockEvent {
        pos: BlockPos,
        state: BlockStateId,
        kind: &'static str,
    },
    /// A queued block event that reached a block entity
    /// (`BaseEntityBlock.triggerEvent`): the client's copy runs
    /// `BlockEntity.triggerEvent(a, b)` (`ClientboundBlockEventPacket`), as
    /// for a bell's ring (event 1, the struck direction).
    BlockEntityEvent {
        pos: BlockPos,
        state: BlockStateId,
        a: i32,
        b: i32,
    },
    /// A player-side effect of [`Behaviours::use_without_item`].
    Player {
        pos: BlockPos,
        state: BlockStateId,
        effect: PlayerEffect,
    },
    /// `Level.gameEvent` (vibration system), by vanilla identifier, with the
    /// affected block state when the event carries one.
    GameEvent {
        pos: BlockPos,
        event: &'static str,
        state: Option<BlockStateId>,
    },
}

/// Storage/context seen by block behaviour. Implemented by server-owned world
/// storage and tests; it is not itself a complete server runtime.
pub trait LevelAccess {
    /// `WorldBorder.isWithinBounds(BlockPos)` for block movement.
    fn within_world_border(&self, _pos: BlockPos) -> bool {
        true
    }

    fn observe_sponge_absorb(&self, _pos: BlockPos) {}
    fn observe_block_changed(&self, _pos: BlockPos, _old: BlockStateId, _new: BlockStateId) {}
    /// Observe `BlockState.handleNeighborChanged` before the block handles it.
    /// The default does nothing; implementations must not alter the update queue.
    fn observe_neighbor_changed(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _changed: BlockId,
        _moved: bool,
    ) {
    }
    /// `getBlockState`. Unloaded or out-of-bounds positions read as air.
    fn block_state(&self, pos: BlockPos) -> BlockStateId;
    /// Storage write (`LevelChunkSection.setBlockState`). Returns the previous
    /// state, or None when the chunk is unloaded or the position/state cannot be stored.
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId>;
    /// Whether the containing chunk is resident; height bounds are separate.
    fn is_loaded(&self, pos: BlockPos) -> bool;
    /// Inclusive minimum and exclusive maximum build height.
    fn min_y(&self) -> i32 {
        -64
    }
    fn max_y(&self) -> i32 {
        320
    }
    /// `getHeight(WORLD_SURFACE, x, z)`: one above the column's highest
    /// block that isn't air. Only air lies above it, so scans for the other
    /// heightmaps start there. The default, `max_y`, scans the whole column.
    fn world_surface(&self, _x: i32, _z: i32) -> i32 {
        self.max_y()
    }
    /// Whether the world's enabled features contain `minecraft:redstone_experiments`.
    fn redstone_experiments(&self) -> bool {
        false
    }
    /// Vanilla `level.random` (a `LegacyRandomSource`).
    fn random(&mut self) -> &mut LegacyRandom;
    fn game_time(&self) -> i64;
    fn block_light(&self, _pos: BlockPos) -> i32 {
        0
    }
    fn sky_light(&self, _pos: BlockPos) -> i32 {
        15
    }
    /// `Level.getSkyDarken` (0 at noon, up to 11 at night).
    fn sky_darken(&self) -> i32 {
        0
    }
    /// `EnvironmentAttributes.SUN_ANGLE` at `pos`, in degrees (0 at noon).
    fn sun_angle(&self, _pos: BlockPos) -> f32 {
        0.0
    }
    fn is_raining(&self) -> bool {
        false
    }
    fn is_thundering(&self) -> bool {
        false
    }
    fn is_raining_at(&self, _pos: BlockPos) -> bool {
        false
    }
    fn dimension(&self) -> Dimension {
        Dimension::Overworld
    }
    /// The level's key.
    fn level_key(&self) -> LevelKey {
        self.dimension().key()
    }
    /// The level's dimension type.
    fn dimension_type(&self) -> &DimensionFacts {
        DimensionFacts::of_builtin_level(self.level_key())
    }
    /// Complete vanilla game-rule state for this level, including rules not
    /// consulted by block simulation. Server owners may override this to
    /// provide the loaded/persisted level settings.
    fn game_rule_catalog(&self) -> &crate::gamerules::GameRules {
        static DEFAULT_RULES: std::sync::LazyLock<crate::gamerules::GameRules> =
            std::sync::LazyLock::new(crate::gamerules::GameRules::default);
        &DEFAULT_RULES
    }
    /// Compact projection of the catalog for hot simulation paths. The
    /// catalog is the single source of defaults and configured values.
    fn game_rules(&self) -> GameRules {
        GameRules::from(self.game_rule_catalog())
    }
    /// `Difficulty.getId()` (peaceful 0 .. hard 3).
    fn difficulty(&self) -> i32 {
        2
    }
    /// `ServerLevel.canSpreadFireAround` (fire-spread radius around players).
    fn can_spread_fire_around(&self, _pos: BlockPos) -> bool {
        true
    }
    /// The `INCREASED_FIRE_BURNOUT` environment attribute at `pos` (a biome
    /// attribute).
    fn increased_fire_burnout(&self, _pos: BlockPos) -> bool {
        false
    }
    /// `Level.isUnobstructed(state, pos, context)`: whether the state's
    /// collision shape at `pos` would intersect an entity.
    fn is_unobstructed(&self, _pos: BlockPos, _state: BlockStateId) -> bool {
        true
    }
    /// Comparator input from a block entity (`getAnalogOutputSignal` of
    /// containers, jukeboxes, lecterns, ...). State-only sources (cake,
    /// composter, cauldrons, beehives, ...) are evaluated by the library.
    fn block_entity_analog_signal(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        0
    }
    /// `ItemFrame.getAnalogOutput` of the one item frame in the block at
    /// `pos` that faces `facing` (`ComparatorBlock.getItemFrame`), or None when
    /// there is not exactly one.
    fn item_frame_analog_output(&self, _pos: BlockPos, _facing: Direction) -> Option<i32> {
        None
    }
    /// `ContainerOpenersCounter.getEntitiesWithContainerOpen` is not empty for
    /// the container at `pos`: someone has it open (a double chest's menu
    /// counts for both halves; a copper golem's open chest too).
    fn container_open(&self, _pos: BlockPos) -> bool {
        false
    }
    /// `ContainerOpenersCounter.recheckOpenCount` for the chest or ender chest
    /// block entity at `pos`: recounts its openers and returns the previous
    /// and new counts (the caller plays open/close effects when they change),
    /// or None when there is no such block entity.
    fn recheck_container_openers(&mut self, _pos: BlockPos) -> Option<(i32, i32)> {
        None
    }
    /// `CommandBlockEntity.setPowered` for the command block entity at `pos`:
    /// stores `powered` and returns the previous flag and `isAutomatic`, or
    /// None when there is no command block entity.
    fn set_command_block_powered(
        &mut self,
        _pos: BlockPos,
        _powered: bool,
    ) -> Option<(bool, bool)> {
        None
    }
    /// `PotentSulfurBlockEntity.resetCountdown` for the potent sulfur block
    /// entity at `pos` (`PotentSulfurBlock.validBlockState`).
    fn reset_potent_sulfur_countdown(&mut self, _pos: BlockPos) {}
    /// Weak/strong signal of a block-entity-driven source (trapped chest
    /// openers); other sources are evaluated by the library.
    fn block_entity_signal(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        0
    }
    /// Biome answers for a precipitation tick at the column of `pos` (see
    /// [`Precipitation`]); None skips snow/ice formation and cauldron filling.
    fn precipitation(&self, _pos: BlockPos) -> Option<Precipitation> {
        None
    }
    /// Count of entities intersecting `area` that trigger pressure plates or
    /// tripwires under `query`.
    fn count_entities(&self, _area: Aabb, _query: EntityQuery) -> usize {
        0
    }
    /// `ServerLevel.shouldTickBlocksAt`: ticket range, independent of chunk ticking readiness.
    fn should_tick_blocks_at(&self, _chunk: ChunkPos) -> bool {
        true
    }
    /// `ServerLevel.isPositionTickingWithEntitiesLoaded` for scheduled ticks.
    fn is_ticking_chunk(&self, _chunk: ChunkPos) -> bool {
        true
    }
    fn emit(&mut self, _event: LevelEvent) {}
    /// `LevelChunk.setBlockState` replaced a block that had a block entity by
    /// another block: run `BlockEntity.preRemoveSideEffects` when
    /// `side_effects` (containers spill their contents, a jukebox pops its
    /// record, a lectern its book...) and remove the block entity. Called
    /// after the storage write, before `affectNeighborsAfterRemoval`. Returns
    /// how many times the side effects called `updateNeighborsAt(pos, block)`
    /// (a jukebox whose song changed), which the caller then runs.
    fn remove_block_entity(
        &mut self,
        _pos: BlockPos,
        _old: BlockStateId,
        _side_effects: bool,
    ) -> u8 {
        0
    }
    /// Ensure a new block's optional entity after its onPlace completed, as
    /// LevelChunk.setBlockState does. The actual state is supplied; an existing
    /// valid same-block instance must survive. Defaults to no additional factory.
    fn ensure_block_entity(&mut self, _pos: BlockPos, _state: BlockStateId) {}
    /// `ShulkerBoxBlockEntity.isClosed` at `pos`: whether the box's lid is
    /// still shut. None when no shulker box block entity lives there, or the
    /// level keeps none: vanilla then reads the box as closed, a full cube.
    fn shulker_is_closed(&self, _pos: BlockPos) -> Option<bool> {
        None
    }
    /// Load saved block-entity data into the block entity of the block just
    /// placed at `pos` (`BlockEntity.loadWithComponents`): a bee nest a grown
    /// tree placed gets its bees. Levels without block entities drop it.
    fn load_block_entity(&mut self, _pos: BlockPos, _nbt: crate::nbt::Compound) {}
    /// A live dungeon's already selected spawner data. Optional and
    /// creation-only: an owning level must preserve every existing entry.
    fn create_feature_spawner(&mut self, _pos: BlockPos, _nbt: crate::nbt::Compound) {}
    /// Biome identifier at `pos` (`minecraft:plains`), for features placed by
    /// block behaviour (bone meal flowers). None when unknown.
    fn biome_name(&self, _pos: BlockPos) -> Option<&str> {
        None
    }
    /// World seed (`WorldGenLevel.getSeed`) for seed-derived feature noise.
    fn world_seed(&self) -> i64 {
        0
    }
    /// `Level.getDayTime` (the overworld clock driving the day timeline's
    /// environment attributes, e.g. eyeblossoms opening at night).
    fn day_time(&self) -> i64 {
        6000
    }
}

/// First server-owned resident block store backing vanilla level behaviour.
///
/// This owns loaded block columns, the build-height bounds, vanilla random
/// source and the two world clocks. It deliberately does not own persistence,
/// heightmaps, lighting, entities or scheduled ticks; callers integrate those
/// systems at the level lifecycle. Missing resident sections read as air.
pub struct ResidentLevel<'registry> {
    registry: &'registry BlockRegistry,
    air: BlockStateId,
    min_y: i32,
    max_y: i32,
    columns: HashMap<ChunkPos, ResidentColumn<'registry>>,
    next_revision: u128,
    random: LegacyRandom,
    game_time: i64,
    day_time: i64,
    ticking_levels: Levels,
}

struct ResidentColumn<'registry> {
    sections: HashMap<i32, BlockSection<'registry>>,
    dirty: bool,
    revision: u128,
}

impl<'registry> ResidentLevel<'registry> {
    /// Create an empty resident level. Build bounds must align to chunk sections.
    pub fn new(
        registry: &'registry BlockRegistry,
        min_y: i32,
        height: i32,
        seed: i64,
    ) -> io::Result<Self> {
        if height <= 0 || min_y.rem_euclid(16) != 0 || height % 16 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "level build bounds must be positive and section-aligned",
            ));
        }
        let max_y = min_y.checked_add(height).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "level build bounds overflow")
        })?;
        let air = registry.default_state("minecraft:air").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "block registry has no air state",
            )
        })?;
        Ok(Self {
            registry,
            air,
            min_y,
            max_y,
            columns: HashMap::new(),
            next_revision: 1,
            random: LegacyRandom::new(seed),
            game_time: 0,
            day_time: 0,
            ticking_levels: Levels::default(),
        })
    }

    /// Insert a resident column and its already-decoded sections. A duplicate
    /// section, out-of-range section or section from another registry is rejected.
    /// Returns false when the column was already resident and leaves it unchanged.
    pub fn load_chunk(
        &mut self,
        position: ChunkPos,
        sections: impl IntoIterator<Item = (i32, BlockSection<'registry>)>,
    ) -> io::Result<bool> {
        if self.columns.contains_key(&position) {
            return Ok(false);
        }
        let min_section = self.min_y >> 4;
        let max_section = self.max_y >> 4;
        let mut resident = ResidentColumn {
            sections: HashMap::new(),
            dirty: false,
            revision: 0,
        };
        for (section_y, section) in sections {
            if !(min_section..max_section).contains(&section_y) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "section outside level build bounds",
                ));
            }
            if !std::ptr::eq(section.registry(), self.registry) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "section uses a different block registry",
                ));
            }
            if resident.sections.insert(section_y, section).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duplicate resident section Y",
                ));
            }
        }
        resident.revision = self.allocate_revision();
        self.columns.insert(position, resident);
        Ok(true)
    }

    fn allocate_revision(&mut self) -> u128 {
        let revision = self.next_revision;
        self.next_revision = self
            .next_revision
            .checked_add(1)
            .expect("resident level revision exhausted");
        revision
    }

    /// Decode a saved chunk's block-state palettes into resident sections.
    /// All other chunk/section data remains in `ChunkData` for its owner to
    /// preserve and save; loading is transactional on malformed or out-of-range
    /// sections. Returns false without changing the existing column if loaded.
    pub fn load_chunk_data(&mut self, data: &crate::chunk::ChunkData) -> io::Result<bool> {
        let mut sections = Vec::new();
        for (section_y, block_states) in data.section_block_states()? {
            let section = match block_states {
                Some(tag) => BlockSection::from_nbt(tag, self.registry)?,
                None => BlockSection::uniform(self.air, self.registry)?,
            };
            sections.push((section_y, section));
        }
        self.load_chunk(data.position(), sections)
    }

    /// Decode a compressed Anvil record and load its resident block sections.
    pub fn load_chunk_record(
        &mut self,
        record: &crate::region::ChunkRecord,
        limits: crate::nbt::Limits,
    ) -> io::Result<bool> {
        let data = crate::chunk::ChunkData::new(record.nbt(limits)?)?;
        self.load_chunk_data(&data)
    }

    /// Read one chunk from an already-open region file and load its resident
    /// block sections. Returns false when the region has no record for it.
    pub fn load_region_chunk(
        &mut self,
        region: &mut crate::region::RegionFile,
        position: ChunkPos,
        limits: crate::nbt::Limits,
    ) -> io::Result<bool> {
        let Some(record) = region.read(position)? else {
            return Ok(false);
        };
        let data = crate::chunk::ChunkData::new(record.nbt(limits)?)?;
        if data.position() != position {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk coordinates do not match the region slot",
            ));
        }
        self.load_chunk_data(&data)
    }

    /// Write resident block-state palettes back to the matching saved chunk.
    /// The chunk's other NBT, including section biomes, light, heightmaps,
    /// entities and scheduled ticks, is left untouched by `ChunkData`.
    pub fn save_chunk_data(&self, data: &mut crate::chunk::ChunkData) -> io::Result<()> {
        let position = data.position();
        let column = self
            .columns
            .get(&position)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "chunk is not resident"))?;
        let mut updates = column
            .sections
            .iter()
            .map(|(&y, section)| Ok((y, section.to_nbt()?)))
            .collect::<io::Result<Vec<_>>>()?;
        updates.sort_by_key(|(y, _)| *y);
        data.set_section_block_states(updates)
    }

    /// Encode the current resident block palettes into a compressed Anvil
    /// record while retaining the supplied chunk's compression and timestamp.
    pub fn save_chunk_record(
        &self,
        data: &mut crate::chunk::ChunkData,
        compression: crate::compression::Compression,
        timestamp: u32,
        limits: crate::nbt::Limits,
    ) -> io::Result<crate::region::ChunkRecord> {
        self.save_chunk_data(data)?;
        crate::region::ChunkRecord::from_nbt(data.root(), compression, timestamp, limits)
    }

    /// Persist the resident palette state into an already-open region file.
    /// File sync, unload/eviction and dirty-state policy remain the caller's
    /// responsibility so a runtime can batch writes and acknowledge them only
    /// after its chosen durability boundary via [`Self::mark_chunk_saved`].
    pub fn save_region_chunk(
        &self,
        region: &mut crate::region::RegionFile,
        data: &mut crate::chunk::ChunkData,
        compression: crate::compression::Compression,
        timestamp: u32,
        limits: crate::nbt::Limits,
    ) -> io::Result<()> {
        if data.position().region() != region.position() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chunk does not belong to this region",
            ));
        }
        let position = data.position();
        let record = self.save_chunk_record(data, compression, timestamp, limits)?;
        region.write(position, &record)
    }

    /// Add an empty loaded column. Intended for new/generated columns and tests;
    /// persisted columns must be decoded before `load_chunk`.
    pub fn load_empty_chunk(&mut self, position: ChunkPos) -> bool {
        let loaded = self
            .load_chunk(position, std::iter::empty())
            .expect("an empty column has no invalid sections");
        if loaded && let Some(column) = self.columns.get_mut(&position) {
            column.dirty = true;
        }
        loaded
    }

    /// Remove a clean resident column; dirty columns must be saved or explicitly
    /// discarded first. Block access then follows unloaded semantics.
    pub fn unload_chunk(&mut self, position: ChunkPos) -> bool {
        if self
            .columns
            .get(&position)
            .is_some_and(|column| column.dirty)
        {
            return false;
        }
        self.columns.remove(&position).is_some()
    }

    /// Explicitly discard a resident column, including any unsaved block edits.
    pub fn discard_chunk(&mut self, position: ChunkPos) -> bool {
        self.columns.remove(&position).is_some()
    }

    /// Whether this resident column has block changes not yet acknowledged as
    /// persisted. `save_region_chunk` alone does not clear it; the caller should
    /// capture [`Self::chunk_revision`] for the saved snapshot and pass it to
    /// `mark_chunk_saved` only after its durability boundary succeeds.
    pub fn is_chunk_dirty(&self, position: ChunkPos) -> bool {
        self.columns
            .get(&position)
            .is_some_and(|column| column.dirty)
    }

    /// Mark a newly generated or externally replaced resident column dirty.
    /// The revision advances even when its current block palettes are unchanged.
    pub fn mark_chunk_dirty(&mut self, position: ChunkPos) -> bool {
        if !self.columns.contains_key(&position) {
            return false;
        }
        let revision = self.allocate_revision();
        let column = self.columns.get_mut(&position).expect("checked above");
        column.dirty = true;
        column.revision = revision;
        true
    }

    /// Return the currently resident columns with unsaved block changes.
    pub fn dirty_chunks(&self) -> Vec<ChunkPos> {
        let mut chunks = self
            .columns
            .iter()
            .filter_map(|(&position, column)| column.dirty.then_some(position))
            .collect::<Vec<_>>();
        chunks.sort_by_key(|position| (position.x, position.z));
        chunks
    }

    /// Revision token for a resident column's current block data. Capture it
    /// for a save snapshot so an asynchronous save cannot acknowledge later
    /// edits accidentally.
    pub fn chunk_revision(&self, position: ChunkPos) -> Option<u128> {
        self.columns.get(&position).map(|column| column.revision)
    }

    /// A caller using another durable store may acknowledge successful commit
    /// here. Clears dirty only if `revision` still identifies the current
    /// block-data snapshot; returns false if absent or modified since capture.
    pub fn mark_chunk_saved(&mut self, position: ChunkPos, revision: u128) -> bool {
        let Some(column) = self.columns.get_mut(&position) else {
            return false;
        };
        if column.revision != revision {
            return false;
        }
        column.dirty = false;
        true
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.columns.len()
    }

    /// Currently resident chunk positions in stable x/z order.
    pub fn loaded_chunks(&self) -> Vec<ChunkPos> {
        let mut chunks = self.columns.keys().copied().collect::<Vec<_>>();
        chunks.sort_by_key(|position| (position.x, position.z));
        chunks
    }

    pub fn set_times(&mut self, game_time: i64, day_time: i64) {
        self.game_time = game_time;
        self.day_time = day_time;
    }

    pub(crate) fn set_chunk_ticking_levels(&mut self, levels: Levels) {
        self.ticking_levels = levels;
    }
}

impl LevelAccess for ResidentLevel<'_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        if !(self.min_y..self.max_y).contains(&pos.y) || !self.columns.contains_key(&pos.chunk()) {
            return self.air;
        }
        self.columns
            .get(&pos.chunk())
            .and_then(|column| column.sections.get(&(pos.y >> 4)))
            .and_then(|section| section.get(pos.section_index()).ok())
            .unwrap_or(self.air)
    }

    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        if !(self.min_y..self.max_y).contains(&pos.y)
            || !self.is_loaded(pos)
            || self.registry.state(state).is_none()
        {
            return None;
        }
        let old = self.block_state(pos);
        if old == state {
            return Some(old);
        }
        let revision = self.allocate_revision();
        let air = self.air;
        let registry = self.registry;
        let column = self.columns.get_mut(&pos.chunk())?;
        let section = column.sections.entry(pos.y >> 4).or_insert_with(|| {
            BlockSection::uniform(air, registry)
                .expect("the registry's air state was validated at construction")
        });
        let previous = section.set(pos.section_index(), state).ok();
        if previous.is_some() {
            column.dirty = true;
            column.revision = revision;
        }
        previous
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.columns.contains_key(&pos.chunk())
    }

    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn max_y(&self) -> i32 {
        self.max_y
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn game_time(&self) -> i64 {
        self.game_time
    }

    fn day_time(&self) -> i64 {
        self.day_time
    }

    fn should_tick_blocks_at(&self, chunk: ChunkPos) -> bool {
        self.ticking_levels.block_ticking(chunk.x, chunk.z)
    }

    fn is_ticking_chunk(&self, chunk: ChunkPos) -> bool {
        self.ticking_levels.block_ticking(chunk.x, chunk.z)
    }
}

/// Raw brightness as `Level.getRawBrightness(pos, skyDarken)`.
pub fn raw_brightness<L: LevelAccess + ?Sized>(level: &L, pos: BlockPos, sky_darken: i32) -> i32 {
    (level.sky_light(pos) - sky_darken).max(level.block_light(pos))
}

/// Access only the gameplay fields needed by level behaviour tables.
/// Both the JSON fallback and the client's typed decoder implement this.
pub trait GameplayStateSource {
    fn flag(&self, name: &str) -> bool;
    fn unsigned(&self, name: &str) -> u64;
    fn number(&self, name: &str) -> f64;
    fn text(&self, name: &str) -> &str;
    fn sound_number(&self, name: &str) -> f64;
    fn sound_id(&self, name: &str) -> &str;
}

/// Immutable vanilla block-behaviour tables. Build once with [`Behaviours::load`].
pub struct Behaviours {
    pub table: StateTable,
    pub props: Props,
    pub named: Named,
    pub tags: Tags,
    all_tags: HashMap<String, Arc<data::BlockSet>>,
    facts: Vec<StateFacts>,
    shapes: Vec<VoxelShape>,
    shape_info: Vec<data::ShapeInfo>,
    pub fluid_tags: data::FluidTags,
    blocks: Vec<BlockData>,
    items: HashMap<String, ItemBinding>,
    potted_by_content: HashMap<BlockId, BlockId>,
    liquid_blocks: FluidBlockBindings,
    /// Uncached (position- or block-entity-dependent) collision shapes.
    dynamic: HashMap<BlockStateId, crate::dynamic_support::DynamicSupport>,
    occlusion_cache: RwLock<HashMap<(i32, i32, u8), bool>>,
    growth: growth::Growth,
}

impl Behaviours {
    /// Load from a prepared client resource directory (`.local/horizon-client/<version>`).
    pub fn load(registry: &BlockRegistry, resources_dir: &Path) -> io::Result<Self> {
        data::load(registry, resources_dir)
    }
    /// Reuse the client's typed gameplay projection while it is borrowed from the JSON bytes.
    pub fn load_with_gameplay<G: GameplayStateSource>(
        registry: &BlockRegistry,
        resources_dir: &Path,
        behaviour: &serde_json::Value,
        gameplay: &[G],
    ) -> io::Result<Self> {
        data::load_with_gameplay(registry, resources_dir, behaviour, gameplay)
    }

    pub fn load_with_behaviour_json(
        registry: &BlockRegistry,
        resources_dir: &Path,
        behaviour: &serde_json::Value,
    ) -> io::Result<Self> {
        data::load_with_behaviour_json(registry, resources_dir, behaviour)
    }

    /// Keep snapshots of the resolved tables and the block-state registry in
    /// `dir` (a game folder's `.horizon-cache`), so later launches of the same
    /// build restore them instead of resolving the resources again. Without
    /// it, [`Self::load`] and `data::load_registry` never cache.
    pub fn set_cache_dir(dir: std::path::PathBuf) {
        data::set_cache_dir(dir);
    }
    pub fn facts(&self, state: BlockStateId) -> &StateFacts {
        &self.facts[state.0 as usize]
    }
    pub fn block_data(&self, block: BlockId) -> &BlockData {
        &self.blocks[block.0 as usize]
    }
    pub fn data(&self, state: BlockStateId) -> &BlockData {
        self.block_data(self.table.block(state))
    }
    pub fn block(&self, state: BlockStateId) -> BlockId {
        self.table.block(state)
    }
    pub fn item_binding(&self, item: &str) -> Option<&ItemBinding> {
        self.items.get(item)
    }
    /// FlowerPotBlock.POTTED_BY_CONTENT: the potted flower pot block that
    /// holds plant, if any.
    pub fn potted_block(&self, plant: BlockId) -> Option<BlockId> {
        self.potted_by_content.get(&plant).copied()
    }
    pub fn tag(&self, name: &str) -> Option<&data::BlockSet> {
        self.all_tags.get(name).map(|s| &**s)
    }
    pub fn fluid(&self, state: BlockStateId) -> FluidState {
        self.facts(state).fluid
    }
    /// `FluidState.createLegacyBlock`.
    pub fn legacy_block(&self, fluid: FluidState) -> BlockStateId {
        self.liquid_blocks.block(fluid)
    }
    /// Cached collision shape (`getCollisionShape(level, pos)` for states whose
    /// shape does not depend on the world); None for dynamic shapes.
    pub fn collision_shape(&self, state: BlockStateId) -> Option<&VoxelShape> {
        let index = self.facts(state).shape;
        (index >= 0).then(|| &self.shapes[index as usize])
    }
    /// Precomputed shape queries; dynamic shapes read as empty.
    pub fn shape_info(&self, state: BlockStateId) -> data::ShapeInfo {
        let index = self.facts(state).shape;
        if index < 0 {
            data::ShapeInfo::default()
        } else {
            self.shape_info[index as usize]
        }
    }
    pub(crate) fn has_flag(&self, state: BlockStateId, flag: u32) -> bool {
        self.facts(state).flags & flag != 0
    }
    pub fn is_air(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::AIR)
    }
    pub fn is_solid(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::SOLID)
    }
    pub fn is_replaceable(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::REPLACEABLE)
    }
    pub fn is_randomly_ticking(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::RANDOM_TICKS)
    }
    /// `isFaceSturdy(level, pos, direction, FULL)` from the empty-level cache.
    pub fn is_face_sturdy(&self, state: BlockStateId, direction: Direction) -> bool {
        self.facts(state).sturdy & (1 << direction as u32) != 0
    }
    pub fn is_face_sturdy_center(&self, state: BlockStateId, direction: Direction) -> bool {
        self.facts(state).sturdy & (1 << (6 + direction as u32)) != 0
    }
    pub fn is_face_sturdy_rigid(&self, state: BlockStateId, direction: Direction) -> bool {
        self.facts(state).sturdy & (1 << (12 + direction as u32)) != 0
    }
    /// Tick type ID of a fluid for [`Ticks::schedule_fluid`].
    pub fn fluid_kind(&self, state: BlockStateId) -> FluidKind {
        self.fluid(state).kind()
    }
}

/// Level mutation entry points (all keep vanilla update semantics).
impl Behaviours {
    /// `Level.setBlock(pos, state, flags)` with vanilla's update limit (512).
    /// Returns false when nothing was written (unloaded, out of bounds, or
    /// the state was already present).
    pub fn set_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        self.sim(level, ticks).set_block(pos, state, flags)
    }
    /// `CopperChestBlock.getFromCopperBlock(copperBlock, facing, level, pos)`.
    pub fn copper_chest_from_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        copper: BlockStateId,
        facing: Direction,
        pos: BlockPos,
    ) -> Option<BlockStateId> {
        self.sim(level, ticks)
            .copper_chest_from_block(copper, facing, pos)
    }
    /// `Level.removeBlock(pos, movedByPiston)`: replace with the fluid's block.
    pub fn remove_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        moved_by_piston: bool,
    ) -> bool {
        self.sim(level, ticks).remove_block(pos, moved_by_piston)
    }
    /// `Level.destroyBlock(pos, dropResources)`: destroy particles (2001),
    /// optional drops, and replacement by the block's fluid.
    pub fn destroy_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        drop: bool,
    ) -> bool {
        self.sim(level, ticks)
            .destroy_block(pos, drop, flags::UPDATE_LIMIT)
    }
    /// A player breaking a block (`playerWillDestroy` + `Level.destroyBlock`
    /// without drops; the caller handles the player's loot/tool). Double
    /// blocks (doors, tall plants, beds) remove their other half as vanilla.
    pub fn player_destroy_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        creative: bool,
        correct_tool: bool,
    ) -> bool {
        self.sim(level, ticks)
            .player_destroy(pos, creative, correct_tool)
    }
    /// `IceBlock.playerDestroy` after the drops: mined ice melts into water
    /// unless the tool prevents ice melting (silk touch). `destroyed` is the
    /// state that was broken.
    pub fn ice_destroyed_by_player<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        destroyed: BlockStateId,
        prevents_melting: bool,
    ) {
        self.sim(level, ticks)
            .ice_destroyed_by_player(pos, destroyed, prevents_melting);
    }
    /// `TurtleEggBlock.playerDestroy` after the drops: a mined stack of eggs
    /// loses one egg and the rest stay. `destroyed` is the state that was broken.
    pub fn turtle_egg_destroyed_by_player<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        destroyed: BlockStateId,
    ) {
        self.sim(level, ticks)
            .turtle_egg_destroyed_by_player(pos, destroyed);
    }
    /// `BaseFireBlock.getState(level, pos)`: soul fire on its base blocks,
    /// otherwise fire shaped by its neighbours.
    pub fn fire_state_for<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> BlockStateId {
        self.sim(level, ticks).fire_state_for(pos)
    }
    /// `BaseFireBlock.canBePlacedAt(level, pos, forwardDirection)`.
    pub fn fire_can_be_placed_at<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        forward: horizon_core::geometry::Direction,
    ) -> bool {
        self.sim(level, ticks).fire_can_be_placed_at(pos, forward)
    }
    /// `PortalShape.findAnyShape`: the frame rectangle around `pos` on one
    /// axis, valid or not.
    pub fn portal_shape<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        x_axis: bool,
    ) -> portal::PortalShape {
        self.sim(level, ticks).find_portal_shape(pos, x_axis)
    }
    /// `PortalShape.findEmptyPortalShape`: a valid frame with no portal block,
    /// trying the preferred axis first.
    pub fn empty_portal_shape<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        x_axis: bool,
    ) -> Option<portal::PortalShape> {
        self.sim(level, ticks).find_empty_portal_shape(pos, x_axis)
    }
    /// `PortalShape.createPortalBlocks`: fill the frame with portal blocks.
    pub fn create_portal_blocks<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        shape: &portal::PortalShape,
    ) {
        self.sim(level, ticks).create_portal_blocks(shape);
    }
    /// `BlockState.canSurvive(level, pos)`.
    pub fn can_survive<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) -> bool {
        self.sim(level, ticks).can_survive(state, pos)
    }
    /// `BlockState.updateShape(level, ticks, pos, direction, neighbourPos,
    /// neighbourState, random)`: the state `state` at `pos` would take after its
    /// neighbour in `direction` became `neighbour` (ticks it requests are queued).
    #[allow(clippy::too_many_arguments)]
    pub fn update_shape<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
    ) -> BlockStateId {
        self.sim(level, ticks)
            .update_shape(state, pos, direction, neighbour_pos, neighbour)
    }
    /// `Block.updateFromNeighbourShapes`: a state adjusted to its neighbours.
    pub fn update_from_neighbour_shapes<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) -> BlockStateId {
        self.sim(level, ticks)
            .update_from_neighbour_shapes(state, pos)
    }
    /// `ServerLevel.updateNeighboursOnBlockSet`: what `/setblock` and `/fill` do for a block once it is in
    /// and its shape updates are done. They place it with `UPDATE_CLIENTS` and not `UPDATE_NEIGHBORS`, so
    /// `setBlock` notified no one: the replaced block's `affectNeighborsAfterRemoval` (when the block
    /// changed), then the neighbours' notice with the new block as its source, then the comparators' of a
    /// block with an analog output.
    pub fn update_neighbours_on_block_set<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        old: BlockStateId,
    ) {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        let block = sim.block_of(state);
        if sim.block_of(old) != block {
            sim.affect_neighbors_after_removal(old, pos, false);
        }
        sim.update_neighbors_at(pos, block);
        if sim.has_analog_output(state) {
            sim.update_neighbour_for_output_signal(pos, block);
        }
    }
    /// `ShulkerBoxBlockEntity.doNeighborUpdates`: the box's collision shape
    /// changes with its lid, so the neighbours' shapes are updated (flag 3)
    /// and then their `neighborChanged` runs, at the four points in `tick`
    /// where the lid starts or finishes opening or closing.
    pub fn shulker_neighbor_updates<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        let mut sim = self.sim(level, ticks);
        sim.update_neighbour_shapes(state, pos, 3, flags::UPDATE_LIMIT);
        let block = sim.block_of(state);
        sim.update_neighbors_at(pos, block);
    }
    /// `Level.updateNeighborsAt(pos, block)`: notify the six neighbours.
    pub fn update_neighbors_at<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        block: BlockId,
    ) {
        self.sim(level, ticks).update_neighbors_at(pos, block);
    }
    /// `LecternBlock.updateBelow` after a page or book changes.
    pub fn lectern_update_below<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        self.sim(level, ticks).lectern_update_below(pos, state);
    }
    /// `TrappedChestBlockEntity.signalOpenCount` after its opener count changes.
    pub fn trapped_chest_update_neighbors<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        self.sim(level, ticks)
            .trapped_chest_update_neighbors(pos, state);
    }
    /// `LightningRodBlock.updateNeighbours` after a strike.
    pub fn lightning_rod_update_neighbors<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        self.sim(level, ticks)
            .lightning_rod_update_neighbors(pos, state);
    }
    /// `Level.updateNeighbourForOutputSignal(pos, block)` for the block at
    /// `pos`: comparators beside it (or behind a conductor there) read their
    /// input again.
    pub fn update_neighbour_for_output_signal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        let mut sim = self.sim(level, ticks);
        let block = self.block(sim.state(pos));
        sim.update_neighbour_for_output_signal(pos, block);
    }
    /// `Level.scheduleTick(pos, block, delay)` for the block at `pos`.
    pub fn schedule_tick<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        delay: i32,
    ) {
        let mut sim = self.sim(level, ticks);
        let block = self.block(sim.state(pos));
        sim.schedule_tick(pos, block, delay);
    }
    /// `BlockState.handleNeighborChanged(level, pos, changedBlock, orientation, movedByPiston)`
    /// for the state currently at `pos`.
    pub fn handle_neighbor_changed<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        changed: BlockId,
    ) {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        sim.neighbor_changed(state, pos, changed, false);
    }
    /// Entity collision with a block (`entityInside`) for the entity-sensing
    /// blocks: pressure plates, tripwires and buttons hit by arrows are
    /// re-evaluated through [`LevelAccess::count_entities`].
    pub fn entity_inside<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        self.sim(level, ticks).entity_inside(pos);
    }
    /// Whether [`Self::entity_inside`] may do anything at a block of `state`:
    /// the entity-sensing blocks' classes it looks for. At any other block it
    /// only reads the state.
    pub fn senses_entities(&self, state: BlockStateId) -> bool {
        let data = self.data(state);
        data.is(classes::Class::BasePressurePlate)
            || data.is(classes::Class::DetectorRail)
            || data.is(classes::Class::Button)
            || data.is(classes::Class::TripWire)
    }
    /// `LilyPadBlock.entityInside` for a boat whose box is inside `pos`.
    pub fn boat_inside<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        self.sim(level, ticks).boat_inside(pos);
    }
    /// `DaylightDetectorBlock.tickEntity` of the daylight detector at `pos`:
    /// its signal follows the sky every 20 game ticks. The detector's ticker
    /// exists only where the dimension has sky light.
    pub fn tick_daylight_detector<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        if !level.dimension_type().height.has_skylight || level.game_time() % 20 != 0 {
            return;
        }
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        if sim.b.data(state).is(classes::Class::DaylightDetector) {
            sim.daylight_detector_update_signal(pos, state);
        }
    }
    /// `BigDripleafBlock.entityInside` for an entity whose box is inside
    /// `pos`: whether it is on the ground and its feet's y (`canEntityTilt`).
    pub fn entity_on_big_dripleaf<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        on_ground: bool,
        feet_y: f64,
    ) {
        self.sim(level, ticks)
            .big_dripleaf_entity_inside(pos, on_ground, feet_y);
    }
    /// `BigDripleafBlock.onProjectileHit`: true when a projectile hit a leaf.
    /// Unlike entityInside, this tilts powered leaves too.
    pub fn big_dripleaf_projectile_hit<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> bool {
        self.sim(level, ticks).big_dripleaf_projectile_hit(pos)
    }
    /// `BellBlock.onProjectileHit` for a projectile that hit `face` of the
    /// block at `pos` at height `hit_y`. False when the block is not a bell
    /// or the hit missed the bell proper.
    pub fn bell_projectile_hit<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        face: Direction,
        hit_y: f64,
    ) -> bool {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        self.data(state).is(classes::Class::Bell)
            && sim.bell_hit(pos, state, face, hit_y - f64::from(pos.y))
    }
    /// `BellBlock.attemptToRing(entity, level, pos, null)`: a mob rings the
    /// bell from its facing. False when the block at `pos` is not a bell.
    pub fn bell_attempt_to_ring<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> bool {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        if !self.data(state).is(classes::Class::Bell) {
            return false;
        }
        let facing = sim.b.facing(state);
        sim.bell_ring(pos, state, facing);
        true
    }
    /// Redstone signal queries (`SignalGetter`).
    pub fn has_neighbor_signal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> bool {
        self.sim(level, ticks).has_neighbor_signal(pos)
    }
    pub fn best_neighbor_signal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> i32 {
        self.sim(level, ticks).best_neighbor_signal(pos)
    }
    /// `BlockState.onProjectileHit` of a target block (`TargetBlock`) hit by
    /// a projectile at `location` on the face with normal `face`; `arrow`
    /// for an `AbstractArrow`. Returns the output strength, or None when the
    /// block at `pos` is not a target block.
    pub fn target_projectile_hit<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        location: [f64; 3],
        face: [i32; 3],
        arrow: bool,
    ) -> Option<i32> {
        self.sim(level, ticks)
            .target_projectile_hit(pos, location, face, arrow)
    }
}

/// One mutable view used by the ported behaviour code.
pub(crate) struct Sim<'a, L: LevelAccess + ?Sized> {
    pub b: &'a Behaviours,
    pub level: &'a mut L,
    pub ticks: &'a mut Ticks,
    /// `RedstoneWireBlock.shouldSignal` is false while a wire samples its
    /// neighbours (`getBlockSignal`).
    pub wire_signal_off: std::cell::Cell<bool>,
}
impl Behaviours {
    pub(crate) fn sim<'a, L: LevelAccess + ?Sized>(
        &'a self,
        level: &'a mut L,
        ticks: &'a mut Ticks,
    ) -> Sim<'a, L> {
        Sim {
            b: self,
            level,
            ticks,
            wire_signal_off: std::cell::Cell::new(false),
        }
    }
}

#[cfg(test)]
mod dimension_key_tests {
    use super::*;

    /// Tests of the global sea level table take turns, and use their own custom keys.
    static SEA_TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn an_unregistered_level_has_the_vanilla_sea_level() {
        let _turn = SEA_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        let unknown = LevelKey::parse("test:sea_unregistered").unwrap();
        assert_eq!(sea_level_of(unknown), 63);
        assert_eq!(sea_level_of(LevelKey::NETHER), 32);
        assert_eq!(sea_level_of(LevelKey::END), 0);
    }

    #[test]
    fn a_registered_sea_level_wins_until_the_table_is_set_again() {
        let _turn = SEA_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        let flat = LevelKey::parse("test:sea_flat").unwrap();
        let tall = LevelKey::parse("test:sea_tall").unwrap();
        set_sea_levels(vec![(tall, 120)]);
        register_sea_level(flat, -63);
        assert_eq!((sea_level_of(flat), sea_level_of(tall)), (-63, 120));
        register_sea_level(flat, 5);
        assert_eq!(sea_level_of(flat), 5);
        set_sea_levels(Vec::new());
        assert_eq!((sea_level_of(flat), sea_level_of(tall)), (63, 63));
    }

    #[test]
    fn dimension_and_key_round_trip_and_sea_levels() {
        for (dim, key, sea) in [
            (Dimension::Overworld, LevelKey::OVERWORLD, 63),
            (Dimension::Nether, LevelKey::NETHER, 32),
            (Dimension::End, LevelKey::END, 0),
        ] {
            assert_eq!(dim.key(), key);
            assert_eq!(Dimension::of_level(key), dim);
            assert_eq!(vanilla_sea_level(key), sea);
        }
        let custom = LevelKey::parse("example:sky").unwrap();
        assert_eq!(Dimension::of_level(custom), Dimension::Custom(custom));
        assert_eq!(vanilla_sea_level(custom), 63);
    }
}

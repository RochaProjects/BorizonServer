//! Behaviour tests over an in-memory level. They need the prepared client
//! resources (`HORIZON_RESOURCES`, or `.local/horizon-client/<version>` at the
//! repository root) and are skipped when those are absent.
use super::*;
use crate::chunk::{BlockProperties, BlockSection, BlockState};
use crate::nbt::{NamedTag, Tag};
use horizon_core::{
    random::RandomSource,
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::OnceLock,
};
struct Fixture {
    registry: BlockRegistry,
    behaviours: Behaviours,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let root = std::env::var_os("HORIZON_RESOURCES")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../.local/horizon-client")
                        .join(horizon_core::VANILLA_VERSION)
                });
            if !root.join("block-behaviours.json").exists() {
                eprintln!("skipping level tests: no resources at {}", root.display());
                return None;
            }
            let registry = data::load_registry(&root).expect("registry");
            let behaviours = Behaviours::load(&registry, &root).expect("behaviours");
            Some(Fixture {
                registry,
                behaviours,
            })
        })
        .as_ref()
}

pub(crate) struct TestLevel {
    blocks: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    pub time: i64,
    pub day_time: i64,
    pub events: Vec<LevelEvent>,
    pub block_light: HashMap<BlockPos, i32>,
    /// `getBrightness(SKY, pos)` everywhere; `None` is 15, full daylight.
    pub sky_light: Option<i32>,
    pub tick_ready: bool,
    pub block_events_ready: bool,
    /// What `recheck_container_openers` answers at a position: the opener
    /// count before and after the recount.
    pub openers: HashMap<BlockPos, (i32, i32)>,
    /// Every position whose container openers were recounted, in order.
    pub opener_rechecks: Vec<BlockPos>,
    /// `ShulkerBoxBlockEntity.isClosed` at a position: `true` for a shut lid,
    /// `false` for an open box. Positions not listed have no block entity.
    pub shulker_closed: HashMap<BlockPos, bool>,
    /// The containers `container_open` reports as open.
    pub opened: HashSet<BlockPos>,
    /// Players standing in every plate and tripwire, by UUID.
    pub occupants: Vec<u128>,
}
impl TestLevel {
    fn new(b: &Behaviours) -> Self {
        Self {
            blocks: HashMap::new(),
            air: b.air(),
            random: LegacyRandom::new(42),
            time: 0,
            day_time: 0,
            events: Vec::new(),
            block_light: HashMap::new(),
            sky_light: None,
            tick_ready: true,
            block_events_ready: true,
            openers: HashMap::new(),
            opener_rechecks: Vec::new(),
            shulker_closed: HashMap::new(),
            opened: HashSet::new(),
            occupants: Vec::new(),
        }
    }
}
impl LevelAccess for TestLevel {
    fn count_entities(&self, _area: Aabb, _query: EntityQuery) -> usize {
        self.occupants.len()
    }
    fn should_tick_blocks_at(&self, _chunk: ChunkPos) -> bool {
        self.block_events_ready
    }
    fn is_ticking_chunk(&self, _chunk: ChunkPos) -> bool {
        self.tick_ready
    }
    fn container_open(&self, pos: BlockPos) -> bool {
        self.opened.contains(&pos)
    }
    fn recheck_container_openers(&mut self, pos: BlockPos) -> Option<(i32, i32)> {
        self.opener_rechecks.push(pos);
        self.openers.get(&pos).copied()
    }
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        self.shulker_closed.get(&pos).copied()
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.blocks.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        Some(self.blocks.insert(pos, state).unwrap_or(self.air))
    }
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
    fn day_time(&self) -> i64 {
        self.day_time
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        self.block_light.get(&pos).copied().unwrap_or(0)
    }
    fn sky_light(&self, _pos: BlockPos) -> i32 {
        self.sky_light.unwrap_or(15)
    }
    fn emit(&mut self, event: LevelEvent) {
        self.events.push(event);
    }
}

struct World<'f> {
    f: &'f Fixture,
    level: TestLevel,
    ticks: Ticks,
}
impl<'f> World<'f> {
    fn new(f: &'f Fixture) -> Self {
        let mut ticks = Ticks::new(0);
        for x in -4..4 {
            for z in -4..4 {
                ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
            }
        }
        Self {
            level: TestLevel::new(&f.behaviours),
            f,
            ticks,
        }
    }
    fn b(&self) -> &'f Behaviours {
        &self.f.behaviours
    }
    fn id(&self, name: &str, props: &[(&str, &str)]) -> BlockStateId {
        let state = BlockState {
            name: format!("minecraft:{name}"),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        self.f
            .registry
            .resolve(&state)
            .unwrap_or_else(|_| panic!("unknown state {name} {props:?}"))
    }
    fn set(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        self.f.behaviours.set_block(
            &mut self.level,
            &mut self.ticks,
            pos,
            state,
            flags::UPDATE_ALL,
        )
    }
    /// Raw storage write without updates (world generation).
    fn put(&mut self, pos: BlockPos, state: BlockStateId) {
        self.level.blocks.insert(pos, state);
    }
    fn get(&self, pos: BlockPos) -> BlockStateId {
        self.level.block_state(pos)
    }
    fn name(&self, pos: BlockPos) -> String {
        let state = self.f.registry.state(self.get(pos)).unwrap();
        let props: Vec<String> = state
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!(
            "{}[{}]",
            state.name.trim_start_matches("minecraft:"),
            props.join(",")
        )
    }
    fn prop(&self, pos: BlockPos, key: &str) -> String {
        self.f
            .registry
            .state(self.get(pos))
            .unwrap()
            .properties
            .get(key)
            .cloned()
            .unwrap_or_default()
    }
    fn tick(&mut self) {
        self.level.time += 1;
        self.f
            .behaviours
            .tick_scheduled(&mut self.level, &mut self.ticks);
    }
    /// Scheduled ticks, block events and block entities, as `ServerLevel.tick`.
    fn full_tick(&mut self) {
        self.tick();
        self.f
            .behaviours
            .run_block_events(&mut self.level, &mut self.ticks);
        self.f
            .behaviours
            .tick_block_entities(&mut self.level, &mut self.ticks);
    }
    fn floor(&mut self, y: i32, radius: i32, block: &str) {
        let state = self.id(block, &[]);
        for x in -radius..=radius {
            for z in -radius..=radius {
                self.put(BlockPos::new(x, y, z), state);
            }
        }
    }
}

macro_rules! world {
    () => {{
        let Some(f) = fixture() else {
            return;
        };
        World::new(f)
    }};
}

#[test]
fn block_observation_follows_successful_commits_only() {
    use std::cell::RefCell;

    #[derive(Debug, PartialEq)]
    enum Observation {
        Block(BlockPos, BlockStateId, BlockStateId),
        Neighbor,
    }
    struct Observer {
        inner: TestLevel,
        loaded: bool,
        seen: RefCell<Vec<Observation>>,
    }
    impl LevelAccess for Observer {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, _pos: BlockPos) -> bool {
            self.loaded
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn observe_block_changed(&self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
            assert_eq!(self.block_state(pos), new);
            self.seen
                .borrow_mut()
                .push(Observation::Block(pos, old, new));
        }
        fn observe_neighbor_changed(&self, _: BlockPos, _: BlockStateId, _: BlockId, _: bool) {
            self.seen.borrow_mut().push(Observation::Neighbor);
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let mut level = Observer {
        inner: TestLevel::new(b),
        loaded: true,
        seen: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    let pos = BlockPos::new(0, 80, 0);
    let stone = f.registry.default_state("minecraft:stone").unwrap();
    assert!(b.set_block(&mut level, &mut ticks, pos, stone, flags::UPDATE_ALL));
    let seen = level.seen.borrow();
    assert_eq!(seen[0], Observation::Block(pos, b.air(), stone));
    assert!(seen[1..].contains(&Observation::Neighbor));
    drop(seen);
    level.seen.borrow_mut().clear();
    assert!(!b.set_block(&mut level, &mut ticks, pos, stone, flags::UPDATE_ALL));
    level.loaded = false;
    assert!(!b.set_block(&mut level, &mut ticks, pos, b.air(), flags::UPDATE_ALL));
    level.loaded = true;
    assert!(!b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(0, 320, 0),
        stone,
        flags::UPDATE_ALL
    ));
    assert!(level.seen.borrow().is_empty());
    assert!(b.set_block(&mut level, &mut ticks, pos, b.air(), flags::UPDATE_ALL));
    assert_eq!(
        level.seen.borrow()[0],
        Observation::Block(pos, stone, b.air())
    );
}

fn tiny_block_registry() -> BlockRegistry {
    let state = |name: &str| BlockState {
        name: name.to_owned(),
        properties: BlockProperties::new(),
    };
    let root = Tag::compound([
        ("DataVersion", Tag::Int(crate::DATA_VERSION)),
        (
            "states",
            Tag::List(vec![
                state("minecraft:air").to_nbt(),
                state("minecraft:stone").to_nbt(),
            ]),
        ),
        (
            "defaults",
            Tag::compound([
                ("minecraft:air", Tag::Int(0)),
                ("minecraft:stone", Tag::Int(1)),
            ]),
        ),
    ]);
    BlockRegistry::from_nbt(&root).unwrap()
}

#[test]
fn resident_level_reads_writes_and_unloads_loaded_columns() {
    let registry = tiny_block_registry();
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    let pos = BlockPos::new(-1, -64, -1);

    assert_eq!(level.block_state(pos), air);
    assert!(!level.is_loaded(pos));
    assert_eq!(level.set_raw(pos, stone), None);
    assert!(level.load_empty_chunk(pos.chunk()));
    assert!(!level.load_empty_chunk(pos.chunk()));
    assert!(level.is_loaded(pos));
    assert_eq!(level.block_state(pos), air);
    assert!(level.is_chunk_dirty(pos.chunk()));
    assert_eq!(level.set_raw(pos, stone), Some(air));
    assert_eq!(level.block_state(pos), stone);
    assert_eq!(level.set_raw(pos, air), Some(stone));

    let above = BlockPos::new(-1, 320, -1);
    assert!(level.is_loaded(above));
    assert_eq!(level.block_state(above), air);
    assert_eq!(level.set_raw(above, stone), None);
    assert_eq!(level.loaded_chunk_count(), 1);
    assert_eq!(level.dirty_chunks(), vec![pos.chunk()]);
    assert!(!level.unload_chunk(pos.chunk()));
    assert!(level.discard_chunk(pos.chunk()));
    assert!(!level.is_loaded(pos));
    assert_eq!(level.block_state(pos), air);
    assert_eq!(level.set_raw(pos, stone), None);
}

#[test]
fn resident_level_accepts_decoded_sections_and_keeps_world_clocks() {
    let registry = tiny_block_registry();
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let mut section = BlockSection::uniform(air, &registry).unwrap();
    let pos = BlockPos::new(0, -64, 0);
    section.set(pos.section_index(), stone).unwrap();

    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    assert!(
        level
            .load_chunk(pos.chunk(), [(pos.y >> 4, section)])
            .unwrap()
    );
    assert_eq!(level.block_state(pos), stone);
    level.set_times(120, 2400);
    assert_eq!(level.game_time(), 120);
    assert_eq!(level.day_time(), 2400);
}

fn saved_test_chunk(
    position: ChunkPos,
    section_y: i8,
    state: &BlockState,
) -> crate::chunk::ChunkData {
    let section = Tag::compound([
        (
            "block_states",
            Tag::compound([("palette", Tag::List(vec![state.to_nbt()]))]),
        ),
        ("biomes", Tag::String("opaque-biome-payload".into())),
        ("Y", Tag::Byte(section_y)),
    ]);
    crate::chunk::ChunkData::new(NamedTag {
        name: Default::default(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("xPos", Tag::Int(position.x)),
            ("zPos", Tag::Int(position.z)),
            ("sections", Tag::List(vec![section])),
            (
                "entities",
                Tag::List(vec![Tag::compound([(
                    "id",
                    Tag::String("test:entity".into()),
                )])]),
            ),
            ("unknown_root_field", Tag::Long(77)),
        ]),
    })
    .unwrap()
}

#[test]
fn resident_level_loads_and_saves_chunk_palettes_preserving_other_nbt() {
    let registry = tiny_block_registry();
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let chunk = ChunkPos::new(-2, 3);
    let mut data = saved_test_chunk(
        chunk,
        -4,
        &BlockState {
            name: "minecraft:air".to_owned(),
            properties: BlockProperties::new(),
        },
    );
    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    assert!(level.load_chunk_data(&data).unwrap());
    assert!(!level.load_chunk_data(&data).unwrap());
    assert!(!level.is_chunk_dirty(chunk));
    let block = BlockPos::new(-32, -64, 48);
    assert_eq!(level.block_state(block), air);
    assert_eq!(level.set_raw(block, stone), Some(air));
    assert!(level.is_chunk_dirty(chunk));
    level.save_chunk_data(&mut data).unwrap();
    assert!(level.is_chunk_dirty(chunk));

    assert_eq!(
        data.block(block).unwrap(),
        Some(BlockState {
            name: "minecraft:stone".to_owned(),
            properties: BlockProperties::new(),
        })
    );
    let root = &data.root().tag;
    assert_eq!(root.get("unknown_root_field"), Some(&Tag::Long(77)));
    assert_eq!(
        root.get("entities"),
        Some(&Tag::List(vec![Tag::compound([(
            "id",
            Tag::String("test:entity".into())
        )])]))
    );
    let Tag::List(sections) = root.get("sections").unwrap() else {
        panic!("sections")
    };
    assert_eq!(
        sections[0].get("biomes"),
        Some(&Tag::String("opaque-biome-payload".into()))
    );

    let region_dir = std::env::temp_dir().join(format!(
        "horizon-resident-level-region-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&region_dir);
    std::fs::create_dir_all(&region_dir).unwrap();
    let region_path = region_dir.join("r.-1.0.mca");
    let save_revision = level.chunk_revision(chunk).unwrap();
    {
        let mut region = crate::region::RegionFile::open_writable(&region_path, true).unwrap();
        level
            .save_region_chunk(
                &mut region,
                &mut data,
                crate::compression::Compression::Zlib,
                123,
                crate::nbt::Limits::saved(),
            )
            .unwrap();
        assert!(level.is_chunk_dirty(chunk));
        region.sync().unwrap();
    }
    assert!(level.mark_chunk_saved(chunk, save_revision));
    assert!(!level.is_chunk_dirty(chunk));
    {
        let mut region = crate::region::RegionFile::open(&region_path).unwrap();
        assert_eq!(region.read(chunk).unwrap().unwrap().timestamp, 123);
    }
    {
        let mut region = crate::region::RegionFile::open(&region_path).unwrap();
        let mut reloaded = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
        assert!(
            reloaded
                .load_region_chunk(&mut region, chunk, crate::nbt::Limits::saved())
                .unwrap()
        );
        assert_eq!(reloaded.block_state(block), stone);
    }
    std::fs::remove_dir_all(region_dir).unwrap();
}

#[test]
fn resident_level_rejects_region_chunk_coordinate_mismatch() {
    let registry = tiny_block_registry();
    let requested = ChunkPos::new(0, 0);
    let embedded = ChunkPos::new(1, 0);
    let data = saved_test_chunk(
        embedded,
        -4,
        &BlockState {
            name: "minecraft:air".to_owned(),
            properties: BlockProperties::new(),
        },
    );
    let record = crate::region::ChunkRecord::from_nbt(
        data.root(),
        crate::compression::Compression::Zlib,
        1,
        crate::nbt::Limits::saved(),
    )
    .unwrap();
    let region_dir = std::env::temp_dir().join(format!(
        "horizon-resident-level-mismatch-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&region_dir);
    std::fs::create_dir_all(&region_dir).unwrap();
    let region_path = region_dir.join("r.0.0.mca");
    {
        let mut region = crate::region::RegionFile::open_writable(&region_path, true).unwrap();
        region.write(requested, &record).unwrap();
        region.sync().unwrap();
    }

    let mut region = crate::region::RegionFile::open(&region_path).unwrap();
    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    let error = level
        .load_region_chunk(&mut region, requested, crate::nbt::Limits::saved())
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(level.loaded_chunk_count(), 0);
    assert!(!level.is_loaded(BlockPos::new(16, -64, 0)));
    drop(region);
    std::fs::remove_dir_all(region_dir).unwrap();
}

#[test]
fn resident_level_does_not_acknowledge_a_stale_save_revision() {
    let registry = tiny_block_registry();
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let chunk = ChunkPos::new(0, 0);
    let block = BlockPos::new(0, -64, 0);
    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    assert!(level.load_empty_chunk(chunk));
    assert_eq!(level.set_raw(block, stone), Some(air));
    let saved_revision = level.chunk_revision(chunk).unwrap();
    assert_eq!(level.set_raw(block, air), Some(stone));

    assert!(!level.mark_chunk_saved(chunk, saved_revision));
    assert!(level.is_chunk_dirty(chunk));
    let current_revision = level.chunk_revision(chunk).unwrap();
    assert!(level.mark_chunk_saved(chunk, current_revision));
    assert!(!level.is_chunk_dirty(chunk));
}

#[test]
fn resident_level_revision_tokens_do_not_repeat_after_unload_and_reload() {
    let registry = tiny_block_registry();
    let chunk = ChunkPos::new(0, 0);
    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    assert!(level.load_empty_chunk(chunk));
    let old_revision = level.chunk_revision(chunk).unwrap();
    assert!(level.discard_chunk(chunk));
    assert!(level.load_empty_chunk(chunk));
    let new_revision = level.chunk_revision(chunk).unwrap();

    assert_ne!(old_revision, new_revision);
    assert!(!level.mark_chunk_saved(chunk, old_revision));
    assert!(level.is_chunk_dirty(chunk));
}

#[test]
fn resident_level_rejects_saved_sections_outside_bounds_without_loading_column() {
    let registry = tiny_block_registry();
    let air = BlockState {
        name: "minecraft:air".to_owned(),
        properties: BlockProperties::new(),
    };
    let chunk = ChunkPos::new(0, 0);
    let data = saved_test_chunk(chunk, 20, &air);
    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    assert!(level.load_chunk_data(&data).is_err());
    assert_eq!(level.loaded_chunk_count(), 0);
}

#[test]
fn scheduled_tick_owner_loads_and_atomically_saves_chunk_tick_lists() {
    let chunk = ChunkPos::new(4, -3);
    let block_tick = SavedTick {
        kind: TickTypeId(7),
        pos: BlockPos::new(64, 20, -48),
        delay: 12,
        priority: TickPriority::High,
    };
    let fluid_tick = SavedTick {
        kind: TickTypeId(3),
        pos: BlockPos::new(65, 21, -47),
        delay: 25,
        priority: TickPriority::Low,
    };
    let mut data = saved_test_chunk(
        chunk,
        1,
        &BlockState {
            name: "minecraft:air".to_owned(),
            properties: BlockProperties::new(),
        },
    );
    data.set_saved_ticks(crate::tick_data::TickKind::Block, &[block_tick], |id| {
        (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned())
    })
    .unwrap();
    data.set_saved_ticks(crate::tick_data::TickKind::Fluid, &[fluid_tick], |id| {
        (id == TickTypeId(3)).then(|| "minecraft:water".to_owned())
    })
    .unwrap();

    let mut ticks = Ticks::new(0);
    let mut malformed = Ticks::new(0);
    assert!(
        malformed
            .load_saved_chunk_ticks(
                chunk,
                &data,
                100,
                |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
                |_| None,
            )
            .is_err()
    );
    assert!(malformed.blocks.chunk(chunk).is_none());
    assert!(malformed.fluids.chunk(chunk).is_none());
    ticks
        .load_saved_chunk_ticks(
            chunk,
            &data,
            100,
            |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
            |name| (name == "minecraft:water").then_some(TickTypeId(3)),
        )
        .unwrap();
    assert_eq!(ticks.blocks.chunk(chunk).unwrap().count(), 1);
    assert_eq!(ticks.fluids.chunk(chunk).unwrap().count(), 1);
    assert!(
        ticks
            .load_saved_chunk_ticks(
                chunk,
                &data,
                100,
                |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
                |name| (name == "minecraft:water").then_some(TickTypeId(3)),
            )
            .is_err()
    );

    let old_block_list = data.root().tag.get("block_ticks").unwrap().clone();
    assert!(
        ticks
            .save_chunk_ticks(
                chunk,
                &mut data,
                100,
                |id| (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned()),
                |_| None,
            )
            .is_err()
    );
    assert_eq!(data.root().tag.get("block_ticks"), Some(&old_block_list));

    ticks
        .save_chunk_ticks(
            chunk,
            &mut data,
            100,
            |id| (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned()),
            |id| (id == TickTypeId(3)).then(|| "minecraft:water".to_owned()),
        )
        .unwrap();
    let restored_blocks = data
        .saved_ticks(crate::tick_data::TickKind::Block, |name| {
            (name == "minecraft:stone").then_some(TickTypeId(7))
        })
        .unwrap()
        .pack(100);
    let restored_fluids = data
        .saved_ticks(crate::tick_data::TickKind::Fluid, |name| {
            (name == "minecraft:water").then_some(TickTypeId(3))
        })
        .unwrap()
        .pack(100);
    assert_eq!(restored_blocks, vec![block_tick]);
    assert_eq!(restored_fluids, vec![fluid_tick]);
}

#[test]
fn resident_dimension_coordinates_region_block_and_tick_lifecycle() {
    let registry = tiny_block_registry();
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let chunk = ChunkPos::new(-2, 3);
    let block = BlockPos::new(-32, -64, 48);
    let block_tick = SavedTick {
        kind: TickTypeId(7),
        pos: BlockPos::new(-31, -63, 49),
        delay: 17,
        priority: TickPriority::Normal,
    };
    let fluid_tick = SavedTick {
        kind: TickTypeId(3),
        pos: BlockPos::new(-30, -62, 50),
        delay: 29,
        priority: TickPriority::Low,
    };
    let mut data = saved_test_chunk(
        chunk,
        -4,
        &BlockState {
            name: "minecraft:air".to_owned(),
            properties: BlockProperties::new(),
        },
    );
    data.set_saved_ticks(crate::tick_data::TickKind::Block, &[block_tick], |id| {
        (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned())
    })
    .unwrap();
    data.set_saved_ticks(crate::tick_data::TickKind::Fluid, &[fluid_tick], |id| {
        (id == TickTypeId(3)).then(|| "minecraft:water".to_owned())
    })
    .unwrap();

    let mut malformed = ResidentDimension::new(
        "minecraft:overworld",
        ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
        Ticks::new(0),
    );
    assert!(
        malformed
            .load_chunk_data(
                &data,
                100,
                |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
                |_| None,
            )
            .is_err()
    );
    assert_eq!(malformed.level().loaded_chunk_count(), 0);
    assert!(!malformed.has_chunk_ticks(chunk));

    let mut dimension = ResidentDimension::new(
        "minecraft:overworld",
        ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
        Ticks::new(0),
    );
    assert_eq!(dimension.key(), "minecraft:overworld");
    dimension.set_times(100, 2400);
    assert_eq!(dimension.level().game_time(), 100);
    assert_eq!(dimension.level().day_time(), 2400);
    assert!(
        dimension
            .load_chunk_data(
                &data,
                100,
                |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
                |name| (name == "minecraft:water").then_some(TickTypeId(3)),
            )
            .unwrap()
    );
    assert!(dimension.has_chunk_ticks(chunk));
    assert_eq!(dimension.ticks().blocks.chunk(chunk).unwrap().count(), 1);
    assert_eq!(dimension.ticks().fluids.chunk(chunk).unwrap().count(), 1);
    assert_eq!(dimension.level_mut().set_raw(block, stone), Some(air));
    let revision = dimension.level().chunk_revision(chunk).unwrap();
    let original_nbt = data.root().tag.clone();
    assert!(
        dimension
            .save_chunk_data(
                &mut data,
                100,
                |id| (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned()),
                |_| None,
            )
            .is_err()
    );
    assert_eq!(data.root().tag, original_nbt);

    let region_dir =
        std::env::temp_dir().join(format!("horizon-resident-dimension-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&region_dir);
    std::fs::create_dir_all(&region_dir).unwrap();
    let region_path = region_dir.join("r.-1.0.mca");
    {
        let mut region = crate::region::RegionFile::open_writable(&region_path, true).unwrap();
        dimension
            .save_region_chunk(
                &mut region,
                &mut data,
                crate::compression::Compression::Zlib,
                123,
                crate::nbt::Limits::saved(),
                100,
                |id| (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned()),
                |id| (id == TickTypeId(3)).then(|| "minecraft:water".to_owned()),
            )
            .unwrap();
        assert!(dimension.level().is_chunk_dirty(chunk));
        region.sync().unwrap();
    }
    assert!(dimension.mark_chunk_saved(chunk, revision));
    assert!(!dimension.level().is_chunk_dirty(chunk));
    assert!(dimension.unload_chunk(chunk));
    assert!(!dimension.has_chunk_ticks(chunk));

    let mut restored = ResidentDimension::new(
        "minecraft:overworld",
        ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
        Ticks::new(0),
    );
    {
        let mut region = crate::region::RegionFile::open(&region_path).unwrap();
        assert!(
            restored
                .load_region_chunk(
                    &mut region,
                    chunk,
                    crate::nbt::Limits::saved(),
                    100,
                    |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
                    |name| (name == "minecraft:water").then_some(TickTypeId(3)),
                )
                .unwrap()
        );
        assert_eq!(region.read(chunk).unwrap().unwrap().timestamp, 123);
    }
    assert_eq!(restored.level().block_state(block), stone);
    assert_eq!(restored.ticks().blocks.chunk(chunk).unwrap().count(), 1);
    assert_eq!(restored.ticks().fluids.chunk(chunk).unwrap().count(), 1);
    std::fs::remove_dir_all(region_dir).unwrap();
}

#[test]
fn local_world_runtime_owns_dimension_clock_and_global_saved_data() {
    let registry = tiny_block_registry();
    let root = std::env::temp_dir().join(format!(
        "horizon-local-world-runtime-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let features = Vec::new();
    let mut world = crate::world_runtime::LocalWorldRuntime::open(&root, &features).unwrap();
    let mut preloaded = ResidentDimension::new(
        "test:preloaded",
        ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
        Ticks::new(0),
    );
    assert!(preloaded.level_mut().load_empty_chunk(ChunkPos::new(0, 0)));
    assert!(world.add_dimension(preloaded).is_err());
    assert!(world.dimension("test:preloaded").is_none());
    let resident = ResidentDimension::new(
        "minecraft:overworld",
        ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
        Ticks::new(0),
    );
    assert!(world.add_dimension(resident).unwrap());
    assert!(
        !world
            .add_dimension(ResidentDimension::new(
                "minecraft:overworld",
                ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    assert!(
        world
            .add_dimension(ResidentDimension::new(
                "minecraft:the_nether",
                ResidentLevel::new(&registry, 0, 256, 43).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    world
        .dimension_mut("minecraft:the_nether")
        .unwrap()
        .resident_mut()
        .level_mut()
        .set_times(999, 0);
    assert_eq!(world.game_time(), 0);
    assert_eq!(world.advance_game_time(), 1);
    assert_eq!(world.advance_game_time(), 2);
    assert_eq!(
        world
            .dimension("minecraft:the_nether")
            .unwrap()
            .resident()
            .level()
            .game_time(),
        2
    );
    assert_eq!(
        world.advance_dimension_day_time("minecraft:overworld", false),
        Some(0)
    );
    assert_eq!(
        world.advance_dimension_day_time("minecraft:overworld", true),
        Some(1)
    );
    assert_eq!(
        world.advance_dimension_day_time("minecraft:the_nether", true),
        Some(1)
    );
    {
        let dimension = world.dimension_mut("minecraft:overworld").unwrap();
        dimension.set_day_time(24_000);
        assert_eq!(dimension.resident().level().game_time(), 2);
        assert_eq!(dimension.resident().level().day_time(), 24_000);
        dimension.border_mut().set_size(512.0);
    }
    world.weather_mut().set_parameters(0, 600, true, false);
    assert!(world.game_rules_mut().set_string("advance_time", "false"));
    world.save_world_data().unwrap();
    drop(world);

    let mut restored = crate::world_runtime::LocalWorldRuntime::open(&root, &features).unwrap();
    let resident = ResidentDimension::new(
        "minecraft:overworld",
        ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
        Ticks::new(0),
    );
    assert!(restored.add_dimension(resident).unwrap());
    assert!(
        restored
            .add_dimension(ResidentDimension::new(
                "minecraft:the_nether",
                ResidentLevel::new(&registry, 0, 256, 43).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    let dimension = restored.dimension("minecraft:overworld").unwrap();
    assert_eq!(dimension.total_ticks(), 24_000);
    assert_eq!(dimension.resident().level().game_time(), 2);
    assert_eq!(dimension.resident().level().day_time(), 24_000);
    assert_eq!(dimension.border().size(), 512.0);
    let nether = restored.dimension("minecraft:the_nether").unwrap();
    assert_eq!(nether.resident().level().game_time(), 2);
    assert_eq!(nether.day_time(), 1);
    assert!(restored.weather().raining);
    assert_eq!(restored.weather().rain_time, 600);
    assert!(!restored.game_rules().bool("advance_time"));
    drop(restored);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_world_runtime_owns_region_chunk_load_save_and_reopen() {
    let registry = tiny_block_registry();
    let root = std::env::temp_dir().join(format!(
        "horizon-local-world-region-runtime-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let features = Vec::new();
    let chunk = ChunkPos::new(-2, 3);
    let block = BlockPos::new(-32, -64, 48);
    let stone = registry.default_state("minecraft:stone").unwrap();
    let chunk_state = BlockState {
        name: "minecraft:air".to_owned(),
        properties: BlockProperties::new(),
    };
    let mut data = saved_test_chunk(chunk, -4, &chunk_state);
    let block_tick = SavedTick {
        kind: TickTypeId(7),
        pos: BlockPos::new(-31, -63, 49),
        delay: 17,
        priority: TickPriority::Normal,
    };
    let fluid_tick = SavedTick {
        kind: TickTypeId(3),
        pos: BlockPos::new(-30, -62, 50),
        delay: 29,
        priority: TickPriority::Low,
    };
    data.set_saved_ticks(crate::tick_data::TickKind::Block, &[block_tick], |id| {
        (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned())
    })
    .unwrap();
    data.set_saved_ticks(crate::tick_data::TickKind::Fluid, &[fluid_tick], |id| {
        (id == TickTypeId(3)).then(|| "minecraft:water".to_owned())
    })
    .unwrap();
    let region_path = root.join("region/r.-1.0.mca");
    std::fs::create_dir_all(region_path.parent().unwrap()).unwrap();
    let mut seed_region = crate::region::RegionFile::open_writable(&region_path, true).unwrap();
    let seed_record = crate::region::ChunkRecord::from_nbt(
        data.root(),
        crate::compression::Compression::Zlib,
        123,
        crate::nbt::Limits::saved(),
    )
    .unwrap();
    seed_region.write(chunk, &seed_record).unwrap();
    seed_region.sync().unwrap();
    drop(seed_region);

    let mut world = crate::world_runtime::LocalWorldRuntime::open(&root, &features).unwrap();
    assert!(
        world
            .add_dimension(ResidentDimension::new(
                "minecraft:overworld",
                ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    let dimension = world.dimension_mut("minecraft:overworld").unwrap();
    dimension.resident_mut().update_chunk_tickets(None);
    let headless = dimension.lifecycle_candidates(0).unwrap();
    assert!(headless.headless_unbounded);
    assert!(headless.candidates.is_empty());
    let missing_chunk = ChunkPos::new(chunk.x - 1, chunk.z);
    dimension
        .resident_mut()
        .update_chunk_tickets(Some(crate::ticket_levels::Sources {
            forced: vec![(chunk.x, chunk.z), (missing_chunk.x, missing_chunk.z)],
            ..Default::default()
        }));
    let ticketed_plan = dimension.lifecycle_candidates(100_000).unwrap();
    assert!(!ticketed_plan.headless_unbounded);
    assert!(ticketed_plan.candidates.contains(&missing_chunk));
    assert!(ticketed_plan.candidates.contains(&chunk));
    assert!(
        ticketed_plan
            .candidates
            .windows(2)
            .all(|pair| (pair[0].x, pair[0].z) < (pair[1].x, pair[1].z))
    );
    assert!(dimension.lifecycle_candidates(1).is_err());
    let loaded = dimension
        .reconcile_ticketed_chunks(
            [chunk, chunk, missing_chunk],
            crate::nbt::Limits::saved(),
            100,
            |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
            |name| (name == "minecraft:water").then_some(TickTypeId(3)),
        )
        .unwrap();
    assert_eq!(loaded.loaded, 1);
    assert_eq!(loaded.missing, 1);
    assert_eq!(
        dimension
            .resident()
            .ticks()
            .blocks
            .chunk(chunk)
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        dimension
            .resident()
            .ticks()
            .fluids
            .chunk(chunk)
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        dimension.resident_mut().level_mut().set_raw(block, stone),
        Some(registry.default_state("minecraft:air").unwrap())
    );
    dimension.resident_mut().update_chunk_tickets(None);
    let headless_owned = dimension.lifecycle_candidates(1).unwrap();
    assert!(headless_owned.headless_unbounded);
    assert_eq!(headless_owned.candidates, vec![chunk]);
    dimension
        .resident_mut()
        .update_chunk_tickets(Some(crate::ticket_levels::Sources::default()));
    let owned_plan = dimension.lifecycle_candidates(1).unwrap();
    assert!(!owned_plan.headless_unbounded);
    assert_eq!(owned_plan.candidates, vec![chunk]);
    assert!(dimension.lifecycle_candidates(0).is_err());
    let deferred = dimension
        .reconcile_ticketed_chunks(
            [chunk],
            crate::nbt::Limits::saved(),
            100,
            |_| None,
            |_| None,
        )
        .unwrap();
    assert_eq!(deferred.deferred_dirty, 1);
    assert!(!dimension.park_chunk(chunk));
    assert!(dimension.resident().is_chunk_dirty(chunk));
    let failed_save = dimension
        .save_chunk_to_region(
            chunk,
            crate::compression::Compression::Zlib,
            123,
            crate::nbt::Limits::saved(),
            100,
            |_| None,
            |_| None,
        )
        .unwrap_err();
    assert_eq!(failed_save.kind(), std::io::ErrorKind::InvalidData);
    assert!(dimension.resident().is_chunk_dirty(chunk));
    assert!(dimension.active_chunk_data(chunk).is_some());
    let failed_sync = dimension
        .save_chunk_to_region_with_sync_for_test(
            chunk,
            crate::compression::Compression::Zlib,
            123,
            crate::nbt::Limits::saved(),
            100,
            |id| (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned()),
            |id| (id == TickTypeId(3)).then(|| "minecraft:water".to_owned()),
            |_| Err(std::io::Error::other("injected region sync failure")),
        )
        .unwrap_err();
    assert_eq!(failed_sync.kind(), std::io::ErrorKind::Other);
    assert!(dimension.resident().is_chunk_dirty(chunk));
    assert!(dimension.active_chunk_data(chunk).is_some());
    assert!(!dimension.park_chunk(chunk));
    assert!(
        dimension
            .save_chunk_to_region(
                chunk,
                crate::compression::Compression::Zlib,
                123,
                crate::nbt::Limits::saved(),
                100,
                |id| (id == TickTypeId(7)).then(|| "minecraft:stone".to_owned()),
                |id| (id == TickTypeId(3)).then(|| "minecraft:water".to_owned()),
            )
            .unwrap()
    );
    assert!(!dimension.resident().is_chunk_dirty(chunk));
    let wrong_position = ChunkPos::new(chunk.x + 1, chunk.z);
    assert!(!dimension.park_chunk(wrong_position));
    assert!(dimension.resident().chunk_save_revision(chunk).is_some());

    let preserved_nbt = dimension
        .active_chunk_data(chunk)
        .unwrap()
        .root()
        .tag
        .clone();
    let parked = dimension
        .reconcile_ticketed_chunks(
            [chunk, chunk],
            crate::nbt::Limits::saved(),
            100,
            |_| None,
            |_| None,
        )
        .unwrap();
    assert_eq!(parked.parked, 1);
    assert!(dimension.active_chunk_data(chunk).is_none());
    assert!(dimension.is_chunk_parked(chunk));
    assert_eq!(dimension.parked_chunk_count(), 1);
    assert_eq!(
        dimension.lifecycle_candidates(1).unwrap().candidates,
        vec![chunk]
    );
    dimension.resident_mut().update_chunk_tickets(None);
    let headless_parked = dimension.lifecycle_candidates(1).unwrap();
    assert!(headless_parked.headless_unbounded);
    assert_eq!(headless_parked.candidates, vec![chunk]);
    assert!(dimension.resident().chunk_save_revision(chunk).is_none());
    assert!(!dimension.park_chunk(chunk));
    assert_eq!(dimension.parked_chunk_count(), 1);

    let activation_error = dimension
        .activate_parked_chunk(chunk, 100, |_| None, |_| None)
        .unwrap_err();
    assert_eq!(activation_error.kind(), std::io::ErrorKind::InvalidData);
    assert!(dimension.is_chunk_parked(chunk));
    assert_eq!(dimension.parked_chunk_count(), 1);
    assert!(dimension.resident().chunk_save_revision(chunk).is_none());
    assert!(!dimension.resident().has_chunk_ticks(chunk));

    dimension
        .resident_mut()
        .update_chunk_tickets(Some(crate::ticket_levels::Sources {
            forced: vec![(chunk.x, chunk.z)],
            ..Default::default()
        }));
    let activated = dimension
        .reconcile_ticketed_chunks(
            [chunk, chunk],
            crate::nbt::Limits::saved(),
            100,
            |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
            |name| (name == "minecraft:water").then_some(TickTypeId(3)),
        )
        .unwrap();
    assert_eq!(activated.activated, 1);
    assert!(!dimension.is_chunk_parked(chunk));
    assert_eq!(dimension.parked_chunk_count(), 0);
    let data = dimension.active_chunk_data(chunk).unwrap();
    assert_eq!(&data.root().tag, &preserved_nbt);
    assert_eq!(
        data.root().tag.get("unknown_root_field"),
        Some(&Tag::Long(77))
    );
    assert_eq!(
        data.root()
            .tag
            .get("sections")
            .and_then(|sections| match sections {
                Tag::List(entries) => entries.first(),
                _ => None,
            })
            .and_then(|section| section.get("biomes")),
        Some(&Tag::String("opaque-biome-payload".into()))
    );
    assert_eq!(
        dimension
            .resident()
            .ticks()
            .blocks
            .chunk(chunk)
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        dimension
            .resident()
            .ticks()
            .fluids
            .chunk(chunk)
            .unwrap()
            .count(),
        1
    );
    assert!(
        !dimension
            .activate_parked_chunk(
                chunk,
                100,
                |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
                |name| (name == "minecraft:water").then_some(TickTypeId(3)),
            )
            .unwrap()
    );
    drop(world);

    let mut restored = crate::world_runtime::LocalWorldRuntime::open(&root, &features).unwrap();
    assert!(
        restored
            .add_dimension(ResidentDimension::new(
                "minecraft:overworld",
                ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    let dimension = restored.dimension_mut("minecraft:overworld").unwrap();
    assert!(
        dimension
            .load_chunk_from_region(
                chunk,
                crate::nbt::Limits::saved(),
                100,
                |name| (name == "minecraft:stone").then_some(TickTypeId(7)),
                |name| (name == "minecraft:water").then_some(TickTypeId(3)),
            )
            .unwrap()
    );
    let reopened = dimension.active_chunk_data(chunk).unwrap();
    assert_eq!(reopened.position(), chunk);
    assert_eq!(
        reopened.root().tag.get("unknown_root_field"),
        Some(&Tag::Long(77))
    );
    assert_eq!(dimension.resident().level().block_state(block), stone);
    assert_eq!(
        dimension
            .resident()
            .ticks()
            .blocks
            .chunk(chunk)
            .unwrap()
            .pack(100),
        vec![block_tick]
    );
    assert_eq!(
        dimension
            .resident()
            .ticks()
            .fluids
            .chunk(chunk)
            .unwrap()
            .pack(100),
        vec![fluid_tick]
    );
    assert!(!dimension.resident().is_chunk_dirty(chunk));
    restored.sync_regions().unwrap();
    drop(restored);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn world_dimension_saves_dirty_chunks_in_budgeted_durable_sweeps() {
    let registry = tiny_block_registry();
    let root =
        std::env::temp_dir().join(format!("horizon-dirty-chunk-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let first = ChunkPos::new(-2, 3);
    let second = ChunkPos::new(35, -1);
    let first_block = BlockPos::new(-32, -64, 48);
    let second_block = BlockPos::new(560, -64, -16);
    let air = BlockState {
        name: "minecraft:air".to_owned(),
        properties: BlockProperties::new(),
    };
    let mut seed = Vec::new();
    for position in [first, second] {
        let data = saved_test_chunk(position, -4, &air);
        let region_pos = position.region();
        let path = root.join(format!("region/r.{}.{}.mca", region_pos.x, region_pos.z));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut region = crate::region::RegionFile::open_writable(&path, true).unwrap();
        let record = crate::region::ChunkRecord::from_nbt(
            data.root(),
            crate::compression::Compression::Zlib,
            1,
            crate::nbt::Limits::saved(),
        )
        .unwrap();
        region.write(position, &record).unwrap();
        region.sync().unwrap();
        seed.push(path);
    }

    let features = Vec::new();
    let mut world = crate::world_runtime::LocalWorldRuntime::open(&root, &features).unwrap();
    assert!(
        world
            .add_dimension(ResidentDimension::new(
                "minecraft:overworld",
                ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    let dimension = world.dimension_mut("minecraft:overworld").unwrap();
    for position in [first, second] {
        assert!(
            dimension
                .load_chunk_from_region(
                    position,
                    crate::nbt::Limits::saved(),
                    0,
                    |_| None,
                    |_| None,
                )
                .unwrap()
        );
    }
    let stone = registry.default_state("minecraft:stone").unwrap();
    assert_eq!(
        dimension
            .resident_mut()
            .level_mut()
            .set_raw(first_block, stone),
        Some(registry.default_state("minecraft:air").unwrap())
    );
    assert_eq!(
        dimension
            .resident_mut()
            .level_mut()
            .set_raw(second_block, stone),
        Some(registry.default_state("minecraft:air").unwrap())
    );

    let none = |_| None;
    let empty = dimension
        .save_dirty_chunks(
            0,
            crate::compression::Compression::Zlib,
            99,
            crate::nbt::Limits::saved(),
            0,
            none,
            none,
        )
        .unwrap();
    assert_eq!(empty.saved, 0);
    assert_eq!(empty.remaining_dirty, 2);

    let partial = dimension
        .save_dirty_chunks(
            1,
            crate::compression::Compression::Zlib,
            100,
            crate::nbt::Limits::saved(),
            0,
            none,
            none,
        )
        .unwrap();
    assert_eq!(partial.saved, 1);
    assert_eq!(partial.remaining_dirty, 1);
    assert!(!dimension.resident().is_chunk_dirty(first));
    assert!(dimension.resident().is_chunk_dirty(second));

    let failed = dimension
        .save_dirty_chunks_with_sync_for_test(
            1,
            crate::compression::Compression::Zlib,
            101,
            crate::nbt::Limits::saved(),
            0,
            none,
            none,
            |_| Err(std::io::Error::other("injected second region sync failure")),
        )
        .unwrap_err();
    assert_eq!(failed.kind(), std::io::ErrorKind::Other);
    assert!(!dimension.resident().is_chunk_dirty(first));
    assert!(dimension.resident().is_chunk_dirty(second));
    assert!(dimension.active_chunk_data(second).is_some());

    let retried = dimension
        .save_dirty_chunks(
            usize::MAX,
            crate::compression::Compression::Zlib,
            102,
            crate::nbt::Limits::saved(),
            0,
            none,
            none,
        )
        .unwrap();
    assert_eq!(retried.saved, 1);
    assert_eq!(retried.remaining_dirty, 0);
    for (position, block, path) in [
        (first, first_block, &seed[0]),
        (second, second_block, &seed[1]),
    ] {
        let mut region = crate::region::RegionFile::open(path).unwrap();
        let record = region.read(position).unwrap().unwrap();
        assert_eq!(record.timestamp, if position == first { 100 } else { 102 });
        let data =
            crate::chunk::ChunkData::new(record.nbt(crate::nbt::Limits::saved()).unwrap()).unwrap();
        assert_eq!(data.block(block).unwrap().unwrap().name, "minecraft:stone");
    }
    world.sync_regions().unwrap();
    drop(world);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_world_runtime_saves_dirty_chunks_in_a_shared_dimension_budget() {
    let registry = tiny_block_registry();
    let root = std::env::temp_dir().join(format!(
        "horizon-world-dirty-save-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let chunk = ChunkPos::new(0, 0);
    let air = BlockState {
        name: "minecraft:air".to_owned(),
        properties: BlockProperties::new(),
    };
    for (directory, section_y) in [
        (root.join("region"), 0),
        (root.join("dimensions/minecraft/the_nether/region"), 0),
    ] {
        std::fs::create_dir_all(&directory).unwrap();
        let data = saved_test_chunk(chunk, section_y, &air);
        let record = crate::region::ChunkRecord::from_nbt(
            data.root(),
            crate::compression::Compression::Zlib,
            1,
            crate::nbt::Limits::saved(),
        )
        .unwrap();
        let mut region =
            crate::region::RegionFile::open_writable(directory.join("r.0.0.mca"), true).unwrap();
        region.write(chunk, &record).unwrap();
        region.sync().unwrap();
    }

    let mut world = crate::world_runtime::LocalWorldRuntime::open(&root, &[]).unwrap();
    assert!(
        world
            .add_dimension(ResidentDimension::new(
                "minecraft:overworld",
                ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    assert!(
        world
            .add_dimension(ResidentDimension::new(
                "minecraft:the_nether",
                ResidentLevel::new(&registry, 0, 256, 43).unwrap(),
                Ticks::new(0),
            ))
            .unwrap()
    );
    for key in ["minecraft:overworld", "minecraft:the_nether"] {
        assert!(
            world
                .dimension_mut(key)
                .unwrap()
                .load_chunk_from_region(chunk, crate::nbt::Limits::saved(), 0, |_| None, |_| None)
                .unwrap()
        );
    }
    let stone = registry.default_state("minecraft:stone").unwrap();
    for (key, block) in [
        ("minecraft:overworld", BlockPos::new(0, -64, 0)),
        ("minecraft:the_nether", BlockPos::new(0, 0, 0)),
    ] {
        assert_eq!(
            world
                .dimension_mut(key)
                .unwrap()
                .resident_mut()
                .level_mut()
                .set_raw(block, stone),
            Some(registry.default_state("minecraft:air").unwrap())
        );
    }

    let first = world
        .save_dirty_chunks(
            1,
            crate::compression::Compression::Zlib,
            10,
            crate::nbt::Limits::saved(),
            |_| None,
            |_| None,
        )
        .unwrap();
    assert_eq!(first.saved, 1);
    assert_eq!(first.remaining_dirty, 1);
    assert!(
        !world
            .dimension("minecraft:overworld")
            .unwrap()
            .resident()
            .is_chunk_dirty(chunk)
    );
    assert!(
        world
            .dimension("minecraft:the_nether")
            .unwrap()
            .resident()
            .is_chunk_dirty(chunk)
    );

    let second = world
        .save_dirty_chunks(
            1,
            crate::compression::Compression::Zlib,
            11,
            crate::nbt::Limits::saved(),
            |_| None,
            |_| None,
        )
        .unwrap();
    assert_eq!(second.saved, 1);
    assert_eq!(second.remaining_dirty, 0);
    world.sync_regions().unwrap();
    drop(world);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn resident_dimension_tracks_tick_only_changes_and_rejects_stale_snapshot_ack() {
    let registry = tiny_block_registry();
    let chunk = ChunkPos::new(2, -1);
    let mut dimension = ResidentDimension::new(
        "minecraft:overworld",
        ResidentLevel::new(&registry, -64, 384, 42).unwrap(),
        Ticks::new(0),
    );
    assert!(dimension.load_empty_chunk(chunk).unwrap());
    assert!(!dimension.level().is_ticking_chunk(chunk));
    dimension.update_chunk_tickets(Some(crate::ticket_levels::Sources {
        forced: vec![(chunk.x, chunk.z)],
        ..Default::default()
    }));
    assert!(dimension.level().is_ticking_chunk(chunk));
    assert!(dimension.level().is_ticking_chunk(ChunkPos::new(3, -1)));
    assert!(!dimension.level().is_ticking_chunk(ChunkPos::new(4, -1)));
    dimension.update_chunk_tickets(None);
    assert!(dimension.level().is_ticking_chunk(ChunkPos::new(4, -1)));
    dimension.update_chunk_tickets(Some(crate::ticket_levels::Sources {
        forced: vec![(chunk.x, chunk.z)],
        ..Default::default()
    }));
    let initial = dimension.chunk_save_revision(chunk).unwrap();
    assert!(dimension.mark_chunk_snapshot_saved(chunk, &initial));
    assert!(!dimension.level().is_chunk_dirty(chunk));

    let first = BlockPos::new(34, 64, -16);
    dimension
        .ticks_mut()
        .schedule_block(100, first, states::BlockId(1), 20, TickPriority::Normal);
    assert!(!dimension.level().is_chunk_dirty(chunk));
    assert!(dimension.is_chunk_dirty(chunk));
    assert_eq!(dimension.dirty_chunks(), [chunk]);
    assert!(!dimension.unload_chunk(chunk));

    let captured = dimension.chunk_save_revision(chunk).unwrap();
    dimension.ticks_mut().schedule_block(
        100,
        BlockPos::new(35, 64, -16),
        states::BlockId(2),
        40,
        TickPriority::Normal,
    );
    assert!(!dimension.mark_chunk_snapshot_saved(chunk, &captured));
    assert!(dimension.is_chunk_dirty(chunk));

    let current = dimension.chunk_save_revision(chunk).unwrap();
    assert!(dimension.mark_chunk_snapshot_saved(chunk, &current));
    assert!(!dimension.is_chunk_dirty(chunk));
    assert!(dimension.unload_chunk(chunk));

    let inconsistent = ChunkPos::new(3, -1);
    assert!(dimension.load_empty_chunk(inconsistent).unwrap());
    let initial = dimension.chunk_save_revision(inconsistent).unwrap();
    assert!(dimension.mark_chunk_snapshot_saved(inconsistent, &initial));
    dimension.ticks_mut().blocks.remove_chunk(inconsistent);
    assert!(dimension.is_chunk_dirty(inconsistent));
    assert!(!dimension.unload_chunk(inconsistent));
    assert!(dimension.discard_chunk(inconsistent));
}

#[test]
fn resident_level_rejects_invalid_bounds_and_sections_transactionally() {
    let registry = tiny_block_registry();
    let air = registry.default_state("minecraft:air").unwrap();
    assert!(ResidentLevel::new(&registry, -63, 16, 42).is_err());
    assert!(ResidentLevel::new(&registry, i32::MAX - 15, 32, 42).is_err());

    let mut level = ResidentLevel::new(&registry, -64, 384, 42).unwrap();
    let out_of_range = BlockSection::uniform(air, &registry).unwrap();
    assert!(
        level
            .load_chunk(ChunkPos::new(0, 0), [(20, out_of_range)])
            .is_err()
    );
    assert!(!level.is_loaded(BlockPos::new(0, 0, 0)));

    let other_registry = tiny_block_registry();
    let other_air = other_registry.default_state("minecraft:air").unwrap();
    let foreign = BlockSection::uniform(other_air, &other_registry).unwrap();
    assert!(
        level
            .load_chunk(ChunkPos::new(0, 0), [(0, foreign)])
            .is_err()
    );
    assert!(!level.is_loaded(BlockPos::new(0, 0, 0)));

    let first = BlockSection::uniform(air, &registry).unwrap();
    let duplicate = BlockSection::uniform(air, &registry).unwrap();
    assert!(
        level
            .load_chunk(ChunkPos::new(0, 0), [(0, first), (0, duplicate)])
            .is_err()
    );
    assert!(!level.is_loaded(BlockPos::new(0, 0, 0)));
    assert_eq!(level.loaded_chunk_count(), 0);
}

#[test]
fn resident_level_is_a_consumer_for_vanilla_block_behaviours() {
    let Some(f) = fixture() else { return };
    let mut level = ResidentLevel::new(&f.registry, -64, 384, 42).unwrap();
    let pos = BlockPos::new(0, 80, 0);
    assert!(level.load_empty_chunk(pos.chunk()));
    let mut ticks = Ticks::new(0);
    let stone = f.registry.default_state("minecraft:stone").unwrap();
    assert!(
        f.behaviours
            .set_block(&mut level, &mut ticks, pos, stone, flags::UPDATE_ALL)
    );
    assert_eq!(level.block_state(pos), stone);
    assert!(f.behaviours.set_block(
        &mut level,
        &mut ticks,
        pos,
        f.behaviours.air(),
        flags::UPDATE_ALL
    ));
}

/// `LevelReader.getMaxLocalRawBrightness`: 15 outside the +-30,000,000 gate,
/// where no chunk is there to answer, and the raw brightness inside it. With
/// the sky light at 0 the two answers differ, so the gate is what is measured.
#[test]
fn max_local_raw_brightness_is_full_outside_the_gate() {
    let Some(f) = fixture() else {
        return;
    };
    let b = &f.behaviours;
    let mut level = TestLevel::new(b);
    level.sky_light = Some(0);
    let mut ticks = Ticks::new(0);
    let sim = b.sim(&mut level, &mut ticks);
    let at = |x: i32, z: i32| sim.max_local_raw_brightness(BlockPos::new(x, 64, z));
    // `x >= -30000000 && z >= -30000000 && x < 30000000 && z < 30000000`.
    assert_eq!((at(-30_000_000, 0), at(29_999_999, 0)), (0, 0));
    assert_eq!((at(-30_000_001, 0), at(30_000_000, 0)), (15, 15));
    assert_eq!((at(0, -30_000_000), at(0, 29_999_999)), (0, 0));
    assert_eq!((at(0, -30_000_001), at(0, 30_000_000)), (15, 15));
}

#[test]
fn neighbor_observation_follows_dispatch_order() {
    use std::cell::RefCell;

    struct Observer {
        inner: TestLevel,
        seen: RefCell<Vec<(BlockPos, BlockStateId, BlockId, bool)>>,
    }
    impl LevelAccess for Observer {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn observe_neighbor_changed(
            &self,
            pos: BlockPos,
            state: BlockStateId,
            changed: BlockId,
            moved: bool,
        ) {
            assert_eq!(self.block_state(pos), state);
            self.seen.borrow_mut().push((pos, state, changed, moved));
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let mut level = Observer {
        inner: TestLevel::new(b),
        seen: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    let changed = b.table.block_by_name("minecraft:stone").unwrap();
    b.update_neighbors_at(&mut level, &mut ticks, BlockPos::new(0, 0, 0), changed);
    let expected = [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ];
    let actual = level.seen.borrow();
    assert_eq!(actual.len(), expected.len());
    for (event, (x, y, z)) in actual.iter().zip(expected) {
        assert_eq!(*event, (BlockPos::new(x, y, z), b.air(), changed, false));
    }
    drop(actual);
    b.sim(&mut level, &mut ticks)
        .neighbor_changed(b.air(), BlockPos::new(3, 4, 5), changed, true);
    assert_eq!(
        level.seen.borrow().last(),
        Some(&(BlockPos::new(3, 4, 5), b.air(), changed, true))
    );
}

/// `WeatheringCopperChestBlock.randomTick`: a copper chest someone has open
/// skips `changeOverTime` and its draws; a closed one draws (#11378).
#[test]
fn a_viewed_copper_chest_does_not_weather() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let pos = BlockPos::new(0, 64, 0);
    let chest = w.id("copper_chest", &[("type", "single")]);
    w.put(pos, chest);
    let draw = |w: &World| w.level.random.clone().next_long();
    w.level.opened.insert(pos);
    let before = draw(&w);
    f.behaviours.random_tick(&mut w.level, &mut w.ticks, pos);
    assert_eq!(draw(&w), before, "an open chest draws nothing");
    w.level.opened.clear();
    f.behaviours.random_tick(&mut w.level, &mut w.ticks, pos);
    assert_ne!(draw(&w), before, "a closed one rolls changeOverTime");
}

#[test]
fn state_table_matches_registry() {
    let Some(f) = fixture() else {
        return;
    };
    let t = &f.behaviours.table;
    assert_eq!(t.state_count(), f.registry.len());
    let stairs = f.registry.default_state("minecraft:oak_stairs").unwrap();
    let key = f.behaviours.props.facing;
    let east = t.with_text(stairs, key, "east").unwrap();
    assert_eq!(f.registry.state(east).unwrap().properties["facing"], "east");
    assert_eq!(t.text(east, f.behaviours.props.half), "bottom");
    assert!(classes::Class::ALL.len() <= 320);
}

#[test]
fn fences_connect_and_disconnect() {
    let mut w = world!();
    let fence = w.id("oak_fence", &[]);
    let a = BlockPos::new(0, 1, 0);
    let c = BlockPos::new(1, 1, 0);
    w.set(a, fence);
    w.set(c, fence);
    assert_eq!(w.prop(a, "east"), "true");
    assert_eq!(w.prop(c, "west"), "true");
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 1, -1), stone);
    assert_eq!(w.prop(a, "north"), "true", "fences connect to sturdy faces");
    let air = w.b().air();
    w.set(c, air);
    assert_eq!(w.prop(a, "east"), "false");
}

#[test]
fn a_barrel_recounts_its_openers_on_its_scheduled_tick() {
    use horizon_core::ticks::TickPriority;
    let mut w = world!();
    let barrel = w.id("barrel", &[("facing", "north"), ("open", "true")]);
    let pos = BlockPos::new(0, 1, 0);
    w.put(pos, barrel);
    let block = w.b().block(barrel);
    // One scheduled tick of the barrel falls due.
    let due = |w: &mut World| {
        w.ticks
            .schedule_block(w.level.time, pos, block, 1, TickPriority::Normal);
        w.tick();
    };
    let sounds = |w: &World, name: &str| {
        w.level
            .events
            .iter()
            .filter(|e| matches!(e, LevelEvent::Sound { sound, .. } if &**sound == name))
            .count()
    };
    let game_events = |w: &World, name: &str| {
        w.level
            .events
            .iter()
            .filter(|e| matches!(e, LevelEvent::GameEvent { event, .. } if *event == name))
            .count()
    };
    // A player walked away while the menu was still counted open.
    w.level.openers.insert(pos, (1, 0));
    due(&mut w);
    assert_eq!(
        w.level.opener_rechecks,
        [pos],
        "BarrelBlock.tick asks its block entity to recheck the openers"
    );
    assert_eq!(
        w.prop(pos, "open"),
        "false",
        "updateBlockState closes the barrel"
    );
    assert_eq!(sounds(&w, "minecraft:block.barrel.close"), 1);
    let pitch = w
        .level
        .events
        .iter()
        .find_map(|e| match e {
            LevelEvent::Sound {
                sound,
                volume,
                pitch,
                ..
            } if &**sound == "minecraft:block.barrel.close" => Some((*volume, *pitch)),
            _ => None,
        })
        .unwrap();
    assert_eq!(pitch.0, 0.5);
    assert!((0.9..1.0).contains(&pitch.1), "pitch {}", pitch.1);
    assert_eq!(game_events(&w, "minecraft:container_close"), 1);
    // A barrel's openerCountChanged is empty: no lid block event (a chest
    // queues one here).
    assert!(
        w.ticks.block_events.is_empty(),
        "a barrel broadcasts no block event"
    );
    // Someone opens it again: the recount finds them and the lid opens.
    w.level.openers.insert(pos, (0, 1));
    due(&mut w);
    assert_eq!(w.prop(pos, "open"), "true");
    assert_eq!(sounds(&w, "minecraft:block.barrel.open"), 1);
    assert_eq!(game_events(&w, "minecraft:container_open"), 1);
    // ContainerOpenersCounter.scheduleRecheck: the recheck repeats while the
    // barrel is open, five ticks later.
    let rechecks = w.level.opener_rechecks.len();
    for _ in 0..5 {
        w.tick();
    }
    assert_eq!(w.level.opener_rechecks.len(), rechecks + 1);
    // Nobody has it open on the next recount: the chain stops there.
    w.level.openers.insert(pos, (1, 0));
    for _ in 0..5 {
        w.tick();
    }
    let rechecks = w.level.opener_rechecks.len();
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.level.opener_rechecks.len(), rechecks);
    assert_eq!(w.prop(pos, "open"), "false");
}

#[test]
fn walls_and_panes_connect() {
    let mut w = world!();
    let wall = w.id("cobblestone_wall", &[]);
    let pane = w.id("glass_pane", &[]);
    w.set(BlockPos::new(0, 1, 0), wall);
    w.set(BlockPos::new(0, 1, 1), wall);
    w.set(BlockPos::new(0, 1, 2), wall);
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "north"), "low");
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "south"), "low");
    assert_eq!(
        w.prop(BlockPos::new(0, 1, 1), "up"),
        "false",
        "straight wall drops its post"
    );
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 2, 1), stone);
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "north"), "tall");
    w.set(BlockPos::new(5, 1, 0), pane);
    w.set(BlockPos::new(6, 1, 0), pane);
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "east"), "true");
}

#[test]
fn stairs_form_corners() {
    let mut w = world!();
    let north = w.id("oak_stairs", &[("facing", "north")]);
    let east = w.id("oak_stairs", &[("facing", "east")]);
    w.set(BlockPos::new(0, 1, 0), north);
    w.set(BlockPos::new(0, 1, -1), east);
    // The north-facing stair has an east-facing stair behind it.
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "shape"), "outer_right");
}

#[test]
fn torch_and_door_halves_break_with_support() {
    let mut w = world!();
    w.floor(0, 3, "stone");
    let torch = w.id("torch", &[]);
    w.set(BlockPos::new(1, 1, 1), torch);
    let door = w.id("oak_door", &[("half", "lower")]);
    let upper = w.id("oak_door", &[("half", "upper")]);
    w.set(BlockPos::new(0, 1, 0), door);
    w.set(BlockPos::new(0, 2, 0), upper);
    let air = w.b().air();
    w.set(BlockPos::new(1, 0, 1), air);
    assert_eq!(
        w.get(BlockPos::new(1, 1, 1)),
        air,
        "torch pops without support"
    );
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::DropResources { .. }))
    );
    w.set(BlockPos::new(0, 1, 0), air);
    assert_eq!(
        w.get(BlockPos::new(0, 2, 0)),
        air,
        "upper door half goes with the lower"
    );
}

#[test]
fn snowy_grass_and_chests_pair() {
    let mut w = world!();
    let grass = w.id("grass_block", &[]);
    w.set(BlockPos::new(0, 0, 0), grass);
    let snow = w.id("snow", &[]);
    w.set(BlockPos::new(0, 1, 0), snow);
    assert_eq!(w.prop(BlockPos::new(0, 0, 0), "snowy"), "true");
    // A left chest's partner sits clockwise of its facing; removing the
    // partner makes it single again.
    let left = w.id("chest", &[("facing", "north"), ("type", "left")]);
    let right = w.id("chest", &[("facing", "north"), ("type", "right")]);
    w.set(BlockPos::new(3, 1, 0), left);
    w.set(BlockPos::new(4, 1, 0), right);
    let air = w.b().air();
    w.set(BlockPos::new(4, 1, 0), air);
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "type"), "single");
}

#[test]
fn sand_falls_and_concrete_solidifies() {
    let mut w = world!();
    let sand = w.id("sand", &[]);
    w.set(BlockPos::new(0, 5, 0), sand);
    w.tick();
    w.tick();
    assert_eq!(w.get(BlockPos::new(0, 5, 0)), w.b().air());
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::FallingBlock { state, .. } if *state == sand))
    );
    w.floor(0, 2, "stone");
    let water = w.id("water", &[]);
    w.put(BlockPos::new(1, 1, 0), water);
    let powder = w.id("white_concrete_powder", &[]);
    let landing = w.b().falling_block_landed(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(0, 1, 0),
        powder,
        false,
    );
    assert_eq!(
        landing,
        falling::Landing::Placed(w.id("white_concrete", &[]))
    );
}

#[test]
fn water_spreads_on_a_floor() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), water);
    for _ in 0..60 {
        w.tick();
    }
    for d in 1..=7 {
        let pos = BlockPos::new(d, 1, 0);
        assert_eq!(
            w.prop(pos, "level"),
            d.to_string(),
            "flow at distance {d}: {}",
            w.name(pos)
        );
    }
    assert_eq!(w.get(BlockPos::new(8, 1, 0)), w.b().air());
    // Diagonals follow Manhattan distance.
    assert_eq!(w.prop(BlockPos::new(2, 1, 2), "level"), "4");
}

#[test]
fn water_keeps_spreading_over_waterlogged_scaffolding() {
    // `FlowingFluid.canPassThroughWall` reads `getCollisionShape(level, pos)`,
    // where scaffolding is the stable shape: a full top face, so the water above
    // it is no water hole and still spreads sideways.
    let mut w = world!();
    w.floor(0, 10, "stone");
    let stone = w.id("stone", &[]);
    for x in 0..=9 {
        w.put(BlockPos::new(x, 1, -1), stone);
        w.put(BlockPos::new(x, 1, 1), stone);
    }
    let scaffolding = w.id(
        "scaffolding",
        &[
            ("bottom", "false"),
            ("distance", "0"),
            ("waterlogged", "true"),
        ],
    );
    // Stone under it keeps it supported, so its own tick leaves it in place.
    w.put(BlockPos::new(5, -1, 0), stone);
    w.put(BlockPos::new(5, 0, 0), scaffolding);
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), water);
    for _ in 0..80 {
        w.tick();
    }
    for d in 1..=7 {
        let pos = BlockPos::new(d, 1, 0);
        assert_eq!(w.prop(pos, "level"), d.to_string(), "flow at {d}");
    }
}

#[test]
fn bucket_source_removal_drains_spread_water() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let at = BlockPos::new(0, 1, 0);
    assert!(
        w.b()
            .bucket_empty(
                &mut w.level,
                &mut w.ticks,
                at,
                None,
                BucketContent::Fluid(crate::fluid::FluidKind::Water),
                "minecraft:item.bucket.empty",
                false,
                false
            )
            .is_some()
    );
    for _ in 0..60 {
        w.tick();
    }
    assert_ne!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        Some("minecraft:water_bucket")
    );
    for _ in 0..120 {
        w.tick();
    }
    for x in -8..=8 {
        for z in -8..=8 {
            assert_eq!(
                w.get(BlockPos::new(x, 1, z)),
                w.b().air(),
                "water remains at {x}, {z}"
            );
        }
    }
}

#[test]
fn water_falls_and_makes_sources() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 4, 0), water);
    for _ in 0..30 {
        w.tick();
    }
    assert_eq!(
        w.prop(BlockPos::new(0, 3, 0), "level"),
        "8",
        "falling water"
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "level"), "8");
    // Two sources make a third (no holes within slope distance, so the
    // sources spread in every direction).
    let mut w = world!();
    w.floor(0, 12, "stone");
    w.set(BlockPos::new(0, 1, 0), water);
    w.set(BlockPos::new(2, 1, 0), water);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), water);
    // On a small platform the water instead seeks the nearest edges.
    let mut w = world!();
    w.floor(0, 4, "stone");
    w.set(BlockPos::new(0, 1, 0), water);
    w.set(BlockPos::new(2, 1, 0), water);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    assert_eq!(w.prop(BlockPos::new(-1, 1, 0), "level"), "1");
}

#[test]
fn lava_meets_water() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let lava = w.id("lava", &[]);
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), lava);
    w.set(BlockPos::new(1, 1, 0), water);
    assert_eq!(w.get(BlockPos::new(0, 1, 0)), w.id("obsidian", &[]));
    let mut w = world!();
    w.floor(0, 4, "stone");
    w.set(BlockPos::new(0, 1, 0), lava);
    for _ in 0..40 {
        w.tick();
    }
    w.set(BlockPos::new(1, 2, 0), water);
    for _ in 0..40 {
        w.tick();
    }
    assert_eq!(
        w.get(BlockPos::new(1, 1, 0)),
        w.id("cobblestone", &[]),
        "{}",
        w.name(BlockPos::new(1, 1, 0))
    );
}

#[test]
fn waterlogged_blocks_schedule_water() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let slab = w.id("oak_slab", &[("waterlogged", "true")]);
    w.set(BlockPos::new(0, 1, 0), slab);
    // A neighbour shape update schedules the slab's water tick.
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 1, 1), stone);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "level"), "1");
}

#[test]
fn redstone_lever_wire_and_lamp() {
    let mut w = world!();
    w.floor(0, 20, "stone");
    let wire = w.id("redstone_wire", &[]);
    let lamp = w.id("redstone_lamp", &[]);
    let lever = w.id("lever", &[("face", "floor"), ("facing", "north")]);
    for x in 1..=5 {
        w.set(BlockPos::new(x, 1, 0), wire);
    }
    w.set(BlockPos::new(6, 1, 0), lamp);
    w.set(BlockPos::new(0, 1, 0), lever);
    let lever_on = w
        .b()
        .table
        .set_bool(w.get(BlockPos::new(0, 1, 0)), w.b().props.powered, true);
    w.set(BlockPos::new(0, 1, 0), lever_on);
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "power"), "15");
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "power"), "11");
    assert_eq!(w.prop(BlockPos::new(6, 1, 0), "lit"), "true");
    let lever_off = w.b().table.set_bool(lever_on, w.b().props.powered, false);
    w.set(BlockPos::new(0, 1, 0), lever_off);
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "power"), "0");
    for _ in 0..5 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(6, 1, 0), "lit"), "false");
}

#[test]
fn repeater_delays_and_torch_inverts() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let block = w.id("redstone_block", &[]);
    // A repeater takes its input from the side it faces.
    let repeater = w.id("repeater", &[("facing", "west")]);
    w.set(BlockPos::new(1, 1, 0), repeater);
    w.set(BlockPos::new(0, 1, 0), block);
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "powered"), "false");
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "powered"), "true");
    let torch = w.id("redstone_torch", &[]);
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(5, 1, 0), stone);
    w.set(BlockPos::new(5, 2, 0), torch);
    // A redstone block only weakly powers the stone: the torch stays lit.
    w.set(BlockPos::new(6, 1, 0), block);
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(5, 2, 0), "lit"), "true");
    // A lever on the stone strongly powers it.
    let air = w.b().air();
    w.set(BlockPos::new(6, 1, 0), air);
    let lever = w.id(
        "lever",
        &[("face", "wall"), ("facing", "east"), ("powered", "true")],
    );
    w.set(BlockPos::new(6, 1, 0), lever);
    // LeverBlock.pull notifies the attached block's neighbours.
    let lever_block = w.b().block(lever);
    w.b().update_neighbors_at(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(5, 1, 0),
        lever_block,
    );
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(5, 2, 0), "lit"), "false");
}

#[test]
fn crops_grow_and_leaves_decay() {
    let mut w = world!();
    let farmland = w.id("farmland", &[("moisture", "7")]);
    let wheat = w.id("wheat", &[]);
    w.put(BlockPos::new(0, 0, 0), farmland);
    w.put(BlockPos::new(0, 1, 0), wheat);
    for _ in 0..400 {
        w.b()
            .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 1, 0));
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "age"), "7");
    let leaves = w.id("oak_leaves", &[("distance", "7")]);
    w.put(BlockPos::new(5, 5, 5), leaves);
    w.b()
        .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(5, 5, 5));
    assert_eq!(w.get(BlockPos::new(5, 5, 5)), w.b().air());
    let log = w.id("oak_log", &[]);
    let leaf = w.id("oak_leaves", &[]);
    w.set(BlockPos::new(9, 5, 5), leaf);
    w.set(BlockPos::new(8, 5, 5), log);
    w.tick();
    assert_eq!(w.prop(BlockPos::new(9, 5, 5), "distance"), "1");
}

impl World<'_> {
    /// Right-click `item` on the `face` of `clicked`, hitting the face centre
    /// (offset vertically by `dy`), looking with `yaw`/`pitch`.
    fn place(
        &mut self,
        item: &str,
        clicked: BlockPos,
        face: Direction,
        yaw: f32,
        pitch: f32,
        dy: f64,
    ) -> PlaceOutcome {
        let [nx, ny, nz] = face.normal();
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(clicked.x) + 0.5 + f64::from(nx) * 0.5,
            f64::from(clicked.y) + 0.5 + f64::from(ny) * 0.5 + dy,
            f64::from(clicked.z) + 0.5 + f64::from(nz) * 0.5,
        );
        let ctx = PlaceContext {
            clicked_pos: clicked,
            face,
            hit,
            inside: false,
            yaw,
            pitch,
            sneaking: false,
            item: format!("minecraft:{item}"),
            can_use_game_master_blocks: false,
        };
        self.f
            .behaviours
            .place_block(&mut self.level, &mut self.ticks, &ctx)
    }
    fn use_block(&mut self, pos: BlockPos) -> InteractionResult {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: true,
            game_master: false,
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z),
        );
        self.f.behaviours.use_without_item(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
        )
    }
    /// Right-click the north face of `pos` with `item` (or an empty hand),
    /// and which of `useItemOn` and `useWithoutItem` answered.
    fn use_branch(&mut self, pos: BlockPos, item: Option<&str>) -> (InteractionResult, UseBranch) {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: false,
            game_master: false,
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z),
        );
        self.f.behaviours.use_item_on_branch(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
            item,
        )
    }
    /// Right-click the north face of `pos` holding `item` in the main hand.
    fn use_item(&mut self, pos: BlockPos, item: &str) -> InteractionResult {
        self.use_item_on(pos, Some(item), 0.5)
    }
    /// Right-click the north face of `pos` with an empty main hand.
    fn use_empty_hand(&mut self, pos: BlockPos) -> InteractionResult {
        self.use_item_on(pos, None, 0.5)
    }
    /// Right-click the north face of `pos` with an empty main hand, hitting
    /// `dy` blocks above the block's floor.
    fn use_empty_hand_at(&mut self, pos: BlockPos, dy: f64) -> InteractionResult {
        self.use_item_on(pos, None, dy)
    }
    fn use_item_on(&mut self, pos: BlockPos, item: Option<&str>, dy: f64) -> InteractionResult {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: true,
            game_master: false,
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + dy,
            f64::from(pos.z),
        );
        self.f.behaviours.use_item_on(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
            item,
        )
    }
}

#[test]
fn use_on_a_block_says_which_branch_answered() {
    // ServerPlayerGameMode.useItemOn: ITEM_USED_ON_BLOCK when useItemOn
    // consumes the click, DEFAULT_BLOCK_USE when useWithoutItem does.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    // A lever has no useItemOn of its own: it flips with any hand.
    let lever = w.id("lever", &[("face", "floor"), ("facing", "north")]);
    for item in [None, Some("minecraft:stick")] {
        w.put(pos, lever);
        let (result, branch) = w.use_branch(pos, item);
        assert!(result.consumes_action(), "{item:?}");
        assert_eq!(branch, UseBranch::WithoutItem, "{item:?}");
    }
    // RespawnAnchorBlock.useItemOn: glowstone charges it.
    let anchor = w.id("respawn_anchor", &[("charges", "0")]);
    w.put(pos, anchor);
    assert_eq!(
        w.use_branch(pos, Some("minecraft:glowstone")),
        (InteractionResult::Success, UseBranch::ItemOn)
    );
    // CandleBlock.useItemOn: an empty hand snuffs a lit candle.
    let candle = w.id("candle", &[("candles", "1"), ("lit", "true")]);
    w.put(pos, candle);
    assert_eq!(
        w.use_branch(pos, None),
        (InteractionResult::Success, UseBranch::ItemOn)
    );
    // SweetBerryBushBlock.useItemOn: bone meal on a growing bush passes on
    // to the item without trying the empty hand.
    let bush = w.id("sweet_berry_bush", &[("age", "0")]);
    w.put(pos, bush);
    assert_eq!(
        w.use_branch(pos, Some("minecraft:bone_meal")),
        (InteractionResult::Pass, UseBranch::ItemOn)
    );
}

#[test]
fn glowstone_charges_a_respawn_anchor_up_to_four() {
    // RespawnAnchorBlock.useItemOn: glowstone on an anchor below 4 charges
    // charges it and the click ends (SUCCESS), so no glowstone is placed (#3034).
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let empty = w.id("respawn_anchor", &[("charges", "0")]);
    w.put(pos, empty);
    for charges in 1..=4 {
        assert_eq!(
            w.use_item(pos, "minecraft:glowstone"),
            InteractionResult::Success
        );
        assert_eq!(w.prop(pos, "charges"), charges.to_string());
    }
    // A full anchor can't be charged: TRY_WITH_EMPTY_HAND, then useWithoutItem
    // (the respawn point), which leaves the charges alone.
    w.use_item(pos, "minecraft:glowstone");
    assert_eq!(w.prop(pos, "charges"), "4");
    // Anything else on an empty anchor passes, so the item is used instead.
    w.put(pos, empty);
    assert_eq!(w.use_item(pos, "minecraft:stone"), InteractionResult::Pass);
    assert_eq!(w.prop(pos, "charges"), "0");
}

#[test]
fn mining_lights_redstone_ore_until_its_random_tick() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    for ore in ["redstone_ore", "deepslate_redstone_ore"] {
        let unlit = w.id(ore, &[("lit", "false")]);
        let lit = w.id(ore, &[("lit", "true")]);
        w.put(pos, unlit);
        w.b().attack_block(&mut w.level, &mut w.ticks, pos);
        assert_eq!(w.get(pos), lit);
        w.b().attack_block(&mut w.level, &mut w.ticks, pos);
        assert_eq!(w.get(pos), lit);
        w.b().random_tick(&mut w.level, &mut w.ticks, pos);
        assert_eq!(w.get(pos), unlit);
    }
    let stone = w.id("stone", &[]);
    w.put(pos, stone);
    w.b().attack_block(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.get(pos), stone);
}

#[test]
fn placement_orients_blocks() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let ground = BlockPos::new(0, 0, 0);
    // Looking north (yaw 180) at the top of the floor.
    w.place("oak_stairs", ground, Direction::Up, 180.0, 30.0, 0.0);
    assert_eq!(
        w.name(BlockPos::new(0, 1, 0)),
        "oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]"
    );
    w.place(
        "oak_log",
        BlockPos::new(1, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    w.place(
        "oak_log",
        BlockPos::new(1, 1, 0),
        Direction::East,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(2, 1, 0), "axis"), "x");
    // Torch against the log's east face becomes a wall torch facing east.
    w.place(
        "torch",
        BlockPos::new(2, 1, 0),
        Direction::East,
        270.0,
        0.0,
        0.0,
    );
    assert_eq!(w.name(BlockPos::new(3, 1, 0)), "wall_torch[facing=east]");
    // Furnaces face the player; observers face where the player looks.
    w.place(
        "furnace",
        BlockPos::new(-2, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-2, 1, 0), "facing"), "north");
    w.place(
        "observer",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 1, 0), "facing"), "south");
}

#[test]
fn placement_merges_slabs_and_stacks() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let ground = BlockPos::new(0, 0, 0);
    w.place("oak_slab", ground, Direction::Up, 0.0, 60.0, 0.0);
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "type"), "bottom");
    w.place(
        "oak_slab",
        BlockPos::new(0, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.5,
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "type"), "double");
    // Clicking the upper half of a side face places a top slab.
    w.place(
        "oak_slab",
        BlockPos::new(0, 1, 0),
        Direction::East,
        0.0,
        0.0,
        0.25,
    );
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "type"), "top");
    w.place(
        "snow",
        BlockPos::new(3, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.place(
        "snow",
        BlockPos::new(3, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.375,
    );
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "layers"), "2");
    w.place(
        "candle",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.place(
        "candle",
        BlockPos::new(-3, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.4,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 1, 0), "candles"), "2");
}

#[test]
fn doors_beds_and_plants_place_both_halves() {
    let mut w = world!();
    w.floor(0, 6, "grass_block");
    let ground = BlockPos::new(0, 0, 0);
    w.place("oak_door", ground, Direction::Up, 180.0, 30.0, 0.0);
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "half"), "lower");
    assert_eq!(w.prop(BlockPos::new(0, 2, 0), "half"), "upper");
    assert_eq!(
        w.use_block(BlockPos::new(0, 1, 0)),
        InteractionResult::Success
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "open"), "true");
    assert_eq!(
        w.prop(BlockPos::new(0, 2, 0), "open"),
        "true",
        "the upper half follows"
    );
    w.place(
        "red_bed",
        BlockPos::new(3, 0, 0),
        Direction::Up,
        180.0,
        30.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "part"), "foot");
    assert_eq!(w.prop(BlockPos::new(3, 1, -1), "part"), "head");
    w.place(
        "sunflower",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        30.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 2, 0), "half"), "upper");
    // Iron doors cannot be opened by hand.
    w.place(
        "iron_door",
        BlockPos::new(5, 0, 0),
        Direction::Up,
        180.0,
        30.0,
        0.0,
    );
    assert_eq!(w.use_block(BlockPos::new(5, 1, 0)), InteractionResult::Pass);
    // A placement into a solid block fails.
    assert_eq!(
        w.place(
            "stone",
            BlockPos::new(0, 0, 0),
            Direction::West,
            0.0,
            0.0,
            0.0
        ),
        PlaceOutcome::Failed
    );
}

#[test]
fn levers_power_wire_through_interaction() {
    let mut w = world!();
    w.floor(0, 8, "stone");
    w.place(
        "lever",
        BlockPos::new(0, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    for x in 1..=3 {
        w.place(
            "redstone",
            BlockPos::new(x, 0, 0),
            Direction::Up,
            0.0,
            60.0,
            0.0,
        );
    }
    assert_eq!(
        w.use_block(BlockPos::new(0, 1, 0)),
        InteractionResult::Success
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "powered"), "true");
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "power"), "13");
    w.use_block(BlockPos::new(0, 1, 0));
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "power"), "0");
    // Buttons release after their pressed time.
    w.place(
        "stone_button",
        BlockPos::new(5, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.use_block(BlockPos::new(5, 1, 0));
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "powered"), "true");
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "powered"), "false");
}

#[test]
fn pistons_push_and_sticky_pistons_pull() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let dirt = w.id("dirt", &[]);
    let air = w.b().air();
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), dirt);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "true");
    assert_eq!(
        w.name(BlockPos::new(1, 1, 0)),
        "piston_head[facing=east,short=false,type=sticky]"
    );
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), dirt);
    assert!(w.ticks.moving_pistons.is_empty());
    w.set(BlockPos::new(-1, 1, 0), air);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "false");
    assert_eq!(
        w.get(BlockPos::new(1, 1, 0)),
        dirt,
        "the sticky piston pulled the block back"
    );
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), air);
}

#[test]
fn tnt_duper_3856_server_still_primes_each_powered_tnt() {
    let mut w = world!();
    let tnt = w.id("tnt", &[]);
    let power = w.id("redstone_block", &[]);
    for pos in [BlockPos::new(0, 1, 0), BlockPos::new(0, 1, 3)] {
        w.put(pos.offset(1, 0, 0), power);
        w.set(pos, tnt);
        assert_eq!(w.get(pos), w.b().air());
        assert_eq!(
            w.level
                .events
                .iter()
                .filter(|e| matches!(e, LevelEvent::PrimeTnt { pos: p, .. } if *p == pos))
                .count(),
            1,
        );
    }
}

/// Breaking a wall torch next to a BUD-powered TNT updates the old block while
/// the piston already holds its saved state. This vanilla duplication must
/// continue to work on the server.
#[test]
fn tnt_duper_3856_piston_keeps_original_and_primes_one_tnt() {
    let Some(f) = fixture() else {
        return;
    };
    let mut w = World::new(f);
    let base = BlockPos::new(0, 2, 0);
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let slime = w.id("slime_block", &[]);
    let tnt = w.id("tnt", &[]);
    let torch = w.id("wall_torch", &[("facing", "north")]);
    let power = w.id("redstone_block", &[]);
    w.put(base, piston);
    w.put(base.offset(1, 0, 0), slime);
    w.put(base.offset(2, 0, 0), slime);
    w.put(base.offset(2, -1, 0), slime);
    w.put(base.offset(2, -1, -1), tnt);
    w.put(base.offset(2, 0, -1), torch);
    // Raw writes model a powered TNT that has not received an update yet.
    w.put(base.offset(2, -1, -2), power);
    w.put(base.offset(-1, 0, 0), power);
    f.behaviours.handle_neighbor_changed(
        &mut w.level,
        &mut w.ticks,
        base,
        f.behaviours.block(power),
    );
    f.behaviours.run_block_events(&mut w.level, &mut w.ticks);
    let primed = w
        .level
        .events
        .iter()
        .filter(|e| matches!(e, LevelEvent::PrimeTnt { .. }))
        .count();
    assert_eq!(primed, 1);
    for _ in 0..10 {
        w.level.time += 1;
        f.behaviours.tick_block_entities(&mut w.level, &mut w.ticks);
    }
    assert_eq!(
        w.get(base.offset(3, -1, -1)),
        tnt,
        "original must survive on the server"
    );
    assert_eq!(
        w.level
            .events
            .iter()
            .filter(|e| matches!(e, LevelEvent::PrimeTnt { .. }))
            .count(),
        1
    );
}
/// `ServerLevel.runBlockEvents` reports a handled event for the clients
/// (`ClientboundBlockEventPacket`), carrying the block the event named.
#[test]
fn a_handled_block_event_reaches_the_clients() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(-1, 1, 0), block);
    w.level.events.clear();
    w.full_tick();
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::BlockEntityEvent {
            pos,
            state,
            a: 0,
            ..
        } if *pos == BlockPos::new(0, 1, 0) && *state == piston
    )));
}

#[test]
fn dripstone_fills_cauldrons() {
    let mut w = world!();
    let block = w.id("dripstone_block", &[]);
    let water = w.id("water", &[]);
    let tip = w.id(
        "pointed_dripstone",
        &[("vertical_direction", "down"), ("thickness", "tip")],
    );
    let cauldron = w.id("cauldron", &[]);
    w.put(BlockPos::new(0, 11, 0), water);
    w.put(BlockPos::new(0, 10, 0), block);
    w.put(BlockPos::new(0, 9, 0), tip);
    w.put(BlockPos::new(0, 5, 0), cauldron);
    for _ in 0..100 {
        w.b()
            .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 9, 0));
    }
    for _ in 0..80 {
        w.tick();
    }
    assert_eq!(w.name(BlockPos::new(0, 5, 0)), "water_cauldron[level=1]");
}

#[test]
fn sculk_sensor_phases_and_signal() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let pos = BlockPos::new(0, 1, 0);
    let sensor = w.id("sculk_sensor", &[]);
    w.set(pos, sensor);
    assert!(w.b().sculk_sensor_can_activate(w.get(pos)));
    w.f.behaviours
        .sculk_sensor_activate(&mut w.level, &mut w.ticks, pos, 11);
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "active");
    assert_eq!(w.prop(pos, "power"), "11");
    // The sensor powers the block below it from the top.
    let below = BlockPos::new(0, 0, 0);
    assert_eq!(
        w.f.behaviours
            .best_neighbor_signal(&mut w.level, &mut w.ticks, below),
        11,
        "the block under an active sensor receives its power"
    );
    for _ in 0..29 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "active");
    w.tick();
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "cooldown");
    assert_eq!(w.prop(pos, "power"), "0");
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "inactive");
}

#[test]
fn calibrated_sensor_is_active_for_ten_ticks() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let pos = BlockPos::new(0, 1, 0);
    let sensor = w.id("calibrated_sculk_sensor", &[("facing", "north")]);
    w.set(pos, sensor);
    w.f.behaviours
        .sculk_sensor_activate(&mut w.level, &mut w.ticks, pos, 4);
    for _ in 0..9 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "active");
    w.tick();
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "cooldown");
    // The block behind (south) feeds the calibration signal.
    let source = w.id("redstone_block", &[]);
    w.set(BlockPos::new(0, 1, 1), source);
    assert_eq!(
        w.f.behaviours
            .calibrated_back_signal(&mut w.level, &mut w.ticks, pos),
        15
    );
}

#[test]
fn sculk_charge_spreads_veins_and_sculk() {
    let mut w = world!();
    w.floor(0, 12, "stone");
    w.floor(-1, 12, "stone");
    let origin = BlockPos::new(0, 1, 0);
    let catalyst = w.id("sculk_catalyst", &[]);
    w.put(origin, catalyst);
    let mut spreader = sculk::SculkSpreader::default();
    spreader.add_cursors(BlockPos::new(2, 1, 0), 40);
    assert_eq!(spreader.cursors.len(), 1);
    for _ in 0..80 {
        w.f.behaviours.sculk_update_cursors(
            &mut w.level,
            &mut w.ticks,
            &mut spreader,
            origin,
            true,
        );
    }
    let sculk = w.id("sculk", &[]);
    let found = (-12..=12)
        .flat_map(|x| (-12..=12).map(move |z| BlockPos::new(x, 0, z)))
        .filter(|p| w.get(*p) == sculk)
        .count();
    assert!(found > 0, "the charge turned stone into sculk");
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::LevelEvent { id: 3006, .. })),
        "charge particles are reported"
    );
}

#[test]
fn shrieker_responds_when_the_shriek_ends() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let pos = BlockPos::new(0, 1, 0);
    let shrieker = w.id("sculk_shrieker", &[("can_summon", "true")]);
    w.set(pos, shrieker);
    w.f.behaviours
        .sculk_shrieker_shriek(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.prop(pos, "shrieking"), "true");
    for _ in 0..89 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "shrieking"), "true");
    w.tick();
    assert_eq!(w.prop(pos, "shrieking"), "false");
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::BlockEvent {
            kind: "shrieker_respond",
            ..
        }
    )));
}

#[test]
fn catalyst_bloom_lasts_eight_ticks() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let catalyst = w.id("sculk_catalyst", &[]);
    w.set(pos, catalyst);
    w.f.behaviours
        .sculk_catalyst_bloom(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.prop(pos, "bloom"), "true");
    for _ in 0..7 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "bloom"), "true");
    w.tick();
    assert_eq!(w.prop(pos, "bloom"), "false");
}

#[test]
fn placing_and_breaking_report_the_affected_state() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let wool = w.id("white_wool", &[]);
    let placed = w.place(
        "white_wool",
        BlockPos::new(0, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert!(matches!(placed, PlaceOutcome::Placed { .. }));
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::GameEvent { event: "minecraft:block_place", state: Some(s), .. } if *s == wool
    )));
    w.level.events.clear();
    w.f.behaviours
        .destroy_block(&mut w.level, &mut w.ticks, BlockPos::new(0, 1, 0), false);
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::GameEvent { event: "minecraft:block_destroy", state: Some(s), .. } if *s == wool
    )));
}

#[test]
fn dispenser_buckets_pick_up_and_empty() {
    let mut w = world!();
    let at = BlockPos::new(0, 10, 0);
    // A dispenser has no hit result: nothing beside to try.
    let empty = |w: &mut World, content, sound: &str, creative| {
        w.f.behaviours
            .bucket_empty(
                &mut w.level,
                &mut w.ticks,
                at,
                None,
                content,
                sound,
                false,
                creative,
            )
            .is_some()
    };
    let water = w.id("water", &[("level", "0")]);
    w.put(at, water);
    let picked = w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false);
    assert_eq!(picked, Some("minecraft:water_bucket"));
    assert_eq!(w.get(at), w.b().air());
    // Flowing water gives nothing.
    let flowing = w.id("water", &[("level", "3")]);
    w.put(at, flowing);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        None
    );
    // A waterlogged slab is drained and stays a slab.
    let slab = w.id("oak_slab", &[("waterlogged", "true")]);
    w.put(at, slab);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        Some("minecraft:water_bucket")
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    // Powder snow comes up as its bucket.
    let snow = w.id("powder_snow", &[]);
    w.put(at, snow);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        Some("minecraft:powder_snow_bucket")
    );
    // A barrier only a creative player fills or drains.
    let barrier = w.id("barrier", &[]);
    w.put(at, barrier);
    let sound = "minecraft:item.bucket.empty";
    let content = BucketContent::Fluid(crate::fluid::FluidKind::Water);
    assert!(!empty(&mut w, content, sound, false));
    assert_eq!(w.prop(at, "waterlogged"), "false");
    assert!(empty(&mut w, content, sound, true));
    assert_eq!(w.prop(at, "waterlogged"), "true");
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        None
    );
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, true),
        Some("minecraft:water_bucket")
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    // Emptying water into air makes a source; stone refuses.
    let air = w.b().air();
    w.put(at, air);
    assert!(empty(&mut w, content, sound, false));
    assert_eq!(w.name(at), "water[level=0]");
    let stone = w.id("stone", &[]);
    w.put(at, stone);
    assert!(!empty(&mut w, content, sound, false));
    // Water fills a waterloggable slab instead of replacing it.
    let dry = w.id("oak_slab", &[]);
    w.put(at, dry);
    assert!(empty(&mut w, content, sound, false));
    assert_eq!(w.prop(at, "waterlogged"), "true");
    // Powder snow only goes into empty space.
    let air = w.b().air();
    w.put(at, air);
    assert!(empty(
        &mut w,
        BucketContent::Powder,
        "minecraft:item.bucket.empty_powder_snow",
        false
    ));
    assert_eq!(w.name(at), "powder_snow[]");
}

#[test]
fn a_bucket_replaces_a_bubble_column_and_breaks_a_flower() {
    let mut w = world!();
    let at = BlockPos::new(0, 10, 0);
    let sound = "minecraft:item.bucket.empty";
    let water = BucketContent::Fluid(crate::fluid::FluidKind::Water);
    let pour = |w: &mut World| {
        w.f.behaviours.bucket_empty(
            &mut w.level,
            &mut w.ticks,
            at,
            None,
            water,
            sound,
            false,
            false,
        )
    };
    let broke = |w: &World| {
        w.level.events.iter().any(|e| {
            matches!(
                e,
                LevelEvent::GameEvent {
                    event: "minecraft:block_destroy",
                    ..
                } | LevelEvent::LevelEvent { id: 2001, .. }
            )
        })
    };
    // `BlockState.liquid()` is true: the column is replaced, not broken.
    w.put(at, w.id("bubble_column", &[("drag", "true")]));
    w.level.events.clear();
    assert_eq!(pour(&mut w), Some(at));
    assert_eq!(w.name(at), "water[level=0]");
    assert!(!broke(&w), "a bubble column is liquid()");
    // A replaceable plant is not `liquid()`: the bucket breaks it first.
    w.put(at, w.id("dandelion", &[]));
    w.level.events.clear();
    assert_eq!(pour(&mut w), Some(at));
    assert_eq!(w.name(at), "water[level=0]");
    assert!(broke(&w), "a flower is broken before the water goes in");
}

#[test]
fn clicked_bucket_pours_beside_a_refusing_block() {
    let mut w = world!();
    let at = BlockPos::new(0, 10, 0);
    let above = BlockPos::new(0, 11, 0);
    let sound = "minecraft:item.bucket.empty";
    let content = BucketContent::Fluid(crate::fluid::FluidKind::Water);
    let air = w.b().air();
    let click = |w: &mut World, sneaking: bool| {
        w.f.behaviours.bucket_empty(
            &mut w.level,
            &mut w.ticks,
            at,
            Some(above),
            content,
            sound,
            sneaking,
            false,
        )
    };
    // Sneak-clicking a slab pours beside it, where Shift no longer counts.
    let slab = w.id("oak_slab", &[]);
    w.put(at, slab);
    assert_eq!(click(&mut w, true), Some(above));
    assert_eq!(w.prop(at, "waterlogged"), "false");
    assert_eq!(w.name(above), "water[level=0]");
    // A double slab holds no water: the click fills the block beside it.
    w.put(above, air);
    let double = w.id("oak_slab", &[("type", "double")]);
    w.put(at, double);
    assert_eq!(click(&mut w, false), Some(above));
    assert_eq!(w.prop(at, "type"), "double");
    assert_eq!(w.name(above), "water[level=0]");
    // Both refusing empties nothing.
    let stone = w.id("stone", &[]);
    w.put(above, stone);
    assert_eq!(click(&mut w, false), None);
    assert_eq!(w.prop(at, "type"), "double");
    assert_eq!(w.get(above), stone);
}

#[test]
fn creaking_heart_follows_its_logs_and_the_night() {
    let mut w = world!();
    let heart_at = BlockPos::new(0, 10, 0);
    let log = |w: &World, axis: &str| w.id("pale_oak_log", &[("axis", axis)]);
    let heart = w.id(
        "creaking_heart",
        &[("axis", "y"), ("creaking_heart_state", "uprooted")],
    );
    let (below, above) = (BlockPos::new(0, 9, 0), BlockPos::new(0, 11, 0));
    w.put(heart_at, heart);
    w.put(below, log(&w, "y"));
    // A log along another axis does not count.
    w.put(above, log(&w, "x"));
    assert!(
        !w.b()
            .creaking_heart_has_logs(&mut w.level, &mut w.ticks, heart_at, heart)
    );
    // Day: a log update wakes the uprooted heart to dormant.
    let up = log(&w, "y");
    w.set(above, up);
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.prop(heart_at, "creaking_heart_state"), "dormant");
    // Night: the same trunk wakes a fresh heart up.
    w.level.day_time = 14000;
    w.put(heart_at, heart);
    w.put(above, w.b().air());
    w.set(above, up);
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.prop(heart_at, "creaking_heart_state"), "awake");
    // Placed without logs it stays uprooted.
    w.put(above, w.b().air());
    w.put(heart_at, heart);
    let air = w.b().air();
    w.set(above, air);
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.prop(heart_at, "creaking_heart_state"), "uprooted");
}

#[test]
fn player_breaks_fire_with_extinguish_not_generic_break_event() {
    for name in ["fire", "soul_fire", "white_wool"] {
        let mut w = world!();
        let pos = BlockPos::new(0, 1, 0);
        let support = w.id("soul_soil", &[]);
        w.put(BlockPos::new(0, 0, 0), support);
        let state = w.id(name, &[]);
        w.put(pos, state);
        w.level.events.clear();
        assert!(
            w.f.behaviours
                .player_destroy_block(&mut w.level, &mut w.ticks, pos, false, true)
        );
        let is_fire = name != "white_wool";
        assert_eq!(
            w.level
                .events
                .iter()
                .any(|e| matches!(e,LevelEvent::LevelEvent {id:1009,pos:p,..} if *p==pos)),
            is_fire,
            "extinguish for {name}"
        );
        assert_eq!(
            w.level
                .events
                .iter()
                .any(|e| matches!(e,LevelEvent::LevelEvent {id:2001,pos:p,..} if *p==pos)),
            !is_fire,
            "generic break for {name}"
        );
        assert_eq!(w.get(pos), w.b().air());
    }
}

#[test]
fn big_dripleaf_tilts_under_an_entity_standing_on_it() {
    let mut w = world!();
    w.floor(0, 2, "clay");
    let pos = BlockPos::new(0, 1, 0);
    let leaf = w.id("big_dripleaf", &[("facing", "north")]);
    w.put(pos, leaf);
    let stand = |w: &mut World, on_ground: bool, feet: f64| {
        w.f.behaviours
            .entity_on_big_dripleaf(&mut w.level, &mut w.ticks, pos, on_ground, feet);
    };
    // canEntityTilt: in the air, or with its feet at or under 0.6875, nothing.
    stand(&mut w, false, 1.9375);
    stand(&mut w, true, 1.6875);
    assert_eq!(w.prop(pos, "tilt"), "none");
    // Standing on the leaf's top (15/16): UNSTABLE, then PARTIAL and FULL
    // 10 ticks apart, and back to NONE 100 ticks later.
    stand(&mut w, true, 1.9375);
    assert_eq!(w.prop(pos, "tilt"), "unstable");
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "tilt"), "partial");
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "tilt"), "full");
    // A tilted leaf doesn't restart under the entity.
    stand(&mut w, true, 1.9375);
    assert_eq!(w.prop(pos, "tilt"), "full");
    for _ in 0..100 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "tilt"), "none");
    // A powered leaf doesn't tilt.
    let power = w.id("redstone_block", &[]);
    w.put(BlockPos::new(1, 1, 0), power);
    stand(&mut w, true, 1.9375);
    assert_eq!(w.prop(pos, "tilt"), "none");
}

#[test]
fn using_storage_and_interactable_blocks_succeeds() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("chest", &[("facing", "north"), ("type", "single")]),
        ("trapped_chest", &[("facing", "north"), ("type", "single")]),
        ("barrel", &[("facing", "up")]),
        ("ender_chest", &[("facing", "north")]),
        ("shulker_box", &[("facing", "up")]),
        ("dispenser", &[("facing", "north")]),
        ("dropper", &[("facing", "north")]),
        ("hopper", &[("facing", "down")]),
        ("crafter", &[("orientation", "north_up")]),
        ("furnace", &[("facing", "north")]),
        ("blast_furnace", &[("facing", "north")]),
        ("smoker", &[("facing", "north")]),
        ("brewing_stand", &[]),
        ("beacon", &[]),
        ("decorated_pot", &[]),
        ("jukebox", &[("has_record", "true")]),
    ];
    for &(name, props) in cases {
        w.put(pos, w.id(name, props));
        assert_eq!(
            w.use_block(pos),
            InteractionResult::Success,
            "{name} should succeed (client swing)"
        );
    }
    w.put(pos, w.id("jukebox", &[("has_record", "false")]));
    assert_eq!(
        w.use_block(pos),
        InteractionResult::Pass,
        "an empty jukebox passes"
    );
}

#[test]
fn open_shulker_box_supports_only_through_its_base_plate() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    for facing in Direction::ALL {
        let prop = match facing {
            Direction::Down => "down",
            Direction::Up => "up",
            Direction::North => "north",
            Direction::South => "south",
            Direction::West => "west",
            Direction::East => "east",
        };
        w.put(pos, w.id("shulker_box", &[("facing", prop)]));
        w.level.shulker_closed.remove(&pos);
        // A shut box, and a box whose block entity the level doesn't keep,
        // both read as a closed full cube.
        {
            let sim = w.b().sim(&mut w.level, &mut w.ticks);
            assert!(
                sim.sturdy(pos, Direction::Up),
                "closed box facing {facing:?}"
            );
            assert!(
                sim.sturdy(pos, Direction::Down),
                "closed box facing {facing:?}"
            );
            assert!(
                sim.can_support_rigid_block(pos),
                "closed box facing {facing:?}"
            );
            assert!(
                sim.can_support_center(pos, Direction::Up),
                "closed box facing {facing:?}"
            );
        }

        w.level.shulker_closed.insert(pos, false);
        let plate = facing.opposite();
        let sim = w.b().sim(&mut w.level, &mut w.ticks);
        assert!(
            sim.sturdy(pos, plate),
            "open box facing {facing:?} keeps its base plate"
        );
        assert!(
            !sim.sturdy(pos, facing),
            "open box facing {facing:?} has no support on its lid side"
        );
        assert_eq!(
            sim.can_support_rigid_block(pos),
            plate == Direction::Up,
            "open box facing {facing:?}"
        );
        assert!(
            sim.can_support_center(pos, plate),
            "open box facing {facing:?} base plate"
        );
        assert!(
            !sim.can_support_center(pos, facing),
            "open box facing {facing:?} lid side"
        );
    }
}

#[test]
fn torch_survives_on_a_box_opening_down_but_not_on_an_open_lid() {
    let mut w = world!();
    let box_pos = BlockPos::new(0, 1, 0);
    let torch_pos = BlockPos::new(0, 2, 0);
    w.put(torch_pos, w.id("torch", &[]));
    let torch = w.get(torch_pos);
    // Facing up, the lid is the top face: open, the torch above loses support.
    w.put(box_pos, w.id("shulker_box", &[("facing", "up")]));
    assert!(
        w.b()
            .sim(&mut w.level, &mut w.ticks)
            .can_survive(torch, torch_pos),
        "a torch stands on a closed box"
    );
    w.level.shulker_closed.insert(box_pos, false);
    assert!(
        !w.b()
            .sim(&mut w.level, &mut w.ticks)
            .can_survive(torch, torch_pos),
        "an open lid doesn't hold a torch"
    );
    // Facing down, the base plate is the top face, so the torch stands.
    w.put(box_pos, w.id("shulker_box", &[("facing", "down")]));
    assert!(
        w.b()
            .sim(&mut w.level, &mut w.ticks)
            .can_survive(torch, torch_pos),
        "the base plate of a box opening down still holds a torch"
    );
}

#[test]
fn creative_breaking_piston_head_does_not_drop_piston() {
    for (piston_name, head_type) in [("piston", "normal"), ("sticky_piston", "sticky")] {
        let mut w = world!();
        let base_pos = BlockPos::new(0, 1, 0);
        let head_pos = BlockPos::new(1, 1, 0);
        let base = w.id(piston_name, &[("extended", "true"), ("facing", "east")]);
        let head = w.id(
            "piston_head",
            &[("type", head_type), ("facing", "east"), ("short", "false")],
        );
        w.put(base_pos, base);
        w.put(head_pos, head);
        w.level.events.clear();

        // Breaking the head in creative mode. Destroying the base first makes the
        // head's shape update remove it, so its own removeBlock finds air.
        w.f.behaviours
            .player_destroy_block(&mut w.level, &mut w.ticks, head_pos, true, true);

        // Both head and base must be removed:
        assert_eq!(w.get(head_pos), w.b().air(), "head should become air");
        assert_eq!(w.get(base_pos), w.b().air(), "base should become air");

        // Must NOT drop the base: the head's own removal (its shape update) drops
        // its empty loot, as vanilla's updateOrDestroy does.
        assert!(
            !w.level
                .events
                .iter()
                .any(|e| matches!(e, LevelEvent::DropResources { pos, .. } if *pos == base_pos)),
            "creative mode breaking {head_type} piston head must not drop the base piston"
        );
    }
}

#[test]
fn survival_breaking_piston_head_drops_piston() {
    for (piston_name, head_type) in [("piston", "normal"), ("sticky_piston", "sticky")] {
        let mut w = world!();
        let base_pos = BlockPos::new(0, 1, 0);
        let head_pos = BlockPos::new(1, 1, 0);
        let base = w.id(piston_name, &[("extended", "true"), ("facing", "east")]);
        let head = w.id(
            "piston_head",
            &[("type", head_type), ("facing", "east"), ("short", "false")],
        );
        w.put(base_pos, base);
        w.put(head_pos, head);
        w.level.events.clear();

        // Breaking the head in survival mode:
        assert!(
            w.f.behaviours
                .player_destroy_block(&mut w.level, &mut w.ticks, head_pos, false, true),
            "head should be destroyed"
        );

        // Both head and base must be removed:
        assert_eq!(w.get(head_pos), w.b().air(), "head should become air");
        assert_eq!(w.get(base_pos), w.b().air(), "base should become air");

        // Must drop the base piston at base_pos:
        assert!(
            w.level
                .events
                .iter()
                .any(|e| matches!(e, LevelEvent::DropResources { pos, .. } if *pos == base_pos)),
            "survival mode breaking {head_type} piston head must drop base piston"
        );
    }
}

/// A lever on the back of a piston stays through extend and retract.
#[test]
fn lever_on_piston_back_survives_extend_and_retract() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    for kind in ["piston", "sticky_piston"] {
        let piston = w.id(kind, &[("facing", "east")]);
        let lever = w.id("lever", &[("face", "wall"), ("facing", "west")]);
        w.set(BlockPos::new(0, 1, 0), piston);
        w.set(BlockPos::new(-1, 1, 0), lever);
        let on = w.b().table.set_bool(lever, w.b().props.powered, true);
        w.set(BlockPos::new(-1, 1, 0), on);
        for _ in 0..4 {
            w.full_tick();
        }
        assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "true", "{kind}");
        assert_eq!(
            w.name(BlockPos::new(-1, 1, 0)),
            "lever[face=wall,facing=west,powered=true]",
            "{kind}"
        );
        w.set(BlockPos::new(-1, 1, 0), lever);
        for _ in 0..4 {
            w.full_tick();
        }
        assert_eq!(
            w.prop(BlockPos::new(0, 1, 0), "extended"),
            "false",
            "{kind}"
        );
        assert_eq!(
            w.name(BlockPos::new(-1, 1, 0)),
            "lever[face=wall,facing=west,powered=false]",
            "{kind}"
        );
    }
}

#[test]
fn dust_on_a_downward_piston_survives_extend_and_retract() {
    // PistonBaseBlock.triggerEvent writes the retracting base as a
    // `moving_piston` and still updates the neighbours around it, so the wire
    // above the base is asked whether it can survive. Vanilla reads that block
    // through its block entity, whose collision shape holds the extended base;
    // the exported support facts see no block entity and call every face
    // unsupported, which broke the dust (#4728).
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "down")]);
    let power = w.id("redstone_block", &[]);
    let wire = w.id("redstone_wire", &[]);
    let air = w.b().air();
    let base = BlockPos::new(0, 3, 0);
    let dust = BlockPos::new(0, 4, 0);
    w.set(base, piston);
    w.set(dust, wire);
    w.set(BlockPos::new(1, 3, 0), power);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(base, "extended"), "true");
    assert!(
        w.name(BlockPos::new(0, 2, 0)).starts_with("piston_head"),
        "the piston pushed its head down"
    );
    assert!(
        w.name(dust).starts_with("redstone_wire"),
        "the dust stays on the extended piston, got {}",
        w.name(dust)
    );
    w.set(BlockPos::new(1, 3, 0), air);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(base, "extended"), "false");
    assert!(
        w.name(dust).starts_with("redstone_wire"),
        "the dust stays through the retract, got {}",
        w.name(dust)
    );
}

#[test]
fn diodes_on_downward_pistons_survive_retraction() {
    // DiodeBlock.canSurviveOn asks `isFaceSturdy(level, pos, UP, RIGID)` of
    // the block below, both in canSurvive and in updateShape with the
    // neighbour's new state. While the base retracts it is a moving_piston,
    // whose block entity shape holds the extended base: a full top face
    // (#12019).
    for base_kind in ["piston", "sticky_piston"] {
        for diode_kind in ["repeater", "comparator"] {
            let mut w = world!();
            w.floor(0, 10, "stone");
            let base = BlockPos::new(0, 3, 0);
            let pos = BlockPos::new(0, 4, 0);
            let power = BlockPos::new(1, 3, 0);
            let diode = w.id(diode_kind, &[("facing", "south")]);
            w.set(base, w.id(base_kind, &[("facing", "down")]));
            w.set(pos, diode);
            w.set(power, w.id("redstone_block", &[]));
            for _ in 0..4 {
                w.full_tick();
            }
            assert_eq!(w.prop(base, "extended"), "true");
            w.set(power, w.b().air());
            for _ in 0..4 {
                w.full_tick();
                assert_eq!(w.get(pos), diode, "{diode_kind} on {base_kind}");
            }
            assert_eq!(w.prop(base, "extended"), "false");
        }
    }
}

#[test]
fn a_candle_on_an_uneaten_cake_makes_a_candle_cake() {
    // CakeBlock.useItemOn: any candle on a cake with no bites turns it into the
    // matching unlit candle cake; a bitten cake does not.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let cake = w.id("cake", &[("bites", "0")]);
    for (item, block) in [
        ("minecraft:candle", "candle_cake"),
        ("minecraft:red_candle", "red_candle_cake"),
    ] {
        w.put(pos, cake);
        assert_eq!(w.use_item(pos, item), InteractionResult::Success);
        assert_eq!(w.get(pos), w.id(block, &[("lit", "false")]));
    }
    let bitten = w.id("cake", &[("bites", "1")]);
    w.put(pos, bitten);
    w.use_item(pos, "minecraft:candle");
    assert_ne!(w.get(pos), w.id("candle_cake", &[("lit", "false")]));
    w.put(pos, cake);
    // Anything else answers TRY_WITH_EMPTY_HAND, and ServerPlayerGameMode
    // then calls useWithoutItem with whatever the main hand holds, so a stone
    // eats a slice of the cake instead of adding a candle to it.
    assert_eq!(
        w.use_item(pos, "minecraft:stone"),
        InteractionResult::Success
    );
    assert_eq!(w.get(pos), bitten);
}

#[test]
fn water_douses_a_lit_campfire_with_the_extinguish_sound() {
    // CampfireBlock.placeLiquid: a lit campfire that takes water goes out
    // and plays GENERIC_EXTINGUISH_FIRE, `entity.generic.extinguish_fire`.
    let mut w = world!();
    let at = BlockPos::new(0, 10, 0);
    w.put(at, w.id("campfire", &[("lit", "true")]));
    w.level.events.clear();
    let placed = w.f.behaviours.bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        None,
        BucketContent::Fluid(crate::fluid::FluidKind::Water),
        "minecraft:item.bucket.empty",
        false,
        false,
    );
    assert_eq!(placed, Some(at));
    assert_eq!(w.prop(at, "lit"), "false");
    assert_eq!(w.prop(at, "waterlogged"), "true");
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::Sound { sound, .. } if &**sound == "minecraft:entity.generic.extinguish_fire"
        )),
        "the extinguish sound"
    );
    // `douse(null, level, pos, state)` ends with
    // `level.gameEvent(null, GameEvent.BLOCK_CHANGE, pos)`, so a sculk sensor
    // hears the fire go out.
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::GameEvent {
                event: "minecraft:block_change",
                pos,
                ..
            } if *pos == at
        )),
        "the block-change game event"
    );
}

#[test]
fn water_douses_a_lit_candle_with_the_candle_sound() {
    // CandleBlock.placeLiquid: a lit candle that takes water runs
    // `AbstractCandleBlock.extinguish` with the new waterlogged state, which
    // sets it unlit, plays CANDLE_EXTINGUISH and posts BLOCK_CHANGE.
    let mut w = world!();
    let at = BlockPos::new(0, 1, 0);
    w.put(at, w.id("candle", &[("lit", "true")]));
    w.level.events.clear();
    let placed = w.f.behaviours.bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        None,
        BucketContent::Fluid(crate::fluid::FluidKind::Water),
        "minecraft:item.bucket.empty",
        false,
        false,
    );
    assert_eq!(placed, Some(at));
    assert_eq!(w.prop(at, "lit"), "false");
    assert_eq!(w.prop(at, "waterlogged"), "true");
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::Sound { sound, .. } if &**sound == "minecraft:block.candle.extinguish"
        )),
        "the candle extinguish sound"
    );
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::GameEvent {
                event: "minecraft:block_change",
                pos,
                ..
            } if *pos == at
        )),
        "the block-change game event"
    );
    // A candle that is already out (no extinguish) posts no such event.
    w.put(at, w.id("candle", &[("lit", "false")]));
    w.level.events.clear();
    w.f.behaviours.bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        None,
        BucketContent::Fluid(crate::fluid::FluidKind::Water),
        "minecraft:item.bucket.empty",
        false,
        false,
    );
    assert!(!w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::GameEvent {
            event: "minecraft:block_change",
            ..
        }
    )));
}

#[test]
fn an_empty_hand_snuffs_a_lit_candle() {
    // CandleBlock.useItemOn: an empty hand on a lit candle (`mayBuild`) runs
    // AbstractCandleBlock.extinguish: unlit, the wick smoke, the extinguish
    // sound, the block-change game event, and nothing drops.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let lit = w.id("candle", &[("lit", "true")]);
    let unlit = w.id("candle", &[("lit", "false")]);
    w.put(pos, lit);
    w.level.events.clear();
    assert_eq!(w.use_empty_hand(pos), InteractionResult::Success);
    assert_eq!(w.get(pos), unlit);
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::Sound { sound, .. } if &**sound == "minecraft:block.candle.extinguish"
        )),
        "the candle extinguish sound"
    );
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::BlockEvent {
                kind: "candle_smoke",
                ..
            }
        )),
        "the wick smoke"
    );
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::GameEvent {
                event: "minecraft:block_change",
                ..
            }
        )),
        "the block-change game event"
    );
    assert!(
        !w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::DropResources { .. })),
        "snuffing drops nothing"
    );
    // A held item, or a candle that is already out, leaves it alone.
    w.put(pos, lit);
    assert_eq!(w.use_item(pos, "minecraft:stick"), InteractionResult::Pass);
    assert_eq!(w.get(pos), lit);
    w.put(pos, unlit);
    assert_eq!(w.use_empty_hand(pos), InteractionResult::Pass);
}

#[test]
fn an_empty_hand_on_a_lit_candle_cake_eats_it_or_snuffs_its_candle() {
    // CandleCakeBlock.useWithoutItem below the candle eats the cake body
    // (CakeBlock.eat on a plain cake), so one bite is gone and the candle with
    // it; on the candle above half a block, CandleCakeBlock.useItemOn snuffs it
    // (`candleHit`) and the cake stays; flint and steel passes, so the cake is
    // not eaten.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let lit = w.id("candle_cake", &[("lit", "true")]);
    w.put(pos, lit);
    assert_eq!(w.use_empty_hand(pos), InteractionResult::Success);
    assert_eq!(w.get(pos), w.id("cake", &[("bites", "1")]));
    w.put(pos, lit);
    w.level.events.clear();
    assert_eq!(w.use_empty_hand_at(pos, 0.75), InteractionResult::Success);
    assert_eq!(w.get(pos), w.id("candle_cake", &[("lit", "false")]));
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::Sound { sound, .. } if &**sound == "minecraft:block.candle.extinguish"
        )),
        "the candle extinguish sound"
    );
    w.put(pos, lit);
    assert_eq!(
        w.use_item(pos, "minecraft:flint_and_steel"),
        InteractionResult::Pass
    );
    assert_eq!(w.get(pos), lit);
}

#[test]
fn flowers_and_saplings_can_be_placed_in_flower_pots() {
    // FlowerPotBlock.useItemOn: placing a plant in an empty pot pots it.
    // Clicking a potted plant with another plant CONSUMEs without change.
    // Clicking a potted plant with an empty hand takes the plant out (useWithoutItem).
    // Clicking an empty pot with empty hand or non-plant item CONSUMEs.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let empty_pot = w.id("flower_pot", &[]);
    for (item, potted_block) in [
        ("minecraft:poppy", "potted_poppy"),
        ("minecraft:dandelion", "potted_dandelion"),
        ("minecraft:oak_sapling", "potted_oak_sapling"),
        ("minecraft:cactus", "potted_cactus"),
        ("minecraft:red_mushroom", "potted_red_mushroom"),
        ("minecraft:fern", "potted_fern"),
    ] {
        w.put(pos, empty_pot);
        assert_eq!(w.use_item(pos, item), InteractionResult::Success);
        assert_eq!(w.get(pos), w.id(potted_block, &[]));

        // Right-clicking a potted plant with another flower does nothing (CONSUME).
        assert_eq!(
            w.use_item(pos, "minecraft:poppy"),
            InteractionResult::Consume
        );
        assert_eq!(w.get(pos), w.id(potted_block, &[]));

        // Right-clicking a potted plant with empty hand extracts the plant.
        assert_eq!(w.use_block(pos), InteractionResult::Success);
        assert_eq!(w.get(pos), empty_pot);
    }

    // Right-clicking an empty flower pot with an empty hand does nothing (CONSUME, no swing).
    w.put(pos, empty_pot);
    let empty_click = w.use_block(pos);
    assert_eq!(empty_click, InteractionResult::Consume);
    assert!(empty_click.consumes_action());
    assert!(!empty_click.should_swing());
    assert_eq!(w.get(pos), empty_pot);

    // Right-clicking an empty flower pot with a non-plant item does nothing (CONSUME, no swing).
    let non_plant_click = w.use_item(pos, "minecraft:stone");
    assert_eq!(non_plant_click, InteractionResult::Consume);
    assert!(non_plant_click.consumes_action());
    assert!(!non_plant_click.should_swing());
    assert_eq!(w.get(pos), empty_pot);
}

#[test]
fn vanilla_flammability_table_keeps_glass_safe_and_burns_planks() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let glass = w.id("glass", &[]);
    let planks = w.id("oak_planks", &[]);
    let fire = BlockPos::new(0, 1, 0);
    w.set(fire.offset(0, -1, 0), w.id("netherrack", &[]));
    w.set(fire.offset(1, 0, 0), glass);
    w.set(fire.offset(-1, 0, 0), planks);
    w.set(fire, w.id("fire", &[]));
    for _ in 0..4000 {
        w.tick();
    }
    assert_eq!(w.get(fire.offset(1, 0, 0)), glass, "glass doesn't burn");
    assert_ne!(w.get(fire.offset(-1, 0, 0)), planks, "planks burn");
}

#[test]
fn vanilla_hay_bale_makes_a_campfire_a_signal_fire() {
    let mut w = world!();
    let (stone, hay) = (w.id("stone", &[]), w.id("hay_block", &[]));
    let ground = BlockPos::new(0, 0, 0);
    let campfire = ground.offset(0, 1, 0);
    let place = |w: &mut World| {
        w.level.blocks.remove(&campfire);
        let ctx = PlaceContext {
            clicked_pos: ground,
            face: Direction::Up,
            hit: horizon_core::geometry::Vec3::new(0.5, 1.0, 0.5),
            inside: false,
            yaw: 0.0,
            pitch: 0.0,
            sneaking: false,
            item: "minecraft:campfire".into(),
            can_use_game_master_blocks: false,
        };
        let b = w.b();
        assert!(matches!(
            b.place_block(&mut w.level, &mut w.ticks, &ctx),
            PlaceOutcome::Placed { .. }
        ));
        w.prop(campfire, "signal_fire")
    };
    w.put(ground, stone);
    assert_eq!(place(&mut w), "false");
    w.put(ground, hay);
    assert_eq!(place(&mut w), "true", "vanilla's hay bale");
    w.set(ground, stone);
    assert_eq!(w.prop(campfire, "signal_fire"), "false");
    w.set(ground, hay);
    assert_eq!(w.prop(campfire, "signal_fire"), "true");
}

#[test]
fn carpet_duplication_machine_8490() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let slime = w.id("slime_block", &[]);
    let air = w.b().air();
    let piston = w.id("sticky_piston", &[("facing", "west")]);
    let power = w.id("redstone_block", &[]);
    let fan = w.id(
        "dead_tube_coral_wall_fan",
        &[("facing", "west"), ("waterlogged", "false")],
    );
    let carpet = w.id("white_carpet", &[]);
    w.set(
        BlockPos::new(4, 1, 0),
        w.id("grass_block", &[("snowy", "false")]),
    );
    for y in 2..6 {
        w.set(BlockPos::new(3, y, 0), slime);
    }
    w.set(BlockPos::new(4, 2, 0), piston);
    w.set(BlockPos::new(2, 2, 0), fan);
    for y in 3..6 {
        w.set(BlockPos::new(2, y, 0), carpet);
    }
    w.set(BlockPos::new(5, 2, 0), power);
    let fan_name = |w: &mut World, x: i32, y: i32| w.name(BlockPos::new(x, y, 0));
    for _ in 0..6 {
        w.full_tick();
    }
    // Pushed one block west: the column lands with its fan and carpets (the
    // fan reads the slime block still shown as a finished moving piston).
    for y in 2..6 {
        assert_eq!(w.get(BlockPos::new(2, y, 0)), slime, "slime y={y}");
    }
    assert!(fan_name(&mut w, 1, 2).starts_with("dead_tube_coral_wall_fan"));
    for y in 3..6 {
        assert_eq!(w.get(BlockPos::new(1, y, 0)), carpet, "carpet y={y}");
    }
    w.set(BlockPos::new(5, 2, 0), air);
    for _ in 0..6 {
        w.full_tick();
    }
    // Pulled back: the machine is whole again at its start.
    for y in 2..6 {
        assert_eq!(w.get(BlockPos::new(3, y, 0)), slime, "pulled slime y={y}");
    }
    assert!(fan_name(&mut w, 2, 2).starts_with("dead_tube_coral_wall_fan"));
    for y in 3..6 {
        assert_eq!(w.get(BlockPos::new(2, y, 0)), carpet, "pulled carpet y={y}");
    }
}

#[test]
fn a_rail_on_a_block_a_piston_pushes_pops_off() {
    // PistonBaseBlock.moveBlocks writes the arriving blocks as `moving_piston`
    // and then updates the neighbours of every source cell. The rail on the
    // second stone asks whether that cell supports it: vanilla reads the moving
    // block's shape offset by `progress - 1`, a whole block back at progress 0,
    // so nothing holds the rail and it pops, though another stone arrives under
    // it (dynamic_support pins the moving shape's support against vanilla).
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "east")]);
    let stone = w.id("stone", &[]);
    let rail = w.id("rail", &[]);
    let power = w.id("redstone_block", &[]);
    let piston_pos = BlockPos::new(0, 1, 0);
    let rail_pos = BlockPos::new(2, 2, 0);
    w.set(piston_pos, piston);
    w.set(BlockPos::new(1, 1, 0), stone);
    w.set(BlockPos::new(2, 1, 0), stone);
    w.set(rail_pos, rail);
    w.set(BlockPos::new(-1, 1, 0), power);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(piston_pos, "extended"), "true");
    assert_eq!(w.name(BlockPos::new(2, 1, 0)), "stone[]");
    assert_eq!(w.name(BlockPos::new(3, 1, 0)), "stone[]");
    assert_eq!(w.name(rail_pos), "air[]");
}

#[test]
fn a_rail_riding_on_slime_stays_on_it() {
    // #8111: the slime block carries the rail on top of it. The rail's block
    // entity can finish while the slime's still holds `moving_piston` at
    // progress 1, whose shape is the slime block in its cell, so the rail is
    // supported both ways.
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let slime = w.id("slime_block", &[]);
    let rail = w.id("rail", &[]);
    let power = w.id("redstone_block", &[]);
    let air = w.b().air();
    let piston_pos = BlockPos::new(0, 1, 0);
    let power_pos = BlockPos::new(-1, 1, 0);
    w.set(piston_pos, piston);
    w.set(BlockPos::new(1, 1, 0), slime);
    w.set(BlockPos::new(1, 2, 0), rail);
    w.set(power_pos, power);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(piston_pos, "extended"), "true");
    assert_eq!(w.name(BlockPos::new(2, 1, 0)), "slime_block[]");
    let pushed = w.name(BlockPos::new(2, 2, 0));
    assert!(pushed.starts_with("rail"), "pushed: got {pushed}");
    w.set(power_pos, air);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(piston_pos, "extended"), "false");
    let pulled = w.name(BlockPos::new(1, 2, 0));
    assert!(pulled.starts_with("rail"), "pulled back: got {pulled}");
}

#[test]
fn fresh_chunk_block_events_do_not_wait_for_ticking_readiness() {
    for (ticketed, ready) in [(true, false), (false, true), (true, true), (false, false)] {
        let mut w = world!();
        w.level.tick_ready = ready;
        w.level.block_events_ready = ticketed;
        let pos = BlockPos::new(0, 1, 0);
        let piston = w.id("piston", &[("facing", "up")]);
        w.set(BlockPos::new(0, 0, 0), w.id("redstone_block", &[]));
        w.set(pos, piston);
        assert_eq!(w.ticks.block_events.len(), 1, "placement queues an event");
        w.f.behaviours.run_block_events(&mut w.level, &mut w.ticks);
        assert_eq!(
            w.prop(pos, "extended"),
            ticketed.to_string(),
            "ticketed={ticketed}, ticking={ready}"
        );
        assert_eq!(w.ticks.block_events.is_empty(), ticketed);
        if ticketed {
            assert!(!w.ticks.moving_pistons.is_empty());
            w.f.behaviours
                .tick_block_entities(&mut w.level, &mut w.ticks);
            assert_eq!(
                w.ticks.moving_pistons[0].1.progress > 0.0,
                ready,
                "moving piston entities still require ticking readiness"
            );
        } else {
            w.level.block_events_ready = true;
            w.f.behaviours.run_block_events(&mut w.level, &mut w.ticks);
            assert_eq!(
                w.prop(pos, "extended"),
                "true",
                "deferred events run once ticketed"
            );
            assert!(w.ticks.block_events.is_empty());
        }
    }
}

mod barrel;
mod pistons;
mod redstone;
mod trigger_tests;

/// `ShulkerBoxBlockEntity.doNeighborUpdates`: the neighbours' shape updates
/// (flag 3) and then their `neighborChanged`, at the four points in
/// `ShulkerBoxBlockEntity.tick` where a lid starts or finishes moving.
#[test]
fn a_shulker_lid_updates_the_shapes_and_redstone_of_its_neighbours() {
    use std::cell::RefCell;

    struct Observer {
        inner: TestLevel,
        seen: RefCell<Vec<(BlockPos, BlockStateId, BlockId, bool)>>,
    }
    impl LevelAccess for Observer {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, _pos: BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn observe_neighbor_changed(
            &self,
            pos: BlockPos,
            state: BlockStateId,
            changed: BlockId,
            moved: bool,
        ) {
            assert_eq!(self.block_state(pos), state);
            self.seen.borrow_mut().push((pos, state, changed, moved));
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let mut level = Observer {
        inner: TestLevel::new(b),
        seen: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    let pos = BlockPos::new(0, 0, 0);
    let state = b.air();
    b.shulker_neighbor_updates(&mut level, &mut ticks, pos, state);
    let expected = [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ];
    let seen = level.seen.borrow();
    assert_eq!(seen.len(), expected.len(), "{seen:?}");
    for (event, (x, y, z)) in seen.iter().zip(expected) {
        assert_eq!(
            *event,
            (BlockPos::new(x, y, z), state, b.block(state), false)
        );
    }
}

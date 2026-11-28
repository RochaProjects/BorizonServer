//! Server-owned PLAY bootstrap with one resident Overworld shared by sessions.
//!
//! This is not yet the authoritative gameplay loop. Player movement uses resident
//! contextual collisions, basic survival mining, and queued chunk streaming. Dropped
//! items have a limited shared gravity/collision/pickup lifecycle;
//! player gravity, mobs, block entities, other interactions, complete entity/chunk
//! persistence, graceful final shutdown, and full vanilla parity remain pending.

use crate::{
    inventory::PlayerInventory,
    item_entity::{DroppedItem, ItemEntityStore},
    login::ConfiguredConnection,
    play_chunk,
    resources::ServerResources,
};
#[cfg(test)]
use horizon_core::geometry::{Aabb, VoxelShape, try_resolve_step_movement};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, RandomSource},
    ticks::TickTypeId,
};
use horizon_protocol::{
    entity_data::{self, Value as EntityDataValue},
    item::{Component as ItemComponent, Stack},
    packets::{configuration::KeepAlive, play},
};
use horizon_world::{
    compression::Compression,
    fluid::FluidKind,
    level::{
        BlockId, LevelAccess, LevelEvent, PlaceContext, PlaceOutcome, PlayerContext,
        ResidentDimension, ResidentLevel, Ticks, data::state_flags,
    },
    loot::{BlockLootContext, LootStack},
    nbt::Limits,
    player_mining::{self, DestroySpeedModifiers},
    ticket_levels::Sources as ChunkTicketSources,
    world_runtime::LocalWorldRuntime,
    worldgen::generator::{GeneratorWorker, OverworldGenerator, ProtoChunk, SavedFinalWrite},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CHUNK_RADIUS: i32 = 1;
const CHUNK_COUNT: usize = 9;
const DEFAULT_VIEW_DISTANCE: i32 = 10;
#[cfg(test)]
const DEFAULT_SIMULATION_DISTANCE: i32 = 10;
const CHUNK_GENERATION_QUEUE: usize = 256;
const CHUNK_GENERATION_IDLE_RELEASE: Duration = Duration::from_secs(5);
const CHUNKS_PER_TICK: f32 = 9.0;
const MAX_CHUNK_BATCH: usize = 64;
const MAX_LIFECYCLE_CANDIDATES: usize = 4096;
const PLAY_TICK_BARRIER: Duration = Duration::from_millis(50);
const SERVER_TICK: Duration = Duration::from_millis(50);
const PACKETS_PER_TICK: usize = 400;
const INBOX_LIMIT: usize = 20 * PACKETS_PER_TICK;
const INBOX_BYTES: usize = 16 << 20;
const NETWORK_CHANNEL_CAPACITY: usize = 8;
const MAX_CLIENT_MOVEMENT_SQUARED_PER_TICK: f64 = 100.0;
const MAX_RETAINED_BLOCK_UPDATES: usize = 65_536;

pub(crate) struct PreparedPlayWorld {
    world_root: PathBuf,
    resources: Arc<ServerResources>,
    dimension_type: i32,
    spawn: [f64; 3],
    chunk_packets: Vec<Vec<u8>>,
    chunk_packet_cache: Mutex<HashMap<ChunkPos, Vec<u8>>>,
    chunk_generation: Option<ChunkGeneration>,
    view_distance: i32,
    simulation_distance: i32,
    game_mode: i32,
    difficulty: i32,
    max_players: usize,
    hardcore: bool,
    runtime: Mutex<LocalWorldRuntime<'static>>,
    player_chunks: Mutex<HashMap<i32, ChunkPos>>,
    block_updates: Mutex<BlockUpdateLog>,
    level_events: Mutex<LevelEventLog>,
    item_entities: ItemEntityStore,
    loot_random: Mutex<LegacyRandom>,
}

struct ChunkGeneration {
    requests: Arc<ChunkRequestQueue>,
    completed: Mutex<Receiver<ChunkWorkResult>>,
    pipeline: Mutex<ChunkPipeline>,
    generation_budget: usize,
    decoration_budget: usize,
    light_budget: usize,
}

struct ChunkPipeline {
    centers: Vec<ChunkPos>,
    radius: i32,
    requested: HashSet<ChunkPos>,
    targets: HashSet<ChunkPos>,
    feature_targets: HashSet<ChunkPos>,
    terrain_targets: HashSet<ChunkPos>,
    protos: HashMap<ChunkPos, Arc<ProtoChunk>>,
    saved_chunks: HashMap<ChunkPos, Arc<horizon_world::chunk::ChunkData>>,
    saved_checked: HashSet<ChunkPos>,
    terrain_jobs: HashSet<ChunkPos>,
    feature_jobs: HashSet<ChunkPos>,
    light_jobs: HashSet<ChunkPos>,
    features: HashSet<ChunkPos>,
    lit: HashSet<ChunkPos>,
    delivered: HashSet<ChunkPos>,
    rank: HashMap<ChunkPos, usize>,
    rank_origins: HashSet<ChunkPos>,
}

impl Default for ChunkPipeline {
    fn default() -> Self {
        Self {
            centers: Vec::new(),
            radius: DEFAULT_VIEW_DISTANCE,
            requested: HashSet::new(),
            targets: HashSet::new(),
            feature_targets: HashSet::new(),
            terrain_targets: HashSet::new(),
            protos: HashMap::new(),
            saved_chunks: HashMap::new(),
            saved_checked: HashSet::new(),
            terrain_jobs: HashSet::new(),
            feature_jobs: HashSet::new(),
            light_jobs: HashSet::new(),
            features: HashSet::new(),
            lit: HashSet::new(),
            delivered: HashSet::new(),
            rank: HashMap::new(),
            rank_origins: HashSet::new(),
        }
    }
}

enum ChunkWork {
    Terrain(ChunkPos),
    Features(
        ChunkPos,
        Box<[ProtoChunk; CHUNK_COUNT]>,
        HashMap<ChunkPos, Arc<horizon_world::chunk::ChunkData>>,
    ),
    Light(ChunkPos, Box<[Arc<ProtoChunk>; CHUNK_COUNT]>),
}

impl ChunkWork {
    fn position(&self) -> ChunkPos {
        match self {
            Self::Terrain(position) | Self::Features(position, _, _) | Self::Light(position, _) => {
                *position
            }
        }
    }

    fn stage_priority(&self) -> i128 {
        match self {
            Self::Light(..) => 0,
            Self::Features(..) => 1,
            Self::Terrain(..) => 2,
        }
    }
}

enum ChunkWorkResult {
    Terrain(ChunkPos, Result<ProtoChunk, String>),
    Features(
        ChunkPos,
        Result<(Box<[ProtoChunk; CHUNK_COUNT]>, Vec<SavedFinalWrite>), String>,
    ),
    Light(
        ChunkPos,
        Result<(Vec<u8>, horizon_world::chunk::ChunkData), String>,
    ),
}

struct GeneratorIdleState {
    generator: OverworldGenerator,
    worker_count: usize,
    idle_workers: Mutex<usize>,
}

impl GeneratorIdleState {
    fn new(generator: OverworldGenerator, worker_count: usize) -> Self {
        Self {
            generator,
            worker_count,
            idle_workers: Mutex::new(0),
        }
    }

    fn worker_became_idle(&self) {
        let mut idle = self
            .idle_workers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *idle = idle.saturating_add(1).min(self.worker_count);
        if *idle == self.worker_count {
            self.generator.release_caches();
        }
    }

    fn worker_became_active(&self) {
        let mut idle = self
            .idle_workers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *idle = idle.saturating_sub(1);
    }
}

#[derive(Default)]
struct ChunkRequestQueue {
    state: Mutex<ChunkRequestState>,
    ready: Condvar,
}

#[derive(Default)]
struct ChunkRequestState {
    pending: VecDeque<(i128, ChunkWork)>,
    closed: bool,
}

impl ChunkRequestQueue {
    fn try_send(
        &self,
        work: ChunkWork,
        priority: i128,
    ) -> Result<(), (ChunkQueueSendError, ChunkWork)> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err((ChunkQueueSendError::Closed, work));
        }
        if state.pending.len() >= CHUNK_GENERATION_QUEUE {
            return Err((ChunkQueueSendError::Full, work));
        }
        let at = state
            .pending
            .iter()
            .position(|(queued_priority, _)| *queued_priority > priority)
            .unwrap_or(state.pending.len());
        state.pending.insert(at, (priority, work));
        self.ready.notify_one();
        Ok(())
    }

    /// `Ok(None)` means shutdown; `Err(())` means the idle wait timed out.
    fn receive_timeout(&self, timeout: Duration) -> Result<Option<ChunkWork>, ()> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let (mut state, wait) = self
            .ready
            .wait_timeout_while(state, timeout, |state| {
                state.pending.is_empty() && !state.closed
            })
            .unwrap_or_else(|error| error.into_inner());
        if let Some((_, work)) = state.pending.pop_front() {
            return Ok(Some(work));
        }
        if state.closed {
            return Ok(None);
        }
        debug_assert!(wait.timed_out());
        Err(())
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        state.pending.clear();
        self.ready.notify_all();
    }

    fn retain(
        &self,
        mut keep: impl FnMut(&ChunkWork) -> bool,
        mut priority: impl FnMut(&ChunkWork) -> i128,
    ) -> Vec<ChunkWork> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut removed = Vec::new();
        let mut retained = VecDeque::with_capacity(state.pending.len());
        while let Some((_, work)) = state.pending.pop_front() {
            if keep(&work) {
                retained.push_back((priority(&work), work));
            } else {
                removed.push(work);
            }
        }
        retained
            .make_contiguous()
            .sort_by_key(|(priority, _)| *priority);
        state.pending = retained;
        removed
    }
}

enum ChunkQueueSendError {
    Full,
    Closed,
}

impl Drop for ChunkGeneration {
    fn drop(&mut self) {
        self.requests.close();
    }
}

impl ChunkGeneration {
    fn start(
        generator: OverworldGenerator,
        resources: Arc<ServerResources>,
        initial_terrain: Option<[ProtoChunk; CHUNK_COUNT]>,
    ) -> Result<Self, String> {
        let request_queue = Arc::new(ChunkRequestQueue::default());
        let (results, completed) = mpsc::sync_channel(CHUNK_GENERATION_QUEUE);
        let worker_count =
            thread::available_parallelism().map_or(1, |count| count.get().div_ceil(2));
        // Match Streaming's per-stage in-flight limits. Bounding queued protos
        // keeps terrain from monopolizing memory before nearby chunks reach
        // FEATURES and LIGHT.
        let generation_budget = worker_count * 3;
        let decoration_budget = worker_count * 2;
        let light_budget = worker_count * 4;
        let idle_state = Arc::new(GeneratorIdleState::new(generator.clone(), worker_count));
        for index in 0..worker_count {
            let generator = generator.clone();
            let resources = resources.clone();
            let idle_state = idle_state.clone();
            let requests = request_queue.clone();
            let results = results.clone();
            let spawn = thread::Builder::new()
                .name(format!("borizon-chunk-generation-{index}"))
                .spawn(move || {
                    let mut worker = generator.worker();
                    let mut idle = false;
                    loop {
                        let request = requests.receive_timeout(CHUNK_GENERATION_IDLE_RELEASE);
                        match request {
                            Ok(Some(work)) => {
                                if idle {
                                    idle_state.worker_became_active();
                                    idle = false;
                                }
                                let result = match work {
                                    ChunkWork::Terrain(position) => {
                                        let result = worker
                                            .generate_proto(position)
                                            .map_err(|error| {
                                                format!("generate terrain chunk ({},{}): {error}", position.x, position.z)
                                            });
                                        ChunkWorkResult::Terrain(position, result)
                                    }
                                    ChunkWork::Features(position, mut region, saved) => {
                                        let result = region
                                            .iter_mut()
                                            .try_for_each(|proto| {
                                                if let Some(data) = saved.get(&proto.position()) {
                                                    proto.restore_saved_final(&data).map_err(
                                                        |error| {
                                                            format!(
                                                                "restore saved final chunk ({},{}): {error}",
                                                                proto.position().x,
                                                                proto.position().z
                                                            )
                                                        },
                                                    )?;
                                                }
                                                Ok::<_, String>(())
                                            })
                                            .and_then(|_| {
                                                worker.decorate(region.as_mut()).map_err(|error| {
                                                    format!("decorate features around ({},{}): {error}", position.x, position.z)
                                                })
                                            })
                                            .and_then(|_| {
                                                for proto in region.iter_mut() {
                                                    proto.finalize_block_entities();
                                                }
                                                if region[4].position() != position
                                                    || !region[4].is_decorated()
                                                {
                                                    return Err(format!(
                                                        "FEATURES stage did not complete chunk ({},{})",
                                                        position.x, position.z
                                                    ));
                                                }
                                                let mut final_writes = Vec::new();
                                                for proto in region.iter() {
                                                    if let Some(data) = saved.get(&proto.position()) {
                                                        final_writes.extend(
                                                            proto
                                                                .saved_final_writes(data)
                                                                .map_err(|error| {
                                                                    format!(
                                                                        "collect saved final writes for ({},{}): {error}",
                                                                        proto.position().x,
                                                                        proto.position().z
                                                                    )
                                                                })?,
                                                        );
                                                    }
                                                }
                                                Ok((region, final_writes))
                                            });
                                        ChunkWorkResult::Features(position, result)
                                    }
                                    ChunkWork::Light(position, region) => {
                                        let refs: [&ProtoChunk; CHUNK_COUNT] =
                                            std::array::from_fn(|index| region[index].as_ref());
                                        let result = resources
                                            .light_occlusion
                                            .light_region_refs(
                                                &refs,
                                                resources.registry,
                                                &resources.behaviours,
                                                true,
                                            )
                                            .and_then(|lights| {
                                                encode_center_chunk(
                                                    &region[4],
                                                    lights[4].clone(),
                                                    &resources,
                                                )
                                            });
                                        ChunkWorkResult::Light(position, result)
                                    }
                                };
                                if results.send(result).is_err() {
                                    break;
                                }
                            }
                            Ok(None) => break,
                            Err(()) => {
                                worker.release();
                                if !idle {
                                    idle_state.worker_became_idle();
                                    idle = true;
                                }
                            }
                        }
                    }
                });
            if let Err(error) = spawn {
                request_queue.close();
                return Err(format!("start server chunk-generation worker: {error}"));
            }
        }
        drop(results);
        let mut pipeline = ChunkPipeline::default();
        if let Some(chunks) = initial_terrain {
            for chunk in chunks {
                pipeline.protos.insert(chunk.position(), Arc::new(chunk));
            }
        }
        Ok(Self {
            requests: request_queue,
            completed: Mutex::new(completed),
            pipeline: Mutex::new(pipeline),
            generation_budget,
            decoration_budget,
            light_budget,
        })
    }

    /// Add a chunk to the desired view. The scheduler expands it into the
    /// TERRAIN, FEATURES and LIGHT dependencies required by the server status graph.
    fn request(&self, position: ChunkPos) -> Result<(), String> {
        self.pipeline
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested
            .insert(position);
        Ok(())
    }

    /// Ask for Anvil snapshots only when the next ordered FEATURES region has
    /// its terrain protos ready, keeping disk work bounded to one region.
    fn saved_snapshot_positions(&self, centers: &[ChunkPos], radius: i32) -> HashSet<ChunkPos> {
        let mut pipeline = self
            .pipeline
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pipeline.set_view(centers, radius);
        let mut features = pipeline
            .feature_targets
            .iter()
            .copied()
            .filter(|center| {
                !pipeline.features.contains(center)
                    && !pipeline.feature_jobs.contains(center)
                    && !pipeline.feature_blocked(*center)
                    && !pipeline.feature_jobs.iter().any(|running| {
                        (running.x - center.x).abs() <= 2 && (running.z - center.z).abs() <= 2
                    })
            })
            .collect::<Vec<_>>();
        features.sort_by_key(|center| pipeline.priority_at(1, *center));
        let Some(center) = features.into_iter().find(|center| {
            region_positions(*center).iter().all(|position| {
                pipeline
                    .protos
                    .get(position)
                    .is_some_and(|proto| Arc::strong_count(proto) == 1)
            })
        }) else {
            return HashSet::new();
        };
        region_positions(center)
            .into_iter()
            .filter(|position| !pipeline.saved_checked.contains(position))
            .collect()
    }

    fn advance(
        &self,
        centers: &[ChunkPos],
        radius: i32,
        saved_snapshots: HashMap<ChunkPos, Option<horizon_world::chunk::ChunkData>>,
    ) -> Result<
        (
            Vec<(ChunkPos, (Vec<u8>, horizon_world::chunk::ChunkData))>,
            Vec<SavedFinalWrite>,
        ),
        String,
    > {
        let receiver = self
            .completed
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut results = Vec::new();
        loop {
            match receiver.try_recv() {
                Ok(result) => results.push(result),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("server chunk-generation worker stopped".into());
                }
            }
        }
        drop(receiver);

        let mut pipeline = self
            .pipeline
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pipeline.set_view(centers, radius);
        for (position, data) in saved_snapshots {
            pipeline.saved_checked.insert(position);
            if let Some(data) = data {
                pipeline.saved_chunks.insert(position, Arc::new(data));
            }
        }
        let mut ready = Vec::new();
        let mut saved_final_writes = Vec::new();
        for result in results {
            match result {
                ChunkWorkResult::Terrain(position, result) => {
                    pipeline.terrain_jobs.remove(&position);
                    let proto = result?;
                    if pipeline.terrain_targets.contains(&position) {
                        pipeline.protos.insert(position, Arc::new(proto));
                    }
                }
                ChunkWorkResult::Features(center, result) => {
                    pipeline.feature_jobs.remove(&center);
                    let (region, writes) = result?;
                    saved_final_writes.extend(writes);
                    for proto in *region {
                        let position = proto.position();
                        if pipeline.terrain_targets.contains(&position) {
                            pipeline.protos.insert(position, Arc::new(proto));
                        }
                    }
                    if pipeline.feature_targets.contains(&center) {
                        pipeline.features.insert(center);
                    }
                }
                ChunkWorkResult::Light(position, result) => {
                    pipeline.light_jobs.remove(&position);
                    let chunk = result?;
                    if pipeline.targets.contains(&position) {
                        pipeline.lit.insert(position);
                        if pipeline.delivered.insert(position) {
                            ready.push((position, chunk));
                        }
                    }
                }
            }
        }

        pipeline.schedule(
            &self.requests,
            self.generation_budget,
            self.decoration_budget,
            self.light_budget,
        )?;
        Ok((ready, saved_final_writes))
    }
}

impl ChunkPipeline {
    fn set_view(&mut self, centers: &[ChunkPos], radius: i32) {
        // A radius of one is used while preparing the initial 3×3 spawn view.
        // The live client view is clamped to at least two by build_with_settings.
        let radius = radius.max(1);
        let centers_changed = self.centers != centers || self.radius != radius;
        self.centers.clear();
        self.centers.extend_from_slice(centers);
        self.radius = radius;

        let mut targets = HashSet::new();
        for center in centers {
            targets.extend(square_chunks(*center, radius));
        }
        if centers.is_empty() {
            targets.extend(self.requested.iter().copied());
        } else {
            self.requested.retain(|position| {
                centers
                    .iter()
                    .any(|center| is_tracked(*center, radius, *position))
            });
            targets.extend(self.requested.iter().copied());
        }
        self.targets = targets;
        self.feature_targets = expand_chunks(&self.targets, 2);
        self.terrain_targets = expand_chunks(&self.feature_targets, 1);

        self.protos
            .retain(|position, _| self.terrain_targets.contains(position));
        self.features
            .retain(|position| self.feature_targets.contains(position));
        self.lit.retain(|position| self.targets.contains(position));
        self.delivered
            .retain(|position| self.targets.contains(position));

        let rank_origins = if centers.is_empty() {
            self.requested.clone()
        } else {
            centers.iter().copied().collect()
        };
        if centers_changed || self.rank_origins != rank_origins {
            self.rank_origins = rank_origins.clone();
            self.rank.clear();
            for center in &rank_origins {
                for (rank, (x, z)) in horizon_world::worldgen::decoration_order::vanilla_order(
                    (center.x, center.z),
                    radius + 2,
                )
                .into_iter()
                .enumerate()
                {
                    self.rank
                        .entry(ChunkPos::new(x, z))
                        .and_modify(|current| *current = (*current).min(rank))
                        .or_insert(rank);
                }
            }
        }

        // A proto is only useful while an unfinished FEATURES center or an
        // unlit target can read its 3×3 region. Keeping every terrain support
        // chunk until it leaves the player's view retains hundreds of large
        // protos after their packet has already been produced.
        let mut needed_protos = HashSet::new();
        for center in self
            .feature_targets
            .difference(&self.features)
            .chain(self.targets.difference(&self.lit))
        {
            needed_protos.extend(region_positions(*center));
        }
        self.protos
            .retain(|position, _| needed_protos.contains(position));
        self.saved_chunks
            .retain(|position, _| self.terrain_targets.contains(position));
        self.saved_checked
            .retain(|position| self.terrain_targets.contains(position));
    }

    fn priority_at(&self, stage: i128, position: ChunkPos) -> i128 {
        let rank = self.rank.get(&position).copied().unwrap_or(usize::MAX / 2);
        stage * (1_i128 << 100)
            + centers_distance_key(&self.centers, position) * (1_i128 << 32)
            + rank as i128
    }

    fn schedule(
        &mut self,
        requests: &ChunkRequestQueue,
        generation_budget: usize,
        decoration_budget: usize,
        light_budget: usize,
    ) -> Result<(), String> {
        let removed = requests.retain(
            |work| match work {
                ChunkWork::Terrain(position) => self.terrain_targets.contains(position),
                ChunkWork::Features(position, _, _) => self.feature_targets.contains(position),
                ChunkWork::Light(position, _) => self.targets.contains(position),
            },
            |work| self.priority_at(work.stage_priority(), work.position()),
        );
        for work in removed {
            match work {
                ChunkWork::Terrain(position) => {
                    self.terrain_jobs.remove(&position);
                }
                ChunkWork::Features(position, region, _) => {
                    self.feature_jobs.remove(&position);
                    for proto in *region {
                        let position = proto.position();
                        if self.terrain_targets.contains(&position) {
                            self.protos.insert(position, Arc::new(proto));
                        }
                    }
                }
                ChunkWork::Light(position, _) => {
                    self.light_jobs.remove(&position);
                }
            }
        }

        let mut terrain = self
            .terrain_targets
            .iter()
            .copied()
            .filter(|position| {
                !self.protos.contains_key(position) && !self.terrain_jobs.contains(position)
            })
            .collect::<Vec<_>>();
        terrain.sort_by_key(|position| self.priority_at(2, *position));
        terrain.truncate(generation_budget.saturating_sub(self.terrain_jobs.len()));
        for position in terrain {
            self.terrain_jobs.insert(position);
            let work = ChunkWork::Terrain(position);
            match requests.try_send(work, self.priority_at(2, position)) {
                Ok(()) => {}
                Err((ChunkQueueSendError::Full, _)) => {
                    self.terrain_jobs.remove(&position);
                    break;
                }
                Err((ChunkQueueSendError::Closed, _)) => {
                    self.terrain_jobs.remove(&position);
                    return Err("server chunk-generation queue is closed".into());
                }
            }
        }

        let mut features = self
            .feature_targets
            .iter()
            .copied()
            .filter(|position| {
                !self.features.contains(position) && !self.feature_jobs.contains(position)
            })
            .collect::<Vec<_>>();
        features.sort_by_key(|position| self.priority_at(1, *position));
        for center in features {
            if self.feature_jobs.len() >= decoration_budget {
                break;
            }
            if self.feature_blocked(center)
                || self.feature_jobs.iter().any(|running| {
                    (running.x - center.x).abs() <= 2 && (running.z - center.z).abs() <= 2
                })
            {
                continue;
            }
            let positions = region_positions(center);
            if positions
                .iter()
                .any(|position| !self.saved_checked.contains(position))
            {
                continue;
            }
            if positions.iter().any(|position| {
                self.protos
                    .get(position)
                    .is_none_or(|proto| Arc::strong_count(proto) != 1)
            }) {
                continue;
            }
            let mut chunks = Vec::with_capacity(CHUNK_COUNT);
            for &position in &positions {
                let proto = self
                    .protos
                    .remove(&position)
                    .expect("checked feature region");
                match Arc::try_unwrap(proto) {
                    Ok(proto) => chunks.push(proto),
                    Err(_) => unreachable!("checked FEATURES protos are uniquely owned"),
                }
            }
            let region: Box<[ProtoChunk; CHUNK_COUNT]> = match chunks.try_into() {
                Ok(region) => Box::new(region),
                Err(_) => unreachable!("FEATURES region has nine proto chunks"),
            };
            self.feature_jobs.insert(center);
            let saved = positions
                .iter()
                .filter_map(|position| {
                    self.saved_chunks
                        .get(position)
                        .cloned()
                        .map(|data| (*position, data))
                })
                .collect();
            let work = ChunkWork::Features(center, region, saved);
            let priority = self.priority_at(1, center);
            match requests.try_send(work, priority) {
                Ok(()) => {}
                Err((ChunkQueueSendError::Full, ChunkWork::Features(position, region, _))) => {
                    self.feature_jobs.remove(&position);
                    for proto in *region {
                        self.protos.insert(proto.position(), Arc::new(proto));
                    }
                    break;
                }
                Err((ChunkQueueSendError::Closed, _)) => {
                    self.feature_jobs.remove(&center);
                    return Err("server chunk-generation queue is closed".into());
                }
                Err(_) => unreachable!("FEATURES submission returns its own job"),
            }
        }

        let mut lights = self
            .targets
            .iter()
            .copied()
            .filter(|position| !self.lit.contains(position) && !self.light_jobs.contains(position))
            .collect::<Vec<_>>();
        lights.sort_by_key(|position| self.priority_at(0, *position));
        for center in lights {
            if self.light_jobs.len() >= light_budget {
                break;
            }
            let local_features_ready = (-2..=2).all(|dz| {
                (-2..=2).all(|dx| {
                    self.features
                        .contains(&ChunkPos::new(center.x + dx, center.z + dz))
                })
            });
            if !local_features_ready {
                continue;
            }
            let positions = region_positions(center);
            let Some(region) = positions
                .iter()
                .map(|position| self.protos.get(position).cloned())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let region: Box<[Arc<ProtoChunk>; CHUNK_COUNT]> = match region.try_into() {
                Ok(region) => Box::new(region),
                Err(_) => unreachable!("light region has nine proto chunks"),
            };
            self.light_jobs.insert(center);
            let work = ChunkWork::Light(center, region);
            let priority = self.priority_at(0, center);
            match requests.try_send(work, priority) {
                Ok(()) => {}
                Err((ChunkQueueSendError::Full, ChunkWork::Light(position, _))) => {
                    self.light_jobs.remove(&position);
                    break;
                }
                Err((ChunkQueueSendError::Closed, _)) => {
                    self.light_jobs.remove(&center);
                    return Err("server chunk-generation queue is closed".into());
                }
                Err(_) => unreachable!("LIGHT submission returns its own job"),
            }
        }

        Ok(())
    }

    fn feature_blocked(&self, center: ChunkPos) -> bool {
        let own_rank = self.rank.get(&center).copied().unwrap_or(usize::MAX);
        (-2..=2).any(|dz| {
            (-2..=2).any(|dx| {
                if (dx, dz) == (0, 0) {
                    return false;
                }
                let neighbor = ChunkPos::new(center.x + dx, center.z + dz);
                self.feature_targets.contains(&neighbor)
                    && self.rank.get(&neighbor).copied().unwrap_or(usize::MAX) < own_rank
                    && !self.features.contains(&neighbor)
            })
        })
    }
}

fn square_chunks(center: ChunkPos, radius: i32) -> HashSet<ChunkPos> {
    let mut chunks = HashSet::new();
    for z in center.z - radius..=center.z + radius {
        for x in center.x - radius..=center.x + radius {
            chunks.insert(ChunkPos::new(x, z));
        }
    }
    chunks
}

fn expand_chunks(chunks: &HashSet<ChunkPos>, radius: i32) -> HashSet<ChunkPos> {
    let mut expanded = HashSet::new();
    for center in chunks {
        expanded.extend(square_chunks(*center, radius));
    }
    expanded
}

fn region_positions(center: ChunkPos) -> [ChunkPos; CHUNK_COUNT] {
    std::array::from_fn(|index| {
        ChunkPos::new(
            center.x + (index % CHUNK_COUNT_SIDE) as i32 - CHUNK_RADIUS,
            center.z + (index / CHUNK_COUNT_SIDE) as i32 - CHUNK_RADIUS,
        )
    })
}

#[derive(Clone, Debug)]
struct WorldBlockUpdate {
    sequence: u64,
    dimension: String,
    pos: [i32; 3],
    state: i32,
}

#[derive(Default)]
struct BlockUpdateLog {
    next_sequence: u64,
    updates: VecDeque<WorldBlockUpdate>,
}

impl BlockUpdateLog {
    fn latest_sequence(&self) -> u64 {
        self.next_sequence.saturating_sub(1)
    }

    fn append(&mut self, dimension: String, pos: [i32; 3], state: i32) -> Result<(), String> {
        let sequence = self.next_sequence.max(1);
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| "PLAY block update sequence exhausted".to_owned())?;
        self.updates.push_back(WorldBlockUpdate {
            sequence,
            dimension,
            pos,
            state,
        });
        while self.updates.len() > MAX_RETAINED_BLOCK_UPDATES {
            self.updates.pop_front();
        }
        Ok(())
    }

    fn since(&self, cursor: u64) -> Result<(Vec<WorldBlockUpdate>, u64), String> {
        let latest = self.latest_sequence();
        let oldest = self
            .updates
            .front()
            .map(|update| update.sequence)
            .unwrap_or(latest);
        if cursor.saturating_add(1) < oldest {
            return Err(format!(
                "PLAY session fell behind block update stream (cursor {cursor}, oldest {oldest})"
            ));
        }
        Ok((
            self.updates
                .iter()
                .filter(|update| update.sequence > cursor)
                .cloned()
                .collect(),
            latest,
        ))
    }
}

#[derive(Clone, Debug)]
struct WorldLevelEvent {
    sequence: u64,
    dimension: String,
    event: PublishedLevelEvent,
}

#[derive(Clone, Debug)]
enum PublishedLevelEvent {
    Level {
        pos: [i32; 3],
        id: i32,
        data: i32,
    },
    Sound {
        pos: [i32; 3],
        sound: String,
        volume: f32,
        pitch: f32,
    },
    Crack {
        entity_id: i32,
        pos: [i32; 3],
        progress: i32,
    },
}

#[derive(Default)]
struct LevelEventLog {
    next_sequence: u64,
    events: VecDeque<WorldLevelEvent>,
}

impl LevelEventLog {
    fn latest_sequence(&self) -> u64 {
        self.next_sequence.saturating_sub(1)
    }

    fn append(&mut self, dimension: String, event: PublishedLevelEvent) -> Result<(), String> {
        let sequence = self.next_sequence.max(1);
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| "PLAY level event sequence exhausted".to_owned())?;
        self.events.push_back(WorldLevelEvent {
            sequence,
            dimension,
            event,
        });
        while self.events.len() > MAX_RETAINED_BLOCK_UPDATES {
            self.events.pop_front();
        }
        Ok(())
    }

    fn since(&self, cursor: u64) -> Result<(Vec<WorldLevelEvent>, u64), String> {
        let latest = self.latest_sequence();
        let oldest = self
            .events
            .front()
            .map(|event| event.sequence)
            .unwrap_or(latest);
        if cursor.saturating_add(1) < oldest {
            return Err(format!(
                "PLAY session fell behind level event stream (cursor {cursor}, oldest {oldest})"
            ));
        }
        Ok((
            self.events
                .iter()
                .filter(|event| event.sequence > cursor)
                .cloned()
                .collect(),
            latest,
        ))
    }
}

fn generated_chunk_data(
    proto: &ProtoChunk,
    resources: &ServerResources,
    light: Option<&horizon_protocol::chunk::LightData>,
    has_skylight: bool,
) -> Result<horizon_world::chunk::ChunkData, String> {
    let mut data = proto
        .to_chunk_data(
            |id| {
                u16::try_from(id.0)
                    .ok()
                    .filter(|id| usize::from(*id) < resources.behaviours.table.block_count())
                    .map(|id| {
                        resources
                            .behaviours
                            .table
                            .info(BlockId(id))
                            .name
                            .to_string()
                    })
            },
            |id| {
                usize::try_from(id.0)
                    .ok()
                    .and_then(|index| FluidKind::ALL.get(index))
                    .map(|fluid| fluid.name().to_owned())
            },
        )
        .map_err(|error| format!("encode generated resident chunk: {error}"))?;
    if let Some(light) = light {
        let section_count = usize::try_from(proto.terrain.height() / 16)
            .map_err(|_| "generated chunk has invalid section count")?;
        if section_count == 0 || proto.terrain.height() % 16 != 0 {
            return Err("generated chunk has invalid section height".into());
        }
        let min_section = proto.terrain.min_y().div_euclid(16);
        let light_layers = light.layers(section_count + 2);
        let mut root = data.into_nbt();
        let horizon_world::nbt::Tag::Compound(fields) = &mut root.tag else {
            unreachable!("generated chunk root is a compound")
        };
        fields.insert(
            "Status".into(),
            horizon_world::nbt::Tag::String("minecraft:full".into()),
        );
        fields.insert("isLightOn".into(), horizon_world::nbt::Tag::Byte(1));
        let Some(horizon_world::nbt::Tag::List(sections)) =
            fields.get_mut(&horizon_world::nbt::JavaString::from("sections"))
        else {
            return Err("generated chunk sections are not a list".into());
        };
        for section in sections {
            let Some(horizon_world::nbt::Tag::Byte(y)) = section.get("Y") else {
                return Err("generated section has no section Y".into());
            };
            let index = usize::try_from(i32::from(*y) - min_section)
                .map_err(|_| "generated section is outside the light range")?;
            let Some((sky, block)) = light_layers.get(index + 1) else {
                return Err("generated section is outside the light payload".into());
            };
            let horizon_world::nbt::Tag::Compound(section) = section else {
                return Err("generated section is not a compound".into());
            };
            if has_skylight {
                if let Some(sky) = sky {
                    section.insert(
                        "SkyLight".into(),
                        horizon_world::nbt::Tag::ByteArray(sky.clone()),
                    );
                }
            }
            if let Some(block) = block {
                section.insert(
                    "BlockLight".into(),
                    horizon_world::nbt::Tag::ByteArray(block.clone()),
                );
            }
        }
        data = horizon_world::chunk::ChunkData::new(root)
            .map_err(|error| format!("finalize generated chunk data: {error}"))?;
    }
    Ok(data)
}

fn resolve_block_tick(resources: &ServerResources, name: &str) -> Option<TickTypeId> {
    resources
        .behaviours
        .table
        .block_by_name(name)
        .map(|id| TickTypeId(u32::from(id.0)))
}

fn resolve_fluid_tick(name: &str) -> Option<TickTypeId> {
    FluidKind::ALL
        .into_iter()
        .find(|fluid| fluid.name() == name)
        .map(|fluid| TickTypeId(fluid.index() as u32))
}

fn block_tick_name(resources: &ServerResources, id: TickTypeId) -> Option<String> {
    u16::try_from(id.0)
        .ok()
        .filter(|id| usize::from(*id) < resources.behaviours.table.block_count())
        .map(|id| {
            resources
                .behaviours
                .table
                .info(BlockId(id))
                .name
                .to_string()
        })
}

fn fluid_tick_name(id: TickTypeId) -> Option<String> {
    usize::try_from(id.0)
        .ok()
        .and_then(|index| FluidKind::ALL.get(index))
        .map(|fluid| fluid.name().to_owned())
}

fn generate_terrain_region(
    worker: &mut GeneratorWorker,
    center: ChunkPos,
) -> Result<[ProtoChunk; CHUNK_COUNT], String> {
    let mut chunks = Vec::with_capacity(CHUNK_COUNT);
    for z in center.z - CHUNK_RADIUS..=center.z + CHUNK_RADIUS {
        for x in center.x - CHUNK_RADIUS..=center.x + CHUNK_RADIUS {
            chunks.push(
                worker
                    .generate_proto(ChunkPos::new(x, z))
                    .map_err(|error| format!("generate chunk ({x},{z}): {error}"))?,
            );
        }
    }
    let region: [ProtoChunk; CHUNK_COUNT] = chunks
        .try_into()
        .map_err(|_| "worldgen did not produce a 3×3 chunk region")?;
    if region[4].position() != center {
        return Err(format!(
            "worldgen did not generate the requested center chunk ({},{})",
            center.x, center.z
        ));
    }
    Ok(region)
}

const CHUNK_COUNT_SIDE: usize = (CHUNK_RADIUS as usize * 2) + 1;

fn save_runtime_dirty_chunks(
    runtime: &mut LocalWorldRuntime<'static>,
    resources: &ServerResources,
) -> Result<usize, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    let report = runtime
        .save_dirty_chunks(
            usize::MAX,
            Compression::Zlib,
            timestamp,
            Limits::saved(),
            |id| block_tick_name(resources, id),
            fluid_tick_name,
        )
        .map_err(|error| format!("save dirty PLAY chunks: {error}"))?;
    if report.remaining_dirty != 0 {
        return Err(format!(
            "{} PLAY chunks remained dirty after a full save sweep",
            report.remaining_dirty
        ));
    }
    Ok(report.saved)
}

fn encode_center_chunk(
    proto: &ProtoChunk,
    light: horizon_protocol::chunk::LightData,
    resources: &ServerResources,
) -> Result<(Vec<u8>, horizon_world::chunk::ChunkData), String> {
    let position = proto.position();
    let mut chunk =
        play_chunk::encode_generated_chunk(proto, resources.registry, &resources.behaviours)?;
    let chunk_data = generated_chunk_data(proto, resources, Some(&light), true)?;
    play_chunk::overlay_saved_block_entities(&mut chunk, &chunk_data, resources)?;
    let packet = play::Clientbound::LevelChunkWithLight(play::LevelChunkWithLight {
        x: position.x,
        z: position.z,
        chunk,
        light,
    })
    .encode();
    Ok((packet, chunk_data))
}

/// Shared world admission state for the PLAY listener. The resident dimension
/// and generated chunk packet cache are shared; authoritative simulation and
/// cross-player updates remain incomplete.
pub(crate) struct PlayServer {
    world: Arc<PreparedPlayWorld>,
    players: Mutex<PlayerRegistry>,
}

struct PlayerRegistry {
    by_uuid: HashMap<u128, i32>,
    next_player_id: i32,
    max_players: usize,
}

impl Default for PlayerRegistry {
    fn default() -> Self {
        Self {
            by_uuid: HashMap::new(),
            next_player_id: 1,
            max_players: 20,
        }
    }
}

/// Owns the shared world clock thread. Dropping or stopping the listener must
/// stop and join this thread before its world resources can be released.
pub(crate) struct WorldTickLoop {
    stop: SyncSender<()>,
    worker: Option<thread::JoinHandle<Result<(), String>>>,
}

impl WorldTickLoop {
    fn join_worker(&mut self) -> Result<(), String> {
        let _ = self.stop.send(());
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| "PLAY world tick owner panicked".to_owned())?
    }
}

impl Drop for WorldTickLoop {
    fn drop(&mut self) {
        if let Err(error) = self.join_worker() {
            eprintln!("Borizon final world save: {error}");
        }
    }
}

impl PlayServer {
    pub(crate) fn new(world: Arc<PreparedPlayWorld>) -> Self {
        let max_players = world.max_players;
        Self {
            world,
            players: Mutex::new(PlayerRegistry {
                next_player_id: 1,
                max_players,
                ..PlayerRegistry::default()
            }),
        }
    }

    /// Start one server-owned world clock. Connection threads never advance
    /// global time, so adding players cannot accelerate the world.
    pub(crate) fn start_world_tick_loop(self: &Arc<Self>) -> std::io::Result<WorldTickLoop> {
        let server = Arc::clone(self);
        let (stop, stop_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("borizon-world-tick".into())
            .spawn(move || -> Result<(), String> {
                // Main's autosave interval at the fixed 20 TPS used here:
                // 300 seconds of ticks, with its 100-tick minimum.
                const AUTOSAVE_TICKS: i64 = 20 * 300;
                let mut ticks_until_autosave = AUTOSAVE_TICKS.max(100);
                let mut next_tick = Instant::now() + SERVER_TICK;
                loop {
                    let now = Instant::now();
                    if now < next_tick {
                        match stop_rx.recv_timeout(next_tick - now) {
                            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                            Err(RecvTimeoutError::Timeout) => {}
                        }
                    }
                    while Instant::now() >= next_tick {
                        ticks_until_autosave -= 1;
                        let autosave = ticks_until_autosave <= 0;
                        if autosave {
                            ticks_until_autosave = AUTOSAVE_TICKS.max(100);
                        }
                        if let Err(error) = Self::tick_shared_world(&server.world, autosave) {
                            eprintln!("Borizon world tick: {error}");
                        }
                        next_tick += SERVER_TICK;
                        match stop_rx.try_recv() {
                            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                                return Self::save_final_world(&server.world);
                            }
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                    }
                }
                Self::save_final_world(&server.world)
            })?;
        Ok(WorldTickLoop {
            stop,
            worker: Some(worker),
        })
    }

    fn save_final_world(world: &PreparedPlayWorld) -> Result<(), String> {
        let mut runtime = world
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        save_runtime_dirty_chunks(&mut runtime, &world.resources)?;
        runtime
            .save_world_data()
            .map_err(|error| format!("save final PLAY world data: {error}"))?;
        runtime
            .sync_regions()
            .map_err(|error| format!("sync final PLAY Anvil regions: {error}"))?;
        Ok(())
    }

    fn tick_shared_world(world: &PreparedPlayWorld, autosave: bool) -> Result<(), String> {
        let mut runtime = world
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        runtime.advance_game_time();
        let daylight = runtime.game_rules().bool("advance_time");
        runtime
            .advance_dimension_day_time("minecraft:overworld", daylight)
            .ok_or_else(|| "PLAY Overworld disappeared from the shared world runtime".to_owned())?;
        runtime.tick_scheduled(&world.resources.behaviours);
        runtime.tick_random_chunks(&world.resources.behaviours);
        runtime.run_block_events(&world.resources.behaviours);
        let resident = runtime
            .dimension("minecraft:overworld")
            .expect("the PLAY Overworld is registered")
            .resident();
        let border = runtime
            .dimension("minecraft:overworld")
            .expect("the PLAY Overworld is registered")
            .border();
        world.item_entities.tick(|item, wanted| {
            let block = BlockPos::new(
                item.position[0].floor() as i32,
                item.position[1].floor() as i32,
                item.position[2].floor() as i32,
            );
            if !resident.level().is_loaded(block) {
                return Ok(None);
            }
            match crate::play_collision::resolve_item_move(
                resident.level(),
                border.geometry(),
                &world.resources,
                item.id,
                item.position,
                wanted,
                item.on_ground,
            ) {
                Ok(moved) => Ok(Some(moved)),
                Err(error) if error.contains("unloaded chunk") => Ok(None),
                Err(error) => Err(format!("tick dropped item {}: {error}", item.id)),
            }
        })?;
        let changes = runtime.take_block_changes();
        let events = runtime.take_level_events();
        let save_result = if autosave {
            save_runtime_dirty_chunks(&mut runtime, &world.resources)
                .and_then(|_| {
                    runtime
                        .save_world_data()
                        .map_err(|error| format!("autosave PLAY world data: {error}"))
                })
                .and_then(|_| {
                    runtime
                        .sync_regions()
                        .map_err(|error| format!("autosave sync PLAY Anvil regions: {error}"))
                })
        } else {
            Ok(())
        };
        drop(runtime);
        world.publish_block_changes(changes)?;
        world.publish_level_events(events)?;
        save_result?;
        Ok(())
    }

    pub(crate) fn online_players(&self) -> usize {
        self.players
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .online()
    }

    pub(crate) fn max_players(&self) -> usize {
        self.world.max_players
    }

    fn register_player(&self, uuid: u128, name: &str) -> Result<i32, String> {
        self.players
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register(uuid, name)
    }

    pub(crate) fn initialize(&self, configured: ConfiguredConnection) -> Result<(), String> {
        let uuid = configured.profile.uuid;
        let player_id = self.register_player(uuid, &configured.profile.name)?;
        let _permit = PlayerPermit {
            players: &self.players,
            uuid,
        };
        self.world.initialize(configured, player_id)
    }
}

impl PlayerRegistry {
    fn online(&self) -> usize {
        self.by_uuid.len()
    }

    fn register(&mut self, uuid: u128, name: &str) -> Result<i32, String> {
        if self.by_uuid.contains_key(&uuid) {
            return Err(format!("player {name} is already connected"));
        }
        if self.by_uuid.len() >= self.max_players {
            return Err("server is full".into());
        }
        let id = self.next_player_id;
        if id <= 0 {
            return Err("server player ID space is exhausted".into());
        }
        self.next_player_id = id.checked_add(1).unwrap_or(0);
        self.by_uuid.insert(uuid, id);
        Ok(id)
    }
}

struct PlayerPermit<'a> {
    players: &'a Mutex<PlayerRegistry>,
    uuid: u128,
}

struct PlayerChunkAdmission<'a> {
    world: &'a PreparedPlayWorld,
    player_id: i32,
    release_on_drop: bool,
}

impl Drop for PlayerChunkAdmission<'_> {
    fn drop(&mut self) {
        if self.release_on_drop {
            let _ = self.world.remove_player_chunk(self.player_id);
        }
    }
}

impl Drop for PlayerPermit<'_> {
    fn drop(&mut self) {
        self.players
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .by_uuid
            .remove(&self.uuid);
    }
}

impl PreparedPlayWorld {
    #[cfg(test)]
    pub(crate) fn build(
        resources: Arc<ServerResources>,
        seed: i64,
        world_root: &Path,
    ) -> Result<Self, String> {
        Self::build_with_settings(
            resources,
            seed,
            world_root,
            DEFAULT_VIEW_DISTANCE,
            DEFAULT_SIMULATION_DISTANCE,
            0,
            1,
            20,
            false,
        )
    }

    pub(crate) fn build_with_settings(
        resources: Arc<ServerResources>,
        seed: i64,
        world_root: &Path,
        view_distance: i32,
        simulation_distance: i32,
        game_mode: i32,
        difficulty: i32,
        max_players: usize,
        hardcore: bool,
    ) -> Result<Self, String> {
        let dimension_type = resources
            .synchronized_registries
            .entry_index("minecraft:dimension_type", "minecraft:overworld")
            .ok_or("synchronized dimension_type registry has no minecraft:overworld")?;
        let saved_world = horizon_world::world_saved_data::WorldSavedData::new(world_root);
        saved_world
            .restore_level_dat_old()
            .map_err(|error| format!("restore PLAY level.dat_old: {error}"))?;
        let level_path = world_root.join("level.dat");
        if !level_path.exists()
            && std::fs::read_dir(world_root)
                .ok()
                .and_then(|mut entries| entries.next())
                .is_some()
        {
            return Err(format!(
                "PLAY world {} contains data but has no readable level.dat or level.dat_old",
                world_root.display()
            ));
        }
        if let Some(root) = horizon_world::datafix::files::read_dat(&level_path)
            .map_err(|error| format!("read PLAY level.dat: {error}"))?
        {
            let data = root.get("Data");
            let version = data.map_or(-1, |data| {
                horizon_world::datafix::files::data_version(data, -1)
            });
            if version > horizon_world::datafix::CURRENT {
                return Err(format!(
                    "PLAY world DataVersion {version} is newer than this server ({})",
                    horizon_world::datafix::CURRENT
                ));
            }
            if version < horizon_world::datafix::CURRENT {
                horizon_world::datafix::files::upgrade_world_with(
                    world_root,
                    &horizon_world::datafix::progress::UpgradeProgress::new(),
                )
                .map_err(|error| format!("upgrade PLAY world data: {error}"))?;
            }
        }
        let saved_basics = saved_world
            .load_world_basics()
            .map_err(|error| format!("read saved PLAY world settings: {error}"))?;
        let seed = saved_basics.seed.unwrap_or(seed);
        let generator = OverworldGenerator::new(resources.worldgen.clone(), seed)
            .map_err(|error| format!("create vanilla Overworld generator: {error}"))?;
        let origin = ChunkPos::new(0, 0);
        let mut worker = generator.worker();
        let (spawn, spawn_center, initial_terrain) = if let Some([x, y, z]) = saved_basics.spawn {
            let spawn = [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5];
            let center = chunk_center(spawn);
            (spawn, center, None)
        } else {
            // Spawn selection needs terrain and its surface map; feature
            // decoration is performed later by the same staged pipeline used
            // for every streamed chunk.
            let origin_region = generate_terrain_region(&mut worker, origin)?;
            let spawn = find_spawn(&origin_region, &resources)?;
            let center = chunk_center(spawn);
            (spawn, center, Some(origin_region))
        };

        let mut runtime = LocalWorldRuntime::open(world_root, &[]).map_err(|error| {
            format!("open PLAY world storage {}: {error}", world_root.display())
        })?;
        for key in [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ] {
            let height = horizon_world::worldgen::overrides::LevelHeight::vanilla(key)
                .ok_or_else(|| format!("missing vanilla height for dimension {key}"))?;
            let resident = ResidentDimension::new(
                key,
                ResidentLevel::new(resources.registry, height.min_y, height.height, seed)
                    .map_err(|error| format!("create resident {key}: {error}"))?,
                Ticks::new(seed as i32),
            );
            runtime
                .add_dimension(resident)
                .map_err(|error| format!("register PLAY dimension {key}: {error}"))?;
        }
        let chunk_packets = Vec::with_capacity(CHUNK_COUNT);
        let chunk_packet_cache = HashMap::with_capacity(CHUNK_COUNT);
        let difficulty_name =
            ["peaceful", "easy", "normal", "hard"][difficulty.clamp(0, 3) as usize];
        let level_name = world_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("world");
        horizon_world::world_saved_data::WorldSavedData::new(world_root)
            .save_world_basics(
                seed,
                [
                    spawn[0].floor() as i32,
                    spawn[1].floor() as i32,
                    spawn[2].floor() as i32,
                ],
                level_name,
                game_mode,
                difficulty_name,
                hardcore,
            )
            .map_err(|error| format!("save PLAY world seed and spawn: {error}"))?;
        let chunk_generation =
            ChunkGeneration::start(generator, resources.clone(), initial_terrain)?;
        let mut prepared = Self {
            world_root: world_root.to_owned(),
            resources,
            dimension_type,
            spawn,
            chunk_packets,
            chunk_packet_cache: Mutex::new(chunk_packet_cache),
            chunk_generation: Some(chunk_generation),
            view_distance: view_distance.clamp(2, 32),
            simulation_distance: simulation_distance.clamp(2, 32),
            game_mode: game_mode.clamp(0, 3),
            difficulty: difficulty.clamp(0, 3),
            max_players: max_players.max(1),
            hardcore,
            runtime: Mutex::new(runtime),
            player_chunks: Mutex::new(HashMap::new()),
            block_updates: Mutex::new(BlockUpdateLog::default()),
            level_events: Mutex::new(LevelEventLog::default()),
            item_entities: ItemEntityStore::default(),
            loot_random: Mutex::new(LegacyRandom::new(seed)),
        };
        prepared.prepare_spawn_chunks(spawn_center)?;
        Ok(prepared)
    }

    pub(crate) fn initialize(
        &self,
        mut configured: ConfiguredConnection,
        player_id: i32,
    ) -> Result<(), String> {
        let saved_player = crate::player_data::load(&self.world_root, configured.profile.uuid)?;
        let player_position = saved_player.map_or(self.spawn, |saved| saved.position);
        let player_yaw = saved_player.map_or(0.0, |saved| saved.yaw);
        let player_pitch = saved_player.map_or(0.0, |saved| saved.pitch);
        let view_distance = configured
            .client_information
            .as_ref()
            .map_or(self.view_distance, |info| i32::from(info.view_distance))
            .clamp(2, self.view_distance);
        let player_block = BlockPos::new(
            player_position[0].floor() as i32,
            player_position[1].floor() as i32,
            player_position[2].floor() as i32,
        );
        let player_chunk = chunk_center(player_position);
        let mut admission = PlayerChunkAdmission {
            world: self,
            player_id,
            release_on_drop: true,
        };
        self.set_player_chunk(player_id, player_chunk)?;
        let initial_chunks = self.prepare_player_chunks(player_chunk)?;
        let player_chunk_loaded = || {
            self.runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .dimension("minecraft:overworld")
                .expect("the Play Overworld is registered")
                .resident()
                .level()
                .is_loaded(player_block)
        };
        if !player_chunk_loaded() {
            return Err("player ticket did not load the saved PLAY position".into());
        }
        // The client sends its terminal CONFIGURATION acknowledgement before
        // installing its PLAY encoder. The reference server hands the session
        // to its game tick before sending the join sequence; mirror that tick
        // boundary so the client's protocol swap has completed.
        std::thread::sleep(PLAY_TICK_BARRIER);
        let dimension = "minecraft:overworld".to_owned();
        let spawn_block = [
            self.spawn[0].floor() as i32,
            self.spawn[1].floor() as i32,
            self.spawn[2].floor() as i32,
        ];
        let border = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .dimension("minecraft:overworld")
            .expect("the Play Overworld is registered")
            .border()
            .clone();
        let [border_center_x, border_center_z] = border.center();
        let packets = [
            play::Clientbound::Login(play::Login {
                player_id,
                hardcore: self.hardcore,
                levels: vec![
                    "minecraft:overworld".into(),
                    "minecraft:the_nether".into(),
                    "minecraft:the_end".into(),
                ],
                max_players: self.max_players.min(i32::MAX as usize) as i32,
                chunk_radius: view_distance,
                simulation_distance: self.simulation_distance,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: play::SpawnInfo {
                    dimension_type: self.dimension_type,
                    dimension: dimension.clone(),
                    seed: 0,
                    game_type: self.game_mode,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                },
                online_mode: false,
                enforces_secure_chat: false,
            }),
            play::Clientbound::ChangeDifficulty(play::ChangeDifficulty {
                difficulty: self.difficulty,
                locked: false,
            }),
            play::Clientbound::PlayerAbilities(play::PlayerAbilities {
                flags: match self.game_mode {
                    1 => 0b1101, // invulnerable, may fly, instant build
                    3 => 0b0111, // invulnerable, flying, may fly
                    _ => 0,
                },
                flying_speed: 0.05,
                walking_speed: 0.1,
            }),
            play::Clientbound::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
                pos: play::GlobalPos {
                    dimension,
                    pos: spawn_block,
                },
                yaw: 0.0,
                pitch: 0.0,
            }),
            // ClientPacketListener keeps the "Loading terrain" screen up until
            // the server announces LEVEL_CHUNKS_LOAD_START (event 13). Vanilla
            // sends it with PlayerList.sendLevelInfo before the chunk stream.
            play::Clientbound::GameEvent(play::GameEvent {
                event: 13,
                param: 0.0,
            }),
            play::Clientbound::SetChunkCacheRadius(play::Distance {
                distance: view_distance,
            }),
            play::Clientbound::SetSimulationDistance(play::Distance {
                distance: self.simulation_distance,
            }),
            play::Clientbound::InitializeBorder(play::InitializeBorder {
                center_x: border_center_x,
                center_z: border_center_z,
                old_size: border.size(),
                new_size: border.target_size(),
                lerp_time: border.remaining_ticks(),
                absolute_max_size: border.absolute_max_size(),
                warning_blocks: border.warning_distance,
                warning_time: border.warning_time,
            }),
            play::Clientbound::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: player_chunk.x,
                z: player_chunk.z,
            }),
            play::Clientbound::ChunkBatchStart(horizon_protocol::packets::Empty),
        ];
        for packet in packets {
            configured
                .connection
                .send(&packet.encode())
                .map_err(|error| format!("send PLAY bootstrap packet: {error}"))?;
        }
        for packet in &initial_chunks {
            configured
                .connection
                .send(packet)
                .map_err(|error| format!("send spawn chunk: {error}"))?;
        }
        configured
            .connection
            .send(
                &play::Clientbound::ChunkBatchFinished(play::ChunkBatchFinished {
                    size: initial_chunks.len() as i32,
                })
                .encode(),
            )
            .map_err(|error| format!("finish spawn chunk batch: {error}"))?;
        configured
            .connection
            .send(
                &play::Clientbound::PlayerPosition(play::PlayerPosition {
                    teleport: 1,
                    position: player_position,
                    delta: [0.0; 3],
                    yaw: player_yaw,
                    pitch: player_pitch,
                    relatives: 0,
                })
                .encode(),
            )
            .map_err(|error| format!("send initial player position: {error}"))?;
        println!(
            "Horizon: entered initial PLAY for {} at ({:.1}, {:.1}, {:.1}); gameplay simulation is incomplete",
            configured.profile.name, player_position[0], player_position[1], player_position[2]
        );
        let player_name = configured.profile.name.clone();
        let player_uuid = configured.profile.uuid;
        admission.release_on_drop = false;
        let result = serve_until_disconnect_with_player_data(
            configured,
            player_position,
            player_yaw,
            player_pitch,
            player_id,
            player_name,
            player_uuid,
            Some(self),
            view_distance,
            player_chunk,
        );
        let cleanup = self.finish_player_session(player_id);
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn finish_player_session(&self, player_id: i32) -> Result<(), String> {
        let mut first_error = self.remove_player_chunk(player_id).err();
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Err(error) = runtime.save_world_data() {
            first_error
                .get_or_insert_with(|| format!("save PLAY world data after session: {error}"));
        }
        if let Err(error) = runtime.sync_regions() {
            first_error
                .get_or_insert_with(|| format!("sync PLAY Anvil regions after session: {error}"));
        }
        first_error.map_or(Ok(()), Err)
    }

    fn prepare_player_chunks(&self, center: ChunkPos) -> Result<Vec<Vec<u8>>, String> {
        let expected = region_positions(center);
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            self.poll_chunk_generation()?;
            let cache = self
                .chunk_packet_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if expected.iter().all(|position| cache.contains_key(position)) {
                return Ok(expected
                    .iter()
                    .map(|position| {
                        cache
                            .get(position)
                            .expect("all initial player chunks were prepared")
                            .clone()
                    })
                    .collect());
            }
            drop(cache);
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out preparing initial player chunks around ({},{})",
                    center.x, center.z
                ));
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn save_player_position(&self, uuid: u128, player: &PlayerSession) -> Result<(), String> {
        crate::player_data::save(
            &self.world_root,
            uuid,
            player.position,
            player.yaw,
            player.pitch,
        )
    }

    fn set_player_chunk(&self, player_id: i32, center: ChunkPos) -> Result<(), String> {
        let changed = {
            let mut players = self
                .player_chunks
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if players.get(&player_id) == Some(&center) {
                false
            } else {
                players.insert(player_id, center);
                true
            }
        };
        if changed {
            self.reconcile_player_chunks()?;
            self.prune_chunk_packet_cache();
        }
        Ok(())
    }

    fn remove_player_chunk(&self, player_id: i32) -> Result<(), String> {
        let removed = self
            .player_chunks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&player_id)
            .is_some();
        if removed {
            self.reconcile_player_chunks()?;
            self.prune_chunk_packet_cache();
        }
        Ok(())
    }

    fn reconcile_player_chunks(&self) -> Result<(), String> {
        let mut players = self
            .player_chunks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .map(|chunk| (chunk.x, chunk.z))
            .collect::<Vec<_>>();
        players.sort_unstable();

        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let game_time = runtime.game_time();
        let dimension = runtime
            .dimension_mut("minecraft:overworld")
            .expect("the Play Overworld is registered");
        dimension
            .resident_mut()
            .update_chunk_tickets(Some(ChunkTicketSources {
                players,
                view_distance: self.view_distance,
                simulation_distance: self.simulation_distance,
                ..ChunkTicketSources::default()
            }));
        let plan = dimension
            .lifecycle_candidates(MAX_LIFECYCLE_CANDIDATES)
            .map_err(|error| format!("plan PLAY chunk lifecycle: {error}"))?;
        dimension
            .reconcile_ticketed_chunks(
                plan.candidates,
                Limits::saved(),
                game_time,
                |name| resolve_block_tick(&self.resources, name),
                resolve_fluid_tick,
            )
            .map_err(|error| format!("reconcile PLAY chunk tickets: {error}"))?;
        Ok(())
    }

    fn resolve_player_move(
        &self,
        entity_id: i32,
        from: [f64; 3],
        requested: [f64; 3],
        fall_distance: f64,
        on_ground: bool,
    ) -> Result<([f64; 3], bool), String> {
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dimension = runtime
            .dimension("minecraft:overworld")
            .expect("the Play Overworld is registered");
        crate::play_collision::resolve_player_move(
            dimension.resident().level(),
            dimension.border().geometry(),
            &self.resources,
            entity_id,
            from,
            requested,
            fall_distance,
            on_ground,
        )
    }

    fn request_chunk(&self, position: ChunkPos) -> Result<(), String> {
        if self
            .chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&position)
        {
            return Ok(());
        }
        self.chunk_generation
            .as_ref()
            .ok_or_else(|| "server chunk-generation worker is unavailable".to_owned())?
            .request(position)
    }

    fn load_saved_chunk_snapshots(
        &self,
        positions: HashSet<ChunkPos>,
    ) -> Result<HashMap<ChunkPos, Option<horizon_world::chunk::ChunkData>>, String> {
        if positions.is_empty() {
            return Ok(HashMap::new());
        }
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let game_time = runtime.game_time();
        let dimension = runtime
            .dimension_mut("minecraft:overworld")
            .expect("the Play Overworld is registered");
        let mut snapshots = HashMap::with_capacity(positions.len());
        for position in positions {
            dimension
                .load_chunk_from_region(
                    position,
                    Limits::saved(),
                    game_time,
                    |name| resolve_block_tick(&self.resources, name),
                    resolve_fluid_tick,
                )
                .map_err(|error| {
                    format!(
                        "read saved neighbor chunk ({},{}): {error}",
                        position.x, position.z
                    )
                })?;
            let active = dimension.active_chunk_data(position).is_some();
            let mut data = dimension.retained_chunk_data(position).cloned();
            if active && let Some(data) = &mut data {
                dimension
                    .resident_mut()
                    .save_chunk_data(
                        data,
                        game_time,
                        |id| block_tick_name(&self.resources, id),
                        fluid_tick_name,
                    )
                    .map_err(|error| {
                        format!(
                            "snapshot active saved neighbor ({},{}): {error}",
                            position.x, position.z
                        )
                    })?;
            }
            snapshots.insert(position, data);
        }
        Ok(snapshots)
    }

    fn apply_saved_final_writes(&self, writes: Vec<SavedFinalWrite>) -> Result<(), String> {
        if writes.is_empty() {
            return Ok(());
        }
        let changes = {
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let game_time = runtime.game_time();
            let dimension = runtime
                .dimension_mut("minecraft:overworld")
                .expect("the Play Overworld is registered");
            let mut block_entity_updates =
                HashMap::<ChunkPos, Vec<(BlockPos, Option<horizon_world::nbt::Compound>)>>::new();
            for write in writes {
                let position = write.position;
                let chunk = position.chunk();
                if dimension.is_chunk_parked(chunk) {
                    dimension
                        .activate_parked_chunk(
                            chunk,
                            game_time,
                            |name| resolve_block_tick(&self.resources, name),
                            resolve_fluid_tick,
                        )
                        .map_err(|error| {
                            format!(
                                "activate saved chunk for final write ({},{}): {error}",
                                chunk.x, chunk.z
                            )
                        })?;
                }
                if !dimension.resident().level().is_loaded(position) {
                    dimension
                        .load_chunk_from_region(
                            chunk,
                            Limits::saved(),
                            game_time,
                            |name| resolve_block_tick(&self.resources, name),
                            resolve_fluid_tick,
                        )
                        .map_err(|error| {
                            format!(
                                "load saved chunk for final write ({},{}): {error}",
                                chunk.x, chunk.z
                            )
                        })?;
                }
                if dimension
                    .resident_mut()
                    .level_mut()
                    .set_raw(position, write.state)
                    .is_none()
                {
                    return Err(format!(
                        "could not apply final worldgen write at ({},{},{})",
                        position.x, position.y, position.z
                    ));
                }
                block_entity_updates
                    .entry(chunk)
                    .or_default()
                    .push((position, write.block_entity));
            }
            for (chunk, updates) in block_entity_updates {
                dimension
                    .update_block_entities(chunk, updates)
                    .map_err(|error| {
                        format!(
                            "apply generated block entities for ({},{}): {error}",
                            chunk.x, chunk.z
                        )
                    })?;
            }
            runtime.take_block_changes()
        };
        self.publish_block_changes(changes)
    }

    fn advance_chunk_generation(
        &self,
        generation: &ChunkGeneration,
        centers: &[ChunkPos],
        radius: i32,
    ) -> Result<Vec<(ChunkPos, (Vec<u8>, horizon_world::chunk::ChunkData))>, String> {
        let positions = generation.saved_snapshot_positions(centers, radius);
        let snapshots = self.load_saved_chunk_snapshots(positions)?;
        let (ready, final_writes) = generation.advance(centers, radius, snapshots)?;
        self.apply_saved_final_writes(final_writes)?;
        Ok(ready)
    }

    fn prepare_spawn_chunks(&mut self, center: ChunkPos) -> Result<(), String> {
        let generation = self
            .chunk_generation
            .as_ref()
            .ok_or_else(|| "server chunk-generation worker is unavailable".to_owned())?;
        let expected = square_chunks(center, CHUNK_RADIUS);
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            for (position, (packet, chunk_data)) in
                self.advance_chunk_generation(generation, &[center], CHUNK_RADIUS)?
            {
                self.install_chunk(position, packet, chunk_data)?;
            }
            let cache = self
                .chunk_packet_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if expected.iter().all(|position| cache.contains_key(position)) {
                self.chunk_packets = region_positions(center)
                    .iter()
                    .map(|position| {
                        cache
                            .get(position)
                            .cloned()
                            .expect("all initial spawn chunks were prepared")
                    })
                    .collect();
                break;
            }
            drop(cache);
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out preparing the initial 3×3 spawn chunks around ({},{})",
                    center.x, center.z
                ));
            }
            thread::sleep(Duration::from_millis(2));
        }
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        save_runtime_dirty_chunks(&mut runtime, &self.resources)?;
        Ok(())
    }

    fn poll_chunk_generation(&self) -> Result<(), String> {
        let Some(generation) = &self.chunk_generation else {
            return Ok(());
        };
        let centers = self
            .player_chunks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .copied()
            .collect::<Vec<_>>();
        for (position, (packet, chunk_data)) in
            self.advance_chunk_generation(generation, &centers, self.view_distance)?
        {
            // A worker may finish a job after the last player moved away.
            // Like the reference scheduler, do not make that stale result
            // resident or keep its packet in the view cache.
            if !centers.is_empty()
                && !centers
                    .iter()
                    .any(|center| is_tracked(*center, self.view_distance, position))
            {
                continue;
            }
            self.install_chunk(position, packet, chunk_data)?;
        }
        Ok(())
    }

    fn prune_chunk_packet_cache(&self) {
        let centers = self
            .player_chunks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .copied()
            .collect::<Vec<_>>();
        self.chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|position, _| {
                centers
                    .iter()
                    .any(|center| is_tracked(*center, self.view_distance, *position))
            });
    }

    #[cfg(test)]
    fn prepare_chunk(&self, position: ChunkPos) -> Result<(), String> {
        self.request_chunk(position)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.poll_chunk_generation()?;
            if self
                .chunk_packet_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .contains_key(&position)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out generating PLAY chunk ({},{})",
                    position.x, position.z
                ));
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn install_chunk(
        &self,
        position: ChunkPos,
        mut packet: Vec<u8>,
        chunk_data: horizon_world::chunk::ChunkData,
    ) -> Result<(), String> {
        if self
            .chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&position)
        {
            return Ok(());
        }
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let game_time = runtime.game_time();
        let level = runtime
            .dimension("minecraft:overworld")
            .expect("the Play Overworld is registered")
            .resident()
            .level();
        let min_y = level.min_y();
        let height = level.max_y() - min_y;
        let probe = BlockPos::new(position.x * 16, min_y, position.z * 16);
        let already_resident = runtime
            .dimension("minecraft:overworld")
            .expect("the Play Overworld is registered")
            .resident()
            .level()
            .is_loaded(probe);
        let dimension = runtime
            .dimension_mut("minecraft:overworld")
            .expect("the Play Overworld is registered");
        if already_resident {
            // A chunk can already be resident as a worldgen support chunk even
            // when it has not yet been sent to this player. Snapshot current
            // block/tick state without sweeping every dirty region to disk.
            let mut snapshot = dimension
                .active_chunk_data(position)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "resident streamed chunk ({},{}) has no active saved data",
                        position.x, position.z
                    )
                })?;
            dimension
                .resident_mut()
                .save_chunk_data(
                    &mut snapshot,
                    game_time,
                    |id| block_tick_name(&self.resources, id),
                    fluid_tick_name,
                )
                .map_err(|error| format!("snapshot resident streamed chunk: {error}"))?;
            play_chunk::overlay_saved_chunk_data(
                &mut packet,
                &snapshot,
                min_y,
                height,
                &self.resources,
            )?;
        } else {
            let loaded = dimension
                .load_chunk_from_region(
                    position,
                    Limits::saved(),
                    game_time,
                    |name| resolve_block_tick(&self.resources, name),
                    resolve_fluid_tick,
                )
                .map_err(|error| {
                    format!(
                        "load persisted streamed chunk ({},{}): {error}",
                        position.x, position.z
                    )
                })?;
            if loaded {
                let saved = dimension
                    .active_chunk_data(position)
                    .expect("loaded PLAY chunk retains its saved NBT");
                play_chunk::overlay_saved_chunk_data(
                    &mut packet,
                    saved,
                    min_y,
                    height,
                    &self.resources,
                )?;
            } else {
                dimension
                    .insert_generated_chunk(
                        chunk_data,
                        game_time,
                        |name| resolve_block_tick(&self.resources, name),
                        resolve_fluid_tick,
                    )
                    .map_err(|error| {
                        format!(
                            "install streamed PLAY chunk ({},{}): {error}",
                            position.x, position.z
                        )
                    })?;
            }
        }
        drop(runtime);
        self.chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(position, packet);
        Ok(())
    }

    fn patch_cached_block_state(
        &self,
        pos: BlockPos,
        state: horizon_world::registry::BlockStateId,
    ) -> Result<(), String> {
        let (min_y, height) = {
            let runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let level = runtime
                .dimension("minecraft:overworld")
                .expect("the Play Overworld is registered")
                .resident()
                .level();
            let min_y = level.min_y();
            (min_y, level.max_y() - min_y)
        };
        let chunk = pos.chunk();
        let mut cache = self
            .chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(packet) = cache.get_mut(&chunk) else {
            return Ok(());
        };
        play_chunk::patch_cached_block_state(
            packet,
            pos,
            state,
            min_y,
            height,
            &self.resources.behaviours,
        )
    }

    fn publish_block_changes(
        &self,
        changes: Vec<(String, BlockPos, horizon_world::registry::BlockStateId)>,
    ) -> Result<(), String> {
        let mut log = self
            .block_updates
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (dimension, pos, state) in changes {
            if dimension == "minecraft:overworld" {
                self.patch_cached_block_state(pos, state)?;
            }
            log.append(dimension, [pos.x, pos.y, pos.z], state.0 as i32)?;
        }
        Ok(())
    }

    fn block_updates_since(&self, cursor: u64) -> Result<(Vec<WorldBlockUpdate>, u64), String> {
        self.block_updates
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .since(cursor)
    }

    fn latest_block_update(&self) -> u64 {
        self.block_updates
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .latest_sequence()
    }

    fn publish_level_events(&self, events: Vec<(String, LevelEvent)>) -> Result<(), String> {
        let mut log = self
            .level_events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (dimension, event) in events {
            let event = match event {
                LevelEvent::LevelEvent { id, pos, data } => PublishedLevelEvent::Level {
                    pos: [pos.x, pos.y, pos.z],
                    id,
                    data,
                },
                LevelEvent::Sound {
                    pos,
                    sound,
                    volume,
                    pitch,
                } => PublishedLevelEvent::Sound {
                    pos: [pos.x, pos.y, pos.z],
                    sound: sound.to_string(),
                    volume,
                    pitch,
                },
                _ => continue,
            };
            log.append(dimension, event)?;
        }
        Ok(())
    }

    fn publish_crack_progress(
        &self,
        entity_id: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> Result<(), String> {
        self.level_events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .append(
                "minecraft:overworld".into(),
                PublishedLevelEvent::Crack {
                    entity_id,
                    pos,
                    progress,
                },
            )
    }

    fn level_events_since(&self, cursor: u64) -> Result<(Vec<WorldLevelEvent>, u64), String> {
        self.level_events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .since(cursor)
    }

    fn latest_level_event_sequence(&self) -> u64 {
        self.level_events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .latest_sequence()
    }

    fn update_client_chunk_view(
        &self,
        writer: &mut horizon_protocol::connection::ConnWriter,
        player: &mut PlayerSession,
        center: ChunkPos,
    ) -> Result<(), String> {
        if center == player.chunk_center {
            return Ok(());
        }
        writer
            .send(
                &play::Clientbound::SetChunkCacheCenter(play::SetChunkCacheCenter {
                    x: center.x,
                    z: center.z,
                })
                .encode(),
            )
            .map_err(|error| format!("update client chunk cache center: {error}"))?;

        let desired = tracked_chunks(center, player.view_distance);
        let mut forgotten = player
            .sent_chunks
            .difference(&desired)
            .copied()
            .collect::<Vec<_>>();
        forgotten.sort_by_key(|chunk| (chunk.z, chunk.x));
        for chunk in forgotten {
            writer
                .send(
                    &play::Clientbound::ForgetLevelChunk(play::ForgetLevelChunk {
                        pos: [chunk.x, chunk.z],
                    })
                    .encode(),
                )
                .map_err(|error| {
                    format!("forget client chunk ({},{}): {error}", chunk.x, chunk.z)
                })?;
        }

        player.sent_chunks.retain(|chunk| desired.contains(chunk));
        requeue_chunks(player, center);
        player.chunk_center = center;
        Ok(())
    }

    fn stream_chunks(
        &self,
        writer: &mut horizon_protocol::connection::ConnWriter,
        player: &mut PlayerSession,
    ) -> Result<(), String> {
        self.poll_chunk_generation()?;
        self.prune_chunk_packet_cache();
        let request_positions = player
            .chunk_queue
            .iter()
            .filter(|chunk| !player.sent_chunks.contains(chunk))
            .take(MAX_CHUNK_BATCH)
            .copied()
            .collect::<Vec<_>>();
        for position in request_positions {
            self.request_chunk(position)?;
        }

        if player.unacknowledged_batches >= player.max_unacknowledged_batches {
            return Ok(());
        }
        player.batch_quota =
            (player.batch_quota + player.chunks_per_tick).min(player.chunks_per_tick.max(1.0));
        let limit = (player.batch_quota.floor() as usize).min(MAX_CHUNK_BATCH);
        if limit == 0 {
            return Ok(());
        }
        let cache = self
            .chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let ready = player
            .chunk_queue
            .iter()
            .filter(|chunk| !player.sent_chunks.contains(chunk) && cache.contains_key(chunk))
            .take(limit)
            .copied()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Ok(());
        }
        writer
            .send(&play::Clientbound::ChunkBatchStart(horizon_protocol::packets::Empty).encode())
            .map_err(|error| format!("start streamed chunk batch: {error}"))?;
        for chunk in &ready {
            writer
                .send(cache.get(chunk).expect("ready chunk packet is cached"))
                .map_err(|error| {
                    format!("send streamed chunk ({},{}): {error}", chunk.x, chunk.z)
                })?;
        }
        drop(cache);
        writer
            .send(
                &play::Clientbound::ChunkBatchFinished(play::ChunkBatchFinished {
                    size: ready.len() as i32,
                })
                .encode(),
            )
            .map_err(|error| format!("finish streamed chunk batch: {error}"))?;
        let ready_set = ready.iter().copied().collect::<HashSet<_>>();
        player
            .chunk_queue
            .retain(|chunk| !ready_set.contains(chunk));
        player.sent_chunks.extend(ready.iter().copied());
        player.batch_quota -= ready.len() as f32;
        player.unacknowledged_batches += 1;
        Ok(())
    }
}

#[cfg(test)]
fn player_box(position: [f64; 3], half_width: f64, height: f64) -> Aabb {
    Aabb::new(
        Vec3::new(
            position[0] - half_width,
            position[1],
            position[2] - half_width,
        ),
        Vec3::new(
            position[0] + half_width,
            position[1] + height,
            position[2] + half_width,
        ),
    )
}

#[cfg(test)]
fn resolve_movement_with_shapes<E>(
    from: [f64; 3],
    requested: [f64; 3],
    on_ground: bool,
    collect: impl FnMut(Aabb) -> Result<Vec<VoxelShape>, E>,
) -> Result<([f64; 3], bool), E> {
    let movement = Vec3::new(
        requested[0] - from[0],
        requested[1] - from[1],
        requested[2] - from[2],
    );
    let resolved = try_resolve_step_movement(
        movement,
        player_box(from, 0.3, 1.8),
        0.6,
        on_ground,
        collect,
    )?;
    Ok((
        [
            from[0] + resolved.x,
            from[1] + resolved.y,
            from[2] + resolved.z,
        ],
        movement.y < 0.0 && resolved.y != movement.y,
    ))
}

fn find_spawn(
    region: &[ProtoChunk; CHUNK_COUNT],
    resources: &ServerResources,
) -> Result<[f64; 3], String> {
    let air = resources
        .registry
        .default_state("minecraft:air")
        .ok_or("block registry is missing minecraft:air")?;
    let mut dry = Vec::new();
    let mut water_surface = Vec::new();
    for chunk in region {
        let chunk_pos = chunk.position();
        for z in 0..16 {
            for x in 0..16 {
                let y = chunk.surface_height(x, z) - 1;
                if !(-63..318).contains(&y) {
                    continue;
                }
                let floor = chunk.block(x, y, z);
                if chunk.block(x, y + 1, z) != air || chunk.block(x, y + 2, z) != air {
                    continue;
                }
                let location = [
                    f64::from(chunk_pos.x * 16 + x) + 0.5,
                    f64::from(y + 1),
                    f64::from(chunk_pos.z * 16 + z) + 0.5,
                ];
                if resources.behaviours.facts(floor).flags & state_flags::FULL_COLLISION != 0
                    && resources.behaviours.facts(floor).fluid
                        == horizon_world::fluid::FluidState::EMPTY
                {
                    let distance = location[0].powi(2) + location[2].powi(2);
                    dry.push((distance, location));
                } else if chunk.block_name(x, y, z) == "minecraft:water" {
                    let distance = location[0].powi(2) + location[2].powi(2);
                    water_surface.push((distance, location));
                }
            }
        }
    }
    dry.sort_by(|a, b| a.0.total_cmp(&b.0));
    water_surface.sort_by(|a, b| a.0.total_cmp(&b.0));
    dry.first()
        .or_else(|| water_surface.first())
        .map(|(_, pos)| *pos)
        .ok_or_else(|| "spawn 3×3 has no exposed full-collision ground or water surface".into())
}

fn chunk_center(position: [f64; 3]) -> ChunkPos {
    ChunkPos::new(
        (position[0].floor() as i32).div_euclid(16),
        (position[2].floor() as i32).div_euclid(16),
    )
}

fn chunks_in_view(center: ChunkPos) -> HashSet<ChunkPos> {
    let mut chunks = HashSet::with_capacity(CHUNK_COUNT);
    for z in center.z - CHUNK_RADIUS..=center.z + CHUNK_RADIUS {
        for x in center.x - CHUNK_RADIUS..=center.x + CHUNK_RADIUS {
            chunks.insert(ChunkPos::new(x, z));
        }
    }
    chunks
}

fn tracked_chunks(center: ChunkPos, radius: i32) -> HashSet<ChunkPos> {
    // Vanilla's chunk map tracks a square centered on the player. Keep this
    // in sync with SetChunkCacheRadius; a circular filter silently leaves the
    // client without the corner chunks it was told to expect.
    let radius = radius.max(2);
    let mut chunks = HashSet::new();
    for z in center.z - radius..=center.z + radius {
        for x in center.x - radius..=center.x + radius {
            chunks.insert(ChunkPos::new(x, z));
        }
    }
    chunks
}

fn requeue_chunks(player: &mut PlayerSession, center: ChunkPos) {
    player.chunk_queue.retain(|chunk| {
        is_tracked(center, player.view_distance, *chunk) && !player.sent_chunks.contains(chunk)
    });
    let queued = player.chunk_queue.iter().copied().collect::<HashSet<_>>();
    let radius = player.view_distance.max(2);
    let mut added = Vec::new();
    for z in center.z - radius..=center.z + radius {
        for x in center.x - radius..=center.x + radius {
            let chunk = ChunkPos::new(x, z);
            if !player.sent_chunks.contains(&chunk) && !queued.contains(&chunk) {
                added.push(chunk);
            }
        }
    }
    let vanilla_order =
        horizon_world::worldgen::decoration_order::vanilla_order((center.x, center.z), radius);
    let rank: HashMap<_, _> = vanilla_order
        .into_iter()
        .enumerate()
        .map(|(rank, position)| (position, rank))
        .collect();
    added.sort_by_key(|chunk| {
        let dx = i64::from(chunk.x - center.x);
        let dz = i64::from(chunk.z - center.z);
        (
            rank.get(&(chunk.x, chunk.z)).copied().unwrap_or(usize::MAX),
            dx * dx + dz * dz,
            chunk.z,
            chunk.x,
        )
    });
    player.chunk_queue.extend(added);
}

fn is_tracked(center: ChunkPos, radius: i32, chunk: ChunkPos) -> bool {
    let radius = radius.max(2);
    (i64::from(chunk.x) - i64::from(center.x)).abs() <= i64::from(radius)
        && (i64::from(chunk.z) - i64::from(center.z)).abs() <= i64::from(radius)
}

fn centers_distance_key(centers: &[ChunkPos], position: ChunkPos) -> i128 {
    centers
        .iter()
        .map(|center| {
            let dx = i128::from(position.x) - i128::from(center.x);
            let dz = i128::from(position.z) - i128::from(center.z);
            dx * dx + dz * dz
        })
        .min()
        .unwrap_or(i128::MAX)
}

#[derive(Clone, Debug, PartialEq)]
struct PlayerSession {
    entity_id: i32,
    name: String,
    position: [f64; 3],
    chunk_center: ChunkPos,
    sent_chunks: HashSet<ChunkPos>,
    view_distance: i32,
    chunk_queue: VecDeque<ChunkPos>,
    chunks_per_tick: f32,
    batch_quota: f32,
    unacknowledged_batches: usize,
    max_unacknowledged_batches: usize,
    yaw: f32,
    pitch: f32,
    on_ground: bool,
    fall_distance: f64,
    last_teleport_id: i32,
    awaiting_teleport: Option<(i32, [f64; 3])>,
    loaded: bool,
    position_packet_this_tick: bool,
    game_mode: i32,
    selected_slot: i16,
    inventory: PlayerInventory,
    input_flags: u8,
    sprinting: bool,
    server_tick: u64,
    digging: Option<DiggingState>,
    delayed_dig: Option<DiggingState>,
    dig_clock: Option<([i32; 3], u64)>,
    world_update_cursor: u64,
    level_event_cursor: u64,
    known_item_entities: HashMap<i32, DroppedItem>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DiggingState {
    pos: [i32; 3],
    started_at: u64,
    per_tick: f32,
    progress: f32,
    last_stage: i32,
}

impl PlayerSession {
    fn new(spawn: [f64; 3], teleport_id: i32, entity_id: i32) -> Self {
        let chunk_center = chunk_center(spawn);
        let mut player = Self {
            entity_id,
            name: "Player".into(),
            position: spawn,
            chunk_center,
            sent_chunks: chunks_in_view(chunk_center),
            view_distance: DEFAULT_VIEW_DISTANCE,
            chunk_queue: VecDeque::new(),
            chunks_per_tick: CHUNKS_PER_TICK,
            batch_quota: 0.0,
            unacknowledged_batches: 1,
            max_unacknowledged_batches: 1,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            fall_distance: 0.0,
            last_teleport_id: teleport_id,
            awaiting_teleport: Some((teleport_id, spawn)),
            loaded: false,
            position_packet_this_tick: false,
            game_mode: 0,
            selected_slot: 0,
            inventory: PlayerInventory::default(),
            input_flags: 0,
            sprinting: false,
            server_tick: 0,
            digging: None,
            delayed_dig: None,
            dig_clock: None,
            world_update_cursor: 0,
            level_event_cursor: 0,
            known_item_entities: HashMap::new(),
        };
        // The spawn batch is sent first, but the rest of the announced view
        // must already be queued even if the player remains in this chunk.
        requeue_chunks(&mut player, chunk_center);
        player
    }

    fn accept_teleport(&mut self, packet: &play::AcceptTeleportation) -> Result<(), String> {
        if packet.id != self.last_teleport_id {
            return Ok(());
        }
        let Some((_, assigned_position)) = self.awaiting_teleport.take() else {
            return Err("client confirmed a teleport that is not pending".into());
        };
        if [packet.x, packet.y, packet.z]
            .iter()
            .any(|value| !value.is_finite())
            || !packet.yaw.is_finite()
            || !packet.pitch.is_finite()
        {
            return Err("client sent invalid movement values while confirming teleport".into());
        }
        self.position = assigned_position;
        self.yaw = wrap_degrees(packet.yaw);
        self.pitch = wrap_degrees(packet.pitch);
        Ok(())
    }

    fn update_movement(
        &mut self,
        position: Option<[f64; 3]>,
        rotation: Option<(f32, f32)>,
        flags: u8,
    ) -> Result<(), String> {
        if position.is_some_and(|position| position.iter().any(|value| !value.is_finite())) {
            return Err("client sent invalid movement coordinates".into());
        }
        let (yaw, pitch) = rotation.unwrap_or_default();
        if !yaw.is_finite() || !pitch.is_finite() {
            return Err("client sent invalid movement rotation".into());
        }
        if position.is_some() {
            if self.position_packet_this_tick {
                return Err("client sent more than one position packet in a tick".into());
            }
            self.position_packet_this_tick = true;
        }
        if !self.loaded || self.awaiting_teleport.is_some() {
            return Ok(());
        }
        if let Some(position) = position {
            self.position = clamp_position(position[0], position[1], position[2]);
        }
        if rotation.is_some() {
            self.yaw = wrap_degrees(yaw);
            self.pitch = wrap_degrees(pitch).clamp(-90.0, 90.0);
        }
        self.on_ground = flags & 1 != 0;
        Ok(())
    }

    fn send_correction(
        &mut self,
        writer: &mut horizon_protocol::connection::ConnWriter,
        position: [f64; 3],
    ) -> Result<(), String> {
        let next = self.last_teleport_id.wrapping_add(1);
        self.last_teleport_id = next;
        self.position = position;
        self.awaiting_teleport = Some((next, position));
        writer
            .send(
                &play::Clientbound::PlayerPosition(play::PlayerPosition {
                    teleport: next,
                    position,
                    delta: [0.0; 3],
                    yaw: self.yaw,
                    pitch: self.pitch,
                    relatives: 0,
                })
                .encode(),
            )
            .map_err(|error| format!("send movement correction: {error}"))
    }

    fn end_tick(&mut self) {
        self.position_packet_this_tick = false;
    }
}

fn clamp_position(x: f64, y: f64, z: f64) -> [f64; 3] {
    [
        x.clamp(-3.0e7, 3.0e7),
        y.clamp(-2.0e7, 2.0e7),
        z.clamp(-3.0e7, 3.0e7),
    ]
}

fn wrap_degrees(degrees: f32) -> f32 {
    let mut wrapped = degrees % 360.0;
    if wrapped >= 180.0 {
        wrapped -= 360.0;
    }
    if wrapped < -180.0 {
        wrapped += 360.0;
    }
    wrapped
}

#[cfg(test)]
fn serve_until_disconnect(
    configured: ConfiguredConnection,
    spawn: [f64; 3],
    entity_id: i32,
    player_name: String,
    world: Option<&PreparedPlayWorld>,
    view_distance: i32,
    _simulation_distance: i32,
    spawn_center: ChunkPos,
) -> Result<(), String> {
    let uuid = configured.profile.uuid;
    serve_until_disconnect_with_player_data(
        configured,
        spawn,
        0.0,
        0.0,
        entity_id,
        player_name,
        uuid,
        world,
        view_distance,
        spawn_center,
    )
}

fn serve_until_disconnect_with_player_data(
    configured: ConfiguredConnection,
    spawn: [f64; 3],
    yaw: f32,
    pitch: f32,
    entity_id: i32,
    player_name: String,
    player_uuid: u128,
    world: Option<&PreparedPlayWorld>,
    view_distance: i32,
    spawn_center: ChunkPos,
) -> Result<(), String> {
    let mut player = PlayerSession::new(spawn, 1, entity_id);
    player.name = player_name;
    player.yaw = yaw;
    player.pitch = pitch;
    player.game_mode = world.map_or(0, |world| world.game_mode);
    player.view_distance = view_distance;
    player.chunk_center = spawn_center;
    requeue_chunks(&mut player, spawn_center);
    let (reader, mut writer) = configured.connection.split();
    reader
        .set_read_timeout(None)
        .map_err(|error| format!("configure PLAY network reader: {error}"))?;
    let closer = reader
        .closer()
        .map_err(|error| format!("create PLAY network closer: {error}"))?;
    let (incoming, received) = mpsc::sync_channel(NETWORK_CHANNEL_CAPACITY);
    let network_reader = thread::Builder::new()
        .name("borizon-play-reader".into())
        .spawn(move || read_play_packets(reader, incoming))
        .map_err(|error| format!("start PLAY network reader: {error}"))?;
    let result = run_play_session(&mut writer, received, &mut player, world);
    closer.close();
    let _ = network_reader.join();
    let saved = world.map_or(Ok(()), |world| {
        world.save_player_position(player_uuid, &player)
    });
    match (result, saved) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn read_play_packets(
    mut reader: horizon_protocol::connection::ConnReader,
    incoming: SyncSender<Result<(i32, Vec<u8>), String>>,
) {
    loop {
        let event = reader.recv().map_err(|error| error.to_string());
        let closed = event.is_err();
        if incoming.send(event).is_err() || closed {
            return;
        }
    }
}

#[derive(Default)]
struct PendingPackets {
    packets: VecDeque<(i32, Vec<u8>)>,
    bytes: usize,
}

impl PendingPackets {
    fn push(&mut self, packet: (i32, Vec<u8>)) -> Result<(), String> {
        let bytes = packet.1.len();
        if self.packets.len() >= INBOX_LIMIT || self.bytes.saturating_add(bytes) > INBOX_BYTES {
            return Err(format!(
                "client exceeded PLAY inbox limit ({} packets, {} bytes)",
                self.packets.len(),
                self.bytes
            ));
        }
        self.bytes += bytes;
        self.packets.push_back(packet);
        Ok(())
    }

    fn pop(&mut self) -> Option<(i32, Vec<u8>)> {
        let packet = self.packets.pop_front()?;
        self.bytes -= packet.1.len();
        Some(packet)
    }
}

fn run_play_session(
    writer: &mut horizon_protocol::connection::ConnWriter,
    received: Receiver<Result<(i32, Vec<u8>), String>>,
    player: &mut PlayerSession,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
    const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(30);

    let mut last_keepalive = Instant::now();
    let mut next_keepalive_id = 1_i64;
    let mut pending_keepalive: Option<(i64, Instant)> = None;
    writer
        .send(&play::Clientbound::ContainerSetContent(player.inventory.content_packet()).encode())
        .map_err(|error| format!("send initial player inventory: {error}"))?;
    if let Some(world) = world {
        player.world_update_cursor = world.latest_block_update();
        player.level_event_cursor = world.latest_level_event_sequence();
    }
    let mut inbox = PendingPackets::default();
    let mut next_tick = Instant::now() + SERVER_TICK;
    loop {
        let now = Instant::now();
        let wait = next_tick.saturating_duration_since(now);
        match received.recv_timeout(wait) {
            Ok(Ok(packet)) => inbox.push(packet)?,
            Ok(Err(error)) => return Err(format!("read PLAY packet: {error}")),
            Err(RecvTimeoutError::Disconnected) => {
                return Err("PLAY network reader stopped unexpectedly".into());
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        maintain_keepalive(
            writer,
            now,
            KEEPALIVE_INTERVAL,
            KEEPALIVE_TIMEOUT,
            &mut last_keepalive,
            &mut pending_keepalive,
            &mut next_keepalive_id,
        )?;
        while now >= next_tick {
            for _ in 0..PACKETS_PER_TICK {
                let Some((id, payload)) = inbox.pop() else {
                    break;
                };
                let packet = play::Serverbound::decode(id, &payload)
                    .map_err(|error| format!("decode PLAY packet: {error}"))?;
                process_play_packet(
                    writer,
                    packet,
                    player,
                    &mut pending_keepalive,
                    &mut last_keepalive,
                    world,
                )?;
            }
            advance_player_mining(writer, player, world)?;
            if let Some(world) = world {
                world.stream_chunks(writer, player)?;
                sync_item_entities(writer, player, world)?;
            }
            next_tick += SERVER_TICK;
        }
    }
}

fn process_play_packet(
    writer: &mut horizon_protocol::connection::ConnWriter,
    packet: play::Serverbound,
    player: &mut PlayerSession,
    pending_keepalive: &mut Option<(i64, Instant)>,
    last_keepalive: &mut Instant,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    match packet {
        play::Serverbound::AcceptTeleportation(packet) => player.accept_teleport(&packet)?,
        play::Serverbound::PlayerAction(packet) => {
            process_player_action(writer, player, packet, world)?;
        }
        play::Serverbound::KeepAlive(response) => match pending_keepalive.take() {
            Some((expected, _)) if response.id == expected => *last_keepalive = Instant::now(),
            Some((expected, _)) => {
                return Err(format!(
                    "client answered PLAY keepalive {} while {} was pending",
                    response.id, expected
                ));
            }
            None => return Err("client answered an unsolicited PLAY keepalive".into()),
        },
        play::Serverbound::PlayerLoaded(_) => player.loaded = true,
        play::Serverbound::ClientTickEnd(_) => player.end_tick(),
        play::Serverbound::PlayerInput(packet) => player.input_flags = packet.flags,
        play::Serverbound::SetCarriedItem(packet) => {
            if (0..9).contains(&packet.slot) {
                player.selected_slot = packet.slot;
            } else {
                writer
                    .send(
                        &play::Clientbound::SetHeldSlot(play::SetHeldSlot {
                            slot: i32::from(player.selected_slot),
                        })
                        .encode(),
                    )
                    .map_err(|error| format!("restore invalid carried-item slot: {error}"))?;
            }
        }
        play::Serverbound::PlayerCommand(packet) if packet.id == player.entity_id => {
            match packet.action {
                1 => player.sprinting = true,
                2 => player.sprinting = false,
                _ => {}
            }
        }
        play::Serverbound::UseItemOn(packet) => {
            process_use_item_on(writer, player, packet, world)?;
        }
        play::Serverbound::UseItem(packet) => {
            writer
                .send(
                    &play::Clientbound::BlockChangedAck(play::BlockChangedAck {
                        sequence: packet.sequence,
                    })
                    .encode(),
                )
                .map_err(|error| format!("acknowledge empty-hand use: {error}"))?;
        }
        play::Serverbound::Chat(packet) => {
            // Secure chat is disabled for this offline listener. Keep the
            // session alive and echo the ordinary unsigned message as a
            // vanilla system component until the shared chat owner is ported.
            let content = format!("<{}> {}", player.name, packet.message);
            writer
                .send(
                    &play::Clientbound::SystemChat(play::SystemChat {
                        content: horizon_world::nbt::Tag::String(content.as_str().into()),
                        overlay: false,
                    })
                    .encode(),
                )
                .map_err(|error| format!("echo offline PLAY chat: {error}"))?;
        }
        play::Serverbound::ChatAck(_) => {}
        play::Serverbound::RecipeBookChangeSettings(_) => {}
        play::Serverbound::ContainerClick(packet) if packet.container == 0 => {
            if packet.state == player.inventory.state_id() {
                player
                    .inventory
                    .click_storage(packet.slot, packet.button, packet.input, |item| {
                        world
                            .and_then(|world| {
                                world
                                    .resources
                                    .item_ids
                                    .name(item)
                                    .map(|name| world.resources.loot.max_stack(name))
                            })
                            .unwrap_or(64)
                    });
            }
            writer
                .send(
                    &play::Clientbound::ContainerSetContent(player.inventory.content_packet())
                        .encode(),
                )
                .map_err(|error| format!("sync player inventory after click: {error}"))?;
        }
        play::Serverbound::SetCreativeModeSlot(packet) => {
            if player.game_mode == 1 {
                player.inventory.set_creative_slot(packet.slot, packet.item);
            }
            writer
                .send(
                    &play::Clientbound::ContainerSetContent(player.inventory.content_packet())
                        .encode(),
                )
                .map_err(|error| format!("resync player inventory after creative slot: {error}"))?;
        }
        play::Serverbound::Punch(_)
        | play::Serverbound::ClientCommand(_)
        | play::Serverbound::ChatSessionUpdate(_)
        | play::Serverbound::RecipeBookSeenRecipe(_)
        | play::Serverbound::SeenAdvancements(_)
        | play::Serverbound::ContainerClose(_)
        | play::Serverbound::ContainerClick(_)
        | play::Serverbound::Interact(_) => {}
        play::Serverbound::MovePlayerPos(packet) => {
            update_player_movement(
                writer,
                player,
                Some([packet.x, packet.y, packet.z]),
                None,
                packet.flags,
                world,
            )?;
        }
        play::Serverbound::MovePlayerPosRot(packet) => update_player_movement(
            writer,
            player,
            Some([packet.x, packet.y, packet.z]),
            Some((packet.yaw, packet.pitch)),
            packet.flags,
            world,
        )?,
        play::Serverbound::MovePlayerRot(packet) => {
            update_player_movement(
                writer,
                player,
                None,
                Some((packet.yaw, packet.pitch)),
                packet.flags,
                world,
            )?;
        }
        play::Serverbound::MovePlayerStatus(packet) => {
            update_player_movement(writer, player, None, None, packet.flags, world)?;
        }
        play::Serverbound::ClientInformation(_) => {}
        play::Serverbound::ChunkBatchReceived(batch) => {
            player.unacknowledged_batches = player.unacknowledged_batches.saturating_sub(1);
            player.chunks_per_tick = if batch.chunks_per_tick.is_finite() {
                batch.chunks_per_tick.clamp(0.01, 64.0)
            } else {
                0.01
            };
            if player.unacknowledged_batches == 0 {
                player.batch_quota = player.batch_quota.max(1.0);
                player.max_unacknowledged_batches = 10;
            }
        }
        play::Serverbound::PingRequest(ping) => writer
            .send(&play::Clientbound::PongResponse(ping).encode())
            .map_err(|error| format!("reply to PLAY ping: {error}"))?,
        packet => {
            return Err(format!(
                "PLAY gameplay packet {} is not implemented yet",
                packet.name()
            ));
        }
    }
    Ok(())
}

fn process_use_item_on(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    action: play::UseItemOn,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    writer
        .send(
            &play::Clientbound::BlockChangedAck(play::BlockChangedAck {
                sequence: action.sequence,
            })
            .encode(),
        )
        .map_err(|error| format!("acknowledge block-use action: {error}"))?;

    // Match the reference's hit-point and reach checks before item/block behaviour.
    if action.hand < 0 || action.hand > 1 || !(0..=5).contains(&action.face) {
        return Ok(());
    }
    let hit_is_near_block = action
        .cursor
        .iter()
        .all(|coordinate| (f64::from(*coordinate) - 0.5).abs() < 1.0000001);
    if !hit_is_near_block || !player.loaded || player.awaiting_teleport.is_some() {
        return Ok(());
    }
    let Some(world) = world else {
        return Ok(());
    };
    let eye = [
        player.position[0],
        player.position[1] + 1.62,
        player.position[2],
    ];
    if !player_mining::within_block_range(eye, 5.5, action.pos) {
        return Ok(());
    }
    let pos = BlockPos::new(action.pos[0], action.pos[1], action.pos[2]);
    let face = Direction::ALL[action.face as usize];
    let hit = Vec3::new(
        f64::from(action.pos[0]) + f64::from(action.cursor[0]),
        f64::from(action.pos[1]) + f64::from(action.cursor[1]),
        f64::from(action.pos[2]) + f64::from(action.cursor[2]),
    );
    let mut runtime = world
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dimension = runtime
        .dimension_mut("minecraft:overworld")
        .expect("the Play Overworld is registered");
    {
        let resident = dimension.resident();
        if !resident.level().is_loaded(pos)
            || !(resident.level().min_y()..resident.level().max_y()).contains(&pos.y)
        {
            return Ok(());
        }
    }

    let held_item = player
        .inventory
        .hand(action.hand, player.selected_slot)
        .and_then(|stack| world.resources.item_ids.name(stack.item))
        .map(str::to_owned);
    // Block interaction gets first refusal. A Pass continues to vanilla
    // BlockItem placement if the selected stack has a registered block item.
    let interaction = dimension.resident_mut().use_without_item(
        &world.resources.behaviours,
        pos,
        face,
        hit,
        PlayerContext {
            yaw: player.yaw,
            pitch: player.pitch,
            may_build: true,
            can_eat: false,
            game_master: false,
        },
    );
    let mut placed = false;
    if interaction == horizon_world::level::InteractionResult::Pass {
        if let Some(item) = held_item.as_deref()
            && world.resources.behaviours.item_binding(item).is_some()
        {
            let outcome = dimension.resident_mut().place_block(
                &world.resources.behaviours,
                &PlaceContext {
                    clicked_pos: pos,
                    face,
                    hit,
                    inside: action.inside,
                    yaw: player.yaw,
                    pitch: player.pitch,
                    sneaking: player.input_flags & 0x20 != 0,
                    item: item.to_owned(),
                    can_use_game_master_blocks: player.game_mode == 1,
                },
            );
            if matches!(outcome, PlaceOutcome::Placed { .. }) {
                placed = true;
                if player.game_mode != 1 {
                    player
                        .inventory
                        .consume_hand(action.hand, player.selected_slot);
                }
            }
        }
    }
    let changes = runtime.take_block_changes();
    let events = runtime.take_level_events();
    drop(runtime);
    world.publish_block_changes(changes)?;
    world.publish_level_events(events)?;
    if placed {
        writer
            .send(
                &play::Clientbound::ContainerSetContent(player.inventory.content_packet()).encode(),
            )
            .map_err(|error| format!("sync inventory after placing block: {error}"))?;
    }
    Ok(())
}

/// The reference's `Game::destroy_progress` order, using the held item's
/// vanilla default `Tool` component. Stack-level component patches, effects,
/// attributes and water modifiers still need their own session owners.
fn player_destroy_progress(
    player: &PlayerSession,
    world: &PreparedPlayWorld,
    state: horizon_world::registry::BlockStateId,
    hardness: f32,
    flags: u32,
) -> f32 {
    let block = world
        .resources
        .registry
        .state(state)
        .map(|state| state.name.as_str())
        .unwrap_or("minecraft:air");
    let held_item = player
        .inventory
        .hotbar(player.selected_slot)
        .and_then(|stack| world.resources.item_ids.name(stack.item));
    let harvest = flags & state_flags::REQUIRES_TOOL == 0
        || world
            .resources
            .loot
            .correct_tool_for_drops(held_item, block);
    let tool_speed = world.resources.loot.mining_speed(held_item, block);
    player_mining::destroy_progress(
        hardness,
        harvest,
        player_mining::destroy_speed(
            tool_speed,
            &DestroySpeedModifiers {
                on_ground: player.on_ground,
                ..DestroySpeedModifiers::default()
            },
        ),
    )
}

fn process_player_action(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    action: play::PlayerAction,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    // Reference order: START, STOP and ABORT are sequence-ACKed before reach
    // or loaded-chunk rejection. CHANGE_DESTROY_DIRECTION is not ACKed.
    if matches!(action.action, 0 | 2 | 3) {
        writer
            .send(
                &play::Clientbound::BlockChangedAck(play::BlockChangedAck {
                    sequence: action.sequence,
                })
                .encode(),
            )
            .map_err(|error| format!("acknowledge block action: {error}"))?;
    }
    if action.action == 1 {
        return Ok(());
    }
    if !matches!(action.action, 0 | 2 | 3) {
        return Err(format!("unsupported PlayerAction {}", action.action));
    }
    if !player.loaded || player.awaiting_teleport.is_some() {
        return Ok(());
    }
    let Some(world) = world else {
        return Ok(());
    };
    let pos = BlockPos::new(action.pos[0], action.pos[1], action.pos[2]);
    let eye = [
        player.position[0],
        player.position[1] + 1.62,
        player.position[2],
    ];
    let in_reach = player_mining::within_block_range(eye, 5.5, action.pos);

    if action.action == 0 {
        if !in_reach {
            return Ok(());
        }
        if let Some(previous_dig) = player.digging.take() {
            if previous_dig.last_stage >= 0 {
                world.publish_crack_progress(player.entity_id, previous_dig.pos, -1)?;
            }
        }
        let runtime = world
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let resident = runtime
            .dimension("minecraft:overworld")
            .expect("the Play Overworld is registered")
            .resident();
        if !resident.level().is_loaded(pos)
            || pos.y < resident.level().min_y()
            || pos.y >= resident.level().max_y()
        {
            return Ok(());
        }
        let state = resident.level().block_state(pos);
        if player.game_mode == 1 {
            player.digging = None;
            drop(runtime);
            destroy_player_block(
                writer,
                player,
                world,
                DiggingState {
                    pos: action.pos,
                    started_at: player.server_tick,
                    per_tick: 1.0,
                    progress: 1.0,
                    last_stage: -1,
                },
            )?;
            return Ok(());
        }
        let facts = world.resources.behaviours.facts(state);
        let per_tick = player_destroy_progress(player, world, state, facts.hardness, facts.flags);
        let mut digging = DiggingState {
            pos: action.pos,
            started_at: player.server_tick,
            per_tick,
            progress: 0.0,
            last_stage: -1,
        };
        if per_tick >= 1.0 {
            player.digging = None;
            drop(runtime);
            destroy_player_block(writer, player, world, digging)?;
        } else {
            player.dig_clock = Some((action.pos, player.server_tick));
            drop(runtime);
            publish_mining_stage(player, world, &mut digging, per_tick)?;
            player.digging = Some(digging);
        }
        return Ok(());
    }

    if action.action == 2 {
        player.digging = None;
        world.publish_crack_progress(player.entity_id, action.pos, -1)?;
        let runtime = world
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let resident = runtime
            .dimension("minecraft:overworld")
            .expect("the Play Overworld is registered")
            .resident();
        let state = resident.level().block_state(pos);
        drop(runtime);
        send_block_update(writer, action.pos, state.0 as i32)?;
        send_crack_progress(writer, player.entity_id, action.pos, -1)?;
        return Ok(());
    }

    let active_dig = player.digging.take().filter(|dig| dig.pos == action.pos);
    let started_at = player
        .dig_clock
        .filter(|(pos, _)| *pos == action.pos)
        .map(|(_, started_at)| started_at);
    if !in_reach {
        return Ok(());
    }
    let Some(mut digging) = active_dig.or_else(|| {
        started_at.map(|started_at| DiggingState {
            pos: action.pos,
            started_at,
            per_tick: 0.0,
            progress: 0.0,
            last_stage: -1,
        })
    }) else {
        return Ok(());
    };
    let runtime = world
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let resident = runtime
        .dimension("minecraft:overworld")
        .expect("the Play Overworld is registered")
        .resident();
    if !resident.level().is_loaded(pos)
        || pos.y < resident.level().min_y()
        || pos.y >= resident.level().max_y()
    {
        return Ok(());
    }
    let state = resident.level().block_state(pos);
    let facts = world.resources.behaviours.facts(state);
    if facts.flags & state_flags::AIR != 0 {
        return Ok(());
    }
    // Match main's `guest_stop_digging`: resolve destroy progress from the
    // player's current held item and the real block/tool rules, not a
    // hand-speed approximation. STOP is allowed to arrive after the held
    // item changed while the client was mining.
    digging.per_tick = player_destroy_progress(player, world, state, facts.hardness, facts.flags);
    drop(runtime);
    digging.progress =
        dig_progress_at_tick(digging.per_tick, player.server_tick, digging.started_at);
    if stop_dig_breaks(digging.progress) {
        destroy_player_block(writer, player, world, digging)?;
    } else if player.delayed_dig.is_none() {
        player.delayed_dig = Some(digging);
    }
    Ok(())
}

/// `lan_actions/dig_stop.rs::stop_dig`: STOP breaks outright at 70%; below
/// that it becomes a server-side delayed dig.
fn stop_dig_breaks(progress: f32) -> bool {
    progress >= 0.7
}

fn dig_progress_at_tick(per_tick: f32, current_tick: u64, started_at: u64) -> f32 {
    per_tick * (current_tick - started_at + 1) as f32
}

fn publish_mining_stage(
    player: &mut PlayerSession,
    world: &PreparedPlayWorld,
    digging: &mut DiggingState,
    progress: f32,
) -> Result<(), String> {
    let stage = (progress * 10.0) as i32;
    if stage != digging.last_stage {
        world.publish_crack_progress(player.entity_id, digging.pos, stage)?;
        digging.last_stage = stage;
    }
    Ok(())
}

fn send_block_update(
    writer: &mut horizon_protocol::connection::ConnWriter,
    pos: [i32; 3],
    state: i32,
) -> Result<(), String> {
    writer
        .send(&play::Clientbound::BlockUpdate(play::BlockUpdate { pos, state }).encode())
        .map_err(|error| format!("send block update: {error}"))
}

fn send_crack_progress(
    writer: &mut horizon_protocol::connection::ConnWriter,
    id: i32,
    pos: [i32; 3],
    progress: i32,
) -> Result<(), String> {
    writer
        .send(
            &play::Clientbound::BlockDestruction(play::BlockDestruction {
                id,
                pos,
                progress: progress as u8,
            })
            .encode(),
        )
        .map_err(|error| format!("send block destruction stage: {error}"))
}

fn destroy_player_block(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    world: &PreparedPlayWorld,
    digging: DiggingState,
) -> Result<(), String> {
    let pos = BlockPos::new(digging.pos[0], digging.pos[1], digging.pos[2]);
    let mut runtime = world
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (destroyed, old_state, correct_tool, block_drops) = {
        let resident = runtime
            .dimension_mut("minecraft:overworld")
            .expect("the Play Overworld is registered")
            .resident_mut();
        if !resident.level().is_loaded(pos) {
            return Ok(());
        }
        let state = resident.level().block_state(pos);
        let block = world
            .resources
            .registry
            .state(state)
            .ok_or("resident block state is absent from the registry")?;
        let flags = world.resources.behaviours.facts(state).flags;
        let held_item = player
            .inventory
            .hotbar(player.selected_slot)
            .and_then(|stack| world.resources.item_ids.name(stack.item));
        let correct_tool = flags & state_flags::REQUIRES_TOOL == 0
            || world
                .resources
                .loot
                .correct_tool_for_drops(held_item, &block.name);
        let block = block.clone();
        let creative = player.game_mode == 1;
        let destroyed = resident.destroy_block_by_player(
            &world.resources.behaviours,
            pos,
            creative,
            correct_tool,
        );
        let block_drops = runtime.game_rules().bool("block_drops");
        (destroyed, block, correct_tool, block_drops)
    };
    let (changes, events) = if destroyed {
        save_runtime_dirty_chunks(&mut runtime, &world.resources)?;
        (runtime.take_block_changes(), runtime.take_level_events())
    } else {
        (Vec::new(), Vec::new())
    };
    drop(runtime);
    if destroyed {
        world.publish_block_changes(changes)?;
        world.publish_level_events(events)?;
        if digging.last_stage >= 0 {
            world.publish_crack_progress(player.entity_id, digging.pos, -1)?;
        }
        if player.game_mode != 1 && correct_tool && block_drops {
            spawn_block_loot(player, world, &old_state, pos)?;
        }
        send_crack_progress(writer, player.entity_id, digging.pos, -1)?;
    }
    Ok(())
}

fn spawn_block_loot(
    player: &PlayerSession,
    world: &PreparedPlayWorld,
    block: &horizon_world::chunk::BlockState,
    pos: BlockPos,
) -> Result<(), String> {
    let Some((namespace, name)) = block.name.split_once(':') else {
        return Ok(());
    };
    let table = format!("{namespace}:blocks/{name}");
    let held_tool = player
        .inventory
        .hotbar(player.selected_slot)
        .and_then(|stack| world.resources.item_ids.name(stack.item))
        .map(LootStack::new);
    let mut context = BlockLootContext {
        block: block.name.clone(),
        tool: held_tool,
        ..BlockLootContext::default()
    };
    for (key, value) in &block.properties {
        context.properties.insert(key.clone(), value.clone());
    }

    let mut random = world
        .loot_random
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let drops = world
        .resources
        .loot
        .roll_block(&table, &mut *random, &context);
    for drop in drops {
        let Some(item_id) = world.resources.item_ids.id(&drop.item) else {
            return Err(format!("loot table produced unknown item {}", drop.item));
        };
        let mut stack = Stack::new(item_id, drop.count);
        if drop.damage > 0 {
            stack.added.push((
                horizon_protocol::item::component::DAMAGE,
                ItemComponent::Int(drop.damage),
            ));
        }
        let enchantments: Vec<(i32, i32)> = drop
            .enchantments
            .iter()
            .filter_map(|(name, level)| {
                world
                    .resources
                    .builtin_registries
                    .get("minecraft:enchantment")?
                    .id(name)
                    .map(|id| (id, *level))
            })
            .collect();
        if !enchantments.is_empty() {
            stack.added.push((
                horizon_protocol::item::component::ENCHANTMENTS,
                ItemComponent::Enchantments(enchantments),
            ));
        }

        // Match Block.popResource's randomized point; the item's own random
        // stream supplies the small constructor velocity until entity ticking
        // and independent per-entity RNG are ported.
        let position = horizon_world::level::removal::pop_resource_position(&mut *random, pos);
        let velocity = [
            random.next_double() * 0.2 - 0.1,
            0.2,
            random.next_double() * 0.2 - 0.1,
        ];
        world.item_entities.spawn(stack, position, velocity)?;
    }
    Ok(())
}

fn send_pending_block_updates(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    world: &PreparedPlayWorld,
) -> Result<(), String> {
    let (updates, latest) = world.block_updates_since(player.world_update_cursor)?;
    for update in updates {
        if update.dimension == "minecraft:overworld"
            && player.sent_chunks.contains(&ChunkPos::new(
                update.pos[0].div_euclid(16),
                update.pos[2].div_euclid(16),
            ))
        {
            send_block_update(writer, update.pos, update.state)?;
        }
    }
    player.world_update_cursor = latest;
    send_pending_level_events(writer, player, world)?;
    Ok(())
}

fn send_pending_level_events(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    world: &PreparedPlayWorld,
) -> Result<(), String> {
    let (events, latest) = world.level_events_since(player.level_event_cursor)?;
    for event in events {
        if event.dimension != "minecraft:overworld" {
            continue;
        }
        let pos = event_pos(&event.event);
        let chunk = ChunkPos::new(pos[0].div_euclid(16), pos[2].div_euclid(16));
        if !player.sent_chunks.contains(&chunk) {
            continue;
        }
        let distance_squared = (0..3)
            .map(|axis| {
                let center = f64::from(pos[axis]) + 0.5;
                (player.position[axis] - center).powi(2)
            })
            .sum::<f64>();
        match event.event {
            PublishedLevelEvent::Crack {
                entity_id,
                pos,
                progress,
            } => {
                let distance_squared = (0..3)
                    .map(|axis| (player.position[axis] - f64::from(pos[axis])).powi(2))
                    .sum::<f64>();
                if entity_id == player.entity_id || distance_squared >= 32.0 * 32.0 {
                    continue;
                }
                writer
                    .send(
                        &play::Clientbound::BlockDestruction(play::BlockDestruction {
                            id: entity_id,
                            pos,
                            progress: progress as u8,
                        })
                        .encode(),
                    )
                    .map_err(|error| format!("send PLAY mining progress: {error}"))?;
            }
            PublishedLevelEvent::Level { pos, id, data } => {
                if distance_squared > 64.0 * 64.0 {
                    continue;
                }
                writer
                    .send(
                        &play::Clientbound::LevelEvent(play::LevelEvent {
                            kind: id,
                            pos,
                            data,
                            global: false,
                        })
                        .encode(),
                    )
                    .map_err(|error| format!("send PLAY level event: {error}"))?;
            }
            PublishedLevelEvent::Sound {
                pos,
                sound,
                volume,
                pitch,
            } => {
                let range = if volume > 1.0 { volume * 16.0 } else { 16.0 };
                if distance_squared > f64::from(range).powi(2) {
                    continue;
                }
                writer
                    .send(
                        &play::Clientbound::Sound(play::Sound {
                            sound: (-1, Some((sound, None))),
                            source: 4,
                            x: pos[0] * 8 + 4,
                            y: pos[1] * 8 + 4,
                            z: pos[2] * 8 + 4,
                            volume,
                            pitch,
                            seed: event.sequence as i64,
                        })
                        .encode(),
                    )
                    .map_err(|error| format!("send PLAY block sound: {error}"))?;
            }
        }
    }
    player.level_event_cursor = latest;
    Ok(())
}

fn sync_item_entities(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    world: &PreparedPlayWorld,
) -> Result<(), String> {
    if !player.loaded {
        return Ok(());
    }
    let pickups =
        world
            .item_entities
            .collect_near(player.position, &mut player.inventory, |stack| {
                world
                    .resources
                    .item_ids
                    .name(stack.item)
                    .map(|name| world.resources.loot.max_stack(name))
                    .unwrap_or(64)
            });
    for pickup in &pickups {
        writer
            .send(
                &play::Clientbound::TakeItemEntity(play::TakeItemEntity {
                    item: pickup.id,
                    player: player.entity_id,
                    amount: pickup.amount,
                })
                .encode(),
            )
            .map_err(|error| format!("send collected item animation: {error}"))?;
    }
    if !pickups.is_empty() {
        writer
            .send(
                &play::Clientbound::ContainerSetContent(player.inventory.content_packet()).encode(),
            )
            .map_err(|error| format!("sync inventory after item pickup: {error}"))?;
    }
    let items = world.item_entities.all();
    let mut visible = HashSet::new();
    for item in items {
        let chunk = ChunkPos::new(
            (item.position[0].floor() as i32).div_euclid(16),
            (item.position[2].floor() as i32).div_euclid(16),
        );
        if !player.sent_chunks.contains(&chunk) {
            continue;
        }
        visible.insert(item.id);
        if let Some(previous) = player.known_item_entities.get(&item.id) {
            if previous.position != item.position {
                writer
                    .send(
                        &play::Clientbound::EntityPositionSync(play::EntityPositionSync {
                            id: item.id,
                            path: play::PositionPath::Linear(item.position),
                            yaw: 0.0,
                            pitch: 0.0,
                            on_ground: item.on_ground,
                        })
                        .encode(),
                    )
                    .map_err(|error| format!("sync dropped item position: {error}"))?;
            }
            if previous.velocity != item.velocity {
                writer
                    .send(
                        &play::Clientbound::SetEntityMotion(play::SetEntityMotion {
                            id: item.id,
                            velocity: item.velocity,
                        })
                        .encode(),
                    )
                    .map_err(|error| format!("sync dropped item velocity: {error}"))?;
            }
            if previous.stack != item.stack {
                send_item_entity_stack(writer, &item)?;
            }
            player.known_item_entities.insert(item.id, item);
            continue;
        }
        let entity_kind = world
            .resources
            .builtin_registries
            .get("minecraft:entity_type")
            .and_then(|ids| ids.id("minecraft:item"))
            .ok_or("registries.json has no minecraft:item entity type")?;
        writer
            .send(
                &play::Clientbound::AddEntity(play::AddEntity {
                    id: item.id,
                    uuid: item.uuid,
                    kind: entity_kind,
                    position: item.position,
                    velocity: item.velocity,
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                })
                .encode(),
            )
            .map_err(|error| format!("send dropped item entity spawn: {error}"))?;
        send_item_entity_stack(writer, &item)?;
        player.known_item_entities.insert(item.id, item);
    }
    let stale: Vec<i32> = player
        .known_item_entities
        .keys()
        .copied()
        .filter(|id| !visible.contains(id))
        .collect();
    if !stale.is_empty() {
        writer
            .send(
                &play::Clientbound::RemoveEntities(play::RemoveEntities { ids: stale.clone() })
                    .encode(),
            )
            .map_err(|error| format!("remove stale dropped item entities: {error}"))?;
        for id in stale {
            player.known_item_entities.remove(&id);
        }
    }
    Ok(())
}

fn send_item_entity_stack(
    writer: &mut horizon_protocol::connection::ConnWriter,
    item: &DroppedItem,
) -> Result<(), String> {
    writer
        .send(
            &play::Clientbound::SetEntityData(play::SetEntityData {
                id: item.id,
                values: vec![(
                    entity_data::ITEM_ENTITY_ITEM,
                    entity_data::serializer::ITEM_STACK,
                    EntityDataValue::Item(Some(item.stack.clone())),
                )],
            })
            .encode(),
        )
        .map_err(|error| format!("send dropped item entity stack: {error}"))
}

fn event_pos(event: &PublishedLevelEvent) -> [i32; 3] {
    match event {
        PublishedLevelEvent::Level { pos, .. }
        | PublishedLevelEvent::Sound { pos, .. }
        | PublishedLevelEvent::Crack { pos, .. } => *pos,
    }
}

fn advance_player_mining(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    let Some(world) = world else {
        player.server_tick = player.server_tick.wrapping_add(1);
        return Ok(());
    };
    let runtime = world
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let resident = runtime
        .dimension("minecraft:overworld")
        .expect("the Play Overworld is registered")
        .resident();
    let mut delayed_completion = None;
    if let Some(mut digging) = player.digging.take() {
        let pos = BlockPos::new(digging.pos[0], digging.pos[1], digging.pos[2]);
        if resident.level().is_loaded(pos)
            && (resident.level().min_y()..resident.level().max_y()).contains(&pos.y)
        {
            let state = resident.level().block_state(pos);
            let facts = world.resources.behaviours.facts(state);
            if facts.flags & state_flags::AIR == 0 {
                digging.per_tick =
                    player_destroy_progress(player, world, state, facts.hardness, facts.flags);
                digging.progress =
                    dig_progress_at_tick(digging.per_tick, player.server_tick, digging.started_at);
                let progress = digging.progress;
                publish_mining_stage(player, world, &mut digging, progress)?;
                player.digging = Some(digging);
            }
        }
    }
    if let Some(mut digging) = player.delayed_dig.take() {
        let pos = BlockPos::new(digging.pos[0], digging.pos[1], digging.pos[2]);
        if resident.level().is_loaded(pos)
            && (resident.level().min_y()..resident.level().max_y()).contains(&pos.y)
        {
            let state = resident.level().block_state(pos);
            let facts = world.resources.behaviours.facts(state);
            if facts.flags & state_flags::AIR == 0 {
                digging.per_tick =
                    player_destroy_progress(player, world, state, facts.hardness, facts.flags);
                digging.progress =
                    dig_progress_at_tick(digging.per_tick, player.server_tick, digging.started_at);
                let progress = digging.progress;
                publish_mining_stage(player, world, &mut digging, progress)?;
                if digging.progress >= 1.0 {
                    delayed_completion = Some(digging);
                } else {
                    player.delayed_dig = Some(digging);
                }
            }
        }
    }
    drop(runtime);
    if let Some(digging) = delayed_completion {
        destroy_player_block(writer, player, world, digging)?;
    }
    player.server_tick = player.server_tick.wrapping_add(1);
    send_pending_block_updates(writer, player, world)?;
    Ok(())
}

fn update_player_movement(
    writer: &mut horizon_protocol::connection::ConnWriter,
    player: &mut PlayerSession,
    position: Option<[f64; 3]>,
    rotation: Option<(f32, f32)>,
    flags: u8,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    let start = player.position;
    let can_move = player.loaded && player.awaiting_teleport.is_none();
    player.update_movement(position, rotation, flags)?;
    if !can_move || position.is_none() {
        return Ok(());
    }
    let requested = player.position;
    let Some(world) = world else {
        return Ok(());
    };
    if exceeds_base_movement_limit(start, requested) {
        return player.send_correction(writer, start);
    }
    match world.resolve_player_move(
        player.entity_id,
        start,
        requested,
        player.fall_distance,
        flags & 1 != 0,
    ) {
        Ok((resolved, grounded)) => {
            player.position = resolved;
            player.on_ground = grounded || flags & 1 != 0;
            if grounded {
                player.fall_distance = 0.0;
            } else if resolved[1] < start[1] {
                player.fall_distance += start[1] - resolved[1];
            }
            if (0..3).any(|axis| (resolved[axis] - requested[axis]).abs() > 1.0e-4) {
                player.send_correction(writer, resolved)?;
            }
            let center = chunk_center(resolved);
            world.update_client_chunk_view(writer, player, center)?;
            requeue_chunks(player, center);
            world.set_player_chunk(player.entity_id, center)?;
        }
        Err(_) => player.send_correction(writer, start)?,
    }
    Ok(())
}

fn exceeds_base_movement_limit(from: [f64; 3], to: [f64; 3]) -> bool {
    (0..3)
        .map(|axis| (to[axis] - from[axis]).powi(2))
        .sum::<f64>()
        > MAX_CLIENT_MOVEMENT_SQUARED_PER_TICK
}

fn maintain_keepalive(
    connection: &mut horizon_protocol::connection::ConnWriter,
    now: Instant,
    interval: Duration,
    timeout: Duration,
    last_keepalive: &mut Instant,
    pending: &mut Option<(i64, Instant)>,
    next_id: &mut i64,
) -> Result<(), String> {
    if let Some((_, sent_at)) = pending {
        if now.duration_since(*sent_at) >= timeout {
            return Err(format!(
                "client did not answer PLAY keepalive within {} seconds",
                timeout.as_secs()
            ));
        }
    } else if now.duration_since(*last_keepalive) >= interval {
        connection
            .send(&play::Clientbound::KeepAlive(KeepAlive { id: *next_id }).encode())
            .map_err(|error| format!("send PLAY keepalive: {error}"))?;
        *pending = Some((*next_id, now));
        *last_keepalive = now;
        *next_id = next_id.wrapping_add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::login::OfflineProfile;
    use horizon_protocol::connection::{Connection, Pipe};

    #[test]
    fn block_update_log_delivers_ordered_updates_from_each_cursor() {
        let mut log = BlockUpdateLog::default();
        log.append("minecraft:overworld".into(), [1, 2, 3], 4)
            .unwrap();
        log.append("minecraft:overworld".into(), [1, 2, 3], 0)
            .unwrap();

        let (all, latest) = log.since(0).unwrap();
        assert_eq!(latest, 2);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].sequence, 1);
        assert_eq!(all[0].state, 4);
        assert_eq!(all[1].sequence, 2);
        assert_eq!(all[1].state, 0);

        let (after_first, latest) = log.since(1).unwrap();
        assert_eq!(latest, 2);
        assert_eq!(after_first.len(), 1);
        assert_eq!(after_first[0].pos, [1, 2, 3]);
    }

    fn receive_clientbound(client: &mut Connection) -> play::Clientbound {
        let (id, payload) = client.recv().unwrap();
        play::Clientbound::decode(id, &payload).unwrap()
    }

    fn test_world_root() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "borizon-play-world-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    fn runtime_with_resident(
        resident: ResidentDimension<'static>,
    ) -> (LocalWorldRuntime<'static>, std::path::PathBuf) {
        let root = test_world_root();
        let mut runtime = LocalWorldRuntime::open(&root, &[]).expect("open temporary world");
        runtime
            .add_dimension(resident)
            .expect("register temporary Overworld");
        (runtime, root)
    }

    fn world_block_state(
        world: &PreparedPlayWorld,
        pos: BlockPos,
    ) -> horizon_world::registry::BlockStateId {
        let runtime = world.runtime.lock().unwrap();
        runtime
            .dimension("minecraft:overworld")
            .unwrap()
            .resident()
            .level()
            .block_state(pos)
    }

    fn set_world_block_state(
        world: &PreparedPlayWorld,
        pos: BlockPos,
        state: horizon_world::registry::BlockStateId,
    ) -> Option<horizon_world::registry::BlockStateId> {
        let mut runtime = world.runtime.lock().unwrap();
        runtime
            .dimension_mut("minecraft:overworld")
            .unwrap()
            .resident_mut()
            .level_mut()
            .set_raw(pos, state)
    }

    fn packet_block_state(packet: &[u8], pos: BlockPos) -> i32 {
        let mut reader = horizon_protocol::buf::Reader::new(packet);
        let id = reader.varint().unwrap();
        let play::Clientbound::LevelChunkWithLight(decoded) =
            play::Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("expected LevelChunkWithLight packet");
        };
        let sections = decoded.chunk.sections(24, 16, 7).unwrap();
        let section_y = pos.y.div_euclid(16);
        let section_index = usize::try_from(section_y + 4).unwrap();
        let index = (pos.y.rem_euclid(16) as usize) << 8
            | (pos.z.rem_euclid(16) as usize) << 4
            | pos.x.rem_euclid(16) as usize;
        sections[section_index].states.get(index)
    }

    fn chunk_data_at(
        source: &horizon_world::chunk::ChunkData,
        position: ChunkPos,
    ) -> horizon_world::chunk::ChunkData {
        let mut nbt = source.clone().into_nbt();
        let horizon_world::nbt::Tag::Compound(root) = &mut nbt.tag else {
            panic!("ChunkData root must be a compound");
        };
        root.insert(
            horizon_world::nbt::JavaString::from("xPos"),
            horizon_world::nbt::Tag::Int(position.x),
        );
        root.insert(
            horizon_world::nbt::JavaString::from("zPos"),
            horizon_world::nbt::Tag::Int(position.z),
        );
        horizon_world::chunk::ChunkData::new(nbt).expect("clone valid Anvil chunk NBT")
    }

    #[test]
    fn stop_dig_matches_reference_seventy_percent_threshold() {
        assert!(!stop_dig_breaks(0.699_999));
        assert!(stop_dig_breaks(0.7));
        assert!(stop_dig_breaks(1.0));
    }
    #[test]
    fn mining_progress_uses_server_ticks_not_client_tick_end_packets() {
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 9);
        player.end_tick();
        player.end_tick();
        assert_eq!(player.server_tick, 0);
        assert_eq!(dig_progress_at_tick(0.1, player.server_tick, 0), 0.1);
        assert_eq!(dig_progress_at_tick(0.1, 3, 0), 0.4);
    }
    #[test]
    fn block_action_ack_is_sent_for_start_stop_abort_but_not_direction_changes() {
        for action in [0, 2, 3] {
            let (server_pipe, client_pipe) = Pipe::pair();
            let (_, mut writer) = Connection::local(server_pipe).split();
            let mut client = Connection::local(client_pipe);
            let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 9);
            process_player_action(
                &mut writer,
                &mut player,
                play::PlayerAction {
                    action,
                    pos: [0, 64, 0],
                    direction: 1,
                    sequence: 120 + action,
                },
                None,
            )
            .unwrap();
            let (id, payload) = client.recv().unwrap();
            assert_eq!(
                play::Clientbound::decode(id, &payload).unwrap(),
                play::Clientbound::BlockChangedAck(play::BlockChangedAck {
                    sequence: 120 + action,
                })
            );
        }

        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 9);
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 1,
                pos: [0, 64, 0],
                direction: 1,
                sequence: 124,
            },
            None,
        )
        .unwrap();
        drop(writer);
        assert!(client.recv().is_err());
    }

    #[test]
    fn container_clicks_move_player_items_using_menu_state() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 9);
        player.inventory.insert_stack(&Stack::new(4, 5), 64);
        player.inventory.content_packet();
        let mut pending_keepalive = None;
        let mut last_keepalive = Instant::now();

        process_play_packet(
            &mut writer,
            play::Serverbound::ContainerClick(play::ContainerClick {
                container: 0,
                state: player.inventory.state_id(),
                slot: 9,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        let snapshot = receive_clientbound(&mut client);
        assert!(matches!(
            snapshot,
            play::Clientbound::ContainerSetContent(play::ContainerSetContent { items, carried: Some(stack), .. })
                if items[9].is_none() && stack == Stack::new(4, 5)
        ));

        process_play_packet(
            &mut writer,
            play::Serverbound::ContainerClick(play::ContainerClick {
                container: 0,
                state: player.inventory.state_id(),
                slot: 36,
                button: 1,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::ContainerSetContent(play::ContainerSetContent { items, carried: Some(stack), .. })
                if items[36].as_ref().is_some_and(|stack| stack.count == 1)
                    && stack.count == 4
        ));
    }

    #[test]
    fn offline_chat_is_echoed_without_ending_the_play_session() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 9);
        player.name = "BorizonTest".into();
        let mut pending_keepalive = None;
        let mut last_keepalive = Instant::now();

        process_play_packet(
            &mut writer,
            play::Serverbound::Chat(play::Chat {
                message: "hello".into(),
                timestamp: 0,
                salt: 0,
                signature: None,
                last_seen: play::LastSeenUpdate {
                    offset: 0,
                    acknowledged: [0; 3],
                    checksum: 0,
                },
            }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();

        assert_eq!(
            receive_clientbound(&mut client),
            play::Clientbound::SystemChat(play::SystemChat {
                content: horizon_world::nbt::Tag::String("<BorizonTest> hello".into()),
                overlay: false,
            })
        );
    }

    #[test]
    fn ordinary_empty_hand_play_packets_are_processed_without_dropping_session() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 9);
        player.loaded = true;
        player.awaiting_teleport = None;
        let mut pending_keepalive = None;
        let mut last_keepalive = Instant::now();

        process_play_packet(
            &mut writer,
            play::Serverbound::PlayerInput(play::PlayerInput { flags: 0x51 }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        process_play_packet(
            &mut writer,
            play::Serverbound::PlayerCommand(play::PlayerCommand {
                id: 9,
                action: 1,
                data: 0,
            }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        process_play_packet(
            &mut writer,
            play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot: 4 }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        assert_eq!(player.input_flags, 0x51);
        assert!(player.sprinting);
        assert_eq!(player.selected_slot, 4);

        process_play_packet(
            &mut writer,
            play::Serverbound::UseItemOn(play::UseItemOn {
                hand: 0,
                pos: [0, 64, 0],
                face: 1,
                cursor: [0.5, 0.5, 0.5],
                inside: true,
                world_border: true,
                sequence: 70,
            }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        process_play_packet(
            &mut writer,
            play::Serverbound::UseItem(play::UseItem {
                hand: 0,
                sequence: 71,
                yaw: 0.0,
                pitch: 0.0,
            }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        for sequence in [70, 71] {
            assert_eq!(
                receive_clientbound(&mut client),
                play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence })
            );
        }

        process_play_packet(
            &mut writer,
            play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot: 9 }),
            &mut player,
            &mut pending_keepalive,
            &mut last_keepalive,
            None,
        )
        .unwrap();
        assert_eq!(
            receive_clientbound(&mut client),
            play::Clientbound::SetHeldSlot(play::SetHeldSlot { slot: 4 })
        );
    }

    #[test]
    fn official_resources_build_complete_play_spawn_view() {
        let Some(resources_path) = std::env::var_os("BORIZON_SERVER_RESOURCES") else {
            eprintln!(
                "skipped: set BORIZON_SERVER_RESOURCES to run the official 26.3 PLAY bootstrap test"
            );
            return;
        };
        let resources = Arc::new(
            crate::resources::load(std::path::Path::new(&resources_path))
                .expect("load official 26.3 server resources"),
        );
        let root = test_world_root();
        let world = PreparedPlayWorld::build(resources.clone(), 0, &root)
            .expect("build the complete official-resource PLAY world");
        let center = chunk_center(world.spawn);
        let expected = chunks_in_view(center);
        let target = {
            let runtime = world.runtime.lock().unwrap();
            let resident = runtime.dimension("minecraft:overworld").unwrap().resident();
            let air = resources.registry.default_state("minecraft:air").unwrap();
            let mut found = None;
            for y in -64..320 {
                let pos = BlockPos::new(center.x * 16 + 8, y, center.z * 16 + 8);
                let state = resident.level().block_state(pos);
                if state != air {
                    found = Some(pos);
                    break;
                }
            }
            found.expect("generated center column contains a non-air block")
        };
        {
            let cache = world.chunk_packet_cache.lock().unwrap();
            assert_eq!(world.chunk_packets.len(), CHUNK_COUNT);
            assert_eq!(cache.len(), CHUNK_COUNT);
            assert_eq!(cache.keys().copied().collect::<HashSet<_>>(), expected);
            assert!(cache.values().all(|packet| !packet.is_empty()));
        }
        {
            let runtime = world.runtime.lock().unwrap();
            let resident = runtime.dimension("minecraft:overworld").unwrap().resident();
            assert_eq!(resident.level().loaded_chunks().len(), CHUNK_COUNT);
            assert!(expected.iter().all(|chunk| {
                resident
                    .level()
                    .is_loaded(BlockPos::new(chunk.x * 16, 0, chunk.z * 16))
            }));
        }

        let air = resources.registry.default_state("minecraft:air").unwrap();
        {
            let mut runtime = world.runtime.lock().unwrap();
            runtime
                .dimension_mut("minecraft:overworld")
                .unwrap()
                .resident_mut()
                .level_mut()
                .set_raw(target, air)
                .expect("target belongs to a loaded generated chunk");
            save_runtime_dirty_chunks(&mut runtime, &resources).expect("save changed block state");
        }
        world
            .chunk_packet_cache
            .lock()
            .unwrap()
            .remove(&target.chunk());
        world
            .prepare_chunk(target.chunk())
            .expect("rebuild a missing packet snapshot for a resident chunk");
        let rebuilt = world
            .chunk_packet_cache
            .lock()
            .unwrap()
            .get(&target.chunk())
            .unwrap()
            .clone();
        assert_eq!(
            packet_block_state(&rebuilt, target),
            i32::try_from(air.0).unwrap()
        );
        drop(world);

        let reopened = PreparedPlayWorld::build(resources, 0, &root)
            .expect("reopen PLAY world from Anvil chunk data");
        let runtime = reopened.runtime.lock().unwrap();
        let resident = runtime.dimension("minecraft:overworld").unwrap().resident();
        assert_eq!(resident.level().block_state(target), air);
        let chunk = reopened
            .chunk_packet_cache
            .lock()
            .unwrap()
            .get(&target.chunk())
            .unwrap()
            .clone();
        assert_eq!(
            packet_block_state(&chunk, target),
            i32::try_from(air.0).unwrap()
        );
        drop(runtime);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn official_server_resources_drive_play_mining_and_contextual_movement() {
        let Some(resources_path) = std::env::var_os("BORIZON_SERVER_RESOURCES") else {
            eprintln!(
                "skipped: set BORIZON_SERVER_RESOURCES to run the official 26.3 server-resource smoke test"
            );
            return;
        };
        let resources = Arc::new(
            crate::resources::load(std::path::Path::new(&resources_path))
                .expect("load official 26.3 server resources"),
        );
        let air = resources
            .registry
            .default_state("minecraft:air")
            .expect("vanilla air state");
        let stone = resources
            .registry
            .default_state("minecraft:stone")
            .expect("vanilla stone state");
        let target = BlockPos::new(0, 64, 0);
        let spawn = [0.5, 65.0, 0.5];
        let resident = ResidentDimension::new(
            "minecraft:overworld",
            ResidentLevel::new(resources.registry, -64, 384, 0).expect("create resident level"),
            Ticks::new(0),
        );
        let mut generator_worker = OverworldGenerator::new(resources.worldgen.clone(), 0)
            .expect("create official Overworld generator")
            .worker();
        let proto = generator_worker
            .generate_proto(ChunkPos::new(0, 0))
            .expect("generate fixture chunk NBT");
        let mut data = generated_chunk_data(&proto, &resources, None, false)
            .expect("encode fixture chunk NBT");
        let air_tag = horizon_world::nbt::Tag::compound([(
            "id",
            horizon_world::nbt::Tag::String(horizon_world::nbt::JavaString::from("minecraft:air")),
        )]);
        let air_palette = horizon_world::chunk::PalettedContainer::uniform(
            air_tag,
            horizon_protocol::chunk::BLOCKS,
            4,
        )
        .unwrap()
        .to_nbt();
        data.set_section_block_states((-4..20).map(|section_y| (section_y, air_palette.clone())))
            .expect("make fixture section block palettes uniformly air");
        let (mut runtime, world_root) = runtime_with_resident(resident);
        let dimension = runtime.dimension_mut("minecraft:overworld").unwrap();
        for z in -1..=1 {
            for x in -1..=2 {
                let position = ChunkPos::new(x, z);
                dimension
                    .insert_generated_chunk(
                        chunk_data_at(&data, position),
                        0,
                        |name| resolve_block_tick(&resources, name),
                        resolve_fluid_tick,
                    )
                    .expect("load persistable movement fixture chunk");
            }
        }
        {
            let resident = runtime
                .dimension_mut("minecraft:overworld")
                .unwrap()
                .resident_mut();
            assert_eq!(resident.level_mut().set_raw(target, stone), Some(air));
            let wall = BlockPos::new(1, 65, 0);
            assert_eq!(resident.level_mut().set_raw(wall, stone), Some(air));
        }
        let mut world = PreparedPlayWorld {
            world_root: world_root.clone(),
            resources,
            dimension_type: 0,
            spawn,
            chunk_packets: Vec::new(),
            chunk_packet_cache: Mutex::new(HashMap::new()),
            chunk_generation: None,
            view_distance: DEFAULT_VIEW_DISTANCE,
            simulation_distance: DEFAULT_SIMULATION_DISTANCE,
            game_mode: 0,
            difficulty: 1,
            max_players: 20,
            hardcore: false,
            runtime: Mutex::new(runtime),
            player_chunks: Mutex::new(HashMap::new()),
            block_updates: Mutex::new(BlockUpdateLog::default()),
            level_events: Mutex::new(LevelEventLog::default()),
            item_entities: ItemEntityStore::default(),
            loot_random: Mutex::new(LegacyRandom::new(0)),
        };
        PlayServer::tick_shared_world(&world, false)
            .expect("advance global and daylight clocks once through the server owner");
        let (updates, _) = world
            .block_updates_since(0)
            .expect("initial resident block changes are published");
        assert!(updates.iter().any(|update| update.pos == [0, 64, 0]));
        {
            let runtime = world.runtime.lock().unwrap();
            let dimension = runtime.dimension("minecraft:overworld").unwrap();
            assert_eq!(runtime.game_time(), 1);
            assert_eq!(dimension.day_time(), 1);
        }

        // A Java client uses UseItemOn for an empty-hand door interaction.
        // Exercise the real behaviour table through the PLAY packet path and
        // verify the resulting state reaches the session's block-update stream.
        let door_pos = BlockPos::new(0, 65, 1);
        let door = world
            .resources
            .registry
            .default_state("minecraft:oak_door")
            .expect("vanilla oak-door state");
        {
            let mut runtime = world.runtime.lock().unwrap();
            let resident = runtime
                .dimension_mut("minecraft:overworld")
                .unwrap()
                .resident_mut();
            assert_eq!(
                resident.level_mut().set_raw(BlockPos::new(0, 64, 1), stone),
                Some(air)
            );
            assert_eq!(resident.level_mut().set_raw(door_pos, door), Some(air));
        }
        let fixture_changes = world.runtime.lock().unwrap().take_block_changes();
        world
            .publish_block_changes(fixture_changes)
            .expect("publish test door fixture state");
        let (interaction_server_pipe, interaction_client_pipe) = Pipe::pair();
        let (_, mut interaction_writer) = Connection::local(interaction_server_pipe).split();
        let mut interaction_client = Connection::local(interaction_client_pipe);
        let mut interactor = PlayerSession::new(spawn, 4, 12);
        interactor.loaded = true;
        interactor.awaiting_teleport = None;
        interactor.sent_chunks.insert(ChunkPos::new(0, 0));
        interactor.world_update_cursor = world.latest_block_update();
        interactor.level_event_cursor = world.latest_level_event_sequence();
        process_use_item_on(
            &mut interaction_writer,
            &mut interactor,
            play::UseItemOn {
                hand: 0,
                pos: [door_pos.x, door_pos.y, door_pos.z],
                face: 1,
                cursor: [0.5, 0.5, 0.5],
                inside: false,
                world_border: false,
                sequence: 42,
            },
            Some(&world),
        )
        .expect("process an empty-hand door interaction");
        let (ack_id, ack_payload) = interaction_client.recv().unwrap();
        assert_eq!(
            play::Clientbound::decode(ack_id, &ack_payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 42 })
        );
        send_pending_block_updates(&mut interaction_writer, &mut interactor, &world)
            .expect("publish the changed door state to its viewer");
        let (update_id, update_payload) = interaction_client.recv().unwrap();
        let opened_door = match play::Clientbound::decode(update_id, &update_payload).unwrap() {
            play::Clientbound::BlockUpdate(update) => update,
            packet => panic!("expected the door block update, got {packet:?}"),
        };
        assert_eq!(opened_door.pos, [door_pos.x, door_pos.y, door_pos.z]);
        assert_ne!(opened_door.state, i32::try_from(door.0).unwrap());
        let (sound_id, sound_payload) = interaction_client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(sound_id, &sound_payload).unwrap(),
            play::Clientbound::Sound(play::Sound {
                sound: (-1, Some((ref sound, None))),
                source: 4,
                ..
            }) if sound == "minecraft:block.wooden_door.open"
        ));

        PlayServer::tick_shared_world(&world, true)
            .expect("autosave chunks and SavedData through the server owner");
        let (movement_server_pipe, movement_client_pipe) = Pipe::pair();
        let (_, mut movement_writer) = Connection::local(movement_server_pipe).split();
        let mut movement_client = Connection::local(movement_client_pipe);
        let mut mover = PlayerSession::new(spawn, 1, 9);
        mover.loaded = true;
        mover.awaiting_teleport = None;
        update_player_movement(
            &mut movement_writer,
            &mut mover,
            Some([2.5, 65.0, 0.5]),
            None,
            0,
            Some(&world),
        )
        .expect("process real PLAY movement packet against official stone shape");
        assert!(
            (0.69..=0.71).contains(&mover.position[0]),
            "player body must stop at the face of the stone wall: {:?}",
            mover.position
        );
        assert!(mover.awaiting_teleport.is_some());
        let (correction_id, correction_payload) = movement_client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(correction_id, &correction_payload).unwrap(),
            play::Clientbound::PlayerPosition(play::PlayerPosition { position, .. })
                if (0.69..=0.71).contains(&position[0])
        ));
        assert!(
            world
                .resolve_player_move(9, [47.5, 65.0, 0.5], [48.5, 65.0, 0.5], 0.0, false)
                .is_err(),
            "the collision resolver must still fail closed before the streaming owner loads a chunk"
        );

        let mut streamer = PlayerSession::new([31.5, 65.0, 0.5], 2, 10);
        streamer.loaded = true;
        streamer.awaiting_teleport = None;
        update_player_movement(
            &mut movement_writer,
            &mut streamer,
            Some([32.5, 65.0, 0.5]),
            None,
            0,
            Some(&world),
        )
        .expect("stream chunks before accepting movement into the next view");
        assert_eq!(streamer.position, [32.5, 65.0, 0.5]);
        let mut streamed_chunks = Vec::new();
        let mut finished = false;
        while !finished {
            let (id, payload) = movement_client.recv().unwrap();
            match play::Clientbound::decode(id, &payload).unwrap() {
                play::Clientbound::LevelChunkWithLight(chunk) => {
                    streamed_chunks.push((chunk.x, chunk.z));
                }
                play::Clientbound::ChunkBatchFinished(batch) => {
                    assert_eq!(batch.size, 3);
                    finished = true;
                }
                play::Clientbound::SetChunkCacheCenter(center) => {
                    assert_eq!((center.x, center.z), (2, 0));
                }
                play::Clientbound::ForgetLevelChunk(_) | play::Clientbound::ChunkBatchStart(_) => {}
                packet => panic!("unexpected packet during chunk view shift: {packet:?}"),
            }
        }
        assert_eq!(streamed_chunks, vec![(3, -1), (3, 0), (3, 1)]);

        let mut blocked_streamer = PlayerSession::new([47.5, 65.0, 0.5], 3, 11);
        blocked_streamer.loaded = true;
        blocked_streamer.awaiting_teleport = None;
        drop(world.chunk_generation.take());
        update_player_movement(
            &mut movement_writer,
            &mut blocked_streamer,
            Some([48.5, 65.0, 0.5]),
            None,
            0,
            Some(&world),
        )
        .expect("a streaming failure must reject movement without closing the session");
        assert_eq!(blocked_streamer.position, [47.5, 65.0, 0.5]);
        let (id, payload) = movement_client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(id, &payload).unwrap(),
            play::Clientbound::PlayerPosition(play::PlayerPosition { position, .. })
                if position == [47.5, 65.0, 0.5]
        ));

        let water = world
            .resources
            .registry
            .default_state("minecraft:water")
            .expect("vanilla water state");
        let powder_snow = world
            .resources
            .registry
            .default_state("minecraft:powder_snow")
            .expect("vanilla powder-snow state");
        assert_eq!(
            set_world_block_state(&world, BlockPos::new(0, 65, 2), water),
            Some(air)
        );
        assert_eq!(
            set_world_block_state(&world, BlockPos::new(0, 65, 3), powder_snow),
            Some(air)
        );
        let (through_water, _) = world
            .resolve_player_move(9, [0.5, 65.0, 2.5], [2.5, 65.0, 2.5], 0.0, false)
            .expect("resolve player-context water movement");
        assert!(
            through_water[0] > 2.49,
            "water must not block a player: {through_water:?}"
        );
        let (through_powder_snow, _) = world
            .resolve_player_move(9, [0.5, 65.0, 3.5], [2.5, 65.0, 3.5], 0.0, false)
            .expect("resolve player-context powder-snow movement");
        assert!(
            through_powder_snow[0] > 2.49,
            "an empty-handed player without leather boots must not stand on powder snow: {through_powder_snow:?}"
        );
        world.runtime.lock().unwrap().take_block_changes(); // fixture-only fluid placement, already present in the client view
        let facts = world.resources.behaviours.facts(stone);
        let correct_tool = facts.flags & state_flags::REQUIRES_TOOL == 0;
        let per_tick = player_mining::destroy_progress(
            facts.hardness,
            correct_tool,
            player_mining::destroy_speed(1.0, &DestroySpeedModifiers::default()),
        );
        assert!((0.0..1.0).contains(&per_tick));

        // The main's STOP path asks the player for destroy progress again,
        // using the currently held tool. A pickaxe switched to after START
        // must therefore count for STOP's 70% decision even when the elapsed
        // bare-hand progress is still below that threshold.
        let (tool_server_pipe, tool_client_pipe) = Pipe::pair();
        let (_, mut tool_writer) = Connection::local(tool_server_pipe).split();
        let mut tool_client = Connection::local(tool_client_pipe);
        let preexisting_item_ids = world
            .item_entities
            .all()
            .into_iter()
            .map(|item| item.id)
            .collect::<HashSet<_>>();
        let mut tool_player = PlayerSession::new(spawn, 5, 13);
        tool_player.loaded = true;
        tool_player.awaiting_teleport = None;
        tool_player.on_ground = true;
        tool_player.world_update_cursor = world.latest_block_update();
        tool_player.level_event_cursor = world.latest_level_event_sequence();
        let (viewer_server_pipe, viewer_client_pipe) = Pipe::pair();
        let (_, mut viewer_writer) = Connection::local(viewer_server_pipe).split();
        let mut viewer_client = Connection::local(viewer_client_pipe);
        let mut nearby_viewer = PlayerSession::new(spawn, 6, 14);
        nearby_viewer.loaded = true;
        nearby_viewer.awaiting_teleport = None;
        nearby_viewer.sent_chunks.insert(ChunkPos::new(0, 0));
        nearby_viewer.level_event_cursor = world.latest_level_event_sequence();
        process_player_action(
            &mut tool_writer,
            &mut tool_player,
            play::PlayerAction {
                action: 0,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 40,
            },
            Some(&world),
        )
        .unwrap();
        assert!(matches!(
            receive_clientbound(&mut tool_client),
            play::Clientbound::BlockChangedAck(_)
        ));
        send_pending_level_events(&mut viewer_writer, &mut nearby_viewer, &world).unwrap();
        assert!(matches!(
            receive_clientbound(&mut viewer_client),
            play::Clientbound::BlockDestruction(play::BlockDestruction {
                id: 13,
                pos,
                progress: 0,
            }) if pos == [target.x, target.y, target.z]
        ));
        let elapsed_ticks = (0.2 / per_tick).ceil() as usize;
        let first_stage_ticks = (0.11 / per_tick).ceil() as usize;
        for _ in 0..first_stage_ticks {
            advance_player_mining(&mut tool_writer, &mut tool_player, Some(&world)).unwrap();
        }
        send_pending_level_events(&mut viewer_writer, &mut nearby_viewer, &world).unwrap();
        assert!(matches!(
            receive_clientbound(&mut viewer_client),
            play::Clientbound::BlockDestruction(play::BlockDestruction {
                id: 13,
                pos,
                progress: 1,
            }) if pos == [target.x, target.y, target.z]
        ));
        for _ in first_stage_ticks..elapsed_ticks {
            advance_player_mining(&mut tool_writer, &mut tool_player, Some(&world)).unwrap();
        }
        let pickaxe = world
            .resources
            .item_ids
            .id("minecraft:diamond_pickaxe")
            .expect("vanilla diamond pickaxe item ID");
        assert!(
            tool_player
                .inventory
                .set_creative_slot(36, Some(horizon_protocol::item::Stack::new(pickaxe, 1)))
        );
        let held_progress =
            player_destroy_progress(&tool_player, &world, stone, facts.hardness, facts.flags);
        let elapsed = (tool_player.server_tick + 1) as f32;
        assert!(
            per_tick * elapsed < 0.7,
            "bare-hand progress must remain below STOP threshold"
        );
        assert!(
            held_progress * elapsed >= 0.7,
            "the real pickaxe must cross STOP threshold"
        );
        process_player_action(
            &mut tool_writer,
            &mut tool_player,
            play::PlayerAction {
                action: 3,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 41,
            },
            Some(&world),
        )
        .unwrap();
        assert!(matches!(
            receive_clientbound(&mut tool_client),
            play::Clientbound::BlockChangedAck(_)
        ));
        assert_eq!(
            world_block_state(&world, target),
            air,
            "STOP must use the held diamond pickaxe"
        );
        for item in world.item_entities.all() {
            if !preexisting_item_ids.contains(&item.id) {
                world.item_entities.remove(item.id);
            }
        }
        assert_eq!(set_world_block_state(&world, target, stone), Some(air));
        world.runtime.lock().unwrap().take_block_changes(); // reset test-only mining mutation

        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new(spawn, 1, 9);
        player.world_update_cursor = world.latest_block_update();
        player.level_event_cursor = world.latest_level_event_sequence();
        player.loaded = true;
        player.on_ground = true;
        player.awaiting_teleport = None;
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 0,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 41,
            },
            Some(&world),
        )
        .unwrap();
        let (ack_id, ack_payload) = client.recv().unwrap();
        assert_eq!(
            play::Clientbound::decode(ack_id, &ack_payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 41 })
        );
        let tick_count = (0.8 / per_tick).ceil() as usize + 1;
        for _ in 0..tick_count {
            advance_player_mining(&mut writer, &mut player, Some(&world)).unwrap();
        }
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 3,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 42,
            },
            Some(&world),
        )
        .unwrap();
        let (ack_id, ack_payload) = client.recv().unwrap();
        assert_eq!(
            play::Clientbound::decode(ack_id, &ack_payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 42 })
        );
        let (stage_id, stage_payload) = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(stage_id, &stage_payload).unwrap(),
            play::Clientbound::BlockDestruction(play::BlockDestruction { progress: 255, .. })
        ));
        advance_player_mining(&mut writer, &mut player, Some(&world)).unwrap();
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockUpdate(_)
        ));
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::LevelEvent(play::LevelEvent { kind: 2001, .. })
        ));
        let after = world_block_state(&world, target);
        assert_eq!(after, air, "the PlayerAction flow must destroy the block");
        assert_ne!(stone, after);

        assert_eq!(set_world_block_state(&world, target, stone), Some(air));
        world.runtime.lock().unwrap().take_block_changes(); // fixture reset before the next simulated client action
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 0,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 43,
            },
            Some(&world),
        )
        .unwrap();
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockChangedAck(_)
        ));
        advance_player_mining(&mut writer, &mut player, Some(&world)).unwrap();
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 2,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 44,
            },
            Some(&world),
        )
        .unwrap();
        let (abort_ack_id, abort_ack_payload) = client.recv().unwrap();
        assert_eq!(
            play::Clientbound::decode(abort_ack_id, &abort_ack_payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 44 })
        );
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockUpdate(_)
        ));
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockDestruction(play::BlockDestruction { progress: 255, .. })
        ));
        assert!(player.digging.is_none());
        assert_eq!(world_block_state(&world, target), stone);

        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 0,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 45,
            },
            Some(&world),
        )
        .unwrap();
        let (start_ack_id, start_ack_payload) = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(start_ack_id, &start_ack_payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 45 })
        ));
        advance_player_mining(&mut writer, &mut player, Some(&world)).unwrap();
        let preserved_clock = player.dig_clock;
        player.position = [100.5, 65.0, 100.5];
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 3,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 46,
            },
            Some(&world),
        )
        .unwrap();
        let (far_stop_ack_id, far_stop_ack_payload) = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(far_stop_ack_id, &far_stop_ack_payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 46 })
        ));
        assert!(
            player.digging.is_none(),
            "out-of-reach STOP ends the active dig"
        );
        assert_eq!(
            player.dig_clock, preserved_clock,
            "START clock remains for the same block"
        );

        player.position = spawn;
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 3,
                pos: [target.x, target.y, target.z],
                direction: 1,
                sequence: 47,
            },
            Some(&world),
        )
        .unwrap();
        let (stop_ack_id, stop_ack_payload) = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(stop_ack_id, &stop_ack_payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 47 })
        ));
        assert!(
            player.delayed_dig.is_some(),
            "sub-70% STOP should be delayed"
        );
        for _ in 0..=(1.0 / per_tick).ceil() as usize {
            if world_block_state(&world, target) == air {
                break;
            }
            advance_player_mining(&mut writer, &mut player, Some(&world)).unwrap();
        }
        assert_eq!(world_block_state(&world, target), air);
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockDestruction(play::BlockDestruction { progress: 255, .. })
        ));
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockUpdate(_)
        ));

        // A block with an ordinary loot table must create a server-owned item
        // entity and replicate both its spawn and item stack to a loaded viewer.
        let dirt = world
            .resources
            .registry
            .default_state("minecraft:dirt")
            .expect("vanilla dirt state");
        assert_eq!(
            set_world_block_state(&world, BlockPos::new(0, 63, 0), stone),
            Some(air)
        );
        assert_eq!(set_world_block_state(&world, target, dirt), Some(air));
        world.runtime.lock().unwrap().take_block_changes();
        let (drop_server_pipe, drop_client_pipe) = Pipe::pair();
        let (_, mut drop_writer) = Connection::local(drop_server_pipe).split();
        let mut drop_client = Connection::local(drop_client_pipe);
        let mut drop_viewer = PlayerSession::new(spawn, 5, 13);
        drop_viewer.loaded = true;
        drop_viewer.awaiting_teleport = None;
        drop_viewer.world_update_cursor = world.latest_block_update();
        drop_viewer.level_event_cursor = world.latest_level_event_sequence();
        destroy_player_block(
            &mut drop_writer,
            &mut drop_viewer,
            &world,
            DiggingState {
                pos: [target.x, target.y, target.z],
                started_at: 0,
                per_tick: 1.0,
                progress: 1.0,
                last_stage: -1,
            },
        )
        .expect("destroy dirt through the server loot path");
        assert_eq!(world_block_state(&world, target), air);
        assert_eq!(world.item_entities.all().len(), 1);
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::BlockDestruction(play::BlockDestruction { progress: 255, .. })
        ));
        sync_item_entities(&mut drop_writer, &mut drop_viewer, &world)
            .expect("replicate dropped item to its loaded viewer");
        let item_id = match receive_clientbound(&mut drop_client) {
            play::Clientbound::AddEntity(packet) => packet.id,
            packet => panic!("expected dropped item spawn, got {packet:?}"),
        };
        let item_stack_id = match receive_clientbound(&mut drop_client) {
            play::Clientbound::SetEntityData(packet) if packet.id == item_id => {
                match &packet.values[0].2 {
                    EntityDataValue::Item(Some(stack)) => stack.item,
                    value => panic!("expected item stack metadata, got {value:?}"),
                }
            }
            packet => panic!("expected dropped item metadata, got {packet:?}"),
        };
        assert_eq!(
            item_stack_id,
            world.resources.item_ids.id("minecraft:dirt").unwrap()
        );

        for _ in 0..40 {
            let runtime = world.runtime.lock().unwrap();
            let resident = runtime.dimension("minecraft:overworld").unwrap().resident();
            world
                .item_entities
                .tick(|item, wanted| {
                    if !resident.level().is_loaded(BlockPos::new(
                        item.position[0].floor() as i32,
                        item.position[1].floor() as i32,
                        item.position[2].floor() as i32,
                    )) {
                        return Ok(None);
                    }
                    match crate::play_collision::resolve_item_move(
                        resident.level(),
                        runtime
                            .dimension("minecraft:overworld")
                            .unwrap()
                            .border()
                            .geometry(),
                        &world.resources,
                        item.id,
                        item.position,
                        wanted,
                        item.on_ground,
                    ) {
                        Ok(moved) => Ok(Some(moved)),
                        Err(error) if error.contains("unloaded chunk") => Ok(None),
                        Err(error) => Err(error),
                    }
                })
                .expect("simulate a dropped item using official block collisions");
        }
        assert!(world.item_entities.all()[0].on_ground);
        sync_item_entities(&mut drop_writer, &mut drop_viewer, &world)
            .expect("collect the grounded item and synchronize inventory");
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::TakeItemEntity(play::TakeItemEntity {
                item,
                player: 13,
                amount: 1,
            }) if item == item_id
        ));
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::ContainerSetContent(play::ContainerSetContent { items, .. })
                if items[9].as_ref().is_some_and(|stack| {
                    stack.item == item_stack_id && stack.count == 1
                })
        ));
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::RemoveEntities(play::RemoveEntities { ids })
                if ids == vec![item_id]
        ));
        let mut click_keepalive = None;
        let mut click_last_keepalive = Instant::now();
        process_play_packet(
            &mut drop_writer,
            play::Serverbound::ContainerClick(play::ContainerClick {
                container: 0,
                state: drop_viewer.inventory.state_id(),
                slot: 9,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
            &mut drop_viewer,
            &mut click_keepalive,
            &mut click_last_keepalive,
            Some(&world),
        )
        .expect("pick up the collected block stack from storage");
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::ContainerSetContent(play::ContainerSetContent {
                items,
                carried: Some(stack),
                ..
            }) if items[9].is_none() && stack.item == item_stack_id
        ));
        process_play_packet(
            &mut drop_writer,
            play::Serverbound::ContainerClick(play::ContainerClick {
                container: 0,
                state: drop_viewer.inventory.state_id(),
                slot: 36,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
            &mut drop_viewer,
            &mut click_keepalive,
            &mut click_last_keepalive,
            Some(&world),
        )
        .expect("move the block stack to the selected hotbar slot");
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::ContainerSetContent(play::ContainerSetContent {
                items,
                carried: None,
                ..
            }) if items[36].as_ref().is_some_and(|stack| stack.item == item_stack_id)
        ));
        process_use_item_on(
            &mut drop_writer,
            &mut drop_viewer,
            play::UseItemOn {
                hand: 0,
                pos: [0, 63, 0],
                face: 5,
                cursor: [0.5, 0.5, 0.5],
                inside: false,
                world_border: false,
                sequence: 43,
            },
            Some(&world),
        )
        .expect("place the collected vanilla block item");
        assert_eq!(
            world_block_state(&world, BlockPos::new(1, 63, 0)),
            dirt,
            "BlockItem placement must use the vanilla behaviour table"
        );
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 43 })
        ));
        assert!(matches!(
            receive_clientbound(&mut drop_client),
            play::Clientbound::ContainerSetContent(play::ContainerSetContent { items, .. })
                if items[36].is_none()
        ));
        assert!(world.item_entities.all().is_empty());

        // The fixture used several independent PlayerSession owners. Release
        // their ticket sources before the lifecycle assertions below.
        world
            .remove_player_chunk(9)
            .expect("release the first synthetic player's ticket");
        world
            .remove_player_chunk(10)
            .expect("release the streamed synthetic player's ticket");
        world
            .set_player_chunk(9, ChunkPos::new(0, 0))
            .expect("install the first vanilla player ticket");
        let owned_chunk_count = {
            let mut runtime = world.runtime.lock().unwrap();
            save_runtime_dirty_chunks(&mut runtime, &world.resources)
                .expect("persist all chunks before ticket-driven parking");
            runtime
                .dimension("minecraft:overworld")
                .unwrap()
                .resident()
                .level()
                .loaded_chunk_count()
        };
        world
            .set_player_chunk(9, ChunkPos::new(40, 0))
            .expect("reconcile after the player leaves the fixture chunks");
        {
            let runtime = world.runtime.lock().unwrap();
            let dimension = runtime.dimension("minecraft:overworld").unwrap();
            assert_eq!(dimension.resident().level().loaded_chunk_count(), 0);
            assert_eq!(dimension.parked_chunk_count(), owned_chunk_count);
        }
        world
            .set_player_chunk(9, ChunkPos::new(0, 0))
            .expect("reactivate the saved fixture chunks when ticketed again");
        {
            let runtime = world.runtime.lock().unwrap();
            let dimension = runtime.dimension("minecraft:overworld").unwrap();
            assert_eq!(
                dimension.resident().level().loaded_chunk_count(),
                owned_chunk_count
            );
            assert_eq!(dimension.parked_chunk_count(), 0);
            assert_eq!(dimension.resident().level().block_state(target), air);
        }
        world
            .finish_player_session(9)
            .expect("release tickets and checkpoint the world on logout");
        {
            let runtime = world.runtime.lock().unwrap();
            let dimension = runtime.dimension("minecraft:overworld").unwrap();
            assert_eq!(dimension.resident().level().loaded_chunk_count(), 0);
            assert_eq!(dimension.parked_chunk_count(), owned_chunk_count);
        }
        assert!(world_root.join("level.dat").is_file());
        let world = Arc::new(world);
        let play_server = Arc::new(PlayServer::new(Arc::clone(&world)));
        let tick_loop = play_server
            .start_world_tick_loop()
            .expect("start the listener-owned world tick thread");
        thread::sleep(Duration::from_millis(125));
        drop(tick_loop);
        let stopped_time = world.runtime.lock().unwrap().game_time();
        thread::sleep(Duration::from_millis(125));
        assert_eq!(
            world.runtime.lock().unwrap().game_time(),
            stopped_time,
            "dropping the listener's tick-loop guard must join and stop world time"
        );
        drop(play_server);
        drop(world);
        std::fs::remove_dir_all(world_root).unwrap();
    }

    #[test]
    fn player_action_start_ack_precedes_loaded_and_world_validation() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 9);
        // Not loaded and still awaiting its spawn teleport: vanilla still ACKs
        // the sequence before rejecting the action for gameplay.
        process_player_action(
            &mut writer,
            &mut player,
            play::PlayerAction {
                action: 0,
                pos: [1, 64, 1],
                direction: 1,
                sequence: 42,
            },
            None,
        )
        .unwrap();
        let (id, payload) = client.recv().unwrap();
        assert_eq!(
            play::Clientbound::decode(id, &payload).unwrap(),
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence: 42 })
        );
        assert!(player.digging.is_none());
    }

    #[test]
    fn movement_resolver_stops_at_wall_and_marks_ground_contact() {
        let wall = VoxelShape::block().moved(Vec3::new(2.0, 1.0, 0.0));
        let (stopped, grounded) =
            resolve_movement_with_shapes([0.5, 1.0, 0.5], [2.5, 1.0, 0.5], false, |_| {
                Ok::<_, String>(vec![wall.clone()])
            })
            .unwrap();
        assert!((stopped[0] - 1.7).abs() < 1.0e-8, "{stopped:?}");
        assert_eq!(stopped[1], 1.0);
        assert!(!grounded);

        let floor = VoxelShape::block();
        let (landed, grounded) =
            resolve_movement_with_shapes([0.5, 2.0, 0.5], [0.5, 0.25, 0.5], false, |_| {
                Ok::<_, String>(vec![floor.clone()])
            })
            .unwrap();
        assert_eq!(landed, [0.5, 1.0, 0.5]);
        assert!(grounded);
    }

    #[test]
    fn movement_correction_waits_for_matching_teleport_ack() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 1);
        player.awaiting_teleport = None;
        player.loaded = true;
        player
            .send_correction(&mut writer, [1.5, 65.0, 0.5])
            .unwrap();
        let (id, payload) = client.recv().unwrap();
        let packet = play::Clientbound::decode(id, &payload).unwrap();
        assert_eq!(
            packet,
            play::Clientbound::PlayerPosition(play::PlayerPosition {
                teleport: 2,
                position: [1.5, 65.0, 0.5],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives: 0,
            })
        );
        player
            .accept_teleport(&play::AcceptTeleportation {
                id: 2,
                x: 1.5,
                y: 65.0,
                z: 0.5,
                yaw: 0.0,
                pitch: 0.0,
            })
            .unwrap();
        assert_eq!(player.position, [1.5, 65.0, 0.5]);
        assert!(player.awaiting_teleport.is_none());
    }

    #[test]
    fn movement_limit_matches_reference_base_distance_threshold() {
        assert!(!exceeds_base_movement_limit([0.0; 3], [10.0, 0.0, 0.0]));
        assert!(exceeds_base_movement_limit([0.0; 3], [10.01, 0.0, 0.0]));
        assert!(exceeds_base_movement_limit([0.0; 3], [8.0, 6.1, 0.0]));
    }

    #[test]
    fn movement_waits_for_loaded_client_and_accepted_teleport() {
        let spawn = [4.5, 64.0, -2.5];
        let mut player = PlayerSession::new(spawn, 1, 1);
        player
            .update_movement(Some([7.5, 64.0, -2.5]), None, 1)
            .unwrap();
        assert_eq!(player.position, spawn);
        player
            .accept_teleport(&play::AcceptTeleportation {
                id: 1,
                x: spawn[0],
                y: spawn[1],
                z: spawn[2],
                yaw: 0.0,
                pitch: 0.0,
            })
            .unwrap();
        player.loaded = true;
        player.end_tick();
        player
            .update_movement(Some([7.5, 65.0, -2.5]), Some((190.0, 10.0)), 1)
            .unwrap();
        assert_eq!(player.position, [7.5, 65.0, -2.5]);
        assert_eq!(player.yaw, -170.0);
        assert!(player.on_ground);
    }

    #[test]
    fn movement_rejects_non_finite_values_and_duplicate_position_in_one_tick() {
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1, 1);
        player.awaiting_teleport = None;
        player.loaded = true;
        assert!(
            player
                .update_movement(Some([f64::NAN, 64.0, 0.5]), None, 0)
                .is_err()
        );
        player
            .update_movement(Some([1.5, 64.0, 0.5]), None, 0)
            .unwrap();
        assert!(
            player
                .update_movement(Some([2.5, 64.0, 0.5]), None, 0)
                .is_err()
        );
        player.end_tick();
        player
            .update_movement(Some([2.5, 64.0, 0.5]), None, 0)
            .unwrap();
    }

    #[test]
    fn play_reader_hands_packets_to_tick_session_and_replies() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let server = thread::spawn(move || {
            serve_until_disconnect(
                ConfiguredConnection {
                    connection: Connection::local(server_pipe),
                    profile: OfflineProfile {
                        name: "TickClient".into(),
                        uuid: 2,
                    },
                    client_information: None,
                },
                [0.5, 64.0, 0.5],
                1,
                "TickClient".into(),
                None,
                DEFAULT_VIEW_DISTANCE,
                DEFAULT_SIMULATION_DISTANCE,
                ChunkPos::new(0, 0),
            )
        });
        let mut client = Connection::local(client_pipe);
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .send(
                &play::Serverbound::PingRequest(horizon_protocol::packets::status::Ping {
                    time: 42,
                })
                .encode(),
            )
            .unwrap();
        let (id, payload) = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(id, &payload).unwrap(),
            play::Clientbound::ContainerSetContent(play::ContainerSetContent {
                container: 0,
                items,
                carried: None,
                ..
            }) if items.len() == crate::inventory::PLAYER_MENU_SLOTS
        ));
        let (id, payload) = client.recv().unwrap();
        assert_eq!(
            play::Clientbound::decode(id, &payload).unwrap(),
            play::Clientbound::PongResponse(horizon_protocol::packets::status::Ping { time: 42 })
        );
        drop(client);
        assert!(
            server
                .join()
                .unwrap()
                .unwrap_err()
                .contains("Connection closed")
        );
    }

    #[test]
    fn pending_play_packets_are_bounded_by_count() {
        let mut pending = PendingPackets::default();
        for id in 0..INBOX_LIMIT as i32 {
            pending.push((id, Vec::new())).unwrap();
        }
        let error = pending.push((INBOX_LIMIT as i32, Vec::new())).unwrap_err();
        assert!(error.contains("inbox limit"));
        assert_eq!(pending.packets.len(), INBOX_LIMIT);
    }

    #[test]
    fn play_server_tracks_unique_player_admissions_and_release() {
        let players = Mutex::new(PlayerRegistry {
            next_player_id: 1,
            ..PlayerRegistry::default()
        });
        let first_id = players.lock().unwrap().register(11, "First").unwrap();
        let permit = PlayerPermit {
            players: &players,
            uuid: 11,
        };
        assert_eq!(players.lock().unwrap().online(), 1);
        assert!(players.lock().unwrap().register(11, "First").is_err());
        assert_eq!(
            players.lock().unwrap().register(12, "Second").unwrap(),
            first_id + 1
        );
        drop(permit);
        assert_eq!(players.lock().unwrap().online(), 1);
        assert!(!players.lock().unwrap().by_uuid.contains_key(&11));
    }

    #[test]
    fn play_server_enforces_capacity() {
        let mut players = PlayerRegistry {
            next_player_id: 1,
            ..PlayerRegistry::default()
        };
        for uuid in 0..20 {
            players.register(uuid, "Player").unwrap();
        }
        assert_eq!(players.online(), 20);
        assert_eq!(players.register(99, "Full").unwrap_err(), "server is full");
    }

    #[test]
    fn idle_play_read_timeout_is_a_poll_not_a_disconnect() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let server = std::thread::spawn(move || {
            serve_until_disconnect(
                ConfiguredConnection {
                    connection: Connection::local(server_pipe),
                    profile: OfflineProfile {
                        name: "IdleClient".into(),
                        uuid: 1,
                    },
                    client_information: None,
                },
                [0.5, 64.0, 0.5],
                1,
                "IdleClient".into(),
                None,
                DEFAULT_VIEW_DISTANCE,
                DEFAULT_SIMULATION_DISTANCE,
                ChunkPos::new(0, 0),
            )
        });
        std::thread::sleep(Duration::from_millis(1200));
        drop(client_pipe);
        let error = server.join().unwrap().unwrap_err();
        assert!(error.contains("Connection closed"), "{error}");
    }

    #[test]
    fn play_keepalive_is_scheduled_when_packets_are_arriving() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut server) = Connection::local(server_pipe).split();
        let mut last = Instant::now();
        let mut pending = None;
        let mut next_id = 7;
        let now = last + Duration::from_secs(11);
        maintain_keepalive(
            &mut server,
            now,
            Duration::from_secs(10),
            Duration::from_secs(30),
            &mut last,
            &mut pending,
            &mut next_id,
        )
        .unwrap();
        let mut client = Connection::local(client_pipe);
        let (id, payload) = client.recv().unwrap();
        assert_eq!(id, play::Clientbound::KeepAlive(KeepAlive { id: 7 }).id());
        assert_eq!(
            play::Clientbound::decode(id, &payload).unwrap(),
            play::Clientbound::KeepAlive(KeepAlive { id: 7 })
        );
        assert_eq!(pending, Some((7, now)));
        assert_eq!(next_id, 8);
    }
}

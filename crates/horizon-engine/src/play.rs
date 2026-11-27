//! Server-owned PLAY bootstrap with one resident Overworld shared by sessions.
//!
//! This is not yet the authoritative gameplay loop. Player movement uses resident
//! contextual collisions, basic survival mining, and synchronous radius-one chunk
//! streaming handle a limited gameplay path; gravity, entity/block-entity simulation,
//! world ticks, other interactions, coherent post-mutation chunk packets, and
//! persistence remain pending.

use crate::{login::ConfiguredConnection, play_chunk, resources::ServerResources};
#[cfg(test)]
use horizon_core::geometry::{Aabb, Vec3, VoxelShape, try_resolve_step_movement};
use horizon_core::{
    position::{BlockPos, ChunkPos},
    ticks::TickTypeId,
};
use horizon_protocol::packets::{configuration::KeepAlive, play};
use horizon_world::{
    fluid::FluidKind,
    level::{BlockId, LevelAccess, ResidentDimension, ResidentLevel, Ticks, data::state_flags},
    player_mining::{self, DestroySpeedModifiers},
    worldgen::generator::{GeneratorWorker, OverworldGenerator, ProtoChunk},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const CHUNK_RADIUS: i32 = 1;
const CHUNK_COUNT: usize = 9;
const PLAY_TICK_BARRIER: Duration = Duration::from_millis(50);
const SERVER_TICK: Duration = Duration::from_millis(50);
const PACKETS_PER_TICK: usize = 400;
const INBOX_LIMIT: usize = 20 * PACKETS_PER_TICK;
const INBOX_BYTES: usize = 16 << 20;
const NETWORK_CHANNEL_CAPACITY: usize = 8;
const MAX_CLIENT_MOVEMENT_SQUARED_PER_TICK: f64 = 100.0;

pub(crate) struct PreparedPlayWorld {
    resources: Arc<ServerResources>,
    dimension_type: i32,
    spawn: [f64; 3],
    chunk_packets: Vec<Vec<u8>>,
    chunk_packet_cache: Mutex<HashMap<ChunkPos, Vec<u8>>>,
    generator_worker: Option<Mutex<GeneratorWorker>>,
    resident: Mutex<ResidentDimension<'static>>,
}

fn generated_chunk_data(
    proto: &ProtoChunk,
    resources: &ServerResources,
) -> Result<horizon_world::chunk::ChunkData, String> {
    proto
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
        .map_err(|error| format!("encode generated resident chunk: {error}"))
}

fn encode_center_chunk(
    proto: &ProtoChunk,
    light: horizon_protocol::chunk::LightData,
    resources: &ServerResources,
) -> Result<(Vec<u8>, horizon_world::chunk::ChunkData), String> {
    let position = proto.position();
    let chunk =
        play_chunk::encode_generated_chunk(proto, resources.registry, &resources.behaviours)?;
    let packet = play::Clientbound::LevelChunkWithLight(play::LevelChunkWithLight {
        x: position.x,
        z: position.z,
        chunk,
        light,
    })
    .encode();
    Ok((packet, generated_chunk_data(proto, resources)?))
}

fn generate_center_chunk(
    worker: &mut GeneratorWorker,
    center: ChunkPos,
    resources: &ServerResources,
) -> Result<(Vec<u8>, horizon_world::chunk::ChunkData), String> {
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
    let mut region: [ProtoChunk; CHUNK_COUNT] = chunks
        .try_into()
        .map_err(|_| "worldgen did not produce a 3×3 chunk region")?;
    worker
        .decorate(&mut region)
        .map_err(|error| format!("decorate chunk ({},{}): {error}", center.x, center.z))?;
    if !region[4].is_decorated() || region[4].position() != center {
        return Err(format!(
            "worldgen did not finalize the requested center chunk ({},{})",
            center.x, center.z
        ));
    }
    let lights = resources.light_occlusion.light_overworld_region(
        &region,
        resources.registry,
        &resources.behaviours,
    )?;
    encode_center_chunk(&region[4], lights[4].clone(), resources)
}

/// Shared world admission state for the PLAY listener. The resident dimension
/// and generated chunk packet cache are shared; authoritative simulation and
/// cross-player updates remain incomplete.
pub(crate) struct PlayServer {
    world: Arc<PreparedPlayWorld>,
    players: Mutex<PlayerRegistry>,
}

#[derive(Default)]
struct PlayerRegistry {
    by_uuid: HashMap<u128, i32>,
    next_player_id: i32,
}

impl PlayServer {
    pub(crate) fn new(world: Arc<PreparedPlayWorld>) -> Self {
        Self {
            world,
            players: Mutex::new(PlayerRegistry {
                next_player_id: 1,
                ..PlayerRegistry::default()
            }),
        }
    }

    pub(crate) fn online_players(&self) -> usize {
        self.players
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .online()
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
        const MAX_PLAYERS: usize = 20;
        if self.by_uuid.contains_key(&uuid) {
            return Err(format!("player {name} is already connected"));
        }
        if self.by_uuid.len() >= MAX_PLAYERS {
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
    pub(crate) fn build(resources: Arc<ServerResources>, seed: i64) -> Result<Self, String> {
        let dimension_type = resources
            .synchronized_registries
            .entry_index("minecraft:dimension_type", "minecraft:overworld")
            .ok_or("synchronized dimension_type registry has no minecraft:overworld")?;
        let generator = OverworldGenerator::new(resources.worldgen.clone(), seed)
            .map_err(|error| format!("create vanilla Overworld generator: {error}"))?;
        let mut worker = generator.worker();
        let mut region = Vec::with_capacity(CHUNK_COUNT);
        for z in -CHUNK_RADIUS..=CHUNK_RADIUS {
            for x in -CHUNK_RADIUS..=CHUNK_RADIUS {
                region.push(
                    worker
                        .generate_proto(ChunkPos::new(x, z))
                        .map_err(|error| format!("generate spawn chunk ({x},{z}): {error}"))?,
                );
            }
        }
        let mut region: [ProtoChunk; CHUNK_COUNT] = region
            .try_into()
            .map_err(|_| "worldgen did not produce the spawn 3×3")?;
        worker
            .decorate(&mut region)
            .map_err(|error| format!("decorate spawn chunks: {error}"))?;
        if !region[4].is_decorated() {
            return Err("worldgen did not bring the spawn chunk to Features status".into());
        }
        let spawn = find_spawn(&region, &resources)?;
        let spawn_center = chunk_center(spawn);

        let mut resident = ResidentDimension::new(
            "minecraft:overworld",
            ResidentLevel::new(resources.registry, -64, 384, seed)
                .map_err(|error| format!("create resident Overworld: {error}"))?,
            Ticks::new(seed as i32),
        );
        let mut chunk_packets = Vec::with_capacity(CHUNK_COUNT);
        let mut chunk_packet_cache = HashMap::with_capacity(CHUNK_COUNT);
        for z in spawn_center.z - CHUNK_RADIUS..=spawn_center.z + CHUNK_RADIUS {
            for x in spawn_center.x - CHUNK_RADIUS..=spawn_center.x + CHUNK_RADIUS {
                let position = ChunkPos::new(x, z);
                let (packet, chunk_data) = if position == region[4].position() {
                    let lights = resources.light_occlusion.light_overworld_region(
                        &region,
                        resources.registry,
                        &resources.behaviours,
                    )?;
                    encode_center_chunk(&region[4], lights[4].clone(), &resources)?
                } else {
                    generate_center_chunk(&mut worker, position, &resources)?
                };
                resident
                    .load_generated_chunk_data(
                        &chunk_data,
                        0,
                        |name| {
                            resources
                                .behaviours
                                .table
                                .block_by_name(name)
                                .map(|id| TickTypeId(u32::from(id.0)))
                        },
                        |name| {
                            FluidKind::ALL
                                .into_iter()
                                .find(|fluid| fluid.name() == name)
                                .map(|fluid| TickTypeId(fluid.index() as u32))
                        },
                    )
                    .map_err(|error| format!("load generated resident chunk: {error}"))?;
                chunk_packet_cache.insert(position, packet.clone());
                chunk_packets.push(packet);
            }
        }
        Ok(Self {
            resources,
            dimension_type,
            spawn,
            chunk_packets,
            chunk_packet_cache: Mutex::new(chunk_packet_cache),
            generator_worker: Some(Mutex::new(worker)),
            resident: Mutex::new(resident),
        })
    }

    pub(crate) fn initialize(
        &self,
        mut configured: ConfiguredConnection,
        player_id: i32,
    ) -> Result<(), String> {
        let spawn_block = BlockPos::new(
            self.spawn[0].floor() as i32,
            self.spawn[1].floor() as i32,
            self.spawn[2].floor() as i32,
        );
        if !self
            .resident
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .level()
            .is_loaded(spawn_block)
        {
            return Err("generated spawn is not loaded in the shared resident world".into());
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
        let spawn_chunk = [spawn_block[0].div_euclid(16), spawn_block[2].div_euclid(16)];
        let packets = [
            play::Clientbound::Login(play::Login {
                player_id,
                hardcore: false,
                levels: vec![
                    "minecraft:overworld".into(),
                    "minecraft:the_nether".into(),
                    "minecraft:the_end".into(),
                ],
                max_players: 20,
                chunk_radius: CHUNK_RADIUS,
                simulation_distance: CHUNK_RADIUS,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: play::SpawnInfo {
                    dimension_type: self.dimension_type,
                    dimension: dimension.clone(),
                    seed: 0,
                    game_type: 0,
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
                difficulty: 1,
                locked: false,
            }),
            play::Clientbound::PlayerAbilities(play::PlayerAbilities {
                flags: 0,
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
            play::Clientbound::SetChunkCacheRadius(play::Distance {
                distance: CHUNK_RADIUS,
            }),
            play::Clientbound::SetSimulationDistance(play::Distance {
                distance: CHUNK_RADIUS,
            }),
            play::Clientbound::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: spawn_chunk[0],
                z: spawn_chunk[1],
            }),
            play::Clientbound::ChunkBatchStart(horizon_protocol::packets::Empty),
        ];
        for packet in packets {
            configured
                .connection
                .send(&packet.encode())
                .map_err(|error| format!("send PLAY bootstrap packet: {error}"))?;
        }
        for packet in &self.chunk_packets {
            configured
                .connection
                .send(packet)
                .map_err(|error| format!("send spawn chunk: {error}"))?;
        }
        configured
            .connection
            .send(
                &play::Clientbound::ChunkBatchFinished(play::ChunkBatchFinished {
                    size: self.chunk_packets.len() as i32,
                })
                .encode(),
            )
            .map_err(|error| format!("finish spawn chunk batch: {error}"))?;
        configured
            .connection
            .send(
                &play::Clientbound::PlayerPosition(play::PlayerPosition {
                    teleport: 1,
                    position: self.spawn,
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                })
                .encode(),
            )
            .map_err(|error| format!("send initial player position: {error}"))?;
        println!(
            "Horizon: entered initial PLAY for {} at ({:.1}, {:.1}, {:.1}); gameplay simulation is incomplete",
            configured.profile.name, self.spawn[0], self.spawn[1], self.spawn[2]
        );
        serve_until_disconnect(configured, self.spawn, player_id, Some(self))
    }

    fn resolve_player_move(
        &self,
        entity_id: i32,
        from: [f64; 3],
        requested: [f64; 3],
        fall_distance: f64,
        on_ground: bool,
    ) -> Result<([f64; 3], bool), String> {
        let resident = self
            .resident
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        crate::play_collision::resolve_player_move(
            resident.level(),
            &self.resources,
            entity_id,
            from,
            requested,
            fall_distance,
            on_ground,
        )
    }

    fn prepare_player_view(&self, player: &PlayerSession, center: ChunkPos) -> Result<(), String> {
        for z in center.z - CHUNK_RADIUS..=center.z + CHUNK_RADIUS {
            for x in center.x - CHUNK_RADIUS..=center.x + CHUNK_RADIUS {
                let position = ChunkPos::new(x, z);
                if !player.sent_chunks.contains(&position) {
                    self.prepare_chunk(position)?;
                }
            }
        }
        Ok(())
    }

    fn prepare_chunk(&self, position: ChunkPos) -> Result<(), String> {
        if self
            .chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&position)
        {
            return Ok(());
        }
        let Some(worker) = &self.generator_worker else {
            return Err(format!(
                "cannot stream missing chunk ({},{}): no server generator worker",
                position.x, position.z
            ));
        };
        let mut worker = worker.lock().unwrap_or_else(|error| error.into_inner());
        if self
            .chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&position)
        {
            return Ok(());
        }
        let (packet, chunk_data) = generate_center_chunk(&mut worker, position, &self.resources)?;
        let mut resident = self
            .resident
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let probe = BlockPos::new(position.x * 16, -64, position.z * 16);
        if resident.level().is_loaded(probe) {
            return Err(format!(
                "resident chunk ({},{}) has no packet snapshot; refusing to send regenerated data",
                position.x, position.z
            ));
        }
        resident
            .load_generated_chunk_data(
                &chunk_data,
                0,
                |name| {
                    self.resources
                        .behaviours
                        .table
                        .block_by_name(name)
                        .map(|id| TickTypeId(u32::from(id.0)))
                },
                |name| {
                    FluidKind::ALL
                        .into_iter()
                        .find(|fluid| fluid.name() == name)
                        .map(|fluid| TickTypeId(fluid.index() as u32))
                },
            )
            .map_err(|error| {
                format!(
                    "load streamed chunk ({},{}): {error}",
                    position.x, position.z
                )
            })?;
        drop(resident);
        self.chunk_packet_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(position, packet);
        Ok(())
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

        let desired = chunks_in_view(center);
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

        let mut added = desired
            .difference(&player.sent_chunks)
            .copied()
            .collect::<Vec<_>>();
        added.sort_by_key(|chunk| (chunk.z, chunk.x));
        if !added.is_empty() {
            writer
                .send(
                    &play::Clientbound::ChunkBatchStart(horizon_protocol::packets::Empty).encode(),
                )
                .map_err(|error| format!("start streamed chunk batch: {error}"))?;
            let cache = self
                .chunk_packet_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for chunk in &added {
                let packet = cache.get(chunk).ok_or_else(|| {
                    format!(
                        "streamed chunk ({},{}) has no encoded packet",
                        chunk.x, chunk.z
                    )
                })?;
                writer.send(packet).map_err(|error| {
                    format!("send streamed chunk ({},{}): {error}", chunk.x, chunk.z)
                })?;
            }
            drop(cache);
            writer
                .send(
                    &play::Clientbound::ChunkBatchFinished(play::ChunkBatchFinished {
                        size: added.len() as i32,
                    })
                    .encode(),
                )
                .map_err(|error| format!("finish streamed chunk batch: {error}"))?;
        }
        player.sent_chunks = desired;
        player.chunk_center = center;
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

#[derive(Clone, Debug, PartialEq)]
struct PlayerSession {
    entity_id: i32,
    position: [f64; 3],
    chunk_center: ChunkPos,
    sent_chunks: HashSet<ChunkPos>,
    yaw: f32,
    pitch: f32,
    on_ground: bool,
    fall_distance: f64,
    last_teleport_id: i32,
    awaiting_teleport: Option<(i32, [f64; 3])>,
    loaded: bool,
    position_packet_this_tick: bool,
    server_tick: u64,
    digging: Option<DiggingState>,
    delayed_dig: Option<DiggingState>,
    dig_clock: Option<([i32; 3], u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DiggingState {
    pos: [i32; 3],
    started_at: u64,
    per_tick: f32,
    progress: f32,
}

impl PlayerSession {
    fn new(spawn: [f64; 3], teleport_id: i32, entity_id: i32) -> Self {
        let chunk_center = chunk_center(spawn);
        Self {
            entity_id,
            position: spawn,
            chunk_center,
            sent_chunks: chunks_in_view(chunk_center),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            fall_distance: 0.0,
            last_teleport_id: teleport_id,
            awaiting_teleport: Some((teleport_id, spawn)),
            loaded: false,
            position_packet_this_tick: false,
            server_tick: 0,
            digging: None,
            delayed_dig: None,
            dig_clock: None,
        }
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

fn serve_until_disconnect(
    configured: ConfiguredConnection,
    spawn: [f64; 3],
    entity_id: i32,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
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
    let result = run_play_session(&mut writer, received, spawn, entity_id, world);
    closer.close();
    let _ = network_reader.join();
    result
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
    spawn: [f64; 3],
    entity_id: i32,
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
    const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(30);

    let mut last_keepalive = Instant::now();
    let mut next_keepalive_id = 1_i64;
    let mut pending_keepalive: Option<(i64, Instant)> = None;
    let mut player = PlayerSession::new(spawn, 1, entity_id);
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
                    &mut player,
                    &mut pending_keepalive,
                    &mut last_keepalive,
                    world,
                )?;
            }
            advance_player_mining(writer, &mut player, world)?;
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
        play::Serverbound::ClientInformation(_) | play::Serverbound::ChunkBatchReceived(_) => {}
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
        let resident = world
            .resident
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !resident.level().is_loaded(pos)
            || pos.y < resident.level().min_y()
            || pos.y >= resident.level().max_y()
        {
            return Ok(());
        }
        let state = resident.level().block_state(pos);
        let facts = world.resources.behaviours.facts(state);
        let correct_tool = facts.flags & state_flags::REQUIRES_TOOL == 0;
        let modifiers = DestroySpeedModifiers {
            on_ground: player.on_ground,
            ..DestroySpeedModifiers::default()
        };
        let per_tick = player_mining::destroy_progress(
            facts.hardness,
            correct_tool,
            player_mining::destroy_speed(1.0, &modifiers),
        );
        let digging = DiggingState {
            pos: action.pos,
            started_at: player.server_tick,
            per_tick,
            progress: 0.0,
        };
        if per_tick >= 1.0 {
            player.digging = None;
            drop(resident);
            destroy_player_block(writer, player.entity_id, world, digging)?;
        } else {
            player.dig_clock = Some((action.pos, player.server_tick));
            player.digging = Some(digging);
        }
        return Ok(());
    }

    if action.action == 2 {
        player.digging = None;
        let resident = world
            .resident
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let state = resident.level().block_state(pos);
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
        })
    }) else {
        return Ok(());
    };
    let resident = world
        .resident
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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
    let correct_tool = facts.flags & state_flags::REQUIRES_TOOL == 0;
    digging.per_tick = player_mining::destroy_progress(
        facts.hardness,
        correct_tool,
        player_mining::destroy_speed(
            1.0,
            &DestroySpeedModifiers {
                on_ground: player.on_ground,
                ..DestroySpeedModifiers::default()
            },
        ),
    );
    drop(resident);
    digging.progress =
        dig_progress_at_tick(digging.per_tick, player.server_tick, digging.started_at);
    if stop_dig_breaks(digging.progress) {
        destroy_player_block(writer, player.entity_id, world, digging)?;
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
    entity_id: i32,
    world: &PreparedPlayWorld,
    digging: DiggingState,
) -> Result<(), String> {
    let pos = BlockPos::new(digging.pos[0], digging.pos[1], digging.pos[2]);
    let mut resident = world
        .resident
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !resident.level().is_loaded(pos) {
        return Ok(());
    }
    let state = resident.level().block_state(pos);
    let correct_tool =
        world.resources.behaviours.facts(state).flags & state_flags::REQUIRES_TOOL == 0;
    let destroyed =
        resident.destroy_block_by_player(&world.resources.behaviours, pos, false, correct_tool);
    let state = resident.level().block_state(pos);
    drop(resident);
    if destroyed {
        send_block_update(writer, digging.pos, state.0 as i32)?;
        send_crack_progress(writer, entity_id, digging.pos, -1)?;
    }
    Ok(())
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
    let resident = world
        .resident
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut delayed_completion = None;
    if let Some(mut digging) = player.digging.take() {
        let pos = BlockPos::new(digging.pos[0], digging.pos[1], digging.pos[2]);
        if resident.level().is_loaded(pos)
            && (resident.level().min_y()..resident.level().max_y()).contains(&pos.y)
        {
            let state = resident.level().block_state(pos);
            let facts = world.resources.behaviours.facts(state);
            if facts.flags & state_flags::AIR == 0 {
                let correct_tool = facts.flags & state_flags::REQUIRES_TOOL == 0;
                digging.per_tick = player_mining::destroy_progress(
                    facts.hardness,
                    correct_tool,
                    player_mining::destroy_speed(
                        1.0,
                        &DestroySpeedModifiers {
                            on_ground: player.on_ground,
                            ..DestroySpeedModifiers::default()
                        },
                    ),
                );
                digging.progress = (digging.progress + digging.per_tick).min(1.0);
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
                let correct_tool = facts.flags & state_flags::REQUIRES_TOOL == 0;
                digging.per_tick = player_mining::destroy_progress(
                    facts.hardness,
                    correct_tool,
                    player_mining::destroy_speed(
                        1.0,
                        &DestroySpeedModifiers {
                            on_ground: player.on_ground,
                            ..DestroySpeedModifiers::default()
                        },
                    ),
                );
                digging.progress =
                    dig_progress_at_tick(digging.per_tick, player.server_tick, digging.started_at);
                if digging.progress >= 1.0 {
                    delayed_completion = Some(digging);
                } else {
                    player.delayed_dig = Some(digging);
                }
            }
        }
    }
    drop(resident);
    if let Some(digging) = delayed_completion {
        destroy_player_block(writer, player.entity_id, world, digging)?;
    }
    player.server_tick = player.server_tick.wrapping_add(1);
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
    let requested_center = chunk_center(requested);
    if let Err(error) = world.prepare_player_view(player, requested_center) {
        eprintln!("Horizon: refusing movement because chunk streaming failed: {error}");
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
            world.update_client_chunk_view(writer, player, chunk_center(resolved))?;
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

    fn receive_clientbound(client: &mut Connection) -> play::Clientbound {
        let (id, payload) = client.recv().unwrap();
        play::Clientbound::decode(id, &payload).unwrap()
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
        let mut resident = ResidentDimension::new(
            "minecraft:overworld",
            ResidentLevel::new(resources.registry, -64, 384, 0).expect("create resident level"),
            Ticks::new(0),
        );
        for z in -1..=1 {
            for x in -1..=2 {
                resident
                    .load_empty_chunk(ChunkPos::new(x, z))
                    .expect("load movement fixture chunk");
            }
        }
        assert_eq!(
            resident.level_mut().set_raw(target, stone),
            Some(air),
            "place vanilla stone into the loaded chunk fixture"
        );
        let wall = BlockPos::new(1, 65, 0);
        assert_eq!(resident.level_mut().set_raw(wall, stone), Some(air));
        let generator_worker = OverworldGenerator::new(resources.worldgen.clone(), 0)
            .expect("create official Overworld generator")
            .worker();
        let mut world = PreparedPlayWorld {
            resources,
            dimension_type: 0,
            spawn,
            chunk_packets: Vec::new(),
            chunk_packet_cache: Mutex::new(HashMap::new()),
            generator_worker: Some(Mutex::new(generator_worker)),
            resident: Mutex::new(resident),
        };
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
        drop(world.generator_worker.take());
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
        {
            let mut resident = world.resident.lock().unwrap();
            assert_eq!(
                resident.level_mut().set_raw(BlockPos::new(0, 65, 2), water),
                Some(air)
            );
            assert_eq!(
                resident
                    .level_mut()
                    .set_raw(BlockPos::new(0, 65, 3), powder_snow),
                Some(air)
            );
        }
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
        let facts = world.resources.behaviours.facts(stone);
        let correct_tool = facts.flags & state_flags::REQUIRES_TOOL == 0;
        let per_tick = player_mining::destroy_progress(
            facts.hardness,
            correct_tool,
            player_mining::destroy_speed(1.0, &DestroySpeedModifiers::default()),
        );
        assert!((0.0..1.0).contains(&per_tick));

        let (server_pipe, client_pipe) = Pipe::pair();
        let (_, mut writer) = Connection::local(server_pipe).split();
        let mut client = Connection::local(client_pipe);
        let mut player = PlayerSession::new(spawn, 1, 9);
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
        let (update_id, update_payload) = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(update_id, &update_payload).unwrap(),
            play::Clientbound::BlockUpdate(_)
        ));
        let (stage_id, stage_payload) = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(stage_id, &stage_payload).unwrap(),
            play::Clientbound::BlockDestruction(play::BlockDestruction { progress: 255, .. })
        ));
        let resident = world.resident.lock().unwrap();
        let after = resident.level().block_state(target);
        assert_eq!(after, air, "the PlayerAction flow must destroy the block");
        assert_ne!(stone, after);
        drop(resident);

        let mut resident = world.resident.lock().unwrap();
        assert_eq!(resident.level_mut().set_raw(target, stone), Some(air));
        drop(resident);
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
        assert_eq!(
            world.resident.lock().unwrap().level().block_state(target),
            stone
        );

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
            if world.resident.lock().unwrap().level().block_state(target) == air {
                break;
            }
            advance_player_mining(&mut writer, &mut player, Some(&world)).unwrap();
        }
        assert_eq!(
            world.resident.lock().unwrap().level().block_state(target),
            air
        );
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockUpdate(_)
        ));
        assert!(matches!(
            receive_clientbound(&mut client),
            play::Clientbound::BlockDestruction(play::BlockDestruction { progress: 255, .. })
        ));
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
                None,
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
                None,
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

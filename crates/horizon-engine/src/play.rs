//! Server-owned PLAY bootstrap with one resident Overworld shared by sessions.
//!
//! This is not yet the authoritative gameplay loop. Player movement is checked
//! against resident static block collisions; gravity, entities, world ticks,
//! block interaction, chunk streaming and persistence remain pending.

use crate::{login::ConfiguredConnection, play_chunk, resources::ServerResources};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape, try_resolve_step_movement},
    position::{BlockPos, ChunkPos},
    ticks::TickTypeId,
};
use horizon_protocol::packets::{configuration::KeepAlive, play};
use horizon_world::{
    fluid::{FluidKind, FluidState},
    level::{BlockId, LevelAccess, ResidentDimension, ResidentLevel, Ticks, data::state_flags},
    worldgen::generator::{OverworldGenerator, ProtoChunk},
};
use std::{
    collections::{HashMap, VecDeque},
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
    resident: Mutex<ResidentDimension<'static>>,
}

/// Shared world admission state for the PLAY listener. The resident dimension
/// is shared; authoritative player simulation and cross-player updates remain
/// incomplete.
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

        let mut resident = ResidentDimension::new(
            "minecraft:overworld",
            ResidentLevel::new(resources.registry, -64, 384, seed)
                .map_err(|error| format!("create resident Overworld: {error}"))?,
            Ticks::new(seed as i32),
        );
        for proto in &region {
            let chunk_data = proto
                .to_chunk_data(
                    |id| {
                        u16::try_from(id.0)
                            .ok()
                            .filter(|id| {
                                usize::from(*id) < resources.behaviours.table.block_count()
                            })
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
        }

        let spawn = find_spawn(&region, &resources)?;
        let lights = resources.light_occlusion.light_overworld_region(
            &region,
            resources.registry,
            &resources.behaviours,
        )?;
        let mut chunk_packets = Vec::with_capacity(CHUNK_COUNT);
        for (index, proto) in region.iter().enumerate() {
            let x = index % 3;
            let z = index / 3;
            let chunk = play_chunk::encode_generated_chunk(
                proto,
                resources.registry,
                &resources.behaviours,
            )?;
            chunk_packets.push(
                play::Clientbound::LevelChunkWithLight(play::LevelChunkWithLight {
                    x: x as i32 - CHUNK_RADIUS,
                    z: z as i32 - CHUNK_RADIUS,
                    chunk,
                    light: lights[index].clone(),
                })
                .encode(),
            );
        }
        Ok(Self {
            resources,
            dimension_type,
            spawn,
            chunk_packets,
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
                chunk_radius: 2,
                simulation_distance: 2,
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
            play::Clientbound::SetChunkCacheRadius(play::Distance { distance: 2 }),
            play::Clientbound::SetSimulationDistance(play::Distance { distance: 2 }),
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
        serve_until_disconnect(configured, self.spawn, Some(self))
    }

    fn resolve_player_move(
        &self,
        from: [f64; 3],
        requested: [f64; 3],
        on_ground: bool,
    ) -> Result<([f64; 3], bool), String> {
        let resident = self
            .resident
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let level = resident.level();
        resolve_movement_with_shapes(from, requested, on_ground, |bounds| {
            let min = bounds.min_position();
            let max = bounds.max_position();
            let floor = |v: f64| v.floor().clamp(-30_000_000.0, 30_000_000.0) as i32;
            let (min_x, min_y, min_z) = (floor(min.x), floor(min.y), floor(min.z));
            let (max_x, max_y, max_z) = (floor(max.x), floor(max.y), floor(max.z));
            // Bound broad-phase work even when a client reports a huge jump.
            let volume = i64::from(max_x - min_x + 1)
                .saturating_mul(i64::from(max_y - min_y + 1))
                .saturating_mul(i64::from(max_z - min_z + 1));
            if volume > 16_384 {
                return Err("movement collision query is too large".to_owned());
            }
            let mut collisions = Vec::new();
            for y in min_y..=max_y {
                for z in min_z..=max_z {
                    for x in min_x..=max_x {
                        let block = BlockPos::new(x, y, z);
                        if !level.is_loaded(block) {
                            return Err("movement reached a chunk that is not loaded".into());
                        }
                        let state = level.block_state(block);
                        if let Some(shape) = self.resources.behaviours.collision_shape(state) {
                            if !shape.is_empty() {
                                collisions.push(shape.moved(Vec3::new(
                                    f64::from(x),
                                    f64::from(y),
                                    f64::from(z),
                                )));
                            }
                        } else {
                            let facts = self.resources.behaviours.facts(state);
                            if facts.fluid == FluidState::EMPTY
                                && facts.flags & state_flags::FULL_COLLISION != 0
                            {
                                collisions.push(VoxelShape::block().moved(Vec3::new(
                                    f64::from(x),
                                    f64::from(y),
                                    f64::from(z),
                                )));
                            }
                        }
                    }
                }
            }
            Ok(collisions)
        })
    }
}

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

#[derive(Clone, Debug, PartialEq)]
struct PlayerSession {
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    on_ground: bool,
    last_teleport_id: i32,
    awaiting_teleport: Option<(i32, [f64; 3])>,
    loaded: bool,
    position_packet_this_tick: bool,
}

impl PlayerSession {
    fn new(spawn: [f64; 3], teleport_id: i32) -> Self {
        Self {
            position: spawn,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            last_teleport_id: teleport_id,
            awaiting_teleport: Some((teleport_id, spawn)),
            loaded: false,
            position_packet_this_tick: false,
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
    let result = run_play_session(&mut writer, received, spawn, world);
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
    world: Option<&PreparedPlayWorld>,
) -> Result<(), String> {
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
    const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(30);

    let mut last_keepalive = Instant::now();
    let mut next_keepalive_id = 1_i64;
    let mut pending_keepalive: Option<(i64, Instant)> = None;
    let mut player = PlayerSession::new(spawn, 1);
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
    match world.resolve_player_move(start, requested, flags & 1 != 0) {
        Ok((resolved, grounded)) => {
            player.position = resolved;
            player.on_ground = grounded || flags & 1 != 0;
            if (0..3).any(|axis| (resolved[axis] - requested[axis]).abs() > 1.0e-4) {
                player.send_correction(writer, resolved)?;
            }
        }
        // Until chunk streaming exists, reaching an unloaded chunk is handled
        // like a refused move: keep the session alive and return the player.
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
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1);
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
        let mut player = PlayerSession::new(spawn, 1);
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
        let mut player = PlayerSession::new([0.5, 64.0, 0.5], 1);
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

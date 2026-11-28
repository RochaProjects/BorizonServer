mod inventory;
mod item_entity;
mod light;
pub mod login;
mod play;
mod play_chunk;
mod play_collision;
mod player_data;
pub mod registry_sync;
mod resources;
mod status;
mod survival;

use horizon_protocol::{buf::Reader, packets::play as play_packets};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

const HELP: &str = "Horizon dedicated engine\n\nUsage:\n  horizon-engine --help\n  horizon-engine --version\n  horizon-engine --preflight\n  horizon-engine --validate-resources <directory>\n  horizon-engine --probe-worldgen <directory> [seed]\n  horizon-engine --probe-chunk-packet <directory> [seed]\n  horizon-engine --status-listen <address>\n  horizon-engine --login-listen <address> <resources>\n  horizon-engine --play-listen <address> <resources> [seed] [world-dir]\n\n--preflight checks eula.txt and loads/writes server.properties in the current directory.\n--validate-resources loads the server-side block registry, vanilla behaviours and worldgen bundle.\n--probe-worldgen generates an Overworld terrain-stage chunk, saves it to a temporary Anvil region, and reopens it; this is not play-ready.\n--probe-chunk-packet generates and decorates a 3x3 Overworld region, then round-trips the center chunk+light PLAY packet; light-level parity and gameplay are still incomplete.\n--status-listen serves vanilla 26.3 server-list status/ping only.\n--login-listen serves offline LOGIN/CONFIGURATION, then closes because PLAY is not implemented.\n--play-listen sends the spawn 3x3 first, then asynchronously streams toward server.properties view-distance; chunks load/save under world-dir (default: server.properties level-name). Seed, view/simulation distance, mode, difficulty, hardcore, max players, MOTD and online-mode use server.properties when omitted. This remains partial PLAY, not full vanilla parity.\nWith no option, starts the current partial PLAY listener from server.properties.";

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Help,
    Version,
    Preflight,
    ValidateResources(PathBuf),
    ProbeWorldgen(PathBuf, i64),
    ProbeChunkPacket(PathBuf, i64),
    StatusListen(String),
    LoginListen(String, PathBuf),
    PlayListen(String, PathBuf, Option<i64>, Option<PathBuf>),
    Run,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(arg) = args.next() else {
        return Ok(Command::Run);
    };
    match arg.to_str() {
        Some("--help" | "-h") => {
            if args.next().is_some() {
                Err("unexpected extra argument; use --help".into())
            } else {
                Ok(Command::Help)
            }
        }
        Some("--version" | "-V") => {
            if args.next().is_some() {
                Err("unexpected extra argument; use --help".into())
            } else {
                Ok(Command::Version)
            }
        }
        Some("--preflight") => {
            if args.next().is_some() {
                Err("unexpected extra argument; use --help".into())
            } else {
                Ok(Command::Preflight)
            }
        }
        Some("--validate-resources") => {
            let Some(directory) = args.next() else {
                return Err("--validate-resources requires a directory".into());
            };
            if args.next().is_some() {
                return Err("unexpected extra argument; use --help".into());
            }
            Ok(Command::ValidateResources(PathBuf::from(directory)))
        }
        Some("--probe-worldgen") => {
            let Some(directory) = args.next() else {
                return Err("--probe-worldgen requires a resource directory".into());
            };
            let seed = match args.next() {
                Some(seed) => seed
                    .to_str()
                    .ok_or("world seed must be valid UTF-8")?
                    .parse::<i64>()
                    .map_err(|_| "world seed must be a signed 64-bit integer")?,
                None => 0,
            };
            if args.next().is_some() {
                return Err("--probe-worldgen accepts at most one seed".into());
            }
            Ok(Command::ProbeWorldgen(PathBuf::from(directory), seed))
        }
        Some("--probe-chunk-packet") => {
            let Some(directory) = args.next() else {
                return Err("--probe-chunk-packet requires a resource directory".into());
            };
            let seed = match args.next() {
                Some(seed) => seed
                    .to_str()
                    .ok_or("world seed must be valid UTF-8")?
                    .parse::<i64>()
                    .map_err(|_| "world seed must be a signed 64-bit integer")?,
                None => 0,
            };
            if args.next().is_some() {
                return Err("--probe-chunk-packet accepts at most one seed".into());
            }
            Ok(Command::ProbeChunkPacket(PathBuf::from(directory), seed))
        }
        Some("--status-listen") => {
            let Some(address) = args.next() else {
                return Err("--status-listen requires an address".into());
            };
            if args.next().is_some() {
                return Err("--status-listen accepts one address".into());
            }
            let address = address
                .into_string()
                .map_err(|_| "listener address must be valid UTF-8")?;
            Ok(Command::StatusListen(address))
        }
        Some("--login-listen") => {
            let Some(address) = args.next() else {
                return Err("--login-listen requires an address and resource directory".into());
            };
            let Some(resources) = args.next() else {
                return Err("--login-listen requires a resource directory".into());
            };
            if args.next().is_some() {
                return Err("--login-listen accepts an address and resource directory".into());
            }
            let address = address
                .into_string()
                .map_err(|_| "listener address must be valid UTF-8")?;
            Ok(Command::LoginListen(address, PathBuf::from(resources)))
        }
        Some("--play-listen") => {
            let Some(address) = args.next() else {
                return Err("--play-listen requires an address and resource directory".into());
            };
            let Some(resources) = args.next() else {
                return Err("--play-listen requires a resource directory".into());
            };
            let seed = match args.next() {
                Some(seed) => Some(
                    seed.to_str()
                        .ok_or("world seed must be valid UTF-8")?
                        .parse::<i64>()
                        .map_err(|_| "world seed must be a signed 64-bit integer")?,
                ),
                None => None,
            };
            let world_root = args.next().map(PathBuf::from);
            if args.next().is_some() {
                return Err(
                    "--play-listen accepts address, resources, optional seed and world-dir".into(),
                );
            }
            let address = address
                .into_string()
                .map_err(|_| "listener address must be valid UTF-8")?;
            Ok(Command::PlayListen(
                address,
                PathBuf::from(resources),
                seed,
                world_root,
            ))
        }
        Some(_) => Err("unknown argument; use --help".into()),
        None => Err("arguments must be valid UTF-8".into()),
    }
}

/// Run the bootstrap command in the current directory.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<String, String> {
    let command = parse_args(args)?;
    match command {
        Command::Help => Ok(HELP.into()),
        Command::Version => Ok(format!("horizon-engine {}", env!("CARGO_PKG_VERSION"))),
        Command::Preflight => {
            let dir = std::env::current_dir().map_err(|e| format!("current directory: {e}"))?;
            preflight(&dir)
        }
        Command::ValidateResources(directory) => {
            let resources = resources::load(&directory)?;
            Ok(format!(
                "Server resources valid: {} block states, {} worldgen biomes, {} synchronized registries ({} entries), {} registry tags, {} built-in registries, {} item IDs, {} loot tables, resources={}",
                resources.registry.len(),
                resources.worldgen.biome_names().len(),
                resources.synchronized_registries.registries().len(),
                resources.synchronized_registries.entry_count(),
                resources.synchronized_registries.tag_count(),
                resources.builtin_registries.registries.len(),
                resources.item_ids.names.len(),
                resources.loot.table_count(),
                directory.display()
            ))
        }
        Command::ProbeWorldgen(directory, seed) => probe_worldgen(&directory, seed),
        Command::ProbeChunkPacket(directory, seed) => probe_chunk_packet(&directory, seed),
        Command::StatusListen(address) => status::listen(address)
            .map(|()| "status listener stopped".into())
            .map_err(|e| format!("status listener: {e}")),
        Command::LoginListen(address, directory) => {
            let resources = resources::load(&directory)?;
            status::listen_with_login(
                address,
                std::sync::Arc::new(resources.synchronized_registries),
            )
            .map(|()| "login probe listener stopped".into())
            .map_err(|e| format!("login probe listener: {e}"))
        }
        Command::PlayListen(address, directory, seed, world_root) => {
            let resources = std::sync::Arc::new(resources::load(&directory)?);
            let mut entropy = horizon_server_files::properties::SystemEntropy::default();
            let properties = horizon_server_files::properties::ServerProperties::from_file(
                &std::path::PathBuf::from("server.properties"),
                &mut entropy,
            )?;
            let seed = seed.unwrap_or(properties.seed);
            let world_root = world_root.unwrap_or_else(|| PathBuf::from(&properties.level_name));
            let world = play::PreparedPlayWorld::build_with_settings(
                resources.clone(),
                seed,
                &world_root,
                properties.view_distance,
                properties.simulation_distance,
                properties.game_mode,
                properties.difficulty,
                properties.max_players.max(1) as usize,
                properties.hardcore,
            )?;
            let online = properties
                .online_mode
                .then(|| login::OnlineAuthenticator::generate(properties.prevent_proxy_connections))
                .transpose()?
                .map(std::sync::Arc::new);
            status::listen_with_play_and_motd(
                address,
                std::sync::Arc::new(resources.synchronized_registries.clone()),
                std::sync::Arc::new(world),
                properties.motd.clone(),
                online,
            )
            .map(|()| "play listener stopped".into())
            .map_err(|e| format!("play listener: {e}"))
        }
        Command::Run => run_default_server(),
    }
}

fn run_default_server() -> Result<String, String> {
    println!("{}", preflight(Path::new("."))?);
    let mut entropy = horizon_server_files::properties::SystemEntropy::default();
    let properties = horizon_server_files::properties::ServerProperties::from_file(
        Path::new("server.properties"),
        &mut entropy,
    )?;
    if !(0..=u16::MAX as i32).contains(&properties.server_port) {
        return Err(format!(
            "server-port {} is outside the valid TCP port range",
            properties.server_port
        ));
    }
    let host = if properties.server_ip.is_empty() {
        "0.0.0.0"
    } else {
        &properties.server_ip
    };
    let address = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        std::net::SocketAddr::new(ip, properties.server_port as u16).to_string()
    } else {
        format!("{host}:{}", properties.server_port)
    };
    let resources = std::sync::Arc::new(resources::load(Path::new("resources"))?);
    let world_root = PathBuf::from(&properties.level_name);
    let world = play::PreparedPlayWorld::build_with_settings(
        resources.clone(),
        properties.seed,
        &world_root,
        properties.view_distance,
        properties.simulation_distance,
        properties.game_mode,
        properties.difficulty,
        properties.max_players.max(1) as usize,
        properties.hardcore,
    )?;
    let online = properties
        .online_mode
        .then(|| login::OnlineAuthenticator::generate(properties.prevent_proxy_connections))
        .transpose()?
        .map(std::sync::Arc::new);
    status::listen_with_play_and_motd(
        address,
        std::sync::Arc::new(resources.synchronized_registries.clone()),
        std::sync::Arc::new(world),
        properties.motd,
        online,
    )
    .map(|()| "play listener stopped".into())
    .map_err(|error| format!("play listener: {error}"))
}

/// Exercise generated chunk palettes and the current light calculation through
/// the PLAY codec. This is a packet-shape probe, not proof of light parity or gameplay.
fn probe_chunk_packet(directory: &Path, seed: i64) -> Result<String, String> {
    let resources = resources::load(directory)?;
    let generator = horizon_world::worldgen::generator::OverworldGenerator::new(
        resources.worldgen.clone(),
        seed,
    )
    .map_err(|error| format!("create vanilla overworld generator: {error}"))?;
    let mut worker = generator.worker();
    let mut region = Vec::with_capacity(9);
    for z in -1..=1 {
        for x in -1..=1 {
            region.push(
                worker
                    .generate_proto(horizon_core::position::ChunkPos::new(x, z))
                    .map_err(|error| format!("generate chunk ({x},{z}) for PLAY probe: {error}"))?,
            );
        }
    }
    let mut region: [horizon_world::worldgen::generator::ProtoChunk; 9] = region
        .try_into()
        .map_err(|_| "worldgen did not produce a complete 3x3 region")?;
    let features = worker
        .decorate(&mut region)
        .map_err(|error| format!("decorate the PLAY probe region: {error}"))?;
    let light_chunks = resources.light_occlusion.light_region(
        &region,
        resources.registry,
        &resources.behaviours,
        true,
    )?;
    let proto = region
        .into_iter()
        .nth(4)
        .ok_or("decorated region has no center chunk")?;
    if !proto.is_decorated() {
        return Err("worldgen did not mark the center chunk as decorated".into());
    }
    let mut occluding_edges = 0_u64;
    for y in -64..320 {
        for z in 0..16 {
            for x in 0..16 {
                let state = proto.block(x, y, z).0;
                for (direction, nx, ny, nz) in [
                    (horizon_core::geometry::Direction::East, x + 1, y, z),
                    (horizon_core::geometry::Direction::Up, x, y + 1, z),
                    (horizon_core::geometry::Direction::South, x, y, z + 1),
                ] {
                    if nx < 16 && ny < 320 && nz < 16 {
                        let neighbor = proto.block(nx, ny, nz).0;
                        occluding_edges += u64::from(
                            resources
                                .light_occlusion
                                .occludes(state, neighbor, direction),
                        );
                    }
                }
            }
        }
    }
    let payload =
        play_chunk::encode_generated_chunk(&proto, resources.registry, &resources.behaviours)?;
    let sections = payload
        .sections(24, 16, 7)
        .map_err(|error| format!("decode encoded PLAY chunk sections: {error}"))?;
    if sections.len() != 24 {
        return Err(format!(
            "expected 24 chunk sections, got {}",
            sections.len()
        ));
    }
    let non_air: i64 = sections
        .iter()
        .map(|section| i64::from(section.non_empty))
        .sum();
    let light_packet =
        play_packets::Clientbound::LevelChunkWithLight(play_packets::LevelChunkWithLight {
            x: 0,
            z: 0,
            chunk: payload.clone(),
            light: light_chunks[4].clone(),
        })
        .encode();
    let mut reader = Reader::new(&light_packet);
    let packet_id = reader
        .varint()
        .map_err(|error| format!("read encoded chunk/light packet id: {error}"))?;
    let decoded = play_packets::Clientbound::decode(packet_id, &light_packet[reader.position()..])
        .map_err(|error| format!("decode encoded vanilla chunk/light packet: {error}"))?;
    if !matches!(decoded, play_packets::Clientbound::LevelChunkWithLight(_)) {
        return Err("encoded chunk/light packet decoded as another PLAY packet".into());
    }
    Ok(format!(
        "Vanilla PLAY chunk+light payload passed: seed={seed}, chunk=0,0, status=Features, sections={}, heightmaps={}, non_air_blocks={non_air}, bytes={}, feature_roots={}, unsupported_features={}, failed_features={}, light_occluding_edges={occluding_edges}, light_packet_bytes={}, lighting=propagated",
        sections.len(),
        payload.heightmaps.len(),
        payload.buffer.len(),
        features.placed,
        features.unsupported,
        features.failed,
        light_packet.len()
    ))
}

/// Generate a real vanilla terrain chunk, save it through Anvil and reopen it.
/// This validates resource, generator, NBT and region-file integration without
/// pretending that lighting, login or the full gameplay loop are implemented.
fn probe_worldgen(directory: &Path, seed: i64) -> Result<String, String> {
    let resources = resources::load(directory)?;
    let generator =
        horizon_world::worldgen::generator::OverworldGenerator::new(resources.worldgen, seed)
            .map_err(|error| format!("create vanilla overworld generator: {error}"))?;
    let position = horizon_core::position::ChunkPos::new(0, 0);
    let mut worker = generator.worker();
    let chunk = worker
        .generate_proto(position)
        .map_err(|error| format!("generate vanilla overworld chunk 0,0: {error}"))?;
    let height = chunk.surface_height(8, 8);
    let biome = chunk.biome_at(8, height.saturating_sub(1), 8).to_owned();
    let block = chunk.block_name(8, height.saturating_sub(1), 8).to_owned();
    let data = chunk
        .to_chunk_data(|_| None, |_| None)
        .map_err(|error| format!("encode generated vanilla chunk: {error}"))?;

    let directory = std::env::temp_dir().join(format!(
        "borizon-server-worldgen-probe-{}",
        std::process::id()
    ));
    if directory.exists() {
        std::fs::remove_dir_all(&directory)
            .map_err(|error| format!("clear previous probe directory: {error}"))?;
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("create probe directory: {error}"))?;
    let region_path = directory.join("r.0.0.mca");
    let persisted = (|| {
        let record = horizon_world::region::ChunkRecord::from_nbt(
            data.root(),
            horizon_world::compression::Compression::Zlib,
            0,
            horizon_world::nbt::Limits::default(),
        )
        .map_err(|error| format!("encode generated chunk for Anvil: {error}"))?;
        let mut region = horizon_world::region::RegionFile::open_writable(&region_path, true)
            .map_err(|error| format!("create probe Anvil region: {error}"))?;
        region
            .write(position, &record)
            .map_err(|error| format!("write generated chunk to Anvil: {error}"))?;
        region
            .sync()
            .map_err(|error| format!("sync probe Anvil region: {error}"))?;
        drop(region);

        let mut reopened = horizon_world::region::RegionFile::open(&region_path)
            .map_err(|error| format!("reopen probe Anvil region: {error}"))?;
        let record = reopened
            .read(position)
            .map_err(|error| format!("read generated chunk from Anvil: {error}"))?
            .ok_or("generated chunk is missing after Anvil reopen")?;
        let nbt = record
            .nbt(horizon_world::nbt::Limits::default())
            .map_err(|error| format!("decode generated chunk from Anvil: {error}"))?;
        let restored = horizon_world::chunk::ChunkData::new(nbt)
            .map_err(|error| format!("validate reopened generated chunk: {error}"))?;
        if restored.position() != position {
            return Err("reopened generated chunk has the wrong coordinates".into());
        }
        Ok::<(), String>(())
    })();
    let cleanup =
        std::fs::remove_dir_all(&directory).map_err(|error| format!("remove probe files: {error}"));
    persisted?;
    cleanup?;

    Ok(format!(
        "Vanilla worldgen + Anvil passed: seed={seed}, chunk=0,0, status=Terrain, surface_y={height}, block={block}, biome={biome}"
    ))
}

/// Validate the EULA and normalize the existing vanilla server.properties file.
/// This does not open a world, bind a socket, or start gameplay.
pub fn preflight(dir: &Path) -> Result<String, String> {
    let eula_path = dir.join("eula.txt");
    let agreed = horizon_server_files::eula::agreed(&eula_path).map_err(|error| {
        format!(
            "EULA not accepted or unreadable ({}): {error}",
            eula_path.display()
        )
    })?;
    if !agreed {
        return Err(format!(
            "EULA not accepted; review {} and set eula=true",
            eula_path.display()
        ));
    }

    let properties_path = dir.join("server.properties");
    let mut entropy = horizon_server_files::properties::SystemEntropy::default();
    let properties = horizon_server_files::properties::ServerProperties::from_file(
        &properties_path,
        &mut entropy,
    )?;
    properties
        .store(&properties_path)
        .map_err(|e| format!("write {}: {e}", properties_path.display()))?;

    Ok(format!(
        "Server preflight passed: level-name={}, server-port={}, properties={}",
        properties.level_name,
        properties.server_port,
        properties_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_args, preflight, run};
    use std::{ffi::OsString, path::PathBuf};

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("horizon-engine-{name}-{}", std::process::id()))
    }

    fn clean_dir(name: &str) -> PathBuf {
        let dir = scratch(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_help_version_and_preflight_without_accepting_unknown_options() {
        assert_eq!(parse_args([OsString::from("--help")]), Ok(Command::Help));
        assert_eq!(
            parse_args([OsString::from("--version")]),
            Ok(Command::Version)
        );
        assert_eq!(
            parse_args([OsString::from("--preflight")]),
            Ok(Command::Preflight)
        );
        assert!(parse_args([OsString::from("--dedicated")]).is_err());
        assert!(parse_args([OsString::from("--help"), OsString::from("extra")]).is_err());
    }

    #[test]
    fn parses_play_listener_world_root_and_default() {
        assert_eq!(
            parse_args(
                [
                    "--play-listen",
                    "127.0.0.1:25565",
                    "resources",
                    "42",
                    "world-save",
                ]
                .map(OsString::from)
            ),
            Ok(Command::PlayListen(
                "127.0.0.1:25565".into(),
                PathBuf::from("resources"),
                Some(42),
                Some(PathBuf::from("world-save")),
            ))
        );
        assert_eq!(
            parse_args(["--play-listen", "127.0.0.1:25565", "resources",].map(OsString::from)),
            Ok(Command::PlayListen(
                "127.0.0.1:25565".into(),
                PathBuf::from("resources"),
                None,
                None,
            ))
        );
    }

    #[test]
    fn help_and_version_run_without_opening_a_world() {
        assert!(
            run([OsString::from("--help")])
                .unwrap()
                .contains("--preflight")
        );
        assert!(
            run([OsString::from("--version")])
                .unwrap()
                .starts_with("horizon-engine ")
        );
    }

    #[test]
    fn default_execution_selects_the_dedicated_server_command() {
        assert!(matches!(parse_args(std::iter::empty()), Ok(Command::Run)));
    }

    #[test]
    fn preflight_creates_default_eula_and_refuses_to_continue() {
        let dir = clean_dir("eula");
        let error = preflight(&dir).unwrap_err();
        assert!(error.contains("EULA not accepted"));
        let contents = std::fs::read_to_string(dir.join("eula.txt")).unwrap();
        assert!(contents.contains("eula=false"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preflight_accepts_eula_and_normalizes_server_properties() {
        let dir = clean_dir("properties");
        std::fs::write(
            dir.join("eula.txt"),
            horizon_server_files::eula::file(0, true),
        )
        .unwrap();
        std::fs::write(dir.join("server.properties"), "server-port=25570\n").unwrap();

        let report = preflight(&dir).unwrap();
        assert!(report.contains("server-port=25570"));
        let properties = std::fs::read_to_string(dir.join("server.properties")).unwrap();
        assert!(properties.contains("server-port=25570"));
        assert!(properties.contains("level-name=world"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

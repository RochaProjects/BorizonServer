//! Runtime server plugins are a separate layer: only the narrow C ABI loader
//! is linked here, while plugin logic stays in independently built libraries.

#[cfg(feature = "server-plugins")]
use super::native_server::NativeSidebar;
#[cfg(feature = "server-plugins")]
use super::native_world_edit::NativeHeightmap;
use super::{
    Game,
    native_server::{NativeCommand, NativeServerPlayer},
};
#[cfg(feature = "server-plugins")]
use horizon_client::{
    items::ItemStack, lan::data::component_nbt, mod_api::GameRuleValue, text::Text,
};
#[cfg(feature = "server-plugins")]
use horizon_server_plugin_loader::{Event, Host, ItemDelta, MenuItem, TransactionError};
#[cfg(feature = "server-plugins")]
use horizon_world::nbt::{Compound, Tag};
#[cfg(feature = "server-plugins")]
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "server-plugins")]
struct PlayerDataResetPlan {
    categories: Vec<String>,
    expires: Instant,
}

#[cfg(feature = "server-plugins")]
static PLAYER_DATA_RESET_PLANS: OnceLock<Mutex<HashMap<String, PlayerDataResetPlan>>> =
    OnceLock::new();
#[cfg(feature = "server-plugins")]
static PLAYER_DATA_RESET_NONCE: AtomicU64 = AtomicU64::new(1);
#[cfg(feature = "server-plugins")]
static SAFE_LOCATION_RNG: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
#[cfg(feature = "server-plugins")]
static DIMENSION_SEED_NONCE: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "server-plugins")]
struct ScheduledPluginTask {
    id: u64,
    plugin: String,
    due_tick: u64,
    period_ticks: Option<u64>,
    data: serde_json::Value,
}

#[cfg(feature = "server-plugins")]
static SCHEDULED_PLUGIN_TASKS: OnceLock<Mutex<Vec<ScheduledPluginTask>>> = OnceLock::new();
#[cfg(feature = "server-plugins")]
static NEXT_SCHEDULED_PLUGIN_TASK: AtomicU64 = AtomicU64::new(1);
#[cfg(feature = "server-plugins")]
static SERVER_PLUGIN_TICK: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "server-plugins")]
struct GameHost<'g, 'a, 'c> {
    game: &'g mut Game<'a>,
    commands: Option<&'c mut Vec<NativeCommand>>,
    suggestions: Option<Vec<String>>,
    /// The guest whose completion request is being handled. During this path
    /// `Game::lan` is temporarily taken by the LAN tick, so player.get cannot
    /// discover this player through the game's global guest list.
    requester: Option<NativeServerPlayer>,
}

#[cfg(feature = "server-plugins")]
fn reset_categories(value: &serde_json::Value) -> Option<Vec<String>> {
    let requested = value.as_array()?;
    let mut categories = Vec::new();
    for item in requested {
        let name = item.as_str()?.to_ascii_lowercase();
        match name.as_str() {
            "all" | "jogadores" | "players" => {
                categories.extend(["advancements", "stats", "playerdata"].map(str::to_owned));
            }
            "advancements" | "conquistas" => categories.push("advancements".into()),
            "stats" | "estatisticas" => categories.push("stats".into()),
            "playerdata" | "dados" => categories.push("playerdata".into()),
            _ => return None,
        }
    }
    categories.sort();
    categories.dedup();
    (!categories.is_empty()).then_some(categories)
}

#[cfg(feature = "server-plugins")]
fn valid_custom_dimension_id(id: &str) -> bool {
    let Some((_namespace, path)) = id.split_once(':') else {
        return false;
    };
    !matches!(
        id,
        "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
    ) && horizon_core::LevelKey::parse(id).is_ok()
        && !path.is_empty()
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

#[cfg(feature = "server-plugins")]
fn valid_plugin_identifier(id: &str) -> bool {
    id.len() <= 256
        && id.split_once(':').is_some_and(|(namespace, path)| {
            !namespace.is_empty()
                && !path.is_empty()
                && namespace
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
                && path
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
                && !path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
        })
}

#[cfg(feature = "server-plugins")]
fn find_safe_location(
    game: &Game<'_>,
    player: u128,
    data: &serde_json::Value,
) -> serde_json::Value {
    let snapshot = game
        .native_server_players()
        .into_iter()
        .find(|snapshot| snapshot.uuid == player)
        .or_else(|| {
            game.native_acting_player()
                .filter(|snapshot| snapshot.uuid == player)
        });
    let Some(snapshot) = snapshot else {
        return serde_json::json!({"ok": false, "error": "player_not_online"});
    };
    let Some(radius) = data
        .get("radius")
        .and_then(serde_json::Value::as_u64)
        .filter(|radius| (8..=512).contains(radius))
    else {
        return serde_json::json!({"ok": false, "error": "invalid_radius", "min": 8, "max": 512});
    };
    let attempts = data
        .get("attempts")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(96)
        .clamp(1, 256) as usize;
    let dimension = data
        .get("dimension")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&snapshot.dimension);
    if dimension != snapshot.dimension {
        return serde_json::json!({"ok": false, "error": "dimension_must_be_current"});
    }
    let center = data
        .get("center")
        .and_then(serde_json::Value::as_array)
        .filter(|center| center.len() == 2)
        .and_then(|center| {
            Some([
                center[0]
                    .as_i64()?
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                center[1]
                    .as_i64()?
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            ])
        })
        .unwrap_or([
            snapshot.position[0].floor() as i32,
            snapshot.position[2].floor() as i32,
        ]);
    let mut columns = Vec::with_capacity(attempts);
    for _ in 0..attempts {
        let seed = SAFE_LOCATION_RNG.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
        let mut value = seed ^ (player as u64) ^ (seed.rotate_left(17));
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        let x =
            ((value.wrapping_mul(0x2545_f491_4f6c_dd1d) % (radius * 2 + 1)) as i64) - radius as i64;
        value = value.wrapping_add(seed.rotate_left(31));
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        let z =
            ((value.wrapping_mul(0x2545_f491_4f6c_dd1d) % (radius * 2 + 1)) as i64) - radius as i64;
        if x * x + z * z > (radius as i64 * radius as i64) {
            continue;
        }
        columns.push((
            center[0].saturating_add(x as i32),
            center[1].saturating_add(z as i32),
        ));
    }
    let Ok(read) = game.native_read_columns(
        dimension,
        &columns,
        NativeHeightmap::MotionBlockingNoLeaves,
        (1, 2),
    ) else {
        return serde_json::json!({"ok": false, "error": "world_read_failed"});
    };
    for (index, base) in read.bases.iter().enumerate() {
        let Some(y) = base else { continue };
        let Some(blocks) = read.blocks.get(index * 3..index * 3 + 3) else {
            continue;
        };
        let name = |block: u16| read.palette.get(usize::from(block)).map(String::as_str);
        let ground = name(blocks[0]).unwrap_or("");
        let feet = name(blocks[1]).unwrap_or("");
        let head = name(blocks[2]).unwrap_or("");
        if ground.ends_with("_leaves")
            || ground.ends_with("_fence")
            || ground.ends_with("_wall")
            || ground.ends_with("_slab")
            || ground.ends_with("_stairs")
            || ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"].contains(&ground)
            || ground.contains("water")
            || ground.contains("lava")
            || ground.ends_with("_fire")
            || !["minecraft:air", "minecraft:cave_air", "minecraft:void_air"].contains(&feet)
            || !["minecraft:air", "minecraft:cave_air", "minecraft:void_air"].contains(&head)
        {
            continue;
        }
        let (x, z) = columns[index];
        return serde_json::json!({
            "ok": true,
            "location": {
                "dimension": dimension,
                "position": [f64::from(x) + 0.5, f64::from(*y), f64::from(z) + 0.5],
                "yaw": snapshot.yaw,
                "pitch": snapshot.pitch,
            },
            "search_scope": "loaded_chunks",
        });
    }
    serde_json::json!({"ok": false, "error": "no_safe_loaded_location", "search_scope": "loaded_chunks"})
}

#[cfg(feature = "server-plugins")]
fn load_dimension(game: &mut Game<'_>, data: &serde_json::Value) -> serde_json::Value {
    let Some(id) = data.get("dimension").and_then(serde_json::Value::as_str) else {
        return serde_json::json!({"ok": false, "error": "invalid_dimension"});
    };
    let dimension = super::Dim::from_id(id);
    if dimension.id() != id || !game.world_dimensions().contains(&dimension) {
        return serde_json::json!({"ok": false, "error": "dimension_not_found"});
    }
    let pinned = data.get("owner").and_then(serde_json::Value::as_str);
    if let Some(owner) = pinned {
        if owner.len() > 128
            || !owner.split_once(':').is_some_and(|(namespace, path)| {
                !namespace.is_empty()
                    && !path.is_empty()
                    && owner
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_:/.-".contains(&byte))
            })
        {
            return serde_json::json!({"ok": false, "error": "invalid_ticket_owner"});
        }
        let chunk = [0, 0];
        let held = game.native_force_chunk_in(owner, id, chunk, true);
        let already_held = game
            .mod_chunk_tickets
            .get(&dimension)
            .and_then(|chunks| chunks.get(&(chunk[0], chunk[1])))
            .is_some_and(|owners| owners.contains(owner));
        if !held && !already_held {
            return serde_json::json!({"ok": false, "error": "dimension_pin_failed"});
        }
    }
    let previous = game.dim;
    if dimension != previous {
        if let Err(error) = game.enter_level(None, dimension) {
            return serde_json::json!({"ok": false, "error": "dimension_load_failed", "detail": error.to_string()});
        }
        if let Err(error) = game.enter_level(None, previous) {
            return serde_json::json!({"ok": false, "error": "dimension_restore_failed", "detail": error.to_string()});
        }
    }
    serde_json::json!({"ok": true, "dimension": id, "loaded": true, "pinned": pinned.is_some()})
}

#[cfg(feature = "server-plugins")]
fn create_custom_dimension(game: &mut Game<'_>, data: &serde_json::Value) -> serde_json::Value {
    let Some(id) = data.get("id").and_then(serde_json::Value::as_str) else {
        return serde_json::json!({"ok": false, "error": "invalid_dimension_id"});
    };
    let Some(preset) = data.get("preset").and_then(serde_json::Value::as_str) else {
        return serde_json::json!({"ok": false, "error": "invalid_preset"});
    };
    if !valid_custom_dimension_id(id) {
        return serde_json::json!({"ok": false, "error": "invalid_dimension_id"});
    }
    if !matches!(preset, "overworld" | "nether" | "end" | "flat") {
        return serde_json::json!({"ok": false, "error": "invalid_preset"});
    }
    if game
        .world_dimensions()
        .iter()
        .any(|dimension| dimension.id() == id)
    {
        return serde_json::json!({"ok": false, "error": "dimension_exists"});
    }
    let path = horizon_client::save::dimension_dir(&game.world_root, id);
    if path.exists() {
        return serde_json::json!({"ok": false, "error": "dimension_data_exists"});
    }
    if let Err(error) =
        horizon_client::save::set_custom_dimension_generator(&game.world_root, id, preset)
    {
        return serde_json::json!({"ok": false, "error": "generator_registration_failed", "detail": error.to_string()});
    }
    if let Err(error) = fs::create_dir_all(path.join("region")) {
        let _ = horizon_client::save::remove_custom_dimension_generator(&game.world_root, id);
        return serde_json::json!({"ok": false, "error": "dimension_create_failed", "detail": error.to_string()});
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let nonce = DIMENSION_SEED_NONCE.fetch_add(1, Ordering::Relaxed);
    let id_hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    });
    let mut seed = now ^ nonce.rotate_left(17) ^ id_hash ^ (game.level.seed as u64);
    seed = (seed ^ (seed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    seed = (seed ^ (seed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    let seed = (seed ^ (seed >> 31)) as i64;
    if let Err(error) = horizon_client::save::set_custom_dimension_seed(&game.world_root, id, seed)
    {
        let _ = fs::remove_dir_all(&path);
        let _ = horizon_client::save::remove_custom_dimension_generator(&game.world_root, id);
        return serde_json::json!({"ok": false, "error": "dimension_seed_save_failed", "detail": error.to_string()});
    }
    let opened = load_dimension(game, &serde_json::json!({"dimension": id}));
    if opened["ok"] != true {
        let _ = fs::remove_dir_all(&path);
        let _ = horizon_client::save::remove_custom_dimension_generator(&game.world_root, id);
        return serde_json::json!({
            "ok": false,
            "error": "dimension_prepare_failed",
            "detail": opened["detail"].as_str().or_else(|| opened["error"].as_str()).unwrap_or("falha ao abrir o gerador"),
        });
    }
    let unloaded = game.native_unload_dimension(id);
    serde_json::json!({"ok": true, "dimension": id, "preset": preset, "loaded": !unloaded})
}

#[cfg(feature = "server-plugins")]
fn delete_custom_dimension(game: &mut Game<'_>, data: &serde_json::Value) -> serde_json::Value {
    let Some(id) = data.get("dimension").and_then(serde_json::Value::as_str) else {
        return serde_json::json!({"ok": false, "error": "invalid_dimension_id"});
    };
    if !valid_custom_dimension_id(id) {
        return serde_json::json!({"ok": false, "error": "protected_or_invalid_dimension"});
    }
    if game
        .native_server_players()
        .iter()
        .any(|player| player.dimension == id)
    {
        return serde_json::json!({"ok": false, "error": "players_in_dimension"});
    }
    let world_root = match game.world_root.canonicalize() {
        Ok(root) => root,
        Err(_) => return serde_json::json!({"ok": false, "error": "world_unavailable"}),
    };
    let path = horizon_client::save::dimension_dir(&world_root, id);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => return serde_json::json!({"ok": false, "error": "dimension_not_found"}),
    };
    let canonical = match path.canonicalize() {
        Ok(path) => path,
        Err(_) => return serde_json::json!({"ok": false, "error": "unsafe_dimension_path"}),
    };
    if metadata.file_type().is_symlink() || !canonical.starts_with(&world_root) {
        return serde_json::json!({"ok": false, "error": "unsafe_dimension_path"});
    }
    if !game.native_unload_dimension(id) {
        return serde_json::json!({"ok": false, "error": "dimension_busy_or_unload_failed"});
    }
    if let Err(error) = fs::remove_dir_all(&canonical) {
        return serde_json::json!({"ok": false, "error": "dimension_delete_failed", "detail": error.to_string()});
    }
    if let Err(error) = horizon_client::save::remove_custom_dimension_generator(&world_root, id) {
        return serde_json::json!({"ok": false, "error": "generator_entry_remove_failed", "detail": error.to_string()});
    }
    let _ = game.native_dimension_clear_game_rule_overrides(id);
    serde_json::json!({"ok": true, "dimension": id})
}

#[cfg(feature = "server-plugins")]
fn copy_import_files(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_import_files(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination)?;
        }
        // Never follow symlinks found in an imported archive/save.
    }
    Ok(())
}

#[cfg(feature = "server-plugins")]
fn copy_import_dimension_data(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target)?;
    for folder in ["region", "entities", "poi"] {
        let from = source.join(folder);
        if from.is_dir() {
            copy_import_files(&from, &target.join(folder))?;
        }
    }
    Ok(())
}

#[cfg(feature = "server-plugins")]
fn import_custom_dimension(game: &mut Game<'_>, data: &serde_json::Value) -> serde_json::Value {
    let Some(id) = data
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| valid_custom_dimension_id(id))
    else {
        return serde_json::json!({"ok":false,"error":"invalid_dimension_id"});
    };
    let Some(plugin) = data
        .get("plugin")
        .and_then(serde_json::Value::as_str)
        .filter(|plugin| valid_plugin_namespace(plugin))
    else {
        return serde_json::json!({"ok":false,"error":"invalid_plugin_id"});
    };
    let Some(source) = data.get("source_dir").and_then(serde_json::Value::as_str) else {
        return serde_json::json!({"ok":false,"error":"invalid_source"});
    };
    let Some(root) = std::env::current_dir().ok() else {
        return serde_json::json!({"ok":false,"error":"server_root_unavailable"});
    };
    let staging = root.join("plugins").join(plugin).join(".imports");
    let (Ok(staging), Ok(source)) = (staging.canonicalize(), Path::new(source).canonicalize())
    else {
        return serde_json::json!({"ok":false,"error":"import_source_not_found"});
    };
    if !source.starts_with(&staging)
        || horizon_client::save::level_dat_issue(&source, false).is_some()
    {
        return serde_json::json!({"ok":false,"error":"invalid_imported_world"});
    }
    let Some(source_dimension) = [
        source.join("dimensions/minecraft/overworld"),
        source.clone(),
    ]
    .into_iter()
    .find(|path| path.join("region").is_dir()) else {
        return serde_json::json!({"ok":false,"error":"overworld_region_missing"});
    };
    let destination = horizon_client::save::dimension_dir(&game.world_root, id);
    if destination.exists()
        || game
            .world_dimensions()
            .iter()
            .any(|dimension| dimension.id() == id)
    {
        return serde_json::json!({"ok":false,"error":"dimension_already_exists"});
    }
    if let Err(error) = copy_import_dimension_data(&source_dimension, &destination) {
        let _ = fs::remove_dir_all(&destination);
        return serde_json::json!({"ok":false,"error":"import_copy_failed","detail":error.to_string()});
    }
    if let Err(error) =
        horizon_client::save::import_overworld_dimension(&game.world_root, &source, id)
    {
        let _ = fs::remove_dir_all(&destination);
        return serde_json::json!({"ok":false,"error":"invalid_world_generator","detail":error.to_string()});
    }
    let opened = load_dimension(game, &serde_json::json!({"dimension":id}));
    if opened["ok"] != true {
        let _ = fs::remove_dir_all(&destination);
        let _ = horizon_client::save::remove_custom_dimension_generator(&game.world_root, id);
        return serde_json::json!({"ok":false,"error":"imported_dimension_open_failed","detail":opened["detail"].as_str().or_else(||opened["error"].as_str()).unwrap_or("failed to open imported dimension")});
    }
    let _ = game.native_unload_dimension(id);
    serde_json::json!({"ok":true,"dimension":id,"chunks_imported":true})
}

#[cfg(feature = "server-plugins")]
fn reset_targets(root: &Path, categories: &[String]) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    for category in categories {
        let paths = match category.as_str() {
            "advancements" => vec![root.join("players/advancements"), root.join("advancements")],
            "stats" => vec![root.join("players/stats"), root.join("stats")],
            "playerdata" => vec![root.join("players/data"), root.join("playerdata")],
            _ => continue,
        };
        targets.extend(paths.into_iter().filter(|path| path.exists()));
    }
    targets
}

#[cfg(feature = "server-plugins")]
fn prepare_player_data_reset(game: &Game<'_>, data: &serde_json::Value) -> serde_json::Value {
    let Some(categories) =
        reset_categories(data.get("categories").unwrap_or(&serde_json::Value::Null))
    else {
        return serde_json::json!({"ok": false, "error": "invalid_categories"});
    };
    let targets = reset_targets(&game.world_root, &categories);
    let requires_force = !game.native_server_players().is_empty()
        && categories.iter().any(|category| category == "playerdata");
    let online_record_reset = !game.native_server_players().is_empty()
        && !categories.iter().any(|category| category == "playerdata")
        && categories
            .iter()
            .any(|category| matches!(category.as_str(), "stats" | "advancements"));
    if targets.is_empty() && !online_record_reset {
        return serde_json::json!({"ok": false, "error": "data_not_found"});
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let nonce = PLAYER_DATA_RESET_NONCE.fetch_add(1, Ordering::Relaxed);
    let token = format!("{now:016x}{nonce:016x}");
    let plans = PLAYER_DATA_RESET_PLANS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut plans = plans.lock().unwrap_or_else(|error| error.into_inner());
    plans.retain(|_, plan| plan.expires > Instant::now());
    plans.insert(
        token.clone(),
        PlayerDataResetPlan {
            categories: categories.clone(),
            expires: Instant::now() + Duration::from_secs(60),
        },
    );
    let affected: Vec<_> = targets
        .iter()
        .filter_map(|path| path.strip_prefix(&game.world_root).ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    serde_json::json!({
        "ok": true,
        "token": token,
        "expires_in_seconds": 60,
        "categories": categories,
        "affected_paths": affected,
        "backup_required": !requires_force,
        "requires_force": requires_force,
    })
}

#[cfg(feature = "server-plugins")]
fn copy_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "refusing to back up a symlink",
            ));
        }
        if kind.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else if kind.is_file() {
            fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}

#[cfg(feature = "server-plugins")]
fn commit_player_data_reset(game: &mut Game<'_>, data: &serde_json::Value) -> serde_json::Value {
    let Some(token) = data.get("token").and_then(serde_json::Value::as_str) else {
        return serde_json::json!({"ok": false, "error": "missing_token"});
    };
    let plans = PLAYER_DATA_RESET_PLANS.get_or_init(|| Mutex::new(HashMap::new()));
    let plan = plans
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(token);
    let Some(plan) = plan else {
        return serde_json::json!({"ok": false, "error": "invalid_or_expired_token"});
    };
    if plan.expires <= Instant::now() {
        return serde_json::json!({"ok": false, "error": "invalid_or_expired_token"});
    }
    if !game.native_server_players().is_empty()
        && plan
            .categories
            .iter()
            .any(|category| category == "playerdata")
    {
        return serde_json::json!({"ok": false, "error": "players_online"});
    }
    let Ok(world_root) = game.world_root.canonicalize() else {
        return serde_json::json!({"ok": false, "error": "world_unavailable"});
    };
    let targets = reset_targets(&world_root, &plan.categories);
    let online_record_reset = !game.native_server_players().is_empty()
        && !plan
            .categories
            .iter()
            .any(|category| category == "playerdata")
        && plan
            .categories
            .iter()
            .any(|category| matches!(category.as_str(), "stats" | "advancements"));
    if targets.is_empty() && !online_record_reset {
        return serde_json::json!({"ok": false, "error": "data_not_found"});
    }
    for target in &targets {
        let Ok(metadata) = fs::symlink_metadata(target) else {
            return serde_json::json!({"ok": false, "error": "data_unavailable"});
        };
        let Ok(canonical) = target.canonicalize() else {
            return serde_json::json!({"ok": false, "error": "data_unavailable"});
        };
        if metadata.file_type().is_symlink() || !canonical.starts_with(&world_root) {
            return serde_json::json!({"ok": false, "error": "unsafe_data_path"});
        }
    }
    let backup_enabled = data
        .get("backup")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let backup_root = if backup_enabled {
        let Ok(server_root) = std::env::current_dir() else {
            return serde_json::json!({"ok": false, "error": "backup_unavailable"});
        };
        let backup_root = server_root
            .join(".horizon")
            .join("backups")
            .join(format!("player-data-{token}"));
        for target in &targets {
            let Ok(relative) = target.strip_prefix(&world_root) else {
                let _ = fs::remove_dir_all(&backup_root);
                return serde_json::json!({"ok": false, "error": "unsafe_data_path"});
            };
            if let Err(error) = copy_directory(target, &backup_root.join(relative)) {
                let _ = fs::remove_dir_all(&backup_root);
                return serde_json::json!({"ok": false, "error": "backup_failed", "detail": error.to_string()});
            }
        }
        if online_record_reset
            && let Err(error) =
                backup_connected_player_records(game, &plan.categories, &backup_root)
        {
            let _ = fs::remove_dir_all(&backup_root);
            return serde_json::json!({"ok": false, "error": "backup_failed", "detail": error.to_string()});
        }
        Some(backup_root)
    } else {
        None
    };
    for target in &targets {
        if let Err(error) = fs::remove_dir_all(target) {
            return serde_json::json!({
                "ok": false,
                "error": "partial_reset_failed",
                "backup": backup_root.as_ref().map(|path| path.to_string_lossy()),
                "detail": error.to_string(),
            });
        }
    }
    reset_connected_player_records(game, &plan.categories);
    serde_json::json!({
        "ok": true,
        "categories": plan.categories,
        "backup": backup_root.as_ref().map(|path| path.to_string_lossy()),
        "removed_paths": targets.iter().filter_map(|path| path.strip_prefix(&world_root).ok()).map(|path| path.to_string_lossy()).collect::<Vec<_>>(),
    })
}

#[cfg(feature = "server-plugins")]
fn backup_connected_player_records(
    game: &mut Game<'_>,
    categories: &[String],
    backup_root: &Path,
) -> std::io::Result<()> {
    let reset_advancements = categories.iter().any(|category| category == "advancements");
    let reset_stats = categories.iter().any(|category| category == "stats");
    fs::create_dir_all(backup_root)?;
    let tree = reset_advancements.then(|| game.advancement_tree());
    let Some(lan) = game.lan.as_ref() else {
        return Ok(());
    };
    for guest in lan
        .hub
        .guests
        .iter()
        .filter(|guest| guest.placed && !guest.released)
    {
        let uuid = horizon_client::lan::playerdata::uuid_ints(guest.uuid);
        if let Some(tree) = &tree {
            let path = super::stats::player_json(backup_root, "advancements", uuid);
            let json = guest
                .advancements
                .to_json(tree, horizon_world::DATA_VERSION);
            let bytes = serde_json::to_vec_pretty(&json).map_err(std::io::Error::other)?;
            horizon_client::save::write_durable(&path, &bytes)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        if reset_stats && let Some(play) = lan.play.get(&guest.conn) {
            let path = super::stats::player_json(backup_root, "stats", uuid);
            let bytes =
                serde_json::to_vec_pretty(&play.stats.to_json()).map_err(std::io::Error::other)?;
            horizon_client::save::write_durable(&path, &bytes)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(feature = "server-plugins")]
fn reset_connected_player_records(game: &mut Game<'_>, categories: &[String]) {
    let reset_advancements = categories.iter().any(|category| category == "advancements");
    let reset_stats = categories.iter().any(|category| category == "stats");
    if !reset_advancements && !reset_stats {
        return;
    }
    let advancement_count = if reset_advancements {
        game.advancement_tree().list.len()
    } else {
        0
    };
    let Some(lan) = game.lan.as_mut() else {
        return;
    };
    let connections: Vec<_> = lan
        .hub
        .guests
        .iter()
        .filter(|guest| guest.placed && !guest.released)
        .map(|guest| guest.conn)
        .collect();
    for conn in connections {
        if reset_advancements {
            if let Some(guest) = lan.hub.guest_mut(conn) {
                guest.advancements = Default::default();
                guest.advancements_loaded = true;
                guest.advancements_changed.extend(0..advancement_count);
            }
            lan.hub.flush_progress(conn);
        }
        if reset_stats && let Some(play) = lan.play.get_mut(&conn) {
            let keys: Vec<_> = play
                .stats
                .values
                .iter()
                .flat_map(|(kind, values)| {
                    values.keys().map(move |key| (kind.clone(), key.clone()))
                })
                .collect();
            for (kind, key) in keys {
                play.stats.reset(&kind, &key);
            }
            let dirty = play.stats.take_dirty();
            lan.hub.send_stats(conn, &dirty);
        }
    }
}

#[cfg(feature = "server-plugins")]
impl Host for GameHost<'_, '_, '_> {
    fn take_command_suggestions(&mut self) -> Vec<String> {
        self.suggestions.take().unwrap_or_default()
    }

    fn register_command(&mut self, usage: &str) {
        if let Some(commands) = self.commands.as_deref_mut() {
            commands.push(NativeCommand::new(usage));
        }
    }

    fn send_message(&mut self, player: u128, message: &str) -> bool {
        self.game.native_tell(player, Text::literal(message))
    }

    fn open_menu(
        &mut self,
        player: u128,
        title: &str,
        rows: u8,
        items: Vec<MenuItem>,
    ) -> Option<i32> {
        let size = usize::from(rows.clamp(1, 6)) * 9;
        let mut stacks = vec![None; size];
        for item in items {
            if item.slot >= size {
                continue;
            }
            let id = if item.item.contains(':') {
                item.item
            } else {
                format!("minecraft:{}", item.item)
            };
            let mut stack = ItemStack::new(&id, item.count.clamp(1, 64));
            if !item.name.is_empty() {
                let color = match item.color.as_str() {
                    "" => "",
                    "black" => "§0",
                    "dark_blue" => "§1",
                    "dark_green" => "§2",
                    "dark_aqua" => "§3",
                    "dark_red" => "§4",
                    "dark_purple" => "§5",
                    "gold" => "§6",
                    "gray" => "§7",
                    "dark_gray" => "§8",
                    "blue" => "§9",
                    "green" => "§a",
                    "aqua" => "§b",
                    "red" => "§c",
                    "light_purple" => "§d",
                    "yellow" => "§e",
                    "white" => "§f",
                    _ => "§f",
                };
                // Parse legacy codes as Minecraft's component serializer does:
                // a color code also explicitly clears italic/bold decorations.
                // This matters because custom item names inherit italic in tooltips.
                let name = Text::literal(format!("§r{color}{}", item.name)).resolve_legacy();
                stack.data_mut().custom_name = Some(component_nbt(&name.component()));
            }
            if !item.lore.is_empty() {
                let lore = serde_json::Value::Array(
                    item.lore
                        .iter()
                        .map(|line| Text::literal(line).resolve_legacy().component())
                        .collect(),
                );
                stack
                    .data_mut()
                    .other
                    .push(("minecraft:lore".into(), component_nbt(&lore)));
            }
            stacks[item.slot] = Some(stack);
        }
        self.game
            .native_open_menu(player, rows, Text::literal(title).resolve_legacy(), stacks)
    }

    fn transact_items(
        &mut self,
        player: u128,
        items: &[ItemDelta],
        finish: &mut dyn FnMut(bool) -> bool,
    ) -> Result<(), TransactionError> {
        let changes: Vec<_> = items
            .iter()
            .map(|item| (item.item.clone(), item.delta))
            .collect();
        self.game
            .native_plugin_item_transaction(player, &changes, finish)
            .map_err(|()| TransactionError)
    }

    fn call_json(&mut self, player: u128, request: serde_json::Value) -> serde_json::Value {
        use horizon_server_plugin_api::operation as op;

        let Some(operation) = request.get("operation").and_then(|v| v.as_str()) else {
            return serde_json::json!({"ok": false, "error": "missing_operation"});
        };
        let data = request.get("data").cloned().unwrap_or_default();
        let fail = |reason: &str| serde_json::json!({"ok": false, "error": reason});
        match operation {
            op::COMMAND_SUGGESTION_ADD => {
                let Some(value) = data.get("value").and_then(|v| v.as_str()) else {
                    return fail("invalid_suggestion");
                };
                if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                    return fail("invalid_suggestion");
                }
                if let Some(suggestions) = self.suggestions.as_mut() {
                    suggestions.push(value.to_owned());
                    serde_json::json!({"ok": true})
                } else {
                    fail("suggestions_unavailable")
                }
            }
            op::SERVER_MAINTENANCE_BEGIN => {
                let Some(lan) = self.game.lan.as_mut() else {
                    return fail("server_unavailable");
                };
                let shared = lan.hub.lan.shared.clone();
                shared.set_maintenance(true);
                let guests: Vec<_> = lan
                    .hub
                    .guests
                    .iter()
                    .filter(|guest| guest.placed && !guest.released)
                    .map(|guest| guest.conn)
                    .collect();
                let message = data
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .filter(|message| !message.is_empty() && message.len() <= 256)
                    .unwrap_or("Server maintenance in progress");
                let reason = Tag::String(horizon_world::nbt::JavaString::from(message));
                for conn in &guests {
                    lan.hub.kick(*conn, reason.clone());
                }
                serde_json::json!({"ok": true, "online": shared.online(), "kicked": guests.len()})
            }
            op::SERVER_MAINTENANCE_STATUS => {
                let Some(lan) = self.game.lan.as_ref() else {
                    return fail("server_unavailable");
                };
                let shared = &lan.hub.lan.shared;
                serde_json::json!({"ok": true, "active": shared.maintenance_active(), "online": shared.online()})
            }
            op::SERVER_MAINTENANCE_END => {
                let Some(lan) = self.game.lan.as_ref() else {
                    return fail("server_unavailable");
                };
                let shared = lan.hub.lan.shared.clone();
                let online = shared.online();
                if online != 0 {
                    return serde_json::json!({"ok": false, "error": "players_online", "online": online});
                }
                shared.set_maintenance(false);
                serde_json::json!({"ok": true, "active": false})
            }
            op::SERVER_LOG => {
                let Some(message) = data
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .filter(|message| !message.is_empty() && message.len() <= 2048)
                else {
                    return fail("invalid_message");
                };
                let level = data
                    .get("level")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("INFO");
                if !matches!(level, "INFO" | "WARN" | "ERROR") {
                    return fail("invalid_level");
                }
                self.game.native_log(level, message);
                serde_json::json!({"ok": true})
            }
            op::SCHEDULER_SCHEDULE => {
                let Some(plugin) = data
                    .get("plugin")
                    .and_then(serde_json::Value::as_str)
                    .filter(|plugin| {
                        !plugin.is_empty()
                            && plugin.len() <= 64
                            && plugin
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
                    })
                else {
                    return fail("invalid_plugin");
                };
                let Some(ticks) = data.get("delay_ticks").and_then(serde_json::Value::as_u64)
                else {
                    return fail("invalid_delay");
                };
                if ticks > 120_000 {
                    return fail("delay_too_large");
                }
                let period_ticks = match data.get("period_ticks") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(value) => match value
                        .as_u64()
                        .filter(|period| (1..=120_000).contains(period))
                    {
                        Some(period) => Some(period),
                        None => return fail("invalid_period"),
                    },
                };
                let payload = data.get("data").cloned().unwrap_or(serde_json::Value::Null);
                if payload.to_string().len() > 16 * 1024 {
                    return fail("task_data_too_large");
                }
                let id = NEXT_SCHEDULED_PLUGIN_TASK.fetch_add(1, Ordering::Relaxed);
                let tasks = SCHEDULED_PLUGIN_TASKS.get_or_init(|| Mutex::new(Vec::new()));
                let mut tasks = tasks.lock().unwrap_or_else(|error| error.into_inner());
                if tasks.len() >= 4096 {
                    return fail("too_many_tasks");
                }
                tasks.push(ScheduledPluginTask {
                    id,
                    plugin: plugin.to_owned(),
                    due_tick: SERVER_PLUGIN_TICK
                        .load(Ordering::Acquire)
                        .saturating_add(ticks),
                    period_ticks,
                    data: payload,
                });
                serde_json::json!({"ok": true, "task_id": id.to_string()})
            }
            op::SCHEDULER_CANCEL => {
                let Some(id) = data.get("task_id").and_then(|value| {
                    value
                        .as_str()
                        .and_then(|id| id.parse::<u64>().ok())
                        .or_else(|| value.as_u64())
                }) else {
                    return fail("invalid_task_id");
                };
                let tasks = SCHEDULED_PLUGIN_TASKS.get_or_init(|| Mutex::new(Vec::new()));
                let mut tasks = tasks.lock().unwrap_or_else(|error| error.into_inner());
                let before = tasks.len();
                tasks.retain(|task| task.id != id);
                serde_json::json!({"ok": true, "cancelled": tasks.len() != before})
            }
            op::COMMAND_REGISTER => {
                let Some(usage) = data.get("usage").and_then(|v| v.as_str()) else {
                    return fail("invalid_usage");
                };
                let level = data
                    .get("default_level")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    .min(4) as u8;
                let mut command = NativeCommand::op(usage, level);
                if let Some(permission) = data.get("permission").and_then(|v| v.as_str()) {
                    if permission.is_empty() || permission.len() > 128 {
                        return fail("invalid_permission");
                    }
                    command = command.permission(permission);
                }
                if let Some(commands) = self.commands.as_deref_mut() {
                    commands.push(command);
                    serde_json::json!({"ok": true})
                } else {
                    fail("commands_can_only_be_registered_during_startup")
                }
            }
            op::PLAYER_COMMAND_EXECUTE => {
                let Some(uuid) = data
                    .get("uuid")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| u128::from_str_radix(value, 16).ok())
                else {
                    return fail("invalid_player_uuid");
                };
                let Some(command) = data
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .filter(|v| !v.trim().is_empty() && v.len() <= 2048)
                else {
                    return fail("invalid_command");
                };
                let result = self.game.native_run_command_as(uuid, command);
                serde_json::json!({
                    "ok": result.ran,
                    "result": result.result,
                    "output": result.output.iter().map(Text::component).collect::<Vec<_>>()
                })
            }
            op::CHAT_SEND => {
                let Some(component) = data.get("component") else {
                    return fail("missing_component");
                };
                let text = horizon_client::packs::component(component);
                serde_json::json!({"ok": self.game.native_tell(player, text)})
            }
            op::MENU_OPEN => {
                let rows = data
                    .get("rows")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3)
                    .clamp(1, 6) as u8;
                let Some(items_json) = data.get("items").and_then(|v| v.as_array()) else {
                    return fail("invalid_items");
                };
                if items_json.len() > 54 {
                    return fail("too_many_items");
                }
                let title = data
                    .get("title")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"text": ""}));
                let title = if let Some(literal) = title.as_str() {
                    Text::literal(literal).resolve_legacy()
                } else {
                    horizon_client::packs::component(&title)
                };
                let size = usize::from(rows) * 9;
                let mut stacks = vec![None; size];
                for entry in items_json {
                    let Some(slot) = entry.get("slot").and_then(|v| v.as_u64()) else {
                        continue;
                    };
                    if slot >= size as u64 {
                        continue;
                    }
                    let Some(id) = entry.get("item").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if id.len() > 256 {
                        continue;
                    }
                    let id = if id.contains(':') {
                        id.to_owned()
                    } else {
                        format!("minecraft:{id}")
                    };
                    let count = entry
                        .get("count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1)
                        .clamp(1, 64) as u32;
                    let mut stack = ItemStack::new(&id, count);
                    if let Some(name) = entry.get("name") {
                        let name = if let Some(literal) = name.as_str() {
                            Text::literal(literal).resolve_legacy().component()
                        } else {
                            name.clone()
                        };
                        stack.data_mut().custom_name = Some(component_nbt(&name));
                    }
                    if let Some(lore) = entry.get("lore").and_then(|v| v.as_array()) {
                        let lore: Vec<_> = lore
                            .iter()
                            .map(|line| {
                                if let Some(literal) = line.as_str() {
                                    Text::literal(literal).resolve_legacy().component()
                                } else {
                                    line.clone()
                                }
                            })
                            .collect();
                        stack.data_mut().other.push((
                            "minecraft:lore".into(),
                            component_nbt(&serde_json::Value::Array(lore)),
                        ));
                    }
                    if let Some(components) = entry.get("components").and_then(|v| v.as_object()) {
                        for (component, value) in components {
                            if component.len() > 128
                                || !component.contains(':')
                                || !component.bytes().all(|byte| {
                                    byte.is_ascii_lowercase()
                                        || byte.is_ascii_digit()
                                        || b"_:/.-".contains(&byte)
                                })
                            {
                                continue;
                            }
                            stack.data_mut().set_loot_component(component, value);
                        }
                    }
                    stacks[slot as usize] = Some(stack);
                }
                let window = self.game.native_open_menu(player, rows, title, stacks);
                serde_json::json!({"ok": window.is_some(), "window_id": window})
            }
            op::INVENTORY_VIEW_OPEN => {
                let Some(target) = data
                    .get("target")
                    .and_then(|value| value.as_str())
                    .and_then(|value| {
                        u128::from_str_radix(value.trim_start_matches("0x"), 16).ok()
                    })
                else {
                    return fail("invalid_target");
                };
                let Some(kind) = data.get("kind").and_then(|value| value.as_str()) else {
                    return fail("invalid_inventory_kind");
                };
                let title = data
                    .get("title")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Inventário");
                let rows = data
                    .get("rows")
                    .and_then(|value| value.as_u64())
                    .and_then(|value| u8::try_from(value).ok());
                let layout = if let Some(value) = data.get("layout") {
                    let Some(slots) = value.as_array() else {
                        return fail("invalid_inventory_layout");
                    };
                    let Some(layout) = slots
                        .iter()
                        .map(|slot| {
                            if slot.is_null() {
                                Some(None)
                            } else {
                                slot.as_u64()
                                    .and_then(|slot| usize::try_from(slot).ok())
                                    .map(Some)
                            }
                        })
                        .collect::<Option<Vec<_>>>()
                    else {
                        return fail("invalid_inventory_layout");
                    };
                    Some(layout)
                } else {
                    None
                };
                let opened = self
                    .game
                    .native_open_inventory_view(player, target, kind, title, rows, layout);
                serde_json::json!({"ok": opened.is_some(), "window": opened})
            }
            op::RESOURCE_PACK_SEND | op::RESOURCE_PACK_SEND_BATCH => {
                let packs: Vec<&serde_json::Value> = if operation == op::RESOURCE_PACK_SEND {
                    vec![&data]
                } else {
                    let Some(packs) = data.get("packs").and_then(|value| value.as_array()) else {
                        return fail("invalid_packs");
                    };
                    if packs.is_empty() || packs.len() > 8 {
                        return fail("invalid_pack_count");
                    }
                    packs.iter().collect()
                };
                let mut sent = 0;
                for pack in packs {
                    let Some(id) = pack
                        .get("id")
                        .and_then(|value| value.as_str())
                        .and_then(|value| u128::from_str_radix(&value.replace('-', ""), 16).ok())
                    else {
                        return serde_json::json!({"ok": false, "sent": sent, "error": "invalid_pack_id"});
                    };
                    let Some(url) = pack.get("url").and_then(|value| value.as_str()) else {
                        return serde_json::json!({"ok": false, "sent": sent, "error": "invalid_url"});
                    };
                    let sha1 = pack
                        .get("sha1")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let prompt = pack.get("prompt").map(horizon_client::packs::component);
                    let required = pack
                        .get("required")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    if !self.game.native_send_resource_pack(
                        player,
                        id,
                        url,
                        sha1,
                        prompt.as_ref(),
                        required,
                    ) {
                        return serde_json::json!({"ok": false, "sent": sent, "error": "pack_send_failed"});
                    }
                    sent += 1;
                }
                serde_json::json!({"ok": true, "sent": sent})
            }
            op::RESOURCE_PACK_REMOVE => {
                let id = data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .and_then(|v| u128::from_str_radix(v, 16).ok());
                serde_json::json!({"ok": self.game.native_pop_resource_pack(player, id)})
            }
            op::PLAYER_GET => {
                let snapshot = self
                    .requester
                    .as_ref()
                    .filter(|snapshot| snapshot.uuid == player)
                    .cloned()
                    .or_else(|| {
                        self.game
                            .native_server_players()
                            .into_iter()
                            .find(|snapshot| snapshot.uuid == player)
                    })
                    .or_else(|| {
                        self.game
                            .native_acting_player()
                            .filter(|snapshot| snapshot.uuid == player)
                    });
                let Some(snapshot) = snapshot else {
                    return fail("player_not_online");
                };
                serde_json::json!({
                    "ok": true,
                    "uuid": format!("{:032x}", snapshot.uuid),
                    "name": snapshot.name,
                    "dimension": snapshot.dimension,
                    "position": snapshot.position,
                    "yaw": snapshot.yaw,
                    "pitch": snapshot.pitch,
                    "game_mode": snapshot.game_mode,
                    "permission_level": snapshot.permission,
                    "health": snapshot.health,
                })
            }
            op::PLAYER_LOOKUP | op::PLAYER_ONLINE_LIST => {
                let players = self.game.native_server_players();
                let result: Vec<_> = if operation == op::PLAYER_LOOKUP {
                    let by_uuid =
                        data.get("uuid")
                            .and_then(|value| value.as_str())
                            .and_then(|value| {
                                u128::from_str_radix(value.trim_start_matches("0x"), 16).ok()
                            });
                    let by_name = data
                        .get("name")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned);
                    let Some(snapshot) = players.into_iter().find(|snapshot| {
                        by_uuid.is_some_and(|uuid| snapshot.uuid == uuid)
                            || by_name
                                .as_ref()
                                .is_some_and(|name| snapshot.name.eq_ignore_ascii_case(name))
                    }) else {
                        return fail("player_not_online");
                    };
                    vec![snapshot]
                } else {
                    players
                };
                serde_json::json!({
                    "ok": true,
                    "players": result.into_iter().map(|snapshot| serde_json::json!({
                        "uuid": format!("{:032x}", snapshot.uuid),
                        "name": snapshot.name,
                        "dimension": snapshot.dimension,
                        "position": snapshot.position,
                        "yaw": snapshot.yaw,
                        "pitch": snapshot.pitch,
                        "game_mode": snapshot.game_mode,
                        "permission_level": snapshot.permission,
                        "health": snapshot.health,
                    })).collect::<Vec<_>>()
                })
            }
            op::PLAYER_STAT_GET => {
                let Some(stat_type) = data.get("type").and_then(serde_json::Value::as_str) else {
                    return fail("invalid_stat_type");
                };
                let Some(key) = data.get("key").and_then(serde_json::Value::as_str) else {
                    return fail("invalid_stat_key");
                };
                if stat_type.len() > 128 || key.len() > 128 {
                    return fail("stat_id_too_long");
                }
                match self.game.native_player_stat(player, stat_type, key) {
                    Some(value) => serde_json::json!({"ok": true, "value": value}),
                    None => fail("player_not_online"),
                }
            }
            op::PLAYER_STATS_GET => match self.game.native_player_stats(player) {
                Some(stats) => serde_json::json!({"ok": true, "stats": stats}),
                None => fail("player_not_online"),
            },
            op::PLAYER_TELEPORT => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(position) = data.get("position").and_then(|v| v.as_array()) else {
                    return fail("invalid_position");
                };
                if position.len() != 3 {
                    return fail("invalid_position");
                }
                let Some(position) = position
                    .iter()
                    .map(serde_json::Value::as_f64)
                    .collect::<Option<Vec<_>>>()
                    .and_then(|p| <[f64; 3]>::try_from(p).ok())
                else {
                    return fail("invalid_position");
                };
                let yaw = data.get("yaw").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let pitch = data.get("pitch").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let target = match data.get("target") {
                    None => player,
                    Some(value) => {
                        let Some(target) = value.as_str().and_then(|value| {
                            u128::from_str_radix(value.trim_start_matches("0x"), 16).ok()
                        }) else {
                            return fail("invalid_target");
                        };
                        target
                    }
                };
                if !self
                    .game
                    .native_server_players()
                    .iter()
                    .any(|snapshot| snapshot.uuid == target)
                    && !self
                        .game
                        .native_acting_player()
                        .is_some_and(|snapshot| snapshot.uuid == target)
                {
                    return fail("target_not_online");
                }
                serde_json::json!({
                    "ok": self.game.native_teleport_player(target, dimension, position, yaw, pitch),
                    "target": format!("{target:032x}")
                })
            }
            op::PLAYER_GAME_MODE_SET => {
                let Some(mode) = data.get("mode").and_then(|v| v.as_i64()) else {
                    return fail("invalid_game_mode");
                };
                serde_json::json!({
                    "ok": self.game.native_set_game_mode(player, mode as i32)
                })
            }
            op::WORLD_LOCATION_FIND_SAFE => find_safe_location(self.game, player, &data),
            op::WORLD_PLAYER_DATA_RESET_PREPARE => prepare_player_data_reset(self.game, &data),
            op::WORLD_PLAYER_DATA_RESET_COMMIT => commit_player_data_reset(self.game, &data),
            op::WORLD_DIMENSIONS_LIST => {
                let dimensions: Vec<_> = self
                    .game
                    .world_dimensions()
                    .into_iter()
                    .map(|dimension| dimension.id())
                    .collect();
                let mut loaded = vec![self.game.dim.id()];
                loaded.extend(
                    self.game
                        .levels
                        .parked_dims()
                        .into_iter()
                        .map(|dim| dim.id()),
                );
                loaded.sort_unstable();
                loaded.dedup();
                serde_json::json!({"ok": true, "dimensions": dimensions, "loaded_dimensions": loaded})
            }
            op::WORLD_GAMERULE_GET => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(rule) = data.get("rule").and_then(|v| v.as_str()) else {
                    return fail("invalid_rule");
                };
                let value = self
                    .game
                    .native_level(dimension)
                    .and_then(|level| self.game.native_level_game_rule(level, rule));
                match value {
                    Some(GameRuleValue::Bool(value)) => {
                        serde_json::json!({"ok": true, "value": value})
                    }
                    Some(GameRuleValue::Int(value)) => {
                        serde_json::json!({"ok": true, "value": value})
                    }
                    None => fail("unknown_or_unloaded_rule"),
                }
            }
            op::WORLD_GAMERULE_SET => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(rule) = data.get("rule").and_then(|v| v.as_str()) else {
                    return fail("invalid_rule");
                };
                let Some(value) = game_rule_value(data.get("value")) else {
                    return fail("invalid_rule_value");
                };
                let changed = self
                    .game
                    .native_level(dimension)
                    .is_some_and(|level| self.game.native_level_set_game_rule(level, rule, value));
                serde_json::json!({"ok": changed})
            }
            op::WORLD_CUSTOM_GAMERULE_GET => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(rule) = data.get("rule").and_then(|v| v.as_str()) else {
                    return fail("invalid_rule");
                };
                match self.game.native_dimension_game_rule(dimension, rule) {
                    Some(GameRuleValue::Bool(value)) => {
                        serde_json::json!({"ok": true, "value": value})
                    }
                    Some(GameRuleValue::Int(value)) => {
                        serde_json::json!({"ok": true, "value": value})
                    }
                    None => fail("unknown_custom_dimension_or_rule"),
                }
            }
            op::WORLD_CUSTOM_GAMERULE_SET => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(rule) = data.get("rule").and_then(|v| v.as_str()) else {
                    return fail("invalid_rule");
                };
                let Some(value) = game_rule_value(data.get("value")) else {
                    return fail("invalid_rule_value");
                };
                serde_json::json!({
                    "ok": self.game.native_dimension_set_game_rule(dimension, rule, Some(value))
                })
            }
            op::WORLD_CUSTOM_GAMERULE_CLEAR => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(rule) = data.get("rule").and_then(|v| v.as_str()) else {
                    return fail("invalid_rule");
                };
                serde_json::json!({
                    "ok": self.game.native_dimension_set_game_rule(dimension, rule, None)
                })
            }
            op::WORLD_DIMENSION_UNLOAD => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                if let Some(owner) = data.get("owner").and_then(serde_json::Value::as_str) {
                    let chunk = [0, 0];
                    let _ = self
                        .game
                        .native_force_chunk_in(owner, dimension, chunk, false);
                }
                serde_json::json!({"ok": self.game.native_unload_dimension(dimension)})
            }
            op::WORLD_DIMENSION_LOAD => load_dimension(self.game, &data),
            op::WORLD_DIMENSION_CREATE => create_custom_dimension(self.game, &data),
            op::WORLD_DIMENSION_IMPORT => import_custom_dimension(self.game, &data),
            op::WORLD_DIMENSION_DELETE => delete_custom_dimension(self.game, &data),
            op::SCOREBOARD_SIDEBAR_SET => {
                let Some(title) = data.get("title") else {
                    return fail("missing_title");
                };
                let Some(lines) = data.get("lines").and_then(|value| value.as_array()) else {
                    return fail("invalid_lines");
                };
                if lines.len() > 15 {
                    return fail("too_many_lines");
                }
                let title = if let Some(text) = title.as_str() {
                    Text::literal(text).resolve_legacy()
                } else {
                    horizon_client::packs::component(title)
                };
                let mut sidebar = NativeSidebar {
                    title,
                    lines: Vec::with_capacity(lines.len()),
                };
                for line in lines {
                    let Some(left) = line.get("left") else {
                        return fail("invalid_line");
                    };
                    let left = if let Some(text) = left.as_str() {
                        Text::literal(text).resolve_legacy()
                    } else {
                        horizon_client::packs::component(left)
                    };
                    let right = line
                        .get("right")
                        .filter(|value| !value.is_null())
                        .map(|value| {
                            if let Some(text) = value.as_str() {
                                Text::literal(text).resolve_legacy()
                            } else {
                                horizon_client::packs::component(value)
                            }
                        });
                    sidebar.lines.push((left, right));
                }
                serde_json::json!({"ok": self.game.native_sidebar(player, Some(&sidebar))})
            }
            op::SCOREBOARD_SIDEBAR_CLEAR => {
                serde_json::json!({"ok": self.game.native_sidebar(player, None)})
            }
            op::PLAYER_DISPLAY_SET => {
                let Some(uuid) = data
                    .get("uuid")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| u128::from_str_radix(value, 16).ok())
                else {
                    return fail("invalid_player_uuid");
                };
                let parse_name = |key: &str| -> Result<Option<Text>, ()> {
                    let Some(value) = data.get(key) else {
                        return Ok(None);
                    };
                    if value.is_null() {
                        return Ok(None);
                    }
                    if let Some(value) = value.as_str() {
                        return Ok(Some(Text::literal(value).resolve_legacy()));
                    }
                    if value.is_object() {
                        return Ok(Some(horizon_client::packs::component(value)));
                    }
                    Err(())
                };
                let Ok(chat) = parse_name("chat") else {
                    return fail("invalid_chat_component");
                };
                let Ok(tab) = parse_name("tab") else {
                    return fail("invalid_tab_component");
                };
                let Ok(nametag_prefix) = parse_name("nametag_prefix") else {
                    return fail("invalid_nametag_prefix_component");
                };
                let Ok(nametag_suffix) = parse_name("nametag_suffix") else {
                    return fail("invalid_nametag_suffix_component");
                };
                let chat_ok = self.game.native_set_display_name(uuid, chat.as_ref());
                let tab_ok = self.game.native_set_list_name(uuid, tab.as_ref());
                let nametag_ok = if data.get("nametag_prefix").is_some()
                    || data.get("nametag_suffix").is_some()
                {
                    self.game.native_set_name_tag_style(
                        uuid,
                        nametag_prefix.as_ref(),
                        nametag_suffix.as_ref(),
                    )
                } else {
                    true
                };
                serde_json::json!({"ok": chat_ok && tab_ok && nametag_ok, "chat": chat_ok, "tab": tab_ok, "nametag": nametag_ok})
            }
            op::ENTITY_TEXT_DISPLAY_CREATE => {
                let Some(dimension) = data.get("dimension").and_then(|value| value.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(position) = json_position(data.get("position")) else {
                    return fail("invalid_position");
                };
                let Some(text) = data.get("text") else {
                    return fail("missing_text");
                };
                let component = if let Some(text) = text.as_str() {
                    Text::literal(text).resolve_legacy().component()
                } else {
                    text.clone()
                };
                let line_width = data
                    .get("line_width")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(200)
                    .clamp(1, 2000) as i32;
                let background = data
                    .get("background")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0x4000_0000)
                    .min(u64::from(u32::MAX)) as u32;
                let view_range = match plugin_display_view_range(&data) {
                    Some(value) => value,
                    None => return fail("invalid_view_range"),
                };
                let mut nbt = Compound::new();
                nbt.insert("text".into(), component_nbt(&component));
                nbt.insert("line_width".into(), Tag::Int(line_width));
                nbt.insert("background".into(), Tag::Int(background as i32));
                nbt.insert("view_range".into(), Tag::Float(view_range.to_bits()));
                nbt.insert("text_opacity".into(), Tag::Byte(-1));
                nbt.insert(
                    "shadow".into(),
                    Tag::Byte(i8::from(
                        data.get("shadow")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false),
                    )),
                );
                nbt.insert(
                    "see_through".into(),
                    Tag::Byte(i8::from(
                        data.get("see_through")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false),
                    )),
                );
                let alignment = match data
                    .get("alignment")
                    .and_then(|value| value.as_str())
                    .unwrap_or("center")
                {
                    value @ ("left" | "center" | "right") => value,
                    _ => return fail("invalid_alignment"),
                };
                nbt.insert("alignment".into(), Tag::String(alignment.into()));
                let billboard = match data
                    .get("billboard")
                    .and_then(|value| value.as_str())
                    .unwrap_or("center")
                {
                    value @ ("fixed" | "vertical" | "horizontal" | "center") => value,
                    _ => return fail("invalid_billboard"),
                };
                nbt.insert("billboard".into(), Tag::String(billboard.into()));
                match self
                    .game
                    .native_summon(dimension, "minecraft:text_display", position, nbt)
                {
                    Some(entity_id) => serde_json::json!({"ok": true, "entity_id": entity_id}),
                    None => fail("entity_spawn_failed"),
                }
            }
            op::ENTITY_SPAWN => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(kind) = data
                    .get("entity_type")
                    .and_then(|v| v.as_str())
                    .filter(|v| {
                        v.len() <= 128
                            && v.split_once(':').is_some_and(|(namespace, path)| {
                                !namespace.is_empty() && !path.is_empty()
                            })
                            && v.bytes().all(|b| {
                                b.is_ascii_lowercase()
                                    || b.is_ascii_digit()
                                    || b"_:/. -".contains(&b)
                            })
                            && !v.contains(' ')
                    })
                else {
                    return fail("invalid_entity_type");
                };
                let Some(uuid) = data
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .and_then(|v| u128::from_str_radix(v, 16).ok())
                else {
                    return fail("invalid_uuid");
                };
                let Some(position) = json_position(data.get("position")) else {
                    return fail("invalid_position");
                };
                let mut nbt = Compound::new();
                for (key, field) in [
                    ("NoAI", "no_ai"),
                    ("Invulnerable", "invulnerable"),
                    ("Silent", "silent"),
                    ("PersistenceRequired", "persistent"),
                    ("CustomNameVisible", "name_visible"),
                ] {
                    if let Some(enabled) = data.get(field).and_then(|v| v.as_bool()) {
                        nbt.insert(key.into(), Tag::Byte(i8::from(enabled)));
                    }
                }
                // Generic clickable hitboxes for plugin-created NPCs. This is
                // vanilla `minecraft:interaction` entity data, exposed through
                // the generic entity.spawn operation rather than NPC-specific code.
                if kind == "minecraft:interaction" {
                    for (key, field) in [
                        ("width", "interaction_width"),
                        ("height", "interaction_height"),
                    ] {
                        if let Some(size) = data.get(field).and_then(|value| value.as_f64())
                            && size.is_finite()
                            && (0.0..=64.0).contains(&size)
                        {
                            nbt.insert(key.into(), Tag::Float((size as f32).to_bits()));
                        }
                    }
                    if let Some(response) = data
                        .get("interaction_response")
                        .and_then(|value| value.as_bool())
                    {
                        nbt.insert("response".into(), Tag::Byte(i8::from(response)));
                    }
                }
                if let Some(name) = data.get("custom_name").and_then(|v| v.as_str()) {
                    if name.len() > 256 {
                        return fail("custom_name_too_long");
                    }
                    let component = Text::literal(name).resolve_legacy().component();
                    nbt.insert("CustomName".into(), component_nbt(&component));
                }
                match self
                    .game
                    .native_summon_with_uuid(dimension, kind, position, nbt, uuid)
                {
                    Some(entity_id) => {
                        serde_json::json!({"ok": true, "uuid": format!("{uuid:032x}"), "entity_id": entity_id})
                    }
                    None => fail("entity_spawn_failed"),
                }
            }
            op::ENTITY_DISPLAY_UPSERT => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(uuid) = data
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .and_then(|v| u128::from_str_radix(v, 16).ok())
                else {
                    return fail("invalid_uuid");
                };
                let Some(position) = json_position(data.get("position")) else {
                    return fail("invalid_position");
                };
                let Some(text) = data.get("text") else {
                    return fail("missing_text");
                };
                let component = if let Some(text) = text.as_str() {
                    Text::literal(text).resolve_legacy().component()
                } else if text.is_object() {
                    text.clone()
                } else {
                    return fail("invalid_text");
                };
                let mut nbt = Compound::new();
                nbt.insert("text".into(), component_nbt(&component));
                nbt.insert(
                    "line_width".into(),
                    Tag::Int(
                        data.get("line_width")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(200)
                            .clamp(1, 2000) as i32,
                    ),
                );
                nbt.insert(
                    "background".into(),
                    Tag::Int(
                        data.get("background")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0x4000_0000)
                            .min(u64::from(u32::MAX)) as u32 as i32,
                    ),
                );
                let view_range = match plugin_display_view_range(&data) {
                    Some(value) => value,
                    None => return fail("invalid_view_range"),
                };
                nbt.insert("view_range".into(), Tag::Float(view_range.to_bits()));
                nbt.insert("text_opacity".into(), Tag::Byte(-1));
                for (key, field) in [("shadow", "shadow"), ("see_through", "see_through")] {
                    nbt.insert(
                        key.into(),
                        Tag::Byte(i8::from(
                            data.get(field).and_then(|v| v.as_bool()).unwrap_or(false),
                        )),
                    );
                }
                let alignment = match data
                    .get("alignment")
                    .and_then(|v| v.as_str())
                    .unwrap_or("center")
                {
                    value @ ("left" | "center" | "right") => value,
                    _ => return fail("invalid_alignment"),
                };
                let billboard = match data
                    .get("billboard")
                    .and_then(|v| v.as_str())
                    .unwrap_or("center")
                {
                    value @ ("fixed" | "vertical" | "horizontal" | "center") => value,
                    _ => return fail("invalid_billboard"),
                };
                nbt.insert("alignment".into(), Tag::String(alignment.into()));
                nbt.insert("billboard".into(), Tag::String(billboard.into()));
                if let Some((old_dimension, old_id, old_kind)) =
                    self.game.native_entity_by_uuid(uuid)
                {
                    if old_kind != "minecraft:text_display" {
                        return fail("uuid_is_not_text_display");
                    }
                    if !self.game.native_remove_entity(&old_dimension, old_id) {
                        return fail("old_display_remove_failed");
                    }
                }
                match self.game.native_summon_with_uuid(
                    dimension,
                    "minecraft:text_display",
                    position,
                    nbt,
                    uuid,
                ) {
                    Some(entity_id) => {
                        serde_json::json!({"ok": true, "uuid": format!("{uuid:032x}"), "entity_id": entity_id})
                    }
                    None => fail("entity_spawn_failed"),
                }
            }
            op::ENTITY_DISPLAY_DELETE => {
                let Some(uuid) = data
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .and_then(|v| u128::from_str_radix(v, 16).ok())
                else {
                    return fail("invalid_uuid");
                };
                let Some((dimension, id, kind)) = self.game.native_entity_by_uuid(uuid) else {
                    return fail("entity_not_found");
                };
                if kind != "minecraft:text_display" {
                    return fail("uuid_is_not_text_display");
                }
                serde_json::json!({"ok": self.game.native_remove_entity(&dimension, id)})
            }
            op::ENTITY_ITEM_DISPLAY_UPSERT => {
                let Some(dimension) = data.get("dimension").and_then(|v| v.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(uuid) = data
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .and_then(|v| u128::from_str_radix(v, 16).ok())
                else {
                    return fail("invalid_uuid");
                };
                let Some(position) = json_position(data.get("position")) else {
                    return fail("invalid_position");
                };
                let Some(item_model) = data
                    .get("item_model")
                    .and_then(|v| v.as_str())
                    .filter(|v| valid_plugin_identifier(v))
                else {
                    return fail("invalid_item_model");
                };
                let vector = |key: &str, len: usize, default: &[f32]| -> Option<Vec<f32>> {
                    let Some(value) = data.get(key) else {
                        return Some(default.to_vec());
                    };
                    let values = value.as_array()?;
                    if values.len() != len {
                        return None;
                    }
                    values
                        .iter()
                        .map(|v| {
                            v.as_f64()
                                .filter(|n| n.is_finite() && n.abs() <= 1.0e6)
                                .map(|n| n as f32)
                        })
                        .collect()
                };
                let Some(translation) = vector("translation", 3, &[0.0; 3]) else {
                    return fail("invalid_translation");
                };
                let Some(rotation) = vector("rotation", 4, &[0.0, 0.0, 0.0, 1.0]) else {
                    return fail("invalid_rotation");
                };
                let Some(scale) = vector("scale", 3, &[1.0; 3]) else {
                    return fail("invalid_scale");
                };
                let norm = rotation.iter().map(|n| n * n).sum::<f32>().sqrt();
                if norm < 1.0e-6 {
                    return fail("invalid_rotation");
                }
                let floats = |v: &[f32]| {
                    Tag::List(
                        v.iter()
                            .map(|n| Tag::Float(if *n == 0.0 { 0 } else { n.to_bits() }))
                            .collect(),
                    )
                };
                let mut transformation = Compound::new();
                transformation.insert("translation".into(), floats(&translation));
                transformation.insert(
                    "left_rotation".into(),
                    floats(&rotation.iter().map(|n| n / norm).collect::<Vec<_>>()),
                );
                transformation.insert("scale".into(), floats(&scale));
                transformation.insert("right_rotation".into(), floats(&[0.0, 0.0, 0.0, 1.0]));
                let mut stack = ItemStack::new("minecraft:paper", 1);
                stack.data_mut().other.push((
                    "minecraft:item_model".into(),
                    Tag::String(item_model.into()),
                ));
                let mut nbt = Compound::new();
                nbt.insert("item".into(), horizon_items::item_nbt::item_to_nbt(&stack));
                nbt.insert("item_display".into(), Tag::String("fixed".into()));
                nbt.insert("transformation".into(), Tag::Compound(transformation));
                nbt.insert("billboard".into(), Tag::Byte(0));
                nbt.insert("view_range".into(), Tag::Float(0.8f32.to_bits()));
                nbt.insert("shadow_radius".into(), Tag::Float(0));
                nbt.insert("shadow_strength".into(), Tag::Float(0));
                if let Some((old_dimension, old_id, old_kind)) =
                    self.game.native_entity_by_uuid(uuid)
                {
                    if old_kind != "minecraft:item_display" {
                        return fail("uuid_is_not_item_display");
                    }
                    if !self.game.native_remove_entity(&old_dimension, old_id) {
                        return fail("old_display_remove_failed");
                    }
                }
                match self.game.native_summon_with_uuid(
                    dimension,
                    "minecraft:item_display",
                    position,
                    nbt,
                    uuid,
                ) {
                    Some(entity_id) => {
                        serde_json::json!({"ok": true, "uuid": format!("{uuid:032x}"), "entity_id": entity_id})
                    }
                    None => fail("entity_spawn_failed"),
                }
            }
            op::ENTITY_REMOVE_UUID => {
                let Some(uuid) = data
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .and_then(|v| u128::from_str_radix(v, 16).ok())
                else {
                    return fail("invalid_uuid");
                };
                let Some((dimension, id, kind)) = self.game.native_entity_by_uuid(uuid) else {
                    return fail("entity_not_found");
                };
                if kind == "minecraft:player" {
                    return fail("cannot_remove_player");
                }
                serde_json::json!({"ok": self.game.native_remove_entity(&dimension, id), "entity_type": kind})
            }
            op::ENTITY_MOVE => {
                let Some(dimension) = data.get("dimension").and_then(|value| value.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(entity_id) = data
                    .get("entity_id")
                    .and_then(|value| value.as_u64())
                    .and_then(|id| u32::try_from(id).ok())
                else {
                    return fail("invalid_entity_id");
                };
                let Some(position) = json_position(data.get("position")) else {
                    return fail("invalid_position");
                };
                let yaw = data
                    .get("yaw")
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0) as f32;
                let pitch = data
                    .get("pitch")
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0) as f32;
                serde_json::json!({"ok": self.game.native_move_entity(dimension, entity_id, position, yaw, pitch)})
            }
            op::ENTITY_REMOVE => {
                let Some(dimension) = data.get("dimension").and_then(|value| value.as_str()) else {
                    return fail("invalid_dimension");
                };
                let Some(entity_id) = data
                    .get("entity_id")
                    .and_then(|value| value.as_u64())
                    .and_then(|id| u32::try_from(id).ok())
                else {
                    return fail("invalid_entity_id");
                };
                serde_json::json!({"ok": self.game.native_remove_entity(dimension, entity_id)})
            }
            op::PLUGIN_STORAGE_GET
            | op::PLUGIN_STORAGE_SET
            | op::PLUGIN_STORAGE_DELETE
            | op::PLUGIN_STORAGE_LIST => plugin_storage(operation, &data),
            _ => fail("unsupported_operation"),
        }
    }
}

#[cfg(feature = "server-plugins")]
fn plugin_storage(operation: &str, data: &serde_json::Value) -> serde_json::Value {
    use horizon_server_plugin_api::operation as op;
    use std::{fs, path::PathBuf};

    let Some(namespace) = data.get("plugin").and_then(|value| value.as_str()) else {
        return serde_json::json!({"ok": false, "error": "invalid_plugin_id"});
    };
    if !valid_plugin_namespace(namespace) {
        return serde_json::json!({"ok": false, "error": "invalid_plugin_id"});
    }
    let key = data.get("key").and_then(|value| value.as_str());
    if operation != op::PLUGIN_STORAGE_LIST && key.is_none() {
        return serde_json::json!({"ok": false, "error": "invalid_key"});
    }
    if key.is_some_and(|key| {
        key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
    }) {
        return serde_json::json!({"ok": false, "error": "invalid_key"});
    }
    let Ok(root) = std::env::current_dir() else {
        return serde_json::json!({"ok": false, "error": "storage_unavailable"});
    };
    let plugin_dir: PathBuf = root.join("plugins").join(namespace);
    let path = plugin_dir.join("storage.json");
    if !path.exists() {
        let legacy = root
            .join("plugins")
            .join("data")
            .join(format!("{namespace}.json"));
        if legacy.exists()
            && fs::create_dir_all(&plugin_dir)
                .and_then(|()| fs::rename(&legacy, &path))
                .is_err()
        {
            return serde_json::json!({"ok": false, "error": "storage_migration_failed"});
        }
    }
    let mut values = match fs::read(&path) {
        Ok(bytes) if bytes.len() <= 4 * 1024 * 1024 => {
            match serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes) {
                Ok(values) => values,
                Err(_) => return serde_json::json!({"ok": false, "error": "storage_corrupt"}),
            }
        }
        Ok(_) => return serde_json::json!({"ok": false, "error": "storage_too_large"}),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(_) => return serde_json::json!({"ok": false, "error": "storage_unavailable"}),
    };
    match operation {
        op::PLUGIN_STORAGE_GET => {
            serde_json::json!({"ok": true, "value": values.get(key.expect("validated key"))})
        }
        op::PLUGIN_STORAGE_LIST => {
            let prefix = key.unwrap_or("");
            let after = data
                .get("after")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let limit = data
                .get("limit")
                .and_then(|value| value.as_u64())
                .unwrap_or(128)
                .clamp(1, 256) as usize;
            let mut keys: Vec<_> = values
                .keys()
                .filter(|candidate| candidate.starts_with(prefix) && candidate.as_str() > after)
                .cloned()
                .collect();
            keys.sort();
            let has_more = keys.len() > limit;
            keys.truncate(limit);
            let next = has_more.then(|| keys.last().cloned()).flatten();
            serde_json::json!({"ok": true, "keys": keys, "next": next})
        }
        op::PLUGIN_STORAGE_DELETE => {
            let removed = values.remove(key.expect("validated key")).is_some();
            if removed && !write_plugin_storage(&path, &values) {
                return serde_json::json!({"ok": false, "error": "storage_write_failed"});
            }
            serde_json::json!({"ok": true, "removed": removed})
        }
        op::PLUGIN_STORAGE_SET => {
            let Some(value) = data.get("value") else {
                return serde_json::json!({"ok": false, "error": "missing_value"});
            };
            let Ok(encoded) = serde_json::to_vec(value) else {
                return serde_json::json!({"ok": false, "error": "invalid_value"});
            };
            if encoded.len() > 8_192 {
                return serde_json::json!({"ok": false, "error": "value_too_large"});
            }
            values.insert(key.expect("validated key").to_owned(), value.clone());
            if !write_plugin_storage(&path, &values) {
                return serde_json::json!({"ok": false, "error": "storage_write_failed"});
            }
            serde_json::json!({"ok": true})
        }
        _ => serde_json::json!({"ok": false, "error": "unsupported_operation"}),
    }
}

#[cfg(feature = "server-plugins")]
fn valid_plugin_namespace(namespace: &str) -> bool {
    !namespace.is_empty()
        && namespace.len() <= 64
        && namespace != "."
        && namespace != ".."
        && namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

#[cfg(feature = "server-plugins")]
fn write_plugin_storage(
    path: &std::path::Path,
    values: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    use std::{fs, io::Write};
    let Ok(bytes) = serde_json::to_vec(values) else {
        return false;
    };
    if bytes.len() > 4 * 1024 * 1024 {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    let temp = path.with_extension("json.tmp");
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
        return false;
    }
    true
}

#[cfg(feature = "server-plugins")]
fn game_rule_value(value: Option<&serde_json::Value>) -> Option<GameRuleValue> {
    match value? {
        serde_json::Value::Bool(value) => Some(GameRuleValue::Bool(*value)),
        serde_json::Value::Number(value) => {
            Some(GameRuleValue::Int(i32::try_from(value.as_i64()?).ok()?))
        }
        _ => None,
    }
}

#[cfg(feature = "server-plugins")]
fn json_position(value: Option<&serde_json::Value>) -> Option<[f64; 3]> {
    let values = value?.as_array()?;
    if values.len() != 3 {
        return None;
    }
    values
        .iter()
        .map(serde_json::Value::as_f64)
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()
}

#[cfg(feature = "server-plugins")]
fn plugin_display_view_range(data: &serde_json::Value) -> Option<f32> {
    let range = match data.get("view_range") {
        Some(value) => value.as_f64()? as f32,
        None => 1.0,
    };
    (range.is_finite() && (0.0..=16.0).contains(&range)).then_some(range)
}

#[cfg(feature = "server-plugins")]
fn root() -> Option<std::path::PathBuf> {
    std::env::current_dir().ok()
}

#[cfg(feature = "server-plugins")]
pub(super) fn register_commands(game: &mut Game<'_>, commands: &mut Vec<NativeCommand>) {
    if game.options.dedicated.is_none() {
        return;
    }
    let Some(root) = root() else {
        return;
    };
    commands.push(NativeCommand::new("plugins"));
    let mut host = GameHost {
        game,
        commands: Some(commands),
        suggestions: None,
        requester: None,
    };
    horizon_server_plugin_loader::register_commands(&root, &mut host);
}

#[cfg(feature = "server-plugins")]
pub(super) fn permission_value(
    game: &Game<'_>,
    player: u128,
    node: &str,
    default_level: u8,
) -> Option<bool> {
    game.options.dedicated.as_ref()?;
    let root = root()?;
    horizon_server_plugin_loader::permission_value(&root, player, node, default_level)
}

#[cfg(feature = "server-plugins")]
pub(super) fn perform_command(
    game: &mut Game<'_>,
    player: &NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    if *ran || game.options.dedicated.is_none() {
        return;
    }
    let Some(root) = root() else {
        return;
    };
    if command.trim().trim_start_matches('/') == "plugins" {
        let plugins = horizon_server_plugin_loader::plugins(&root);
        let message = if plugins.is_empty() {
            "Nenhum plugin carregado.".to_owned()
        } else {
            let entries = plugins
                .iter()
                .map(|plugin| {
                    if plugin.enabled {
                        format!("§a{}", plugin.name)
                    } else {
                        format!("§c{} (desativado)", plugin.name)
                    }
                })
                .collect::<Vec<_>>()
                .join("§7, ");
            format!("§7Plugins ({}): {entries}", plugins.len())
        };
        game.native_tell(player.uuid, Text::literal(message).resolve_legacy());
        *ran = true;
        return;
    }
    let mut host = GameHost {
        game,
        commands: None,
        suggestions: None,
        requester: None,
    };
    *ran = horizon_server_plugin_loader::perform_command(&root, &mut host, player.uuid, command);
}

#[cfg(feature = "server-plugins")]
pub(super) fn tab_complete(
    game: &mut Game<'_>,
    player: u128,
    line: &str,
    requester: Option<NativeServerPlayer>,
) -> Vec<String> {
    if game.options.dedicated.is_none() {
        return Vec::new();
    }
    let Some(root) = root() else {
        return Vec::new();
    };
    let mut host = GameHost {
        game,
        commands: None,
        suggestions: Some(Vec::new()),
        requester,
    };
    horizon_server_plugin_loader::tab_complete(&root, &mut host, player, line)
}

#[cfg(feature = "server-plugins")]
pub(super) fn run_scheduled_tasks(game: &mut Game<'_>) {
    if game.options.dedicated.is_none() {
        return;
    }
    let due = {
        let tasks = SCHEDULED_PLUGIN_TASKS.get_or_init(|| Mutex::new(Vec::new()));
        let mut tasks = tasks.lock().unwrap_or_else(|error| error.into_inner());
        let now = SERVER_PLUGIN_TICK.load(Ordering::Acquire);
        let mut due = Vec::new();
        tasks.retain_mut(|task| {
            if task.due_tick <= now {
                due.push(ScheduledPluginTask {
                    id: task.id,
                    plugin: task.plugin.clone(),
                    due_tick: task.due_tick,
                    period_ticks: task.period_ticks,
                    data: task.data.clone(),
                });
                if let Some(period) = task.period_ticks {
                    task.due_tick = now.saturating_add(period);
                    true
                } else {
                    false
                }
            } else {
                true
            }
        });
        due
    };
    for task in due {
        let _ = dispatch_event(
            game,
            horizon_server_plugin_api::event::SERVER_TASK,
            0,
            &task.plugin,
            [0; 3],
            task.id.min(u64::from(u32::MAX)) as u32,
            serde_json::json!({
                "task_id": task.id.to_string(),
                "plugin_id": task.plugin,
                "data": task.data,
            }),
        );
    }
}

#[cfg(feature = "server-plugins")]
pub(super) fn begin_server_tick() {
    SERVER_PLUGIN_TICK.fetch_add(1, Ordering::AcqRel);
}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn run_scheduled_tasks(_: &mut Game<'_>) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn begin_server_tick() {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn tab_complete(
    _: &mut Game<'_>,
    _: u128,
    _: &str,
    _: Option<NativeServerPlayer>,
) -> Vec<String> {
    Vec::new()
}

#[cfg(feature = "server-plugins")]
pub(super) fn menu_clicked(
    game: &mut Game<'_>,
    player: u128,
    window: i32,
    slot: i32,
    button: i32,
    mode: i32,
) {
    if game.options.dedicated.is_none() {
        return;
    }
    let Some(root) = root() else {
        return;
    };
    let mut host = GameHost {
        game,
        commands: None,
        suggestions: None,
        requester: None,
    };
    horizon_server_plugin_loader::menu_click(&root, &mut host, player, window, slot, button, mode);
}

#[cfg(feature = "server-plugins")]
pub(super) fn dispatch_event(
    game: &mut Game<'_>,
    kind: u32,
    player: u128,
    name: &str,
    position: [i32; 3],
    data_id: u32,
    payload: serde_json::Value,
) -> bool {
    if game.options.dedicated.is_none() {
        return false;
    }
    let Some(root) = root() else {
        return false;
    };
    let event = Event {
        kind,
        player,
        name: name.to_owned(),
        dimension: game.dim.id().to_owned(),
        window_id: -1,
        position,
        data_id,
        payload,
    };
    let mut host = GameHost {
        game,
        commands: None,
        suggestions: None,
        requester: None,
    };
    horizon_server_plugin_loader::dispatch_event(&root, &mut host, &event)
}

#[cfg(feature = "server-plugins")]
pub(super) fn menu_closed(game: &mut Game<'_>, player: u128, window: i32) {
    if game.options.dedicated.is_none() {
        return;
    }
    let Some(root) = root() else {
        return;
    };
    let event = Event {
        kind: horizon_server_plugin_api::event::MENU_CLOSED,
        player,
        name: String::new(),
        dimension: game.dim.id().to_owned(),
        window_id: window,
        position: [0; 3],
        data_id: 0,
        payload: serde_json::Value::Null,
    };
    let mut host = GameHost {
        game,
        commands: None,
        suggestions: None,
        requester: None,
    };
    horizon_server_plugin_loader::dispatch_event(&root, &mut host, &event);
}

#[cfg(feature = "server-plugins")]
pub(super) fn block_break(game: &mut Game<'_>, pos: [i32; 3], data_id: u32) -> bool {
    let Some(player) = game.native_acting_player() else {
        return false;
    };
    let (uuid, name) = (player.uuid, player.name.clone());
    dispatch_event(
        game,
        horizon_server_plugin_api::event::BLOCK_BREAK,
        uuid,
        &name,
        pos,
        data_id,
        serde_json::Value::Null,
    )
}

#[cfg(feature = "server-plugins")]
pub(super) fn player_join(game: &mut Game<'_>, player: u128, name: &str) {
    dispatch_event(
        game,
        horizon_server_plugin_api::event::PLAYER_JOIN,
        player,
        name,
        [0; 3],
        0,
        serde_json::Value::Null,
    );
}

#[cfg(feature = "server-plugins")]
pub(super) fn player_quit(game: &mut Game<'_>, player: u128, name: &str) {
    let last_location = game
        .native_server_players()
        .into_iter()
        .find(|snapshot| snapshot.uuid == player)
        .or_else(|| {
            game.native_acting_player()
                .filter(|snapshot| snapshot.uuid == player)
        })
        .map(|snapshot| {
            serde_json::json!({
                "dimension": snapshot.dimension,
                "position": snapshot.position,
                "yaw": snapshot.yaw,
                "pitch": snapshot.pitch,
            })
        });
    dispatch_event(
        game,
        horizon_server_plugin_api::event::PLAYER_QUIT,
        player,
        name,
        [0; 3],
        0,
        serde_json::json!({"last_location": last_location}),
    );
}

#[cfg(feature = "server-plugins")]
pub(super) fn player_changed_dimension(
    game: &mut Game<'_>,
    player: u128,
    name: &str,
    from: &str,
    to: &str,
) {
    dispatch_event(
        game,
        horizon_server_plugin_api::event::PLAYER_CHANGED_DIMENSION,
        player,
        name,
        [0; 3],
        0,
        serde_json::json!({"from": from, "to": to}),
    );
}

#[cfg(feature = "server-plugins")]
pub(super) fn player_teleported(
    game: &mut Game<'_>,
    player: u128,
    name: &str,
    from: [f64; 3],
    to: [f64; 3],
    dimension: &str,
) {
    dispatch_event(
        game,
        horizon_server_plugin_api::event::PLAYER_TELEPORT,
        player,
        name,
        to.map(|value| value.floor() as i32),
        0,
        serde_json::json!({"from": from, "to": to, "dimension": dimension}),
    );
}

#[cfg(feature = "server-plugins")]
pub(super) fn player_chat(game: &mut Game<'_>, player: u128, name: &str, text: &str) {
    dispatch_event(
        game,
        horizon_server_plugin_api::event::CHAT,
        player,
        name,
        [0; 3],
        0,
        serde_json::json!({"message": text, "phase": "after_broadcast"}),
    );
}

#[cfg(feature = "server-plugins")]
pub(super) fn block_place(game: &mut Game<'_>, pos: [i32; 3], data_id: u32) {
    let Some(player) = game.native_acting_player() else {
        return;
    };
    dispatch_event(
        game,
        horizon_server_plugin_api::event::BLOCK_PLACE,
        player.uuid,
        &player.name,
        pos,
        data_id,
        serde_json::Value::Null,
    );
}

#[cfg(feature = "server-plugins")]
pub(super) fn player_death(
    game: &mut Game<'_>,
    entity: u32,
    damage_type: &str,
    attacker: u32,
) -> bool {
    let Some(player) = game
        .native_server_players()
        .into_iter()
        .find(|player| player.entity_id == entity as i32)
    else {
        return false;
    };
    dispatch_event(
        game,
        horizon_server_plugin_api::event::PLAYER_DEATH,
        player.uuid,
        &player.name,
        player.position.map(|value| value.floor() as i32),
        0,
        serde_json::json!({
            "damage_type": damage_type,
            "attacker_entity_id": attacker,
            "location": {
                "dimension": player.dimension,
                "position": player.position,
                "yaw": player.yaw,
                "pitch": player.pitch,
            }
        }),
    )
}

#[cfg(feature = "server-plugins")]
pub(super) fn resource_pack_status(
    game: &mut Game<'_>,
    player: u128,
    name: &str,
    pack: u128,
    status: i32,
) {
    dispatch_event(
        game,
        horizon_server_plugin_api::event::RESOURCE_PACK_STATUS,
        player,
        name,
        [0; 3],
        status as u32,
        serde_json::json!({"pack_id": format!("{pack:032x}"), "status": status}),
    );
}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn register_commands(_: &mut Game<'_>, _: &mut Vec<NativeCommand>) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn permission_value(_: &Game<'_>, _: u128, _: &str, _: u8) -> Option<bool> {
    None
}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn perform_command(_: &mut Game<'_>, _: &NativeServerPlayer, _: &str, _: &mut bool) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn menu_clicked(_: &mut Game<'_>, _: u128, _: i32, _: i32, _: i32, _: i32) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn menu_closed(_: &mut Game<'_>, _: u128, _: i32) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn block_break(_: &mut Game<'_>, _: [i32; 3], _: u32) -> bool {
    false
}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn player_join(_: &mut Game<'_>, _: u128, _: &str) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn player_quit(_: &mut Game<'_>, _: u128, _: &str) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn player_changed_dimension(_: &mut Game<'_>, _: u128, _: &str, _: &str, _: &str) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn player_teleported(
    _: &mut Game<'_>,
    _: u128,
    _: &str,
    _: [f64; 3],
    _: [f64; 3],
    _: &str,
) {
}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn player_chat(_: &mut Game<'_>, _: u128, _: &str, _: &str) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn block_place(_: &mut Game<'_>, _: [i32; 3], _: u32) {}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn player_death(_: &mut Game<'_>, _: u32, _: &str, _: u32) -> bool {
    false
}

#[cfg(not(feature = "server-plugins"))]
pub(super) fn resource_pack_status(_: &mut Game<'_>, _: u128, _: &str, _: u128, _: i32) {}

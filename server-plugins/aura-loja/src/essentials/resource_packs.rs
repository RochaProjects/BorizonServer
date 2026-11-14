//! Packs Java enviados em lote após o jogador entrar no mundo.

use horizon_server_plugin_api::{HostApiV3, event, operation};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

const PLUGIN_ID: &str = "AuraEssentials";
const CONFIG_FILE: &str = "resource-packs.json";
const DEFAULT_CONFIG: &str = r#"{
  "enabled": true,
  "packs": [
    {
      "id": "3c20f2ab-7617-48df-8a21-5a42f7329c14",
      "name": "Kingdoms",
      "url": "https://github.com/RochaProjects/AuraEssentials/releases/latest/download/kingdoms-glyph-pack-1.21.10.zip",
      "sha1": "707aaf51bfaeed6bbeee44e58fb66a176de8c439",
      "required": true,
      "prompt": "Baixar textura do servidor"
    },
    {
      "id": "d9f851a8-0b93-4c91-9f57-f9496cb7a103",
      "name": "AuraNPC",
      "url": "https://gitlab.com/rochaproject/archive/-/raw/main/auraessentials/AuraNPC.zip?ref_type=heads",
      "sha1": "5ea0e0f8252d6c4f62097cc97bac447749b44032",
      "required": false,
      "prompt": "Baixar modelos 3D dos NPCs"
    }
  ]
}"#;

#[derive(Deserialize)]
struct Config {
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    packs: Vec<Pack>,
}

#[derive(Deserialize)]
struct Pack {
    id: String,
    name: Option<String>,
    url: String,
    sha1: String,
    #[serde(default)]
    required: bool,
    prompt: Option<String>,
}

#[derive(Default)]
struct PlayerPackReport {
    expected: HashMap<String, String>,
    terminal_status: HashMap<String, i64>,
}

static PLAYER_PACK_REPORTS: OnceLock<Mutex<HashMap<u128, PlayerPackReport>>> = OnceLock::new();

fn enabled_by_default() -> bool {
    true
}

fn config_path() -> Option<PathBuf> {
    Some(
        std::env::current_dir()
            .ok()?
            .join("plugins")
            .join(PLUGIN_ID)
            .join(CONFIG_FILE),
    )
}

pub(super) fn ensure_default_file() {
    let Some(path) = config_path() else { return };
    if path.exists() {
        return;
    }
    let Some(parent) = path.parent() else { return };
    if let Err(error) = fs::create_dir_all(parent).and_then(|()| fs::write(&path, DEFAULT_CONFIG)) {
        eprintln!("[AuraEssentials] Não foi possível criar {CONFIG_FILE}: {error}");
    }
}

pub(super) fn event(host: &HostApiV3, kind: u32, payload: &serde_json::Value) {
    if kind == event::RESOURCE_PACK_STATUS {
        record_pack_status(payload);
        return;
    }
    if kind == event::PLAYER_QUIT {
        if let Some(player) = event_player(payload) {
            reports()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&player);
        }
        return;
    }
    if kind != event::PLAYER_JOIN {
        return;
    }
    // PLAYER_JOIN is dispatched after the server has placed the player in a world,
    // so the backend can offer packs without relying on pre-login or a scheduled task.
    let Some(player) = payload.get("player").and_then(serde_json::Value::as_str) else {
        return;
    };
    let Some(path) = config_path() else { return };
    let Ok(bytes) = fs::read(path) else { return };
    let Ok(config) = serde_json::from_slice::<Config>(&bytes) else {
        return;
    };
    if !config.enabled {
        return;
    }
    let configured: Vec<_> = config
        .packs
        .into_iter()
        .filter(|pack| {
            pack.id.replace('-', "").len() == 32
                && pack
                    .id
                    .replace('-', "")
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                && !pack.url.is_empty()
                && pack.sha1.len() == 40
                && pack.sha1.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .take(8)
        .collect();
    if configured.is_empty() {
        return;
    }
    let Ok(player) = u128::from_str_radix(&player.replace('-', ""), 16) else {
        return;
    };
    let expected: HashMap<_, _> = configured
        .iter()
        .map(|pack| {
            (
                normalize_id(&pack.id),
                pack.name.clone().unwrap_or_else(|| "Resource pack".into()),
            )
        })
        .collect();
    let packs: Vec<_> = configured
        .iter()
        .map(|pack| {
            let mut value = serde_json::json!({
                "id": pack.id,
                "url": pack.url,
                "sha1": pack.sha1,
                "required": pack.required,
            });
            if let Some(prompt) = &pack.prompt {
                value["prompt"] = serde_json::Value::String(prompt.clone());
            }
            value
        })
        .collect();
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::RESOURCE_PACK_SEND_BATCH,
            "data": {"packs": packs}
        }),
    );
    if result.as_ref().is_some_and(|result| {
        result["ok"] == true && result["sent"].as_u64() == Some(expected.len() as u64)
    }) {
        reports().lock().unwrap_or_else(|e| e.into_inner()).insert(
            player,
            PlayerPackReport {
                expected,
                terminal_status: HashMap::new(),
            },
        );
    } else {
        return;
    }
}

fn reports() -> &'static Mutex<HashMap<u128, PlayerPackReport>> {
    PLAYER_PACK_REPORTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn event_player(payload: &serde_json::Value) -> Option<u128> {
    u128::from_str_radix(&payload["player"].as_str()?.replace('-', ""), 16).ok()
}

fn normalize_id(id: &str) -> String {
    id.replace('-', "").to_ascii_lowercase()
}

fn record_pack_status(payload: &serde_json::Value) {
    let Some(player) = event_player(payload) else {
        return;
    };
    let Some(pack_id) = payload["data"]["pack_id"].as_str() else {
        return;
    };
    let Some(status) = payload["data"]["status"].as_i64() else {
        return;
    };
    // 3 (accepted) and 4 (downloaded) are progress. Only report the final
    // successfully-loaded or rejected/failed status for each pack.
    if !matches!(status, 0 | 1 | 2 | 5 | 6 | 7) {
        return;
    }
    let report = {
        let mut reports = reports().lock().unwrap_or_else(|e| e.into_inner());
        let Some(report) = reports.get_mut(&player) else {
            return;
        };
        let pack_id = normalize_id(pack_id);
        if !report.expected.contains_key(&pack_id) {
            return;
        }
        report.terminal_status.entry(pack_id).or_insert(status);
        if report.terminal_status.len() < report.expected.len() {
            return;
        }
        reports.remove(&player)
    };
    let Some(report) = report else { return };
    let name = payload["name"].as_str().unwrap_or("jogador");
    let rejected: Vec<_> = report
        .expected
        .iter()
        .filter(|(id, _)| report.terminal_status.get(*id) != Some(&0))
        .map(|(_, name)| name.as_str())
        .collect();
    if rejected.is_empty() {
        eprintln!("[AuraEssentials] Texturas carregadas com sucesso para {name}.");
    } else {
        eprintln!(
            "[AuraEssentials] Texturas rejeitadas ou com falha para {name}: {}.",
            rejected.join(", ")
        );
    }
}

fn call_json(
    host: &HostApiV3,
    player: u128,
    request: serde_json::Value,
) -> Option<serde_json::Value> {
    let callback = host.call_json?;
    let input = request.to_string();
    let mut output = vec![0_u8; 65_536];
    let mut output_len = 0_usize;
    // SAFETY: input/output buffers live throughout the synchronous host callback.
    let (high, low) = ((player >> 64) as u64, player as u64);
    let status = unsafe {
        callback(
            host.context,
            high,
            low,
            input.as_ptr(),
            input.len(),
            output.as_mut_ptr(),
            output.len(),
            &mut output_len,
        )
    };
    (status == horizon_server_plugin_api::result::OK && output_len <= output.len())
        .then(|| serde_json::from_slice(&output[..output_len]).ok())
        .flatten()
}

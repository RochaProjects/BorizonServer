//! Comandos essenciais que usam apenas operações genéricas do SDK público.

use crate::plugin_api::{call_json, message};
use horizon_server_plugin_api::{HostApiV3, operation};

const STORAGE_ID: &str = "AuraEssentials";

mod teleport;
mod worlds;

pub fn register_commands(host: &HostApiV3) {
    teleport::register_commands(host);
    worlds::register_commands(host);
    for (usage, level, permission) in [
        ("sethome <nome:word>", 0, None),
        ("home [nome:word]", 0, None),
        ("homes", 0, None),
        ("delhome <nome:word>", 0, None),
        ("setwarp <nome:word>", 2, Some("auraessentials.warp.admin")),
        ("warp <nome:word>", 0, None),
        ("warps", 0, None),
        ("delwarp <nome:word>", 2, Some("auraessentials.warp.admin")),
        ("rtp", 0, None),
    ] {
        let mut data = serde_json::json!({"usage": usage, "default_level": level});
        if let Some(permission) = permission {
            data["permission"] = serde_json::Value::String(permission.into());
        }
        let _ = call_json(
            host,
            0,
            serde_json::json!({"operation": operation::COMMAND_REGISTER, "data": data}),
        );
    }
}

pub fn command(host: &HostApiV3, player: u128, command: &str) -> bool {
    if teleport::command(host, player, command) {
        return true;
    }
    if worlds::command(host, player, command) {
        return true;
    }
    let mut parts = command.split_whitespace();
    let Some(root) = parts.next() else {
        return false;
    };
    let args: Vec<_> = parts.collect();
    match root {
        "sethome" if args.len() == 1 => save_home(host, player, args[0]),
        "home" if args.len() <= 1 => {
            if let Some(name) = args.first() {
                teleport_home(host, player, name)
            } else {
                list_homes(host, player)
            }
        }
        "homes" if args.is_empty() => list_homes(host, player),
        "delhome" if args.len() == 1 => delete_home(host, player, args[0]),
        "setwarp" if args.len() == 1 => save_warp(host, player, args[0]),
        "warp" if args.len() == 1 => teleport_warp(host, player, args[0]),
        "warps" if args.is_empty() => list_warps(host, player),
        "delwarp" if args.len() == 1 => delete_warp(host, player, args[0]),
        "rtp" if args.is_empty() => worlds::random_teleport(host, player),
        _ => false,
    }
}

pub fn event(host: &HostApiV3, kind: u32, payload: &serde_json::Value) {
    teleport::event(host, kind, payload);
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

fn player_snapshot(host: &HostApiV3, player: u128) -> Option<serde_json::Value> {
    call_json(
        host,
        player,
        serde_json::json!({"operation": operation::PLAYER_GET, "data": {}}),
    )
    .filter(|result| result["ok"] == true)
}

fn storage_get(host: &HostApiV3, key: &str) -> Option<serde_json::Value> {
    call_json(
        host,
        0,
        serde_json::json!({
            "operation": operation::PLUGIN_STORAGE_GET,
            "data": {"plugin": STORAGE_ID, "key": key},
        }),
    )
    .filter(|result| result["ok"] == true)
    .and_then(|result| result.get("value").cloned())
    .filter(|value| !value.is_null())
}

fn storage_set(host: &HostApiV3, key: &str, value: serde_json::Value) -> bool {
    call_json(
        host,
        0,
        serde_json::json!({
            "operation": operation::PLUGIN_STORAGE_SET,
            "data": {"plugin": STORAGE_ID, "key": key, "value": value},
        }),
    )
    .is_some_and(|result| result["ok"] == true)
}

fn storage_list(host: &HostApiV3, prefix: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut after = String::new();
    loop {
        let Some(result) = call_json(
            host,
            0,
            serde_json::json!({
                "operation": operation::PLUGIN_STORAGE_LIST,
                "data": {"plugin": STORAGE_ID, "key": prefix, "after": after, "limit": 256},
            }),
        ) else {
            break;
        };
        if result["ok"] != true {
            break;
        }
        let Some(page) = result["keys"].as_array() else {
            break;
        };
        keys.extend(
            page.iter()
                .filter_map(|key| key.as_str().map(str::to_owned)),
        );
        let Some(next) = result["next"].as_str() else {
            break;
        };
        after = next.to_owned();
    }
    keys
}

fn save_home(host: &HostApiV3, player: u128, name: &str) -> bool {
    if !valid_name(name) {
        message(
            host,
            player,
            "Use um nome de 1 a 32 caracteres (letras, números, _ ou -).",
        );
        return true;
    }
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui ler sua posição atual.");
        return true;
    };
    let uuid = snapshot["uuid"].as_str().unwrap_or_default();
    let key = format!("home:{uuid}:{}", name.to_ascii_lowercase());
    if storage_set(host, &key, location(&snapshot)) {
        message(
            host,
            player,
            &format!("Casa '{name}' salva nesta dimensão."),
        );
    } else {
        message(host, player, "Não foi possível salvar a casa.");
    }
    true
}

fn list_homes(host: &HostApiV3, player: u128) -> bool {
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui consultar suas casas.");
        return true;
    };
    let prefix = format!("home:{}:", snapshot["uuid"].as_str().unwrap_or_default());
    let names = storage_list(host, &prefix)
        .into_iter()
        .map(|key| key[prefix.len()..].to_owned())
        .collect::<Vec<_>>();
    if names.is_empty() {
        message(
            host,
            player,
            "Você ainda não tem casas. Use /sethome <nome>.",
        );
    } else {
        message(host, player, &format!("Suas casas: {}", names.join(", ")));
    }
    true
}

fn delete_home(host: &HostApiV3, player: u128, name: &str) -> bool {
    if !valid_name(name) {
        message(host, player, "Nome de casa inválido.");
        return true;
    }
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui consultar suas casas.");
        return true;
    };
    let key = format!(
        "home:{}:{}",
        snapshot["uuid"].as_str().unwrap_or_default(),
        name.to_ascii_lowercase()
    );
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::PLUGIN_STORAGE_DELETE,
            "data": {"plugin": STORAGE_ID, "key": key},
        }),
    );
    message(
        host,
        player,
        if result.is_some_and(|result| result["ok"] == true) {
            "Casa removida."
        } else {
            "Essa casa não existe ou não pôde ser removida."
        },
    );
    true
}

fn teleport_home(host: &HostApiV3, player: u128, name: &str) -> bool {
    if !valid_name(name) {
        message(host, player, "Nome de casa inválido.");
        return true;
    }
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui consultar suas casas.");
        return true;
    };
    let key = format!(
        "home:{}:{}",
        snapshot["uuid"].as_str().unwrap_or_default(),
        name.to_ascii_lowercase()
    );
    let home = storage_get(host, &key);
    match home {
        Some(target) => teleport(host, player, &target, &format!("Indo para casa '{name}'.")),
        None => message(
            host,
            player,
            &format!("Casa '{name}' não encontrada. Use /homes."),
        ),
    }
    true
}

fn save_warp(host: &HostApiV3, player: u128, name: &str) -> bool {
    if !valid_name(name) {
        message(host, player, "Nome inválido para warp.");
        return true;
    }
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui ler sua posição atual.");
        return true;
    };
    let key = format!("warp:{}", name.to_ascii_lowercase());
    if storage_set(host, &key, location(&snapshot)) {
        message(host, player, &format!("Warp '{name}' salvo."));
    } else {
        message(host, player, "Não foi possível salvar o warp.");
    }
    true
}

fn list_warps(host: &HostApiV3, player: u128) -> bool {
    let prefix = "warp:";
    let names = storage_list(host, prefix)
        .into_iter()
        .map(|key| key[prefix.len()..].to_owned())
        .collect::<Vec<_>>();
    if names.is_empty() {
        message(host, player, "Não há warps configurados.");
    } else {
        message(host, player, &format!("Warps: {}", names.join(", ")));
    }
    true
}

fn delete_warp(host: &HostApiV3, player: u128, name: &str) -> bool {
    if !valid_name(name) {
        message(host, player, "Nome de warp inválido.");
        return true;
    }
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::PLUGIN_STORAGE_DELETE,
            "data": {"plugin": STORAGE_ID, "key": format!("warp:{}", name.to_ascii_lowercase())},
        }),
    );
    message(
        host,
        player,
        if result.is_some_and(|result| result["ok"] == true) {
            "Warp removido."
        } else {
            "Esse warp não existe ou não pôde ser removido."
        },
    );
    true
}

fn teleport_warp(host: &HostApiV3, player: u128, name: &str) -> bool {
    if !valid_name(name) {
        message(host, player, "Nome de warp inválido.");
        return true;
    }
    let key = format!("warp:{}", name.to_ascii_lowercase());
    let warp = storage_get(host, &key);
    match warp {
        Some(target) => teleport(host, player, &target, &format!("Indo para warp '{name}'.")),
        None => message(
            host,
            player,
            &format!("Warp '{name}' não encontrado. Use /warps."),
        ),
    }
    true
}

fn location(snapshot: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "dimension": snapshot["dimension"],
        "position": snapshot["position"],
        "yaw": snapshot["yaw"],
        "pitch": snapshot["pitch"],
    })
}

fn teleport(host: &HostApiV3, player: u128, target: &serde_json::Value, success: &str) {
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::PLAYER_TELEPORT,
            "data": {
                "dimension": target["dimension"],
                "position": target["position"],
                "yaw": target["yaw"],
                "pitch": target["pitch"],
            },
        }),
    );
    if result.is_some_and(|result| result["ok"] == true) {
        message(host, player, success);
    } else {
        message(
            host,
            player,
            "O destino existe, mas a dimensão não está carregada ou o teleporte falhou.",
        );
    }
}

//! Cargos explícitos: OP/permissões controlam acesso, nunca identidade visual.
use super::{player_snapshot, storage_get, storage_set};
use crate::plugin_api::{call_json, message};
use horizon_server_plugin_api::{HostApiV3, event, operation};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

static ROLE_CACHE: OnceLock<Mutex<HashMap<u128, String>>> = OnceLock::new();

const ROLE_KEYS: &[&str] = &[
    "membro",
    "dono",
    "admin",
    "vip",
    "diamante",
    "ferro",
    "ouro",
    "pedra",
    "novato",
    "end",
    "nether",
    "espaco",
    "fogo",
    "agua",
    "ar",
    "twitch",
    "youtuber",
    "atirador",
    "comando",
    "construtor",
    "legend",
    "discord",
    "helper",
    "reino",
    "morto",
    "hacker",
    "designer",
    "killer",
    "pixel",
    "dev",
];

fn key(uuid: u128) -> String {
    format!("role:{uuid:032x}")
}

pub(super) fn role_for_player(host: &HostApiV3, uuid: u128) -> String {
    if let Some(role) = ROLE_CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&uuid)
        .cloned()
    {
        return role;
    }
    let role = storage_get(host, &key(uuid))
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|role| ROLE_KEYS.contains(&role.as_str()))
        .unwrap_or_else(|| "membro".into());
    ROLE_CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(uuid, role.clone());
    role
}

pub(super) fn register_commands(host: &HostApiV3) {
    for usage in ["entregarcargo <jogador:player_name> <cargo:word>", "cargos"] {
        let level = if usage.starts_with("entregarcargo") {
            2
        } else {
            0
        };
        let _ = call_json(
            host,
            0,
            serde_json::json!({
                "operation": operation::COMMAND_REGISTER,
                "data": {"usage": usage, "default_level": level}
            }),
        );
    }
}

pub(super) fn command(host: &HostApiV3, actor: u128, command: &str) -> bool {
    let words: Vec<_> = command.split_whitespace().collect();
    match words.as_slice() {
        ["cargos"] => {
            message(host, actor, &format!("Cargos: {}", ROLE_KEYS.join(", ")));
            true
        }
        ["entregarcargo", target_name, role] => {
            let Some(snapshot) = player_snapshot(host, actor) else {
                return true;
            };
            if snapshot["permission_level"].as_u64().unwrap_or(0) < 2 {
                message(host, actor, "Você não tem permissão para entregar cargos.");
                return true;
            }
            if !ROLE_KEYS.contains(role) {
                message(
                    host,
                    actor,
                    &format!(
                        "Cargo inválido. Cargos disponíveis: {}",
                        ROLE_KEYS.join(", ")
                    ),
                );
                return true;
            }
            let Some(target) = call_json(
                host,
                actor,
                serde_json::json!({
                    "operation": operation::PLAYER_LOOKUP, "data": {"name": target_name}
                }),
            )
            .filter(|v| v["ok"] == true)
            .and_then(|v| v["players"].as_array()?.first().cloned()) else {
                message(host, actor, "Jogador não encontrado.");
                return true;
            };
            let Some(uuid) = target["uuid"]
                .as_str()
                .and_then(|s| u128::from_str_radix(s, 16).ok())
            else {
                return true;
            };
            if !storage_set(host, &key(uuid), serde_json::json!(role)) {
                message(host, actor, "Não foi possível salvar o cargo.");
                return true;
            }
            ROLE_CACHE
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(uuid, (*role).into());
            message(
                host,
                actor,
                &format!(
                    "Cargo {role} atribuído a {}.",
                    target["name"].as_str().unwrap_or(target_name)
                ),
            );
            apply_visuals(host, uuid);
            // Changing a role changes the nametag too; update the online player's
            // scoreboard team immediately instead of waiting for a reconnect.
            true
        }
        _ => false,
    }
}

pub(super) fn suggestions(host: &HostApiV3, actor: u128, line: &str) {
    let words: Vec<_> = line.split_whitespace().collect();
    let trailing = line.ends_with(' ');
    let candidates: Vec<String> = match words.as_slice() {
        ["entregarcargo"] => call_json(
            host,
            actor,
            serde_json::json!({"operation": operation::PLAYER_ONLINE_LIST, "data": {}}),
        )
        .and_then(|v| v["players"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect(),
        ["entregarcargo", _] if trailing => ROLE_KEYS.iter().map(|s| (*s).to_owned()).collect(),
        ["entregarcargo", _, _] => ROLE_KEYS.iter().map(|s| (*s).to_owned()).collect(),
        _ => Vec::new(),
    };
    let current = if line.ends_with(' ') {
        ""
    } else {
        words.last().copied().unwrap_or("")
    };
    for value in candidates.into_iter().filter(|v| v.starts_with(current)) {
        let _ = call_json(
            host,
            actor,
            serde_json::json!({"operation": operation::COMMAND_SUGGESTION_ADD, "data": {"value": value}}),
        );
    }
}

pub(super) fn event(host: &HostApiV3, kind: u32, payload: &serde_json::Value) {
    if kind != event::PLAYER_JOIN {
        return;
    }
    let Some(uuid) = payload["player"]
        .as_str()
        .and_then(|s| u128::from_str_radix(s, 16).ok())
    else {
        return;
    };
    // Materialize the default once; granting a role remains an explicit command.
    if storage_get(host, &key(uuid)).is_none() {
        let _ = storage_set(host, &key(uuid), serde_json::json!("membro"));
    }
    apply_visuals(host, uuid);
}

fn apply_visuals(host: &HostApiV3, uuid: u128) {
    let Some(snapshot) = player_snapshot(host, uuid) else {
        return;
    };
    let role = role_for_player(host, uuid);
    let glyph = match role.as_str() {
        "dono" => "\u{e800}",
        "admin" => "\u{e801}",
        "membro" => "\u{e802}",
        "vip" => "\u{e803}",
        "diamante" => "\u{e804}",
        "ferro" => "\u{e805}",
        "ouro" => "\u{e806}",
        "pedra" => "\u{e807}",
        "novato" => "\u{e808}",
        "end" => "\u{e809}",
        "nether" => "\u{e80a}",
        "espaco" => "\u{e80b}",
        "fogo" => "\u{e80c}",
        "agua" => "\u{e80d}",
        "ar" => "\u{e80e}",
        "twitch" => "\u{e80f}",
        "youtuber" => "\u{e810}",
        "atirador" => "\u{e811}",
        "comando" => "\u{e812}",
        "construtor" => "\u{e813}",
        "legend" => "\u{e814}",
        "discord" => "\u{e815}",
        "helper" => "\u{e816}",
        "reino" => "\u{e817}",
        "morto" => "\u{e818}",
        "hacker" => "\u{e819}",
        "designer" => "\u{e81a}",
        "killer" => "\u{e81b}",
        "pixel" => "\u{e81c}",
        _ => "\u{e81d}",
    };
    let nick = snapshot["name"].as_str().unwrap_or("");
    let tab = format!("{glyph} {nick}");
    let money = balance_units(host, uuid, "aura:coins");
    let suffix = json!({
        "text": format!(" ${}", format_units(money)),
        "color": "green"
    });
    let _ = call_json(
        host,
        uuid,
        serde_json::json!({
            "operation": operation::PLAYER_DISPLAY_SET,
            "data": {
                "uuid": format!("{uuid:032x}"),
                "chat": nick,
                "tab": tab,
                "nametag_prefix": format!("{glyph} {nick} "),
                "nametag_suffix": suffix
            }
        }),
    );
}

pub(super) fn refresh_visuals(host: &HostApiV3, uuid: u128) {
    apply_visuals(host, uuid);
}

fn balance_units(host: &HostApiV3, uuid: u128, currency: &str) -> i64 {
    let Some(get) = host.common.get_balance else {
        return 0;
    };
    let mut value = 0_i64;
    // SAFETY: synchronous call; currency and output remain valid for its duration.
    let status = unsafe {
        get(
            host.common.context,
            (uuid >> 64) as u64,
            uuid as u64,
            currency.as_ptr(),
            currency.len(),
            &mut value,
        )
    };
    if status == horizon_server_plugin_api::result::OK {
        value
    } else {
        0
    }
}

fn format_units(value: i64) -> String {
    let abs = value.unsigned_abs();
    let cents = abs % 100;
    let amount = if cents == 0 {
        (abs / 100).to_string()
    } else {
        format!(
            "{},{}",
            abs / 100,
            format!("{cents:02}").trim_end_matches('0')
        )
    };
    if value < 0 {
        format!("-{amount}")
    } else {
        amount
    }
}

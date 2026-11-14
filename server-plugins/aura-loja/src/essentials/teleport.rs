use crate::plugin_api::{call_json, message};
use horizon_server_plugin_api::{HostApiV3, event, operation};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const STORAGE_ID: &str = "AuraEssentials";
const REQUEST_TTL_MS: u64 = 30_000;

#[derive(Clone, Copy)]
struct TpaRequest {
    requester: u128,
    recipient: u128,
    here: bool,
    expires_at: u64,
}

static REQUESTS: OnceLock<Mutex<HashMap<u128, TpaRequest>>> = OnceLock::new();

fn requests() -> &'static Mutex<HashMap<u128, TpaRequest>> {
    REQUESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn register_commands(host: &HostApiV3) {
    for usage in [
        "tpa <jogador:player_name>",
        "tpahere <jogador:player_name>",
        "aceitar [jogador:player_name]",
        "negar [jogador:player_name]",
        "back",
    ] {
        let _ = call_json(
            host,
            0,
            serde_json::json!({
                "operation": operation::COMMAND_REGISTER,
                "data": {"usage": usage, "default_level": 0},
            }),
        );
    }
}

pub(super) fn command(host: &HostApiV3, player: u128, command: &str) -> bool {
    let mut args = command.split_whitespace();
    let Some(root) = args.next() else {
        return false;
    };
    let arg = args.next();
    match root {
        "tpa" | "tpahere" if arg.is_some() && args.next().is_none() => {
            request_teleport(host, player, arg.expect("matched above"), root == "tpahere")
        }
        "aceitar" if args.next().is_none() => accept(host, player, arg),
        "negar" if args.next().is_none() => deny(host, player, arg),
        "back" if arg.is_none() && args.next().is_none() => back(host, player),
        _ => false,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

fn uuid(result: &serde_json::Value) -> Option<u128> {
    result
        .get("uuid")?
        .as_str()
        .and_then(|text| u128::from_str_radix(text, 16).ok())
}

fn lookup(host: &HostApiV3, player: u128, name: &str) -> Option<(u128, String)> {
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::PLAYER_LOOKUP,
            "data": {"name": name},
        }),
    )?;
    let snapshot = result.get("players")?.as_array()?.first()?;
    Some((uuid(snapshot)?, snapshot.get("name")?.as_str()?.to_owned()))
}

fn snapshot(host: &HostApiV3, player: u128) -> Option<serde_json::Value> {
    call_json(
        host,
        player,
        serde_json::json!({"operation": operation::PLAYER_GET, "data": {}}),
    )
    .filter(|result| result["ok"] == true)
}

fn request_teleport(host: &HostApiV3, requester: u128, name: &str, here: bool) -> bool {
    let Some(origin) = snapshot(host, requester) else {
        message(host, requester, "Não consegui consultar seu jogador.");
        return true;
    };
    let Some((recipient, recipient_name)) = lookup(host, requester, name) else {
        message(host, requester, "Jogador não encontrado ou offline.");
        return true;
    };
    if recipient == requester {
        message(
            host,
            requester,
            "Você não pode pedir teleporte para si mesmo.",
        );
        return true;
    }
    let Some(target) = snapshot_for(host, requester, recipient) else {
        message(host, requester, "Jogador não encontrado ou offline.");
        return true;
    };
    if origin["dimension"] != target["dimension"] {
        // The vanilla teleport operation can cross dimensions, but TPA policy is
        // intentionally based on each dimension's loaded/allowed state.
        let dimensions = call_json(
            host,
            requester,
            serde_json::json!({"operation": operation::WORLD_DIMENSIONS_LIST, "data": {}}),
        );
        if !dimensions.is_some_and(|list| {
            list["dimensions"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == target["dimension"].as_str())
            })
        }) {
            message(
                host,
                requester,
                "A dimensão do jogador não está disponível.",
            );
            return true;
        }
    }
    let request = TpaRequest {
        requester,
        recipient,
        here,
        expires_at: now_ms().saturating_add(REQUEST_TTL_MS),
    };
    let mut pending = requests().lock().unwrap_or_else(|error| error.into_inner());
    pending.retain(|_, entry| entry.expires_at > now_ms());
    pending.insert(recipient, request);
    drop(pending);
    let kind = if here { "tpahere" } else { "tpa" };
    message(
        host,
        requester,
        &format!("Pedido {kind} enviado para {recipient_name}; expira em 30 segundos."),
    );
    message(
        host,
        recipient,
        &format!(
            "{kind} de {}. Use /aceitar ou /negar em até 30 segundos.",
            origin["name"].as_str().unwrap_or("jogador")
        ),
    );
    true
}

fn snapshot_for(host: &HostApiV3, actor: u128, target: u128) -> Option<serde_json::Value> {
    let result = call_json(
        host,
        actor,
        serde_json::json!({
            "operation": operation::PLAYER_LOOKUP,
            "data": {"uuid": format!("{target:032x}")},
        }),
    )?;
    result.get("players")?.as_array()?.first().cloned()
}

fn accept(host: &HostApiV3, recipient: u128, requester_name: Option<&str>) -> bool {
    let request = requests()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&recipient);
    let Some(request) = request else {
        message(
            host,
            recipient,
            "Você não tem pedido de teleporte pendente.",
        );
        return true;
    };
    if request.expires_at <= now_ms() {
        message(host, recipient, "Esse pedido de teleporte expirou.");
        return true;
    }
    let Some(requester) = snapshot_for(host, recipient, request.requester) else {
        message(
            host,
            recipient,
            "Quem enviou o pedido não está mais online.",
        );
        return true;
    };
    if requester_name.is_some_and(|name| {
        !requester["name"]
            .as_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
    }) {
        message(
            host,
            recipient,
            "O pedido pendente não corresponde a esse jogador.",
        );
        requests()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(request.recipient, request);
        return true;
    }
    let (target, destination) = if request.here {
        (
            request.recipient,
            snapshot_for(host, recipient, request.requester),
        )
    } else {
        (request.requester, snapshot(host, recipient))
    };
    let Some(destination) = destination else {
        message(
            host,
            recipient,
            "Não consegui consultar a posição de destino.",
        );
        return true;
    };
    let result = call_json(
        host,
        recipient,
        serde_json::json!({
            "operation": operation::PLAYER_TELEPORT,
            "data": {
                "target": format!("{target:032x}"),
                "dimension": destination["dimension"],
                "position": destination["position"],
                "yaw": destination["yaw"],
                "pitch": destination["pitch"],
            },
        }),
    );
    if result.is_some_and(|result| result["ok"] == true) {
        message(host, recipient, "Pedido de teleporte aceito.");
        message(
            host,
            request.requester,
            "Seu pedido de teleporte foi aceito.",
        );
    } else {
        message(host, recipient, "O teleporte falhou; ninguém foi movido.");
        message(
            host,
            request.requester,
            "O teleporte falhou; tente novamente.",
        );
    }
    true
}

fn deny(host: &HostApiV3, recipient: u128, requester_name: Option<&str>) -> bool {
    let mut pending = requests().lock().unwrap_or_else(|error| error.into_inner());
    let Some(request) = pending.get(&recipient).copied() else {
        message(
            host,
            recipient,
            "Você não tem pedido de teleporte pendente.",
        );
        return true;
    };
    if request.expires_at <= now_ms() {
        pending.remove(&recipient);
        message(host, recipient, "Esse pedido de teleporte expirou.");
        return true;
    }
    if let Some(name) = requester_name {
        let target = snapshot_for(host, recipient, request.requester);
        if !target
            .as_ref()
            .and_then(|value| value["name"].as_str())
            .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
        {
            message(
                host,
                recipient,
                "O pedido pendente não corresponde a esse jogador.",
            );
            return true;
        }
    }
    pending.remove(&recipient);
    drop(pending);
    message(host, recipient, "Pedido de teleporte recusado.");
    message(
        host,
        request.requester,
        "Seu pedido de teleporte foi recusado.",
    );
    true
}

fn back(host: &HostApiV3, player: u128) -> bool {
    let key = format!("back:{player:032x}");
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::PLUGIN_STORAGE_GET,
            "data": {"plugin": STORAGE_ID, "key": key},
        }),
    );
    let Some(location) = result
        .filter(|result| result["ok"] == true)
        .and_then(|result| result.get("value").cloned())
        .filter(|value| !value.is_null())
    else {
        message(
            host,
            player,
            "Você ainda não tem uma posição de morte salva.",
        );
        return true;
    };
    let teleported = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::PLAYER_TELEPORT,
            "data": {
                "dimension": location["dimension"],
                "position": location["position"],
                "yaw": location["yaw"],
                "pitch": location["pitch"],
            },
        }),
    )
    .is_some_and(|result| result["ok"] == true);
    message(
        host,
        player,
        if teleported {
            "Voltando ao local da sua última morte."
        } else {
            "A dimensão da morte não está disponível; não foi possível voltar."
        },
    );
    true
}

pub(super) fn event(host: &HostApiV3, kind: u32, payload: &serde_json::Value) {
    let Some(player) = payload
        .get("player")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| u128::from_str_radix(value, 16).ok())
    else {
        return;
    };
    match kind {
        event::PLAYER_DEATH => {
            if let Some(location) = payload.get("data").and_then(|data| data.get("location")) {
                let _ = call_json(
                    host,
                    player,
                    serde_json::json!({
                        "operation": operation::PLUGIN_STORAGE_SET,
                        "data": {
                            "plugin": STORAGE_ID,
                            "key": format!("back:{player:032x}"),
                            "value": location,
                        },
                    }),
                );
            }
        }
        event::PLAYER_QUIT => {
            if let Some(location) = payload
                .get("data")
                .and_then(|data| data.get("last_location"))
                .filter(|location| !location.is_null())
            {
                let _ = call_json(
                    host,
                    player,
                    serde_json::json!({
                        "operation": operation::PLUGIN_STORAGE_SET,
                        "data": {
                            "plugin": STORAGE_ID,
                            "key": format!("back:{player:032x}"),
                            "value": location,
                        },
                    }),
                );
            }
            requests()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retain(|recipient, request| *recipient != player && request.requester != player);
        }
        _ => {}
    }
}

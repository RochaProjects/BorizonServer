//! World utilities built on generic dimension, gamerule and location calls.

use super::{player_snapshot, storage_get, storage_set, teleport, valid_name};
use crate::plugin_api::{call_json, message};
use horizon_server_plugin_api::{HostApiV3, operation};

pub(super) fn register_commands(host: &HostApiV3) {
    for usage in [
        "mundo",
        "mundo listar",
        "mundo criar <id:word> <preset:word>",
        "mundo tp <id:word>",
        "mundo gamerule <id:word> <regra:word> <valor:greedy_string>",
        "mundo apagar <id:word>",
        "mundo confirmar <token:word>",
        "mundo reset <categoria:word>",
        "mundo reset confirmar <token:word>",
        "mundo renomear <id:word> <nome:greedy_string>",
    ] {
        let permission = if usage.contains("criar")
            || usage.contains("apagar")
            || usage.contains("confirmar")
            || usage.contains("reset")
            || usage.contains("gamerule")
            || usage.contains("renomear")
        {
            Some("auraessentials.mundo.admin")
        } else {
            None
        };
        let mut data = serde_json::json!({"usage": usage, "default_level": if permission.is_some() { 2 } else { 0 }});
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

pub(super) fn command(host: &HostApiV3, player: u128, command: &str) -> bool {
    let args: Vec<_> = command.split_whitespace().collect();
    if args.first() != Some(&"mundo") {
        return false;
    }
    match args.as_slice() {
        ["mundo"] | ["mundo", "listar"] => list(host, player),
        ["mundo", "criar", id, preset] => create(host, player, id, preset),
        ["mundo", "tp", id] => go_to(host, player, id),
        ["mundo", "gamerule", id, rule, value] => gamerule(host, player, id, rule, value),
        ["mundo", "apagar", id] => prepare_delete(host, player, id),
        ["mundo", "confirmar", token] => delete(host, player, token),
        ["mundo", "reset", category] => prepare_reset(host, player, category),
        ["mundo", "reset", "confirmar", token] => reset(host, player, token),
        ["mundo", "renomear", id, name] => rename(host, player, id, name),
        _ => {
            message(
                host,
                player,
                "Uso: /mundo listar | criar <id> <overworld|nether|end|flat> | tp <id> | apagar <id> | gamerule <id> <regra> <valor> | reset <stats|advancements|playerdata|all>.",
            );
            true
        }
    }
}

fn list(host: &HostApiV3, player: u128) -> bool {
    let result = call_json(
        host,
        player,
        serde_json::json!({"operation": operation::WORLD_DIMENSIONS_LIST, "data": {}}),
    );
    let ids = result
        .and_then(|value| value["dimensions"].as_array().cloned())
        .unwrap_or_default();
    let aliases = storage_get(host, "world:aliases").unwrap_or_default();
    let names = ids
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(|id| {
            aliases
                .get(id)
                .and_then(serde_json::Value::as_str)
                .map_or_else(|| id.to_owned(), |alias| format!("{id} ({alias})"))
        })
        .collect::<Vec<_>>();
    message(
        host,
        player,
        if names.is_empty() {
            "Nenhuma dimensão carregada.".to_owned()
        } else {
            format!("Dimensões carregadas: {}", names.join(", "))
        }
        .as_str(),
    );
    true
}

fn create(host: &HostApiV3, player: u128, id: &str, preset: &str) -> bool {
    if !id.contains(':')
        || id.len() > 96
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_/-:".contains(&b))
    {
        message(
            host,
            player,
            "Use um ID namespaced, por exemplo auramundo:mina.",
        );
        return true;
    }
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_DIMENSION_CREATE,
            "data": {"id": id, "preset": preset},
        }),
    );
    message(
        host,
        player,
        if result.as_ref().is_some_and(|value| value["ok"] == true) {
            format!("Dimensão {id} criada com preset {preset}. Use /mundo tp {id} para entrar.")
        } else {
            format!(
                "Não foi possível criar a dimensão: {}.",
                result
                    .as_ref()
                    .and_then(|value| value["error"].as_str())
                    .unwrap_or("falha")
            )
        }
        .as_str(),
    );
    true
}

fn go_to(host: &HostApiV3, player: u128, id: &str) -> bool {
    let aliases = storage_get(host, "world:aliases").unwrap_or_default();
    let id = aliases
        .as_object()
        .and_then(|aliases| {
            aliases.iter().find(|(_, alias)| {
                alias
                    .as_str()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(id))
            })
        })
        .map_or(id, |(id, _)| id.as_str());
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui consultar sua posição.");
        return true;
    };
    // Spawn at the current coordinates. The server's dimension transfer hook
    // resolves the destination level; no unsafe assumption about spawn height.
    let target = serde_json::json!({
        "dimension": id,
        "position": snapshot["position"],
        "yaw": snapshot["yaw"],
        "pitch": snapshot["pitch"],
    });
    teleport(host, player, &target, &format!("Teleportado para {id}."));
    true
}

fn gamerule(host: &HostApiV3, player: u128, id: &str, rule: &str, text: &str) -> bool {
    let value = match text.parse::<bool>() {
        Ok(value) => serde_json::json!(value),
        Err(_) => match text.parse::<i32>() {
            Ok(value) => serde_json::json!(value),
            Err(_) => {
                message(
                    host,
                    player,
                    "O valor precisa ser true, false ou um número inteiro.",
                );
                return true;
            }
        },
    };
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_CUSTOM_GAMERULE_SET,
            "data": {"dimension": id, "rule": rule, "value": value},
        }),
    );
    message(
        host,
        player,
        if result.as_ref().is_some_and(|value| value["ok"] == true) {
            "Regra personalizada atualizada."
        } else {
            "Não foi possível alterar a regra; confirme a dimensão e o nome da gamerule."
        },
    );
    true
}

fn prepare_delete(host: &HostApiV3, player: u128, id: &str) -> bool {
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_DIMENSION_DELETE_PREPARE,
            "data": {"dimension": id},
        }),
    );
    if result.as_ref().is_some_and(|value| value["ok"] == true) {
        let token = result
            .as_ref()
            .and_then(|value| value["token"].as_str())
            .unwrap_or_default();
        message(
            host,
            player,
            &format!(
                "Backup obrigatório será criado. Confirme em 60 segundos: /mundo confirmar {token}"
            ),
        );
    } else {
        message(
            host,
            player,
            &format!(
                "Não foi possível preparar remoção: {}.",
                result
                    .as_ref()
                    .and_then(|value| value["error"].as_str())
                    .unwrap_or("falha")
            ),
        );
    }
    true
}

fn delete(host: &HostApiV3, player: u128, token: &str) -> bool {
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_DIMENSION_DELETE_COMMIT,
            "data": {"token": token},
        }),
    );
    message(
        host,
        player,
        if result.as_ref().is_some_and(|value| value["ok"] == true) {
            format!(
                "Dimensão removida; backup em {}.",
                result
                    .as_ref()
                    .and_then(|value| value["backup"].as_str())
                    .unwrap_or("plugins/AuraEssentials/backups")
            )
        } else {
            format!(
                "Remoção cancelada: {}.",
                result
                    .as_ref()
                    .and_then(|value| value["error"].as_str())
                    .unwrap_or("falha")
            )
        }
        .as_str(),
    );
    true
}

fn prepare_reset(host: &HostApiV3, player: u128, category: &str) -> bool {
    let categories: &[&str] = match category {
        "stats" | "estatisticas" => &["stats"],
        "advancements" | "conquistas" => &["advancements"],
        "playerdata" | "dados" => &["playerdata"],
        "all" | "tudo" => &["all"],
        _ => {
            message(
                host,
                player,
                "Categoria: stats, advancements, playerdata ou all.",
            );
            return true;
        }
    };
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_PLAYER_DATA_RESET_PREPARE,
            "data": {"categories": categories},
        }),
    );
    if result.as_ref().is_some_and(|value| value["ok"] == true) {
        let token = result
            .as_ref()
            .and_then(|value| value["token"].as_str())
            .unwrap_or_default();
        message(
            host,
            player,
            &format!(
                "Reset global preparado com backup. Confirme em 60 segundos: /mundo reset confirmar {token}"
            ),
        );
    } else {
        message(
            host,
            player,
            &format!(
                "Reset não preparado: {}.",
                result
                    .as_ref()
                    .and_then(|value| value["error"].as_str())
                    .unwrap_or("falha")
            ),
        );
    }
    true
}

fn reset(host: &HostApiV3, player: u128, token: &str) -> bool {
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_PLAYER_DATA_RESET_COMMIT,
            "data": {"token": token},
        }),
    );
    message(
        host,
        player,
        if result.as_ref().is_some_and(|value| value["ok"] == true) {
            "Dados globais resetados; o backup foi salvo em plugins/AuraEssentials/backups."
                .to_owned()
        } else {
            format!(
                "Reset cancelado: {}.",
                result
                    .as_ref()
                    .and_then(|value| value["error"].as_str())
                    .unwrap_or("falha")
            )
        }
        .as_str(),
    );
    true
}

fn rename(host: &HostApiV3, player: u128, id: &str, name: &str) -> bool {
    if !valid_name(name) {
        message(
            host,
            player,
            "Nome inválido; use letras, números, _ e - (até 32 caracteres).",
        );
        return true;
    }
    let aliases = storage_get(host, "world:aliases")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut aliases: serde_json::Map<String, serde_json::Value> = aliases;
    if aliases.iter().any(|(known_id, alias)| {
        known_id != id
            && alias
                .as_str()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(name))
    }) {
        message(host, player, "Esse nome já está em uso por outra dimensão.");
        return true;
    }
    aliases.insert(id.to_owned(), serde_json::Value::String(name.to_owned()));
    let ok = storage_set(host, "world:aliases", serde_json::Value::Object(aliases));
    message(
        host,
        player,
        if ok {
            "Nome de exibição salvo. O ID interno da dimensão permanece igual."
        } else {
            "Não foi possível salvar o nome."
        },
    );
    true
}

pub(super) fn random_teleport(host: &HostApiV3, player: u128) -> bool {
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui consultar sua posição.");
        return true;
    };
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_LOCATION_FIND_SAFE,
            "data": {"dimension": snapshot["dimension"], "radius": 512, "attempts": 192},
        }),
    );
    if let Some(location) = result
        .filter(|value| value["ok"] == true)
        .and_then(|value| value.get("location").cloned())
    {
        teleport(host, player, &location, "Teleportado para um local seguro.");
    } else {
        message(
            host,
            player,
            "Não achei um local seguro nos chunks já carregados. Tente novamente mais perto de uma área explorada.",
        );
    }
    true
}

//! World utilities built on generic dimension, gamerule and location calls.

use super::{player_snapshot, storage_get, storage_set, teleport, valid_name};
use crate::plugin_api::{call_json, message};
use horizon_server_plugin_api::{HostApiV3, operation};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

static PENDING_DELETE_CONFIRMATIONS: OnceLock<Mutex<HashMap<u128, (String, Instant)>>> =
    OnceLock::new();

fn pending_deletions() -> &'static Mutex<HashMap<u128, (String, Instant)>> {
    PENDING_DELETE_CONFIRMATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn pending_delete_token(player: u128) -> Option<String> {
    let mut pending = pending_deletions()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    pending.retain(|_, (_, expires)| *expires > Instant::now());
    pending.get(&player).map(|(token, _)| token.clone())
}

pub(super) fn register_commands(host: &HostApiV3) {
    for usage in [
        "mundo",
        "mundo listar",
        "mundo criar <nome:word> <preset:word>",
        "mundo tp <nome:word?>",
        "mundo carregar <nome:word?>",
        "mundo descarregar <nome:word?>",
        "mundo spawn [nome:word?]",
        "mundo definirspawn",
        "mundo definirprincipal <nome:word?>",
        "mundo removerprincipal",
        "mundo gamerule <nome:word?> <regra:word> <valor:greedy_string>",
        "mundo apagar <nome:word?>",
        "mundo remover <nome:word?>",
        "mundo confirmar <token:word?>",
        "mundo reset <categoria:word?>",
        "mundo reset <categoria:word?> force",
        "mundo renomear <nome:word?> <novo_nome:greedy_string>",
    ] {
        let permission = if usage.contains("criar")
            || usage.contains("carregar")
            || usage.contains("descarregar")
            || usage.contains("apagar")
            || usage.contains("mundo remover")
            || usage.contains("confirmar")
            || usage.contains("reset")
            || usage.contains("gamerule")
            || usage.contains("renomear")
            || usage.contains("definirspawn")
            || usage.contains("principal")
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
    if let Some(primary) =
        storage_get(host, "world:primary").and_then(|value| value.as_str().map(str::to_owned))
    {
        let _ = call_json(
            host,
            0,
            serde_json::json!({
                "operation": operation::WORLD_DIMENSION_LOAD,
                "data": {"dimension":primary, "owner":"auraessentials:primary"},
            }),
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
        ["mundo", "criar", name, preset] => create(host, player, name, preset),
        ["mundo", "tp", id] => go_to(host, player, id),
        ["mundo", "carregar", id] => load(host, player, id),
        ["mundo", "descarregar", id] => unload(host, player, id),
        ["mundo", "spawn"] => spawn(host, player, None),
        ["mundo", "spawn", id] => spawn(host, player, Some(id)),
        ["mundo", "definirspawn"] => set_spawn(host, player),
        ["mundo", "definirprincipal", id] => set_primary(host, player, id),
        ["mundo", "removerprincipal"] => clear_primary(host, player),
        ["mundo", "gamerule", id, rule, value] => gamerule(host, player, id, rule, value),
        ["mundo", "apagar", name] | ["mundo", "remover", name] => {
            prepare_delete(host, player, name)
        }
        ["mundo", "confirmar", token] => delete(host, player, token),
        ["mundo", "reset", category] => prepare_reset(host, player, category),
        ["mundo", "reset", category, "force"] => force_reset(host, player, category),
        ["mundo", "renomear", id, name] => rename(host, player, id, name),
        _ => {
            message(
                host,
                player,
                "Uso: /mundo listar | criar <nome> <preset> | tp <nome> | spawn [nome] | definirspawn | definirprincipal <nome> | removerprincipal | remover <nome> | gamerule <nome> <regra> <valor> | reset <conquistas|estatisticas> | reset jogadores force.",
            );
            true
        }
    }
}

pub(super) fn event(host: &HostApiV3, kind: u32, payload: &serde_json::Value) {
    if kind == horizon_server_plugin_api::event::SERVER_TASK {
        run_reset_task(host, payload);
        return;
    }
    if kind != horizon_server_plugin_api::event::PLAYER_JOIN {
        if kind == horizon_server_plugin_api::event::PLAYER_QUIT
            && let Some(player) = payload
                .get("player")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| u128::from_str_radix(value, 16).ok())
        {
            pending_deletions()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&player);
        }
        return;
    }
    let Some(primary) =
        storage_get(host, "world:primary").and_then(|v| v.as_str().map(str::to_owned))
    else {
        return;
    };
    let Some(uuid) = payload
        .get("player")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| u128::from_str_radix(s, 16).ok())
    else {
        return;
    };
    let target = storage_get(host, &format!("world:spawn:{primary}")).or_else(|| {
        player_snapshot(host, uuid).map(|snapshot| {
            serde_json::json!({
                "dimension": primary,
                "position": snapshot["position"],
                "yaw": snapshot["yaw"],
                "pitch": snapshot["pitch"],
            })
        })
    });
    if let Some(target) = target {
        teleport(
            host,
            uuid,
            &target,
            &format!("Dimensão principal: {}.", dimension_name(host, &primary)),
        );
    }
}

fn list(host: &HostApiV3, player: u128) -> bool {
    let result = call_json(
        host,
        player,
        serde_json::json!({"operation": operation::WORLD_DIMENSIONS_LIST, "data": {}}),
    );
    if !result.as_ref().is_some_and(|value| value["ok"] == true) {
        message(
            host,
            player,
            "Não foi possível consultar o estado das dimensões agora.",
        );
        return true;
    }
    let ids = result
        .as_ref()
        .and_then(|value| value["dimensions"].as_array().cloned())
        .unwrap_or_default();
    let loaded: Vec<String> = result
        .as_ref()
        .and_then(|value| value["loaded_dimensions"].as_array())
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect();
    let names = ids
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(|id| {
            let state = if loaded.iter().any(|item| item == id) {
                "carregado"
            } else {
                "descarregado"
            };
            let name = dimension_name(host, id);
            let label = if name == id {
                id.to_owned()
            } else {
                format!("{id} ({name})")
            };
            format!("{label} [{state}]")
        })
        .collect::<Vec<_>>();
    message(
        host,
        player,
        if names.is_empty() {
            "Nenhuma dimensão criada.".to_owned()
        } else {
            format!("Dimensões: {}", names.join(", "))
        }
        .as_str(),
    );
    true
}

fn create(host: &HostApiV3, player: u128, name: &str, preset: &str) -> bool {
    let (id, alias) = if valid_name(name) && !name.contains(':') {
        (
            format!("minecraft:{}", name.to_ascii_lowercase()),
            Some(name),
        )
    } else {
        message(
            host,
            player,
            "Use apenas o nome da dimensão, sem namespace: letras, números, _ ou - (até 32 caracteres).",
        );
        return true;
    };
    if let Some(alias) = alias
        && (reserved_dimension_name(alias)
            || storage_get(host, "world:aliases")
                .and_then(|value| value.as_object().cloned())
                .is_some_and(|aliases| {
                    aliases.iter().any(|(known_id, value)| {
                        known_id != &id
                            && value
                                .as_str()
                                .is_some_and(|known| known.eq_ignore_ascii_case(alias))
                    })
                }))
    {
        message(
            host,
            player,
            "Esse nome já está sendo usado por outra dimensão.",
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
    if result.as_ref().is_some_and(|value| value["ok"] == true) {
        if let Some(alias) = alias {
            let mut aliases = storage_get(host, "world:aliases")
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            aliases.insert(id.clone(), serde_json::Value::String(alias.to_owned()));
            if !storage_set(host, "world:aliases", serde_json::Value::Object(aliases)) {
                message(
                    host,
                    player,
                    &format!("Dimensão criada como {id}, mas não consegui salvar o nome {alias}."),
                );
                return true;
            }
        }
        let label = alias.unwrap_or(&id);
        message(
            host,
            player,
            &format!("Dimensão {label} criada. Use /mundo tp {label} para entrar."),
        );
    } else {
        message(
            host,
            player,
            &format!(
                "Não foi possível criar a dimensão: {}.",
                result
                    .as_ref()
                    .and_then(|value| value["error"].as_str())
                    .unwrap_or("falha")
            ),
        );
    }
    true
}

fn dimension_name(host: &HostApiV3, id: &str) -> String {
    if let Some(alias) = storage_get(host, "world:aliases").and_then(|value| {
        value
            .get(id)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }) {
        return alias;
    }
    match id {
        "minecraft:overworld" => "overworld".into(),
        "minecraft:the_nether" => "nether".into(),
        "minecraft:the_end" => "end".into(),
        _ => id.strip_prefix("minecraft:").unwrap_or(id).to_owned(),
    }
}

fn reserved_dimension_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "world" | "overworld" | "nether" | "end"
    )
}

pub(super) fn dimension_names(host: &HostApiV3, player: u128) -> Vec<String> {
    call_json(
        host,
        player,
        serde_json::json!({"operation":operation::WORLD_DIMENSIONS_LIST,"data":{}}),
    )
    .and_then(|value| value["dimensions"].as_array().cloned())
    .unwrap_or_default()
    .iter()
    .filter_map(serde_json::Value::as_str)
    .map(|id| dimension_name(host, id))
    .collect()
}

fn go_to(host: &HostApiV3, player: u128, id: &str) -> bool {
    let id = resolve_dimension(host, id);
    if !dimension_exists(host, player, &id) {
        message(
            host,
            player,
            "Dimensão não encontrada. Use /mundo listar para ver as dimensões existentes.",
        );
        return true;
    }
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui consultar sua posição.");
        return true;
    };
    let target = storage_get(host, &format!("world:spawn:{id}")).unwrap_or_else(|| {
        serde_json::json!({
            "dimension": id,
            "position": snapshot["position"],
            "yaw": snapshot["yaw"],
            "pitch": snapshot["pitch"],
        })
    });
    let has_spawn = storage_get(host, &format!("world:spawn:{id}")).is_some();
    let display = dimension_name(host, &id);
    let notice = if has_spawn {
        format!("Teleportado para {display}.")
    } else {
        format!("Primeira entrada em {display}. Defina o spawn com /mundo definirspawn.")
    };
    teleport(host, player, &target, &notice);
    true
}

fn resolve_dimension(host: &HostApiV3, name: &str) -> String {
    if name.contains(':') {
        return name.to_owned();
    }
    match name.to_ascii_lowercase().as_str() {
        "overworld" | "world" => return "minecraft:overworld".into(),
        "nether" => return "minecraft:the_nether".into(),
        "end" => return "minecraft:the_end".into(),
        _ => {}
    }
    let alias = storage_get(host, "world:aliases")
        .and_then(|v| v.as_object().cloned())
        .and_then(|aliases| {
            aliases
                .into_iter()
                .find(|(_, alias)| alias.as_str().is_some_and(|a| a.eq_ignore_ascii_case(name)))
                .map(|(id, _)| id)
        });
    if let Some(id) = alias {
        return id;
    }
    let known = call_json(
        host,
        0,
        serde_json::json!({"operation":operation::WORLD_DIMENSIONS_LIST,"data":{}}),
    )
    .and_then(|value| value["dimensions"].as_array().cloned())
    .unwrap_or_default();
    known
        .iter()
        .filter_map(serde_json::Value::as_str)
        .find(|id| dimension_name(host, id).eq_ignore_ascii_case(name))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("minecraft:{}", name.to_ascii_lowercase()))
}

fn dimension_exists(host: &HostApiV3, player: u128, id: &str) -> bool {
    call_json(
        host,
        player,
        serde_json::json!({"operation":operation::WORLD_DIMENSIONS_LIST,"data":{}}),
    )
    .and_then(|value| value["dimensions"].as_array().cloned())
    .is_some_and(|dimensions| dimensions.iter().any(|value| value.as_str() == Some(id)))
}

fn load(host: &HostApiV3, player: u128, name: &str) -> bool {
    let id = resolve_dimension(host, name);
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_DIMENSION_LOAD,
            "data": {"dimension":id, "owner":"auraessentials:manual"},
        }),
    );
    message(
        host,
        player,
        if result.as_ref().is_some_and(|value| value["ok"] == true) {
            format!(
                "Dimensão {} carregada e mantida aberta.",
                dimension_name(host, &id)
            )
        } else {
            format!(
                "Não foi possível carregar a dimensão: {}.",
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

fn unload(host: &HostApiV3, player: u128, name: &str) -> bool {
    let id = resolve_dimension(host, name);
    if storage_get(host, "world:primary")
        .and_then(|value| value.as_str().map(str::to_owned))
        .as_deref()
        == Some(&id)
    {
        message(
            host,
            player,
            "Essa é a dimensão principal. Remova-a como principal antes de descarregar.",
        );
        return true;
    }
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_DIMENSION_UNLOAD,
            "data": {"dimension":id, "owner":"auraessentials:manual"},
        }),
    );
    message(
        host,
        player,
        if result.as_ref().is_some_and(|value| value["ok"] == true) {
            format!("Dimensão {} descarregada.", dimension_name(host, &id))
        } else {
            "Não foi possível descarregar: ela pode estar em uso por jogadores ou tickets."
                .to_owned()
        }
        .as_str(),
    );
    true
}

fn set_spawn(host: &HostApiV3, player: u128) -> bool {
    let Some(snapshot) = player_snapshot(host, player) else {
        message(host, player, "Não consegui consultar sua posição.");
        return true;
    };
    let id = snapshot["dimension"]
        .as_str()
        .unwrap_or("minecraft:overworld");
    if storage_set(
        host,
        &format!("world:spawn:{id}"),
        serde_json::json!({"dimension":id,"position":snapshot["position"],"yaw":snapshot["yaw"],"pitch":snapshot["pitch"]}),
    ) {
        message(
            host,
            player,
            &format!("Spawn da dimensão {id} definido na sua posição."),
        );
    } else {
        message(host, player, "Não foi possível salvar o spawn da dimensão.");
    }
    true
}

fn spawn(host: &HostApiV3, player: u128, requested: Option<&str>) -> bool {
    let current = player_snapshot(host, player);
    let id = requested
        .map(|name| resolve_dimension(host, name))
        .or_else(|| {
            current
                .as_ref()
                .and_then(|v| v["dimension"].as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| "minecraft:overworld".into());
    match storage_get(host, &format!("world:spawn:{id}")) {
        Some(target) => teleport(
            host,
            player,
            &target,
            &format!("Teleportado ao spawn de {id}."),
        ),
        None if requested.is_some() => {
            let _ = go_to(host, player, &id);
        }
        None => message(
            host,
            player,
            "O spawn desta dimensão ainda não foi definido.",
        ),
    }
    true
}

fn set_primary(host: &HostApiV3, player: u128, name: &str) -> bool {
    let id = resolve_dimension(host, name);
    if !dimension_exists(host, player, &id) {
        message(host, player, "Dimensão não encontrada.");
        return true;
    }
    let loaded = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_DIMENSION_LOAD,
            "data": {"dimension":id, "owner":"auraessentials:primary"},
        }),
    );
    if !loaded.as_ref().is_some_and(|value| value["ok"] == true) {
        message(
            host,
            player,
            "Não foi possível carregar a dimensão para torná-la principal.",
        );
        return true;
    }
    let previous =
        storage_get(host, "world:primary").and_then(|value| value.as_str().map(str::to_owned));
    if storage_set(host, "world:primary", serde_json::json!(id)) {
        if let Some(previous) = previous.filter(|previous| previous != &id) {
            let _ = call_json(
                host,
                player,
                serde_json::json!({
                    "operation":operation::WORLD_DIMENSION_UNLOAD,
                    "data":{"dimension":previous,"owner":"auraessentials:primary"},
                }),
            );
        }
        message(
            host,
            player,
            &format!(
                "Dimensão principal definida: {}. Ela será carregada ao iniciar o servidor.",
                dimension_name(host, &id)
            ),
        );
    } else {
        message(
            host,
            player,
            "Não foi possível salvar a dimensão principal.",
        );
    }
    true
}

fn clear_primary(host: &HostApiV3, player: u128) -> bool {
    if let Some(id) =
        storage_get(host, "world:primary").and_then(|value| value.as_str().map(str::to_owned))
    {
        let _ = call_json(
            host,
            player,
            serde_json::json!({
                "operation":operation::WORLD_DIMENSION_UNLOAD,
                "data":{"dimension":id,"owner":"auraessentials:primary"},
            }),
        );
    }
    let result = call_json(
        host,
        player,
        serde_json::json!({"operation":operation::PLUGIN_STORAGE_DELETE,"data":{"plugin":"AuraEssentials","key":"world:primary"}}),
    );
    message(
        host,
        player,
        if result.is_some_and(|v| v["ok"] == true) {
            "Dimensão principal removida."
        } else {
            "Não havia dimensão principal definida."
        },
    );
    true
}

fn gamerule(host: &HostApiV3, player: u128, id: &str, rule: &str, text: &str) -> bool {
    let id = resolve_dimension(host, id);
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
    let id = resolve_dimension(host, id);
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
        if !token.is_empty() {
            pending_deletions()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(
                    player,
                    (token.to_owned(), Instant::now() + Duration::from_secs(60)),
                );
        }
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
    pending_deletions()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&player);
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

fn reset_categories(category: &str) -> Option<&'static [&'static str]> {
    match category {
        "stats" | "estatisticas" => Some(&["stats"]),
        "advancements" | "conquistas" => Some(&["advancements"]),
        "playerdata" | "dados" => Some(&["playerdata"]),
        "jogadores" | "all" | "tudo" => Some(&["all"]),
        _ => None,
    }
}

fn prepare_reset(host: &HostApiV3, player: u128, category: &str) -> bool {
    let Some(categories) = reset_categories(category) else {
        message(
            host,
            player,
            "Categoria: conquistas, estatisticas ou jogadores force.",
        );
        return true;
    };
    if categories
        .iter()
        .any(|value| *value == "all" || *value == "playerdata")
    {
        message(
            host,
            player,
            "Para apagar os dados de todos os jogadores sem backup, use /mundo reset jogadores force.",
        );
        return true;
    }
    let Some(token) = prepare_reset_plan(host, player, categories) else {
        return true;
    };
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_PLAYER_DATA_RESET_COMMIT,
            "data": {"token": token, "backup": true},
        }),
    );
    if result.as_ref().is_some_and(|value| value["ok"] == true) {
        message(
            host,
            player,
            "Conquistas/estatísticas globais resetadas; o backup foi salvo em plugins/AuraEssentials/backups.",
        );
    } else {
        message(
            host,
            player,
            &format!(
                "Reset cancelado: {}.",
                result
                    .as_ref()
                    .and_then(|value| value["error"].as_str())
                    .unwrap_or("falha")
            ),
        );
    }
    true
}

fn force_reset(host: &HostApiV3, player: u128, category: &str) -> bool {
    let Some(categories) = reset_categories(category) else {
        message(
            host,
            player,
            "Categoria: jogadores. Use /mundo reset jogadores force.",
        );
        return true;
    };
    if !categories
        .iter()
        .any(|value| *value == "all" || *value == "playerdata")
    {
        message(
            host,
            player,
            "force é usado para apagar dados de jogadores: /mundo reset jogadores force.",
        );
        return true;
    }
    let Some(token) = prepare_reset_plan(host, player, categories) else {
        return true;
    };
    let scheduled = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::SCHEDULER_SCHEDULE,
            "data": {"plugin":"AuraEssentials", "delay_ticks":1, "data":{"kind":"world_player_data_reset", "token":token}},
        }),
    );
    let Some(task_id) = scheduled
        .filter(|value| value["ok"] == true)
        .and_then(|value| value["task_id"].as_str().map(str::to_owned))
    else {
        message(
            host,
            player,
            "Reset não iniciado: falhou o agendamento seguro.",
        );
        return true;
    };
    let started = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::SERVER_MAINTENANCE_BEGIN,
            "data": {"message":"Server maintenance in progress"},
        }),
    );
    if !started.as_ref().is_some_and(|value| value["ok"] == true) {
        let _ = call_json(
            host,
            player,
            serde_json::json!({"operation":operation::SCHEDULER_CANCEL,"data":{"task_id":task_id}}),
        );
        message(
            host,
            player,
            "Reset não iniciado: não foi possível entrar em manutenção.",
        );
        return true;
    }
    message(
        host,
        player,
        "Reset iniciado: jogadores expulsos, novas entradas bloqueadas e dados apagados sem backup.",
    );
    true
}

fn prepare_reset_plan(host: &HostApiV3, player: u128, categories: &[&str]) -> Option<String> {
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_PLAYER_DATA_RESET_PREPARE,
            "data": {"categories": categories},
        }),
    );
    if let Some(token) = result
        .as_ref()
        .filter(|value| value["ok"] == true)
        .and_then(|value| value["token"].as_str())
    {
        return Some(token.to_owned());
    }
    message(
        host,
        player,
        match result.as_ref().and_then(|value| value["error"].as_str()) {
            Some("data_not_found") => "Não há arquivos dessa categoria para resetar.",
            _ => "Não foi possível preparar o reset global.",
        },
    );
    None
}

fn run_reset_task(host: &HostApiV3, payload: &serde_json::Value) {
    let task = &payload["data"];
    if task["plugin_id"] != "AuraEssentials" || task["data"]["kind"] != "world_player_data_reset" {
        return;
    }
    if task["task_id"].as_str().is_none() {
        return;
    }
    let Some(token) = task["data"]["token"].as_str() else {
        log(
            host,
            "ERROR",
            "Reset force sem token na tarefa; manutenção permanece ativa.",
        );
        return;
    };
    let status = call_json(
        host,
        0,
        serde_json::json!({"operation":operation::SERVER_MAINTENANCE_STATUS,"data":{}}),
    );
    let Some(online) = status
        .filter(|value| value["ok"] == true)
        .and_then(|value| value["online"].as_u64())
    else {
        log(
            host,
            "ERROR",
            "Não foi possível verificar sessões; manutenção permanece ativa.",
        );
        return;
    };
    if online > 0 {
        let scheduled = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::SCHEDULER_SCHEDULE,"data":{"plugin":"AuraEssentials","delay_ticks":1,"data":{"kind":"world_player_data_reset","token":token}}}),
        );
        let Some(_next_id) = scheduled
            .filter(|value| value["ok"] == true)
            .and_then(|value| value["task_id"].as_str().map(str::to_owned))
        else {
            log(
                host,
                "ERROR",
                "Ainda há jogadores conectados e não foi possível reagendar; manutenção permanece ativa.",
            );
            return;
        };
        return;
    }
    let result = call_json(
        host,
        0,
        serde_json::json!({"operation":operation::WORLD_PLAYER_DATA_RESET_COMMIT,"data":{"token":token,"backup":false}}),
    );
    if !result.as_ref().is_some_and(|value| value["ok"] == true) {
        let reason = result
            .as_ref()
            .and_then(|value| value["error"].as_str())
            .unwrap_or("erro desconhecido");
        let ended = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::SERVER_MAINTENANCE_END,"data":{}}),
        );
        log(
            host,
            "ERROR",
            &format!(
                "Reset force falhou ({reason}); {}.",
                if ended.as_ref().is_some_and(|value| value["ok"] == true) {
                    "manutenção encerrada"
                } else {
                    "manutenção permanece ativa"
                }
            ),
        );
        return;
    }
    let ended = call_json(
        host,
        0,
        serde_json::json!({"operation":operation::SERVER_MAINTENANCE_END,"data":{}}),
    );
    if !ended.as_ref().is_some_and(|value| value["ok"] == true) {
        log(
            host,
            "ERROR",
            "Dados resetados, mas não foi possível encerrar manutenção; novas entradas continuam bloqueadas.",
        );
        return;
    }
    log(
        host,
        "INFO",
        "Reset global de dados dos jogadores concluído sem backup; manutenção encerrada.",
    );
}

fn log(host: &HostApiV3, level: &str, message: &str) {
    let _ = call_json(
        host,
        0,
        serde_json::json!({"operation":operation::SERVER_LOG,"data":{"level":level,"message":message}}),
    );
}

fn rename(host: &HostApiV3, player: u128, id: &str, name: &str) -> bool {
    if !valid_name(name) || reserved_dimension_name(name) {
        message(
            host,
            player,
            "Nome inválido; use letras, números, _ e - (até 32 caracteres).",
        );
        return true;
    }
    let id = resolve_dimension(host, id);
    let aliases = storage_get(host, "world:aliases")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut aliases: serde_json::Map<String, serde_json::Value> = aliases;
    if aliases.iter().any(|(known_id, alias)| {
        known_id != &id
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

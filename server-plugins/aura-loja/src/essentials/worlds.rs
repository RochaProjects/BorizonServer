//! World utilities built on generic dimension, gamerule and location calls.

use super::{player_snapshot, storage_get, storage_set, teleport, valid_name};
use crate::plugin_api::{call_json, get_public_http_bytes, message};
use horizon_server_plugin_api::{HostApiV3, operation};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
use zip::ZipArchive;

enum ImportState {
    Running,
    Ready(PathBuf),
    Failed(String),
}
static IMPORTS: OnceLock<Mutex<HashMap<u128, ImportState>>> = OnceLock::new();
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
        "mundo gamerule <nome:word?> <regra:word> <valor:word>",
        "mundo remover <nome:word?>",
        "mundo reset <categoria:word?>",
        "mundo reset <categoria:word?> force",
        "mundo renomear <nome:word?> <novo_nome:word>",
        "mundo importar <nome:word> <url:text>",
    ] {
        let permission = if usage.contains("criar")
            || usage.contains("carregar")
            || usage.contains("descarregar")
            || usage.contains("mundo remover")
            || usage.contains("reset")
            || usage.contains("gamerule")
            || usage.contains("renomear")
            || usage.contains("importar")
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
        ["mundo", "remover", name] => delete(host, player, name),
        ["mundo", "reset", category] => prepare_reset(host, player, category),
        ["mundo", "reset", category, "force"] => force_reset(host, player, category),
        ["mundo", "renomear", id, name] => rename(host, player, id, name),
        ["mundo", "importar", name, url @ ..] if !url.is_empty() => {
            start_import(host, player, name, &url.join(" "))
        }
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
        run_import_task(host, payload);
        return;
    }
    if kind != horizon_server_plugin_api::event::PLAYER_JOIN {
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

fn start_import(host: &HostApiV3, player: u128, name: &str, url: &str) -> bool {
    if !valid_name(name) || reserved_dimension_name(name) || name.contains(':') {
        message(host, player, "Nome de dimensão inválido ou reservado.");
        return true;
    }
    let url = url.trim().to_owned();
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048 {
        message(host, player, "A URL precisa ser HTTP ou HTTPS.");
        return true;
    }
    let id = format!("minecraft:{}", name.to_ascii_lowercase());
    if dimension_exists(host, player, &id) {
        message(host, player, "Já existe uma dimensão com esse nome.");
        return true;
    }
    let jobs = IMPORTS.get_or_init(Default::default);
    let mut jobs = jobs.lock().unwrap_or_else(|e| e.into_inner());
    if matches!(jobs.get(&player), Some(ImportState::Running)) {
        message(host, player, "Você já tem uma importação em andamento.");
        return true;
    }
    jobs.insert(player, ImportState::Running);
    drop(jobs);
    let scheduled = call_json(
        host,
        player,
        serde_json::json!({"operation":operation::SCHEDULER_SCHEDULE,"data":{"plugin":"AuraEssentials","delay_ticks":20,"period_ticks":20,"data":{"kind":"world_import_poll","player":format!("{player:032x}"),"id":id}}}),
    );
    if !scheduled.as_ref().is_some_and(|v| v["ok"] == true) {
        IMPORTS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player);
        message(
            host,
            player,
            "Não foi possível agendar a validação da importação.",
        );
        return true;
    }
    let world_name = name.to_owned();
    std::thread::spawn(move || {
        let result = download_world(&url, &world_name);
        let mut jobs = IMPORTS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        jobs.insert(
            player,
            result.map_or_else(ImportState::Failed, ImportState::Ready),
        );
    });
    message(
        host,
        player,
        "Download e validação iniciados em background; o servidor continua respondendo.",
    );
    true
}

fn download_world(url: &str, name: &str) -> Result<PathBuf, String> {
    const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
    const MAX_EXTRACTED: u64 = 2 * 1024 * 1024 * 1024;
    let bytes =
        get_public_http_bytes(url, MAX_ARCHIVE).map_err(|e| format!("Download falhou: {e}"))?;
    let mut archive = ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Arquivo ZIP inválido: {e}"))?;
    let candidates: Vec<String> = archive
        .file_names()
        .filter(|path| {
            path.ends_with("level.dat")
                && !path.ends_with("level.dat_old")
                && path.split('/').count() <= 4
        })
        .map(|path| path.trim_end_matches("level.dat").to_owned())
        .collect();
    if candidates.len() != 1 {
        return Err(if candidates.is_empty() {
            "O ZIP não contém level.dat em uma pasta de mundo válida.".into()
        } else {
            "O ZIP contém mais de um mundo; envie um arquivo com apenas um mundo.".into()
        });
    }
    let prefix = &candidates[0];
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |v| v.as_millis());
    let root = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("plugins/.horizon-world-imports");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let stem = format!("{}-{now}", name.to_ascii_lowercase());
    let mut destination = root.join(&stem);
    let mut suffix = 0;
    while destination.exists() {
        suffix += 1;
        destination = root.join(format!("{stem}-{suffix}"));
    }
    fs::create_dir(&destination).map_err(|e| e.to_string())?;
    let extracted = (|| -> Result<(), String> {
        let mut total = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            let raw = entry.name();
            let Some(relative) = raw.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            let rel = Path::new(relative);
            if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
                return Err("O ZIP contém caminho inseguro.".into());
            }
            let target = destination.join(enclosed.strip_prefix(prefix).unwrap_or(rel));
            if entry.is_dir() {
                fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                continue;
            }
            let Some(parent) = target.parent() else {
                return Err("Caminho inválido no ZIP.".into());
            };
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let mut file = fs::File::create(&target).map_err(|e| e.to_string())?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = entry.read(&mut buffer).map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                total = total.saturating_add(read as u64);
                if total > MAX_EXTRACTED {
                    return Err("O conteúdo extraído excede 2 GB.".into());
                }
                file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })();
    if let Err(error) = extracted {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    if !destination.join("level.dat").is_file() {
        let _ = fs::remove_dir_all(&destination);
        return Err("O ZIP não contém level.dat no diretório selecionado.".into());
    }
    Ok(destination)
}

fn run_import_task(host: &HostApiV3, payload: &serde_json::Value) {
    if payload["data"]["kind"] != "world_import_poll" {
        return;
    }
    let Some(player) = payload["data"]["player"]
        .as_str()
        .and_then(|s| u128::from_str_radix(s, 16).ok())
    else {
        return;
    };
    let Some(id) = payload["data"]["id"].as_str().map(str::to_owned) else {
        return;
    };
    let state = IMPORTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&player);
    let Some(state) = state else { return };
    let Some(task) = payload["task_id"]
        .as_str()
        .and_then(|v| v.parse::<u64>().ok())
    else {
        return;
    };
    let finish = |host: &HostApiV3| {
        let _ = call_json(
            host,
            player,
            serde_json::json!({"operation":operation::SCHEDULER_CANCEL,"data":{"task_id":task}}),
        );
    };
    match state {
        ImportState::Running => {
            IMPORTS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(player, ImportState::Running);
        }
        ImportState::Failed(error) => {
            finish(host);
            message(host, player, &format!("Importação cancelada: {error}"));
        }
        ImportState::Ready(path) => {
            let result = call_json(
                host,
                player,
                serde_json::json!({"operation":operation::WORLD_DIMENSION_IMPORT,"data":{"id":id,"source_dir":path}}),
            );
            let _ = fs::remove_dir_all(&path);
            finish(host);
            if result.as_ref().is_some_and(|v| v["ok"] == true) {
                let alias = id.strip_prefix("minecraft:").unwrap_or(&id);
                let mut aliases = storage_get(host, "world:aliases")
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                aliases.insert(id.clone(), serde_json::Value::String(alias.into()));
                let _ = storage_set(host, "world:aliases", serde_json::Value::Object(aliases));
                message(
                    host,
                    player,
                    &format!(
                        "Mundo importado como dimensão {alias}. Use /mundo tp {alias} para visitar."
                    ),
                );
            } else {
                message(
                    host,
                    player,
                    &format!(
                        "Mundo baixado, mas a validação/importação falhou: {}.",
                        result
                            .as_ref()
                            .and_then(|v| v["error"].as_str())
                            .unwrap_or("falha")
                    ),
                );
            }
        }
    }
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
    let target = storage_get(host, &format!("world:spawn:{id}"));
    if snapshot["dimension"].as_str() == Some(id.as_str()) && target.is_none() {
        message(
            host,
            player,
            "Você já está nessa dimensão. Defina o ponto de chegada com /mundo definirspawn.",
        );
        return true;
    }
    let target = target.unwrap_or_else(|| {
        serde_json::json!({
            "dimension": id,
            "position": [0.5, 100.0, 0.5],
            "yaw": snapshot["yaw"],
            "pitch": snapshot["pitch"],
        })
    });
    let display = dimension_name(host, &id);
    let notice = if storage_get(host, &format!("world:spawn:{id}")).is_some() {
        format!("Teleportado para {display}.")
    } else {
        format!("Primeira entrada em {display}. Defina o ponto de chegada com /mundo definirspawn.")
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

fn delete(host: &HostApiV3, player: u128, id: &str) -> bool {
    let id = resolve_dimension(host, id);
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::WORLD_DIMENSION_DELETE,
            "data": {"dimension": id},
        }),
    );
    let text = if result.as_ref().is_some_and(|value| value["ok"] == true) {
        if storage_get(host, "world:primary")
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some(id.as_str())
        {
            let _ = call_json(
                host,
                player,
                serde_json::json!({"operation": operation::PLUGIN_STORAGE_DELETE, "data": {"plugin": "AuraEssentials", "key": "world:primary"}}),
            );
        }
        let _ = call_json(
            host,
            player,
            serde_json::json!({"operation": operation::PLUGIN_STORAGE_DELETE, "data": {"plugin": "AuraEssentials", "key": format!("world:spawn:{id}")}}),
        );
        if let Some(mut aliases) =
            storage_get(host, "world:aliases").and_then(|value| value.as_object().cloned())
        {
            aliases.remove(&id);
            let _ = storage_set(host, "world:aliases", serde_json::Value::Object(aliases));
        }
        format!("Dimensão {} removida.", dimension_name(host, &id))
    } else {
        format!(
            "Não foi possível remover a dimensão: {}.",
            result
                .as_ref()
                .and_then(|value| value["error"].as_str())
                .unwrap_or("falha")
        )
    };
    message(host, player, &text);
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

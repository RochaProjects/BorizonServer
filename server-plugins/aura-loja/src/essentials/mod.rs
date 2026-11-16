//! Comandos essenciais que usam apenas operações genéricas do SDK público.

use crate::plugin_api::{add_suggestions, call_json, completion_input, message};
use horizon_server_plugin_api::{HostApiV3, event, operation};

const STORAGE_ID: &str = "AuraEssentials";

mod holograms;
mod npcs;
mod resource_packs;
mod roles;
mod scoreboard;
mod teleport;
mod worlds;

pub fn ensure_resource_pack_config() {
    resource_packs::ensure_default_file();
}

pub fn register_commands(host: &HostApiV3) {
    scoreboard::start(host);
    roles::register_commands(host);
    holograms::register_commands(host);
    npcs::register_commands(host);
    npcs::ensure_default_import(host);
    teleport::register_commands(host);
    worlds::register_commands(host);
    for (usage, level, permission) in [
        ("sethome <nome:word>", 0, None),
        ("home [nome:word?]", 0, None),
        ("homes", 0, None),
        ("delhome <nome:word?>", 0, None),
        ("setwarp <nome:word>", 2, Some("auraessentials.warp.admin")),
        ("warp [nome:word?]", 0, None),
        ("warps", 0, None),
        ("delwarp <nome:word?>", 2, Some("auraessentials.warp.admin")),
        ("rtp", 0, None),
        (
            "invsee <jogador:player_name>",
            2,
            Some("auraessentials.inventario.ver"),
        ),
        (
            "endersee <jogador:player_name>",
            2,
            Some("auraessentials.inventario.ver"),
        ),
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
    if roles::command(host, player, command) {
        return true;
    }
    if holograms::command(host, player, command) {
        return true;
    }
    if npcs::command(host, player, command) {
        return true;
    }
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
        "warp" if args.len() <= 1 => args.first().map_or_else(
            || open_warps(host, player),
            |name| teleport_warp(host, player, name),
        ),
        "warps" if args.is_empty() => open_warps(host, player),
        "delwarp" if args.len() == 1 => delete_warp(host, player, args[0]),
        "rtp" if args.is_empty() => worlds::random_teleport(host, player),
        "invsee" | "endersee" if args.len() == 1 => {
            open_inventory_view(host, player, args[0], root == "endersee")
        }
        _ => false,
    }
}

pub fn suggestions(host: &HostApiV3, player: u128, line: &str) {
    roles::suggestions(host, player, line);
    holograms::suggestions(host, player, line);
    npcs::suggestions(host, player, line);
    let (words, current) = completion_input(line);
    let (root, args) = words.split_first().map_or(("", &[][..]), |(r, a)| (*r, a));
    if !matches!(
        root,
        "home" | "delhome" | "warp" | "delwarp" | "mundo" | "invsee" | "endersee"
    ) {
        return;
    }
    let mut candidates = Vec::new();
    let snapshot = player_snapshot(host, player);
    let admin = snapshot
        .as_ref()
        .and_then(|snapshot| snapshot["permission_level"].as_u64())
        .is_some_and(|level| level >= 2);
    match (root, args) {
        ("home", []) | ("home", [_]) | ("delhome", []) | ("delhome", [_]) => {
            if let Some(snapshot) = snapshot.as_ref() {
                let prefix = format!("home:{}:", snapshot["uuid"].as_str().unwrap_or_default());
                candidates.extend(
                    storage_list(host, &prefix)
                        .into_iter()
                        .map(|key| key[prefix.len()..].to_owned()),
                );
            }
        }
        ("warp", []) | ("warp", [_]) | ("delwarp", []) | ("delwarp", [_]) => {
            if root == "delwarp" && !admin {
                return;
            }
            candidates.extend(
                storage_list(host, "warp:")
                    .into_iter()
                    .map(|key| key[5..].to_owned()),
            );
        }
        ("mundo", []) => {
            if admin {
                candidates.extend(
                    [
                        "listar",
                        "criar",
                        "tp",
                        "carregar",
                        "descarregar",
                        "spawn",
                        "definirspawn",
                        "definirprincipal",
                        "removerprincipal",
                        "gamerule",
                        "reset",
                        "renomear",
                        "remover",
                        "importar",
                    ]
                    .map(str::to_owned),
                );
            } else {
                candidates.extend(["listar", "tp", "spawn"].map(str::to_owned));
            }
        }
        ("mundo", ["tp"])
        | ("mundo", ["tp", _])
        | ("mundo", ["carregar"])
        | ("mundo", ["carregar", _])
        | ("mundo", ["descarregar"])
        | ("mundo", ["descarregar", _])
        | ("mundo", ["spawn"])
        | ("mundo", ["spawn", _])
        | ("mundo", ["definirprincipal"])
        | ("mundo", ["definirprincipal", _]) => {
            if admin
                || !matches!(
                    args.first().copied(),
                    Some("carregar" | "descarregar" | "definirprincipal")
                )
            {
                candidates.extend(worlds::dimension_names(host, player));
            }
        }
        ("mundo", ["remover"])
        | ("mundo", ["remover", _])
        | ("mundo", ["renomear"])
        | ("mundo", ["renomear", _]) => {
            if admin {
                candidates.extend(worlds::dimension_names(host, player));
            }
        }
        ("mundo", ["gamerule"]) => {
            if admin {
                candidates.extend(worlds::dimension_names(host, player));
            }
        }
        ("mundo", ["gamerule", _]) => {
            if admin {
                candidates.extend(
                    [
                        "keep_inventory",
                        "do_daylight_cycle",
                        "do_mob_spawning",
                        "do_fire_tick",
                        "mob_griefing",
                        "random_tick_speed",
                    ]
                    .map(str::to_owned),
                );
            }
        }
        ("mundo", ["gamerule", _, rule])
            if admin
                && !matches!(
                    *rule,
                    "keep_inventory"
                        | "do_daylight_cycle"
                        | "do_mob_spawning"
                        | "do_fire_tick"
                        | "mob_griefing"
                        | "random_tick_speed"
                ) =>
        {
            candidates.extend(
                [
                    "keep_inventory",
                    "do_daylight_cycle",
                    "do_mob_spawning",
                    "do_fire_tick",
                    "mob_griefing",
                    "random_tick_speed",
                ]
                .map(str::to_owned),
            );
        }
        ("mundo", ["gamerule", _, rule])
            if admin
                && matches!(
                    *rule,
                    "keep_inventory"
                        | "do_daylight_cycle"
                        | "do_mob_spawning"
                        | "do_fire_tick"
                        | "mob_griefing"
                        | "random_tick_speed"
                ) =>
        {
            if *rule == "random_tick_speed" {
                candidates.extend(["0", "1", "3"].map(str::to_owned));
            } else {
                candidates.extend(["true", "false"].map(str::to_owned));
            }
        }
        ("mundo", ["reset"]) => {
            if admin {
                candidates.extend(["conquistas", "estatisticas", "jogadores"].map(str::to_owned));
            }
        }
        ("mundo", ["reset", category])
            if matches!(
                *category,
                "jogadores" | "all" | "tudo" | "playerdata" | "dados"
            ) =>
        {
            if admin {
                candidates.push("force".to_owned());
            }
        }
        ("mundo", ["reset", _]) => {
            if admin {
                candidates.extend(["conquistas", "estatisticas", "jogadores"].map(str::to_owned));
            }
        }
        ("mundo", ["criar", _]) => {
            if admin {
                candidates.extend(["overworld", "nether", "end", "flat"].map(str::to_owned))
            }
        }
        ("invsee", []) | ("invsee", [_]) | ("endersee", []) | ("endersee", [_]) => {
            if let Some(result) = call_json(
                host,
                player,
                serde_json::json!({"operation": operation::PLAYER_ONLINE_LIST, "data": {}}),
            ) {
                candidates.extend(
                    result["players"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|entry| entry["name"].as_str().map(str::to_owned)),
                );
            }
        }
        _ => {}
    }
    // Brigadier already completes registered literals. This callback supplies only
    // argument values, consistently filtering the token currently under the cursor.
    add_suggestions(host, player, current, candidates);
}

fn open_inventory_view(host: &HostApiV3, player: u128, name: &str, ender: bool) -> bool {
    let Some(target) = call_json(
        host,
        player,
        serde_json::json!({"operation": operation::PLAYER_LOOKUP, "data": {"name": name}}),
    )
    .filter(|value| value["ok"] == true)
    .and_then(|value| value["players"].as_array()?.first().cloned()) else {
        message(host, player, "Jogador não está online.");
        return true;
    };
    let Some(uuid) = target["uuid"].as_str() else {
        message(host, player, "Não consegui identificar o jogador.");
        return true;
    };
    let kind = if ender { "ender" } else { "player" };
    let title = if ender {
        format!("Ender Chest de {}", target["name"].as_str().unwrap_or(name))
    } else {
        format!("Inventário de {}", target["name"].as_str().unwrap_or(name))
    };
    let scheduled = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::SCHEDULER_SCHEDULE,
            "data": {"plugin": "AuraEssentials", "delay_ticks": 1,
                "data": {"kind": "inventory_view_open", "viewer": format!("{player:032x}"),
                    "target": uuid, "inventory": kind, "title": title}},
        }),
    );
    if scheduled.as_ref().is_some_and(|value| value["ok"] == true) {
        message(host, player, &format!("Abrindo o {title}."));
    } else {
        message(
            host,
            player,
            "Não foi possível agendar a abertura desse inventário.",
        );
    }
    true
}

pub fn event(host: &HostApiV3, kind: u32, payload: &serde_json::Value) {
    if kind == event::SERVER_TASK && payload["data"]["kind"] == "inventory_view_open" {
        finish_inventory_view(host, &payload["data"]);
        return;
    }
    teleport::event(host, kind, payload);
    worlds::event(host, kind, payload);
    npcs::event(host, kind, payload);
    resource_packs::event(host, kind, payload);
    scoreboard::event(host, kind, payload);
    roles::event(host, kind, payload);
}

fn finish_inventory_view(host: &HostApiV3, task: &serde_json::Value) {
    let Some(viewer) = task["viewer"]
        .as_str()
        .and_then(|value| u128::from_str_radix(value, 16).ok())
    else {
        return;
    };
    let (Some(target), Some(kind), Some(title)) = (
        task["target"].as_str(),
        task["inventory"].as_str(),
        task["title"].as_str(),
    ) else {
        return;
    };
    let result = call_json(
        host,
        viewer,
        serde_json::json!({
            "operation": operation::INVENTORY_VIEW_OPEN,
            "data": {"target": target, "kind": kind, "title": title},
        }),
    );
    if result.as_ref().is_some_and(|value| value["ok"] == true) {
        message(host, viewer, &format!("Inventário de {title} aberto."));
    } else {
        message(
            host,
            viewer,
            "Não foi possível abrir esse inventário; o jogador pode ter saído ou morrido.",
        );
    }
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

pub(crate) fn teleport_warp(host: &HostApiV3, player: u128, name: &str) -> bool {
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

fn open_warps(host: &HostApiV3, player: u128) -> bool {
    open_warps_page(host, player, 0)
}

pub(crate) fn open_warps_page(host: &HostApiV3, player: u128, page: usize) -> bool {
    let keys = storage_list(host, "warp:");
    if keys.is_empty() {
        message(host, player, "Não há warps configurados.");
        return true;
    }
    let pages = keys.len().div_ceil(45);
    let page = page.min(pages.saturating_sub(1));
    let start = page * 45;
    let mut mapping = std::collections::HashMap::new();
    let mut items: Vec<_> = keys.iter().skip(start).take(45).enumerate().map(|(slot, key)| {
        let name = key[5..].to_owned();
        mapping.insert(slot as i32, name.clone());
        serde_json::json!({"slot": slot, "item": "minecraft:ender_pearl", "name": {"text": name, "color": "aqua", "italic": false}, "lore": ["Clique para teleportar"]})
    }).collect();
    if page > 0 {
        items.push(serde_json::json!({"slot":45,"item":"minecraft:arrow","name":{"text":"Página anterior","italic":false}}));
    }
    items.push(serde_json::json!({"slot":49,"item":"minecraft:paper","name":{"text":format!("Página {}/{}", page + 1, pages),"italic":false}}));
    if page + 1 < pages {
        items.push(serde_json::json!({"slot":53,"item":"minecraft:arrow","name":{"text":"Próxima página","italic":false}}));
    }
    let result = call_json(
        host,
        player,
        serde_json::json!({"operation": operation::MENU_OPEN, "data": {"title": {"text": "Warps", "italic": false}, "rows": 6, "items": items}}),
    );
    if let Some(window) = result
        .as_ref()
        .filter(|v| v["ok"] == true)
        .and_then(|v| v["window_id"].as_i64())
        .and_then(|n| i32::try_from(n).ok())
    {
        crate::shop::state()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .views
            .insert(
                (player, window),
                crate::shop::View::Warps {
                    page,
                    pages,
                    slots: mapping,
                },
            );
    } else {
        message(host, player, "Não foi possível abrir a lista de warps.");
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
    let snapshot = player_snapshot(host, player);
    let current = snapshot
        .as_ref()
        .and_then(|snapshot| snapshot["dimension"].as_str().map(str::to_owned));
    let destination = target["dimension"].as_str();
    if current.as_deref() == destination
        && snapshot.as_ref().is_some_and(|snapshot| {
            snapshot["position"] == target["position"]
                && snapshot["yaw"] == target["yaw"]
                && snapshot["pitch"] == target["pitch"]
        })
    {
        message(host, player, "Você já está nesse local.");
        return;
    }
    if let (Some(current), Some(destination)) = (current, destination)
        && current != destination
    {
        let loaded = call_json(
            host,
            player,
            serde_json::json!({
                "operation": operation::WORLD_DIMENSION_LOAD,
                "data": {"dimension": destination},
            }),
        );
        if loaded.as_ref().is_none_or(|result| result["ok"] != true) {
            let reason = loaded
                .as_ref()
                .and_then(|value| value["detail"].as_str())
                .or_else(|| loaded.as_ref().and_then(|value| value["error"].as_str()))
                .unwrap_or("falha ao abrir a dimensão");
            message(
                host,
                player,
                &format!("Não foi possível abrir a dimensão de destino: {reason}"),
            );
            return;
        }
    }
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
    match result {
        Some(result) if result["ok"] == true => message(host, player, success),
        Some(result) => message(
            host,
            player,
            &format!(
                "O Horizon recusou o teleporte: {}.",
                result["error"].as_str().unwrap_or("destino inválido")
            ),
        ),
        None => message(host, player, "A API de teleporte do Horizon não respondeu."),
    }
}

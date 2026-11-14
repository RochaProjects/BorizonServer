//! Holograms are plugin definitions rendered as ordinary vanilla text_display entities.
use crate::plugin_api::{add_suggestions, call_json, completion_input, message};
use horizon_server_plugin_api::{HostApiV3, operation};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{fs, path::PathBuf};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
struct Hologram {
    id: String,
    dimension: String,
    position: [f64; 3],
    title: String,
    description: String,
    split_background: bool,
    background: u32,
}
impl Default for Hologram {
    fn default() -> Self {
        Self {
            id: String::new(),
            dimension: String::new(),
            position: [0.0; 3],
            title: String::new(),
            description: String::new(),
            split_background: true,
            background: 0x900a0a0a,
        }
    }
}
fn file() -> Option<PathBuf> {
    Some(
        std::env::current_dir()
            .ok()?
            .join("plugins/AuraEssentials/hologramas.json"),
    )
}
fn load() -> Vec<Hologram> {
    file()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save(entries: &[Hologram]) -> bool {
    let Some(path) = file() else { return false };
    let Some(parent) = path.parent() else {
        return false;
    };
    fs::create_dir_all(parent)
        .and_then(|()| serde_json::to_vec_pretty(entries).map_err(std::io::Error::other))
        .and_then(|b| fs::write(path, b))
        .is_ok()
}
fn uuid(id: &str, line: usize) -> String {
    let mut a = 0xcbf29ce484222325u64;
    let mut b = 0x9e3779b97f4a7c15u64;
    for byte in format!("hologram:{}:{line}", id.to_ascii_lowercase()).bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        b = (b ^ a.rotate_left(13) ^ u64::from(byte)).wrapping_mul(0x9e3779b185ebca87);
    }
    format!("{a:016x}{b:016x}")
}
fn lines(entry: &Hologram) -> Vec<String> {
    let title = normalize_text(&entry.title);
    let description = normalize_text(&entry.description);
    if !entry.split_background {
        let text = if description.is_empty() {
            title
        } else {
            format!("{title}\n{description}")
        };
        return vec![text];
    }
    title
        .lines()
        .chain(description.lines())
        .map(str::to_owned)
        .collect()
}

fn normalize_text(text: &str) -> String {
    text.replace("\\n", "\n")
}

pub(super) fn legacy_component(raw: &str) -> Value {
    let mut extra = Vec::new();
    let mut text = String::new();
    let mut style = Map::new();
    let mut chars = raw.chars().peekable();
    let flush = |text: &mut String, style: &Map<String, Value>, extra: &mut Vec<Value>| {
        if !text.is_empty() {
            let mut part = Map::new();
            part.insert("text".into(), Value::String(std::mem::take(text)));
            for (k, v) in style {
                part.insert(k.clone(), v.clone());
            }
            extra.push(Value::Object(part));
        }
    };
    while let Some(c) = chars.next() {
        if c != '&' {
            text.push(c);
            continue;
        }
        let Some(code) = chars.next() else {
            text.push(c);
            break;
        };
        let lower = code.to_ascii_lowercase();
        let color = match lower {
            '0' => Some("#000000"),
            '1' => Some("#0000aa"),
            '2' => Some("#00aa00"),
            '3' => Some("#00aaaa"),
            '4' => Some("#aa0000"),
            '5' => Some("#aa00aa"),
            '6' => Some("#ffaa00"),
            '7' => Some("#aaaaaa"),
            '8' => Some("#555555"),
            '9' => Some("#5555ff"),
            'a' => Some("#55ff55"),
            'b' => Some("#55ffff"),
            'c' => Some("#ff5555"),
            'd' => Some("#ff55ff"),
            'e' => Some("#ffff55"),
            'f' => Some("#ffffff"),
            _ => None,
        };
        if lower == '#' {
            let hex: String = chars.by_ref().take(6).collect();
            if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                flush(&mut text, &style, &mut extra);
                style.insert("color".into(), Value::String(format!("#{hex}")));
                for k in [
                    "bold",
                    "italic",
                    "underlined",
                    "strikethrough",
                    "obfuscated",
                ] {
                    style.remove(k);
                }
                continue;
            }
            text.push('&');
            text.push(code);
            text.push_str(&hex);
            continue;
        }
        if lower == 'x' {
            let mut hex = String::new();
            let mut valid = true;
            for _ in 0..6 {
                if chars.next() != Some('&') {
                    valid = false;
                    break;
                }
                if let Some(digit) = chars.next() {
                    hex.push(digit);
                } else {
                    valid = false;
                    break;
                }
            }
            if valid && hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                flush(&mut text, &style, &mut extra);
                style.insert("color".into(), Value::String(format!("#{hex}")));
                for k in [
                    "bold",
                    "italic",
                    "underlined",
                    "strikethrough",
                    "obfuscated",
                ] {
                    style.remove(k);
                }
                continue;
            }
            text.push('&');
            text.push(code);
            text.push_str(&hex);
            continue;
        }
        if let Some(color) = color {
            flush(&mut text, &style, &mut extra);
            style.clear();
            style.insert("color".into(), Value::String(color.into()));
            continue;
        }
        let key = match lower {
            'k' => Some("obfuscated"),
            'l' => Some("bold"),
            'm' => Some("strikethrough"),
            'n' => Some("underlined"),
            'o' => Some("italic"),
            _ => None,
        };
        if let Some(key) = key {
            flush(&mut text, &style, &mut extra);
            style.insert(key.into(), Value::Bool(true));
            continue;
        }
        if lower == 'r' {
            flush(&mut text, &style, &mut extra);
            style.clear();
            continue;
        }
        text.push('&');
        text.push(code);
    }
    flush(&mut text, &style, &mut extra);
    let mut root = Map::new();
    root.insert("text".into(), Value::String(String::new()));
    root.insert("extra".into(), Value::Array(extra));
    Value::Object(root)
}
fn apply(host: &HostApiV3, entry: &Hologram) -> bool {
    let lines = lines(entry);
    if lines.is_empty() || lines.len() > 80 {
        return false;
    }
    let mut ok = true;
    for (index, text) in lines.iter().enumerate() {
        let position = [
            entry.position[0],
            entry.position[1] - index as f64 * 0.27,
            entry.position[2],
        ];
        let background = entry.background;
        let result = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::ENTITY_DISPLAY_UPSERT,"data":{
            "dimension":entry.dimension,"uuid":uuid(&entry.id,index),"position":position,"text":legacy_component(text),
                "line_width":220,"background":background,"shadow":true,"see_through":false,"alignment":"center","billboard":"center"
            }}),
        );
        ok &= result.is_some_and(|value| value["ok"] == true);
    }
    // Delete stale lines when an edited hologram became shorter.
    for index in lines.len()..80 {
        let Some(result) = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::ENTITY_DISPLAY_DELETE,"data":{"uuid":uuid(&entry.id,index)}}),
        ) else {
            break;
        };
        if result["error"] == "entity_not_found" {
            break;
        }
    }
    ok
}
fn delete(host: &HostApiV3, id: &str) {
    for index in 0..80 {
        let Some(result) = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::ENTITY_DISPLAY_DELETE,"data":{"uuid":uuid(id,index)}}),
        ) else {
            break;
        };
        if result["error"] == "entity_not_found" {
            break;
        }
    }
}
pub(super) fn register_commands(host: &HostApiV3) {
    for usage in [
        "holograma",
        "holograma listar",
        "holograma criar <id:word> <texto:text>",
        "holograma editar <id:word> titulo <texto:text>",
        "holograma editar <id:word> descricao <texto:text>",
        "holograma editar <id:word> fundo <modo:word>",
        "holograma editar <id:word> cor <cor:word>",
        "holograma editar <id:word> posicao",
        "holograma renomear <id:word> <novo:word>",
        "holograma remover <id:word>",
    ] {
        let level = if usage == "holograma" || usage == "holograma listar" {
            0
        } else {
            2
        };
        let _ = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::COMMAND_REGISTER,"data":{"usage":usage,"default_level":level}}),
        );
    }
}
pub(super) fn command(host: &HostApiV3, player: u128, command: &str) -> bool {
    let words: Vec<_> = command.split_whitespace().collect();
    if words.first() != Some(&"holograma") {
        return false;
    }
    match words.as_slice() {
        ["holograma"] | ["holograma", "listar"] => {
            let entries = load();
            message(
                host,
                player,
                &if entries.is_empty() {
                    "Nenhum holograma cadastrado.".into()
                } else {
                    format!(
                        "Hologramas: {}",
                        entries
                            .iter()
                            .map(|e| e.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            );
        }
        ["holograma", "criar", id, text @ ..] if !text.is_empty() => {
            if id.len() > 48
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            {
                message(host, player, "ID inválido; use letras, números, _ ou -.");
                return true;
            }
            let Some(snapshot) = call_json(
                host,
                player,
                serde_json::json!({"operation":operation::PLAYER_GET,"data":{}}),
            )
            .filter(|v| v["ok"] == true) else {
                return true;
            };
            let (Some(dimension), Some(position)) = (
                snapshot["dimension"].as_str(),
                snapshot["position"]
                    .as_array()
                    .and_then(|p| Some([p[0].as_f64()?, p[1].as_f64()? + 2.0, p[2].as_f64()?])),
            ) else {
                return true;
            };
            let mut entries = load();
            if entries.iter().any(|e| e.id.eq_ignore_ascii_case(id)) {
                message(host, player, "Já existe um holograma com esse ID.");
                return true;
            }
            let entry = Hologram {
                id: (*id).into(),
                dimension: dimension.into(),
                position,
                title: text.join(" "),
                ..Default::default()
            };
            if apply(host, &entry) && {
                entries.push(entry);
                save(&entries)
            } {
                message(host, player, "Holograma criado.");
            } else {
                message(
                    host,
                    player,
                    "Não foi possível criar o holograma; confira se a dimensão e o chunk estão carregados.",
                );
            }
        }
        ["holograma", "editar", id, "titulo", text @ ..] if !text.is_empty() => {
            update(host, player, id, |e| e.title = text.join(" "))
        }
        ["holograma", "editar", id, "descricao", text @ ..] if !text.is_empty() => {
            update(host, player, id, |e| e.description = text.join(" "))
        }
        ["holograma", "editar", id, "fundo", mode] => update(host, player, id, |e| {
            e.split_background = matches!(*mode, "separado" | "sim" | "on")
        }),
        ["holograma", "editar", id, "cor", color] => {
            let raw = color.trim_start_matches('#');
            if raw.len() != 6 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
                message(host, player, "Use uma cor hexadecimal RRGGBB.");
                return true;
            }
            let rgb = u32::from_str_radix(raw, 16).unwrap_or(0x0a0a0a);
            update(host, player, id, |e| e.background = 0x90000000 | rgb);
        }
        ["holograma", "editar", id, "posicao"] => {
            let Some(snapshot) = call_json(
                host,
                player,
                serde_json::json!({"operation":operation::PLAYER_GET,"data":{}}),
            )
            .filter(|v| v["ok"] == true) else {
                return true;
            };
            let (Some(dimension), Some(position)) = (
                snapshot["dimension"].as_str(),
                snapshot["position"]
                    .as_array()
                    .and_then(|p| Some([p[0].as_f64()?, p[1].as_f64()? + 2.0, p[2].as_f64()?])),
            ) else {
                return true;
            };
            update(host, player, id, |e| {
                e.dimension = dimension.into();
                e.position = position;
            });
        }
        ["holograma", "renomear", id, new] => {
            let mut entries = load();
            let Some(i) = entries.iter().position(|e| e.id.eq_ignore_ascii_case(id)) else {
                message(host, player, "Holograma não encontrado.");
                return true;
            };
            if entries.iter().any(|e| e.id.eq_ignore_ascii_case(new)) {
                message(host, player, "Já existe um holograma com esse ID.");
                return true;
            }
            let old = entries[i].clone();
            delete(host, id);
            entries[i].id = (*new).into();
            if apply(host, &entries[i]) && save(&entries) {
                message(host, player, "Holograma renomeado.");
            } else {
                delete(host, new);
                entries[i] = old.clone();
                let _ = apply(host, &old);
                message(host, player, "Não foi possível renomear o holograma.");
            }
        }
        ["holograma", "remover", id] => {
            let mut entries = load();
            let Some(i) = entries.iter().position(|e| e.id.eq_ignore_ascii_case(id)) else {
                message(host, player, "Holograma não encontrado.");
                return true;
            };
            delete(host, id);
            entries.remove(i);
            if save(&entries) {
                message(host, player, "Holograma removido.");
            }
        }
        _ => message(
            host,
            player,
            "Uso: /holograma listar | criar <id> <texto> | editar <id> titulo|descricao|fundo|cor|posicao ... | renomear <id> <novo> | remover <id>.",
        ),
    }
    true
}
fn update(host: &HostApiV3, player: u128, id: &str, change: impl FnOnce(&mut Hologram)) {
    let mut entries = load();
    let Some(i) = entries.iter().position(|e| e.id.eq_ignore_ascii_case(id)) else {
        message(host, player, "Holograma não encontrado.");
        return;
    };
    let old = entries[i].clone();
    change(&mut entries[i]);
    if apply(host, &entries[i]) && save(&entries) {
        message(host, player, "Holograma atualizado.");
    } else {
        delete(host, id);
        entries[i] = old.clone();
        let _ = apply(host, &old);
        message(host, player, "Não foi possível atualizar o holograma.");
    }
}
pub(super) fn suggestions(host: &HostApiV3, player: u128, line: &str) {
    let (words, current) = completion_input(line);
    if words.first() != Some(&"holograma") {
        return;
    }
    let admin = super::player_snapshot(host, player)
        .and_then(|snapshot| snapshot["permission_level"].as_u64())
        .is_some_and(|level| level >= 2);
    let entries = load();
    let candidates: Vec<String> = match words.as_slice() {
        ["holograma"] => {
            let mut commands = vec!["listar".to_owned()];
            if admin {
                commands.extend(["criar", "editar", "renomear", "remover"].map(str::to_owned));
            }
            commands
        }
        ["holograma", "editar"] | ["holograma", "renomear"] | ["holograma", "remover"] if admin => {
            entries.iter().map(|entry| entry.id.clone()).collect()
        }
        ["holograma", "editar", _] if admin => ["titulo", "descricao", "fundo", "cor", "posicao"]
            .map(str::to_owned)
            .into(),
        ["holograma", "editar", _, "fundo"] if admin => {
            ["separado", "unico"].map(str::to_owned).into()
        }
        _ => Vec::new(),
    };
    add_suggestions(host, player, current, candidates);
}

//! AuraNPC model NPCs: each Bedrock cube is rendered by a vanilla Java item_display
//! referencing the matching `aura:npc/.../cube_N` item model in the AuraNPC resource pack.
use crate::plugin_api::{call_json, get_public_http_bytes, message};
use glam::{EulerRot, Mat4, Quat, Vec3};
use horizon_server_plugin_api::{HostApiV3, operation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use zip::ZipArchive;

const DEFAULT_PACK_URL: &str =
    "https://gitlab.com/rochaproject/archive/-/raw/main/auraessentials/AuraNPC.zip?ref_type=heads";
const DEFAULT_PACK_SHA1: &str = "5ea0e0f8252d6c4f62097cc97bac447749b44032";
const MAX_ZIP_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MODELS: usize = 2000;
const MAX_CUBES: usize = 300;
enum ImportState {
    Running,
    Finished(Result<usize, String>),
}
static IMPORTS: OnceLock<Mutex<std::collections::HashMap<u128, ImportState>>> = OnceLock::new();

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
struct Npc {
    id: String,
    dimension: String,
    position: [f64; 3],
    yaw: f32,
    model: String,
    name: String,
    action: String,
    command: String,
}
impl Default for Npc {
    fn default() -> Self {
        Self {
            id: String::new(),
            dimension: String::new(),
            position: [0.0; 3],
            yaw: 0.0,
            model: String::new(),
            name: String::new(),
            action: "nenhuma".into(),
            command: String::new(),
        }
    }
}
fn plugin_dir() -> Option<PathBuf> {
    Some(std::env::current_dir().ok()?.join("plugins/AuraEssentials"))
}
fn npc_file() -> Option<PathBuf> {
    Some(plugin_dir()?.join("npcs.json"))
}
fn model_root() -> Option<PathBuf> {
    Some(plugin_dir()?.join("npc-models/imported"))
}
fn read() -> Vec<Npc> {
    npc_file()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn write(rows: &[Npc]) -> bool {
    let Some(path) = npc_file() else { return false };
    let Some(parent) = path.parent() else {
        return false;
    };
    fs::create_dir_all(parent)
        .and_then(|()| serde_json::to_vec_pretty(rows).map_err(std::io::Error::other))
        .and_then(|b| fs::write(path, b))
        .is_ok()
}
fn stable_uuid(id: &str, part: usize) -> String {
    let mut a = 0xcbf29ce484222325u64;
    let mut b = 0x9e3779b97f4a7c15u64;
    for byte in format!("aura-npc:{}:{part}", id.to_ascii_lowercase()).bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        b = (b ^ a.rotate_left(11) ^ u64::from(byte)).wrapping_mul(0x9e3779b185ebca87);
    }
    format!("{a:016x}{b:016x}")
}
fn safe_id(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}
fn model_file(entity: &str) -> Option<PathBuf> {
    Some(model_root()?.join(format!("{}.json", safe_id(entity))))
}
fn models() -> Vec<String> {
    let Some(root) = model_root() else {
        return vec![];
    };
    let Ok(entries) = fs::read_dir(root) else {
        return vec![];
    };
    entries
        .flatten()
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter_map(|v| v["entity"].as_str().map(str::to_owned))
        .collect()
}

/// Imports only the preconverted server geometry and confirms all referenced Java cube models exist.
fn import_pack(url: &str, expected_sha1: &str) -> Result<usize, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("A URL precisa usar HTTP ou HTTPS.".into());
    }
    if url.len() > 2048 {
        return Err("URL muito longa.".into());
    }
    let bytes =
        get_public_http_bytes(url, MAX_ZIP_BYTES).map_err(|e| format!("Download falhou: {e}"))?;
    if !expected_sha1.is_empty() {
        let got = format!("{:x}", Sha1::digest(&bytes));
        if !got.eq_ignore_ascii_case(expected_sha1) {
            return Err(format!("SHA-1 não confere (recebido {got})."));
        }
    }
    let mut archive =
        ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("ZIP inválido: {e}"))?;
    let mut manifest = String::new();
    archive
        .by_name("aura-npc-pack.json")
        .map_err(|_| "O ZIP não contém aura-npc-pack.json do AuraNPC.".to_owned())?
        .take(2 * 1024 * 1024)
        .read_to_string(&mut manifest)
        .map_err(|e| e.to_string())?;
    let index: Value =
        serde_json::from_str(&manifest).map_err(|e| format!("Manifest inválido: {e}"))?;
    if index["format"] != "aura-npc-pack-v1" {
        return Err("Versão do manifest AuraNPC não suportada.".into());
    }
    let records = index["models"]
        .as_array()
        .ok_or("Manifest sem lista de modelos.")?;
    if records.is_empty() || records.len() > MAX_MODELS {
        return Err("Quantidade de modelos no manifest inválida.".into());
    }
    let root = model_root().ok_or("Pasta do plugin indisponível.")?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut imported = 0;
    for rec in records {
        let Some(entity) = rec["entity"].as_str().filter(|s| valid_model_id(s)) else {
            continue;
        };
        let Some(model_path) = safe_zip_path(rec["model"].as_str().unwrap_or("")) else {
            continue;
        };
        let Some(texture_path) = safe_zip_path(rec["texture"].as_str().unwrap_or("")) else {
            continue;
        };
        let mut model_bytes = Vec::new();
        archive
            .by_name(&model_path)
            .map_err(|_| format!("Geometria ausente: {model_path}"))?
            .take(4 * 1024 * 1024)
            .read_to_end(&mut model_bytes)
            .map_err(|e| e.to_string())?;
        let mut texture = Vec::new();
        archive
            .by_name(&texture_path)
            .map_err(|_| format!("Textura ausente: {texture_path}"))?
            .take(32 * 1024 * 1024)
            .read_to_end(&mut texture)
            .map_err(|e| e.to_string())?;
        let staged: Value = serde_json::from_slice(&model_bytes)
            .map_err(|e| format!("JSON de geometria inválido: {e}"))?;
        if staged["format"] != "aura-bedrock-model-stage-v1"
            || !staged["entity"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(entity))
            || !staged["geometryData"]["bones"].is_array()
            || !texture.starts_with(b"\x89PNG\r\n\x1a\n")
        {
            return Err(format!("Modelo AuraNPC inválido: {entity}"));
        }
        let geometry = &staged["geometryData"];
        let cube_count = geometry["bones"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| b["cubes"].as_array().map_or(0, Vec::len))
            .sum::<usize>();
        if cube_count == 0 || cube_count > MAX_CUBES {
            return Err(format!(
                "Quantidade de cubos inválida em {entity}: {cube_count}"
            ));
        }
        let key = safe_id(entity);
        let item_root = format!("assets/aura/items/npc/{key}/cube_");
        let model_root = format!("assets/aura/models/item/npc/{key}/cube_");
        for i in 0..cube_count {
            let item_path = format!("{item_root}{i}.json");
            let model_path = format!("{model_root}{i}.json");
            let item_exists = archive
                .file_names()
                .filter_map(Result::ok)
                .any(|name| name == item_path);
            let model_exists = archive
                .file_names()
                .filter_map(Result::ok)
                .any(|name| name == model_path);
            if !item_exists || !model_exists {
                return Err(format!(
                    "O resource pack não possui os modelos Java do cubo {i} ({entity})."
                ));
            }
        }
        let mut geometry_out = serde_json::Map::new();
        geometry_out.insert("entity".into(), Value::String(entity.into()));
        geometry_out.insert("geometryData".into(), geometry.clone());
        geometry_out.insert("cube_count".into(), Value::from(cube_count));
        fs::write(
            root.join(format!("{key}.json")),
            serde_json::to_vec(&Value::Object(geometry_out)).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        imported += 1;
    }
    if imported == 0 {
        return Err("Nenhum modelo AuraNPC válido foi encontrado.".into());
    }
    Ok(imported)
}
fn safe_zip_path(path: &str) -> Option<String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|p| p == ".." || p == ".")
    {
        None
    } else {
        Some(path.into())
    }
}
fn valid_model_id(id: &str) -> bool {
    id.len() <= 128
        && id.split_once(':').is_some_and(|(n, p)| {
            !n.is_empty()
                && !p.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_:.-/".contains(&b))
                && !id.contains("..")
        })
}
fn valid_npc_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn vec3(v: &Value, default: Vec3) -> Vec3 {
    let Some(a) = v.as_array().filter(|a| a.len() >= 3) else {
        return default;
    };
    Vec3::new(
        a[0].as_f64().unwrap_or(0.0) as f32,
        a[1].as_f64().unwrap_or(0.0) as f32,
        a[2].as_f64().unwrap_or(0.0) as f32,
    )
}
fn rotation(v: &Value) -> Quat {
    let d = vec3(v, Vec3::ZERO);
    let r = Vec3::new(-d.x.to_radians(), -d.y.to_radians(), d.z.to_radians());
    Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z)
}
fn bedrock_pos(v: Vec3) -> Vec3 {
    Vec3::new(-v.x, v.y, v.z)
}
fn bone_transforms(geometry: &Value) -> Result<(Vec<Value>, Vec<Mat4>, Vec<Quat>), String> {
    let bones = geometry["bones"]
        .as_array()
        .ok_or("Geometria sem bones.")?
        .clone();
    if bones.len() > 1024 {
        return Err("Excesso de bones.".into());
    }
    let mut matrices = vec![None; bones.len()];
    let mut rotations = vec![None; bones.len()];
    let names: Vec<_> = bones
        .iter()
        .map(|b| b["name"].as_str().unwrap_or("").to_owned())
        .collect();
    fn visit(
        i: usize,
        bones: &[Value],
        names: &[String],
        matrices: &mut [Option<Mat4>],
        rotations: &mut [Option<Quat>],
        visiting: &mut [bool],
    ) -> Result<(Mat4, Quat), String> {
        if let (Some(m), Some(q)) = (matrices[i], rotations[i]) {
            return Ok((m, q));
        }
        if visiting[i] {
            return Err("Ciclo na hierarquia de bones.".into());
        }
        visiting[i] = true;
        let bone = &bones[i];
        let pivot = bedrock_pos(vec3(&bone["pivot"], Vec3::ZERO));
        let r = rotation(&bone["rotation"]);
        let local =
            Mat4::from_translation(pivot) * Mat4::from_quat(r) * Mat4::from_translation(-pivot);
        let parent = bone["parent"]
            .as_str()
            .and_then(|name| names.iter().position(|n| n == name));
        let (matrix, q) = if let Some(parent) = parent {
            let (pm, pq) = visit(parent, bones, names, matrices, rotations, visiting)?;
            (pm * local, pq * r)
        } else {
            (local, r)
        };
        visiting[i] = false;
        matrices[i] = Some(matrix);
        rotations[i] = Some(q);
        Ok((matrix, q))
    }
    for i in 0..bones.len() {
        visit(
            i,
            &bones,
            &names,
            &mut matrices,
            &mut rotations,
            &mut vec![false; bones.len()],
        )?;
    }
    Ok((
        bones,
        matrices.into_iter().map(Option::unwrap).collect(),
        rotations.into_iter().map(Option::unwrap).collect(),
    ))
}
fn spawn(host: &HostApiV3, npc: &Npc) -> bool {
    let Some(path) = model_file(&npc.model) else {
        return false;
    };
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(model) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let key = safe_id(&npc.model);
    let Ok((bones, matrices, rotations)) = bone_transforms(&model["geometryData"]) else {
        return false;
    };
    let body = Quat::from_rotation_y((-npc.yaw).to_radians());
    let mut part = 0usize;
    let mut cube_index = 0usize;
    let mut success = true;
    for (bi, bone) in bones.iter().enumerate() {
        let Some(cubes) = bone["cubes"].as_array() else {
            continue;
        };
        for cube in cubes {
            let model_index = cube_index;
            cube_index += 1;
            if cube["uv"].is_null() {
                continue;
            }
            let origin = vec3(&cube["origin"], Vec3::ZERO);
            let size = vec3(&cube["size"], Vec3::ZERO);
            if size.abs().element_sum() == 0.0 {
                continue;
            }
            let center = bedrock_pos(origin + size * 0.5);
            let pivot = bedrock_pos(vec3(&cube["pivot"], center));
            let cube_rot = rotation(&cube["rotation"]);
            let local = Mat4::from_translation(pivot)
                * Mat4::from_quat(cube_rot)
                * Mat4::from_translation(-pivot);
            let model_center = (matrices[bi] * local).transform_point3(center);
            let offset = body * (model_center / 16.0);
            let rot = body * rotations[bi] * cube_rot * Quat::from_rotation_y(std::f32::consts::PI);
            let pos = [
                npc.position[0] + f64::from(offset.x),
                npc.position[1] + f64::from(offset.y),
                npc.position[2] + f64::from(offset.z),
            ];
            let result = call_json(
                host,
                0,
                serde_json::json!({"operation":operation::ENTITY_ITEM_DISPLAY_UPSERT,"data":{"dimension":npc.dimension,"uuid":stable_uuid(&npc.id,model_index),"position":pos,"item_model":format!("aura:npc/{key}/cube_{model_index}"),"rotation":[rot.x,rot.y,rot.z,rot.w],"scale":[1.0,1.0,1.0]}}),
            );
            success &= result.is_some_and(|v| v["ok"] == true);
            part += 1;
        }
    }
    // Label is a normal text display above the model; no armor stand/player mannequin is used.
    let label_uuid = stable_uuid(&npc.id, MAX_CUBES + 1);
    if npc.name.is_empty() {
        let _ = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::ENTITY_DISPLAY_DELETE,"data":{"uuid":label_uuid}}),
        );
    } else {
        let label = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::ENTITY_DISPLAY_UPSERT,"data":{"dimension":npc.dimension,"uuid":label_uuid,"position":[npc.position[0],npc.position[1]+2.2,npc.position[2]],"text":super::holograms::legacy_component(&npc.name),"line_width":220,"background":0,"shadow":true,"alignment":"center","billboard":"center"}}),
        );
        success &= label.is_some_and(|v| v["ok"] == true);
    }
    if !success || part == 0 {
        for index in 0..cube_index.min(MAX_CUBES) {
            let _ = call_json(
                host,
                0,
                serde_json::json!({"operation":operation::ENTITY_REMOVE_UUID,"data":{"uuid":stable_uuid(&npc.id,index)}}),
            );
        }
        let _ = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::ENTITY_DISPLAY_DELETE,"data":{"uuid":label_uuid}}),
        );
        return false;
    }
    true
}
fn remove(host: &HostApiV3, npc: &Npc) -> bool {
    let Some(path) = model_file(&npc.model) else {
        return false;
    };
    let count = fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["cube_count"].as_u64())
        .unwrap_or(MAX_CUBES as u64) as usize;
    let mut ok = true;
    for i in 0..count.min(MAX_CUBES) {
        if let Some(v) = call_json(
            host,
            0,
            serde_json::json!({"operation":operation::ENTITY_REMOVE_UUID,"data":{"uuid":stable_uuid(&npc.id,i)}}),
        ) {
            ok &= v["ok"] == true || v["error"] == "entity_not_found";
        }
    }
    let _ = call_json(
        host,
        0,
        serde_json::json!({"operation":operation::ENTITY_DISPLAY_DELETE,"data":{"uuid":stable_uuid(&npc.id,MAX_CUBES+1)}}),
    );
    ok
}

pub(super) fn register_commands(host: &HostApiV3) {
    for usage in [
        "npc",
        "npc listar",
        "npc modelos",
        "npc importar <url:text>",
        "npc model <id:word> <modelo:word>",
        "npc remover <id:word>",
        "npc editar <id:word> nome <texto:text>",
        "npc editar <id:word> modelo <modelo:word>",
        "npc editar <id:word> acao <acao:word>",
        "npc editar <id:word> comando <texto:text>",
        "npc editar <id:word> posicao",
    ] {
        let level = if matches!(usage, "npc" | "npc listar" | "npc modelos") {
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
    let w: Vec<_> = command.split_whitespace().collect();
    if w.first() != Some(&"npc") {
        return false;
    }
    match w.as_slice() {
        ["npc"] | ["npc", "listar"] => {
            let rows = read();
            message(
                host,
                player,
                &if rows.is_empty() {
                    "Nenhum NPC cadastrado.".into()
                } else {
                    format!(
                        "NPCs: {}",
                        rows.iter()
                            .map(|n| format!("{} ({})", n.id, n.model))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            );
        }
        ["npc", "model", id, model] => {
            if !valid_npc_id(id) {
                message(
                    host,
                    player,
                    "ID inválido; use letras, números, _ ou - (máximo 48 caracteres).",
                );
                return true;
            }
            if !valid_model_id(model) || !models().iter().any(|m| m.eq_ignore_ascii_case(model)) {
                message(
                    host,
                    player,
                    "Modelo não importado. Use /npc importar <URL> e /npc modelos.",
                );
                return true;
            }
            let Some(p) = call_json(
                host,
                player,
                serde_json::json!({"operation":operation::PLAYER_GET,"data":{}}),
            )
            .filter(|v| v["ok"] == true) else {
                return true;
            };
            let (Some(dim), Some(pos)) = (
                p["dimension"].as_str(),
                p["position"]
                    .as_array()
                    .and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])),
            ) else {
                return true;
            };
            let mut rows = read();
            if rows.iter().any(|n| n.id.eq_ignore_ascii_case(id)) {
                message(host, player, "Já existe NPC com esse ID.");
                return true;
            }
            let npc = Npc {
                id: (*id).into(),
                dimension: dim.into(),
                position: pos,
                yaw: p["yaw"].as_f64().unwrap_or(0.0) as f32,
                model: (*model).into(),
                name: (*id).into(),
                ..Default::default()
            };
            if !spawn(host, &npc) {
                message(
                    host,
                    player,
                    "Não foi possível renderizar o modelo. Confirme o pack AuraNPC, a dimensão e o chunk carregados.",
                );
            } else {
                rows.push(npc.clone());
                if write(&rows) {
                    message(host, player, "NPC de modelo 3D AuraNPC criado.");
                } else {
                    rows.pop();
                    let _ = remove(host, &npc);
                    message(
                        host,
                        player,
                        "NPC renderizado, mas não foi possível salvar a configuração; as peças foram removidas.",
                    );
                }
            }
        }
        ["npc", "importar", url @ ..] if !url.is_empty() => {
            let url = url.join(" ");
            let expected = if url == DEFAULT_PACK_URL {
                DEFAULT_PACK_SHA1.to_owned()
            } else {
                String::new()
            };
            let mut jobs = IMPORTS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if matches!(jobs.get(&player), Some(ImportState::Running)) {
                message(
                    host,
                    player,
                    "Você já tem uma importação de modelos em andamento.",
                );
                return true;
            }
            jobs.insert(player, ImportState::Running);
            drop(jobs);
            let scheduled = call_json(
                host,
                player,
                serde_json::json!({"operation":operation::SCHEDULER_SCHEDULE,"data":{"plugin":"AuraEssentials","delay_ticks":20,"period_ticks":20,"data":{"kind":"npc_import_poll","player":format!("{player:032x}")}}}),
            );
            if !scheduled.as_ref().is_some_and(|v| v["ok"] == true) {
                IMPORTS
                    .get_or_init(Default::default)
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&player);
                message(host, player, "Não foi possível acompanhar a importação.");
                return true;
            }
            message(
                host,
                player,
                "Download e validação do AuraNPC iniciados em background.",
            );
            std::thread::spawn(move || {
                let result = import_pack(&url, &expected);
                IMPORTS
                    .get_or_init(Default::default)
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(player, ImportState::Finished(result));
            });
        }
        ["npc", "modelos"] => {
            let list = models();
            message(
                host,
                player,
                &if list.is_empty() {
                    "Nenhum modelo importado. Use /npc importar <URL>.".into()
                } else {
                    format!("Modelos AuraNPC: {}", list.join(", "))
                },
            );
        }
        ["npc", "remover", id] => {
            let mut rows = read();
            let Some(i) = rows.iter().position(|n| n.id.eq_ignore_ascii_case(id)) else {
                message(host, player, "NPC não encontrado.");
                return true;
            };
            let npc = rows.remove(i);
            if remove(host, &npc) && write(&rows) {
                message(host, player, "NPC removido.");
            } else {
                rows.insert(i, npc);
                message(
                    host,
                    player,
                    "Não foi possível remover todas as peças; confira se dimensão/chunks estão carregados.",
                );
            }
        }
        ["npc", "editar", id, "nome", name @ ..] if !name.is_empty() => {
            update(host, player, id, |n| n.name = name.join(" "))
        }
        ["npc", "editar", id, "modelo", model] => {
            if !models().iter().any(|m| m.eq_ignore_ascii_case(model)) {
                message(host, player, "Modelo AuraNPC não importado.");
                return true;
            }
            update(host, player, id, |n| n.model = (*model).into());
        }
        ["npc", "editar", id, "acao", action] => {
            if !["nenhuma", "comando"].contains(action) {
                message(host, player, "Ação disponível agora: nenhuma ou comando.");
                return true;
            }
            update(host, player, id, |n| n.action = (*action).into());
        }
        ["npc", "editar", id, "comando", value @ ..] if !value.is_empty() => {
            update(host, player, id, |n| n.command = value.join(" "))
        }
        ["npc", "editar", id, "posicao"] => {
            let Some(p) = call_json(
                host,
                player,
                serde_json::json!({"operation":operation::PLAYER_GET,"data":{}}),
            )
            .filter(|v| v["ok"] == true) else {
                return true;
            };
            let (Some(dim), Some(pos)) = (
                p["dimension"].as_str(),
                p["position"]
                    .as_array()
                    .and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])),
            ) else {
                return true;
            };
            update(host, player, id, |n| {
                n.dimension = dim.into();
                n.position = pos;
            });
        }
        _ => message(
            host,
            player,
            "Uso: /npc importar <URL> | modelos | model <id> <namespace:entidade> | listar | editar <id> nome|modelo|acao|comando|posicao ... | remover <id>.",
        ),
    }
    true
}
pub(super) fn event(host: &HostApiV3, kind: u32, payload: &Value) {
    if kind == horizon_server_plugin_api::event::PLAYER_INTERACT_ENTITY {
        let Some(entity_uuid) = payload["data"]["entity_uuid"].as_str() else {
            return;
        };
        let Some(player) = payload["player"]
            .as_str()
            .and_then(|s| u128::from_str_radix(s, 16).ok())
        else {
            return;
        };
        let Some(npc) = read().into_iter().find(|npc| {
            let count = model_file(&npc.model)
                .and_then(|p| fs::read(p).ok())
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v["cube_count"].as_u64())
                .unwrap_or(MAX_CUBES as u64)
                .min(MAX_CUBES as u64) as usize;
            (0..=count).any(|part| stable_uuid(&npc.id, part) == entity_uuid)
        }) else {
            return;
        };
        if npc.action == "comando" && !npc.command.trim().is_empty() {
            let command = npc
                .command
                .replace("{player}", payload["name"].as_str().unwrap_or(""));
            let result = call_json(
                host,
                player,
                serde_json::json!({
                    "operation": operation::PLAYER_COMMAND_EXECUTE,
                    "data": {"uuid": format!("{player:032x}"), "command": command}
                }),
            );
            if !result.as_ref().is_some_and(|value| value["ok"] == true) {
                message(
                    host,
                    player,
                    "O comando configurado para este NPC não pôde ser executado.",
                );
            }
        }
        return;
    }
    if kind != horizon_server_plugin_api::event::SERVER_TASK
        || payload["data"]["kind"] != "npc_import_poll"
    {
        return;
    }
    let Some(player) = payload["data"]["player"]
        .as_str()
        .and_then(|s| u128::from_str_radix(s, 16).ok())
    else {
        return;
    };
    let Some(task) = payload["task_id"]
        .as_str()
        .and_then(|v| v.parse::<u64>().ok())
    else {
        return;
    };
    let state = IMPORTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&player);
    match state {
        Some(ImportState::Running) => {
            IMPORTS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(player, ImportState::Running);
        }
        Some(ImportState::Finished(result)) => {
            let _ = call_json(
                host,
                player,
                serde_json::json!({"operation":operation::SCHEDULER_CANCEL,"data":{"task_id":task}}),
            );
            match result {
                Ok(count) => message(
                    host,
                    player,
                    &format!(
                        "{count} modelos 3D AuraNPC importados. Use /npc modelos e /npc model <id> <namespace:entidade>."
                    ),
                ),
                Err(error) => message(host, player, &format!("Importação falhou: {error}")),
            }
        }
        None => {}
    }
}
fn update(host: &HostApiV3, player: u128, id: &str, change: impl FnOnce(&mut Npc)) {
    let mut rows = read();
    let Some(i) = rows.iter().position(|n| n.id.eq_ignore_ascii_case(id)) else {
        message(host, player, "NPC não encontrado.");
        return;
    };
    let old = rows[i].clone();
    if !remove(host, &old) {
        message(
            host,
            player,
            "Não foi possível localizar/remover as peças antigas; carregue o chunk.",
        );
        return;
    }
    change(&mut rows[i]);
    if spawn(host, &rows[i]) && write(&rows) {
        message(host, player, "NPC atualizado.");
    } else {
        let _ = remove(host, &rows[i]);
        rows[i] = old.clone();
        let _ = spawn(host, &old);
        message(
            host,
            player,
            "Falha ao atualizar NPC; configuração anterior restaurada.",
        );
    }
}
pub(super) fn suggestions(host: &HostApiV3, player: u128, line: &str) {
    let w: Vec<_> = line.split_whitespace().collect();
    let rows = read();
    let vals: Vec<String> = match w.as_slice() {
        ["npc"] => [
            "listar", "modelos", "importar", "model", "editar", "remover",
        ]
        .map(str::to_owned)
        .into(),
        ["npc", "model", _] | ["npc", "editar", _, "modelo"] => models(),
        ["npc", "editar"] | ["npc", "remover"] => rows.iter().map(|n| n.id.clone()).collect(),
        ["npc", "editar", _, _] => ["nome", "modelo", "acao", "comando", "posicao"]
            .map(str::to_owned)
            .into(),
        ["npc", "editar", _, "acao", _] => ["nenhuma", "comando"].map(str::to_owned).into(),
        _ => vec![],
    };
    let cur = if line.ends_with(' ') {
        ""
    } else {
        w.last().copied().unwrap_or("")
    };
    for v in vals
        .into_iter()
        .filter(|v| v.to_lowercase().starts_with(&cur.to_lowercase()))
    {
        let _ = call_json(
            host,
            player,
            serde_json::json!({"operation":operation::COMMAND_SUGGESTION_ADD,"data":{"value":v}}),
        );
    }
}

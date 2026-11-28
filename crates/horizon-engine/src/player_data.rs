//! Vanilla player position storage (`players/data/<uuid>.dat`).

use horizon_world::{
    datafix::files::{read_dat, write_dat},
    nbt::Tag,
};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SavedPlayer {
    pub(crate) position: [f64; 3],
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
}

fn path(world: &Path, uuid: u128) -> PathBuf {
    world.join("players/data").join(format!(
        "{}.dat",
        horizon_protocol::crypto::uuid_string(uuid)
    ))
}

pub(crate) fn load(world: &Path, uuid: u128) -> Result<Option<SavedPlayer>, String> {
    let file = path(world, uuid);
    let backup = file.with_extension("dat_old");
    let root = match read_dat(&file) {
        Ok(Some(root)) => Some(root),
        Ok(None) => read_dat(&backup)
            .map_err(|error| format!("read backup player data {}: {error}", backup.display()))?,
        Err(primary) => match read_dat(&backup) {
            Ok(Some(root)) => Some(root),
            Ok(None) => {
                return Err(format!("read player data {}: {primary}", file.display()));
            }
            Err(backup_error) => {
                return Err(format!(
                    "read player data {}: {primary}; backup {}: {backup_error}",
                    file.display(),
                    backup.display()
                ));
            }
        },
    };
    let Some(root) = root else {
        return Ok(None);
    };
    if let Some(Tag::String(dimension)) = root.get("Dimension")
        && dimension.to_string_lossy() != "minecraft:overworld"
    {
        return Ok(None);
    }
    let Some(Tag::List(position)) = root.get("Pos") else {
        return Ok(None);
    };
    let [Tag::Double(x), Tag::Double(y), Tag::Double(z)] = position.as_slice() else {
        return Ok(None);
    };
    let position = [f64::from_bits(*x), f64::from_bits(*y), f64::from_bits(*z)];
    if position.iter().any(|value| !value.is_finite())
        || position[0].abs() > 30_000_000.0
        || position[1].abs() > 20_000_000.0
        || position[2].abs() > 30_000_000.0
    {
        return Ok(None);
    }
    let (yaw, pitch) = match root.get("Rotation") {
        Some(Tag::List(rotation)) => match rotation.as_slice() {
            [Tag::Float(yaw), Tag::Float(pitch)] => (f32::from_bits(*yaw), f32::from_bits(*pitch)),
            _ => (0.0, 0.0),
        },
        _ => (0.0, 0.0),
    };
    if !yaw.is_finite() || !pitch.is_finite() {
        return Ok(None);
    }
    Ok(Some(SavedPlayer {
        position,
        yaw,
        pitch,
    }))
}

pub(crate) fn save(
    world: &Path,
    uuid: u128,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
) -> Result<(), String> {
    if position.iter().any(|value| !value.is_finite()) || !yaw.is_finite() || !pitch.is_finite() {
        return Err("refusing to save non-finite player coordinates or rotation".into());
    }
    let file = path(world, uuid);
    let backup = file.with_extension("dat_old");
    let existing = match read_dat(&file) {
        Ok(Some(root)) => Some(root),
        Ok(None) => read_dat(&backup)
            .map_err(|error| format!("read backup player data {}: {error}", backup.display()))?,
        Err(primary) => match read_dat(&backup) {
            Ok(root) => root,
            Err(backup_error) => {
                return Err(format!(
                    "read existing player data {}: {primary}; backup {}: {backup_error}",
                    file.display(),
                    backup.display()
                ));
            }
        },
    };
    let mut root = existing.unwrap_or_else(|| Tag::Compound(Default::default()));
    let Tag::Compound(fields) = &mut root else {
        return Err(format!("player data {} is not a compound", file.display()));
    };
    fields.insert(
        "UUID".into(),
        Tag::IntArray(
            (0..4)
                .map(|index| (uuid >> (96 - index * 32)) as i32)
                .collect(),
        ),
    );
    fields.insert(
        "DataVersion".into(),
        Tag::Int(horizon_world::datafix::CURRENT),
    );
    fields.insert(
        "Pos".into(),
        Tag::List(
            position
                .into_iter()
                .map(|value| Tag::Double(value.to_bits()))
                .collect(),
        ),
    );
    fields.insert(
        "Rotation".into(),
        Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(pitch.to_bits())]),
    );
    fields.insert(
        "Dimension".into(),
        Tag::String("minecraft:overworld".into()),
    );
    let temporary = file.with_extension("dat.tmp");
    write_dat(&temporary, &root).map_err(|error| {
        format!(
            "write temporary player data {}: {error}",
            temporary.display()
        )
    })?;
    File::open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            format!(
                "sync temporary player data {}: {error}",
                temporary.display()
            )
        })?;
    if backup.exists() {
        fs::remove_file(&backup)
            .map_err(|error| format!("remove old player backup {}: {error}", backup.display()))?;
    }
    if file.exists() {
        fs::rename(&file, &backup)
            .map_err(|error| format!("rotate player data {} to backup: {error}", file.display()))?;
    }
    if let Err(error) = fs::rename(&temporary, &file) {
        if !file.exists() && backup.exists() {
            let _ = fs::rename(&backup, &file);
        }
        return Err(format!("replace player data {}: {error}", file.display()));
    }
    if let Some(parent) = file.parent() {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("sync player data directory {}: {error}", parent.display()))?;
    }
    Ok(())
}

//! Server-owned local storage for vanilla world scoreboard and per-dimension borders.
//!
//! This adapter owns file paths and the SavedData NBT envelope only. The server
//! runtime remains responsible for deciding when a world/level is ready to load
//! or save, and for excluding remote worlds from local persistence.
use crate::{
    DATA_VERSION,
    datafix::{
        files::{read_dat, write_dat},
        schema::Ty,
        update_saved_data,
    },
    identifier::Identifier,
    nbt::{Compound, Tag},
    scoreboard::Scoreboard,
    world_border::WorldBorder,
};
use std::{
    io,
    path::{Component as PathComponent, Path, PathBuf},
};

/// The local SavedData files belonging to one vanilla world directory.
#[derive(Clone, Debug)]
pub struct WorldSavedData {
    world_root: PathBuf,
}

impl WorldSavedData {
    pub fn new(world_root: impl Into<PathBuf>) -> Self {
        Self {
            world_root: world_root.into(),
        }
    }

    /// The world-global `data/minecraft/scoreboard.dat`.
    pub fn scoreboard_path(&self) -> PathBuf {
        self.world_root.join("data/minecraft/scoreboard.dat")
    }

    /// This dimension's `data/minecraft/world_border.dat`.
    pub fn world_border_path(&self, dimension: &str) -> io::Result<PathBuf> {
        let dimension = parse_dimension(dimension)?;
        Ok(self
            .world_root
            .join("dimensions")
            .join(dimension.namespace())
            .join(dimension.path())
            .join("data/minecraft/world_border.dat"))
    }

    /// Load the global scoreboard, defaulting when its file or `data` is absent.
    pub fn load_scoreboard(&self) -> io::Result<Scoreboard> {
        Ok(
            read_saved_data(&self.scoreboard_path(), Ty::SavedScoreboard)?
                .as_ref()
                .map(Scoreboard::unpack)
                .unwrap_or_default(),
        )
    }

    /// Save the global scoreboard with the current data version.
    /// A new untouched default scoreboard remains absent, as in vanilla.
    pub fn save_scoreboard(&self, board: &Scoreboard) -> io::Result<()> {
        let path = self.scoreboard_path();
        if board == &Scoreboard::default() && !path.exists() {
            return Ok(());
        }
        write_saved_data(&path, board.pack())
    }

    /// Load one dimension's border. Only the Overworld falls back to the old
    /// root-level Horizon path used before borders became per-dimension data.
    pub fn load_world_border(&self, dimension: &str) -> io::Result<WorldBorder> {
        let id = parse_dimension(dimension)?;
        let path = self
            .world_root
            .join("dimensions")
            .join(id.namespace())
            .join(id.path())
            .join("data/minecraft/world_border.dat");
        let mut data = read_saved_data(&path, Ty::SavedWorldBorder)?;
        if data.is_none() && is_overworld(&id) {
            data = read_saved_data(
                &self.world_root.join("data/minecraft/world_border.dat"),
                Ty::SavedWorldBorder,
            )?;
        }
        Ok(data.as_ref().map(WorldBorder::unpack).unwrap_or_default())
    }

    /// Save one dimension's border. Vanilla omits a default border file.
    pub fn save_world_border(&self, dimension: &str, border: &WorldBorder) -> io::Result<()> {
        let path = self.world_border_path(dimension)?;
        if border == &WorldBorder::default() {
            return Ok(());
        }
        write_saved_data(&path, border.pack())
    }
}

fn parse_dimension(text: &str) -> io::Result<Identifier> {
    let id = Identifier::parse(text).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid dimension identifier {text:?}: {e}"),
        )
    })?;
    // Resource identifiers may contain slashes, but filesystem paths must not
    // accept absolute, empty, current-directory, or parent components.
    if id.path().is_empty()
        || id
            .path()
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(id.path())
            .components()
            .any(|component| !matches!(component, PathComponent::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("dimension path is not a safe relative path: {text:?}"),
        ));
    }
    Ok(id)
}

fn is_overworld(id: &Identifier) -> bool {
    id.namespace() == "minecraft" && id.path() == "overworld"
}

fn read_saved_data(path: &Path, ty: Ty) -> io::Result<Option<Tag>> {
    let Some(mut root) = read_dat(path)? else {
        return Ok(None);
    };
    update_saved_data(ty, &mut root);
    Ok(Some(
        root.get("data").cloned().unwrap_or_else(empty_compound),
    ))
}

fn write_saved_data(path: &Path, data: Tag) -> io::Result<()> {
    write_dat(
        path,
        &Tag::compound([("data", data), ("DataVersion", Tag::Int(DATA_VERSION))]),
    )
}

fn empty_compound() -> Tag {
    Tag::Compound(Compound::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scoreboard::RenderType, world_border::MAX_SIZE};
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TempWorld(PathBuf);

    impl TempWorld {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "horizon-world-saved-data-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn store(&self) -> WorldSavedData {
            WorldSavedData::new(&self.0)
        }
    }

    impl Drop for TempWorld {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scoreboard_round_trips_as_world_global_saved_data() {
        let world = TempWorld::new();
        let store = world.store();
        let mut board = Scoreboard::default();
        assert!(board.add_objective(
            "kills",
            "dummy",
            crate::scoreboard::literal("Kills"),
            RenderType::Integer,
            false,
            None
        ));
        board.score_mut("Alex", "kills").value = 27;

        store.save_scoreboard(&board).unwrap();
        assert_eq!(store.load_scoreboard().unwrap(), board);
        let root = read_dat(&store.scoreboard_path()).unwrap().unwrap();
        assert_eq!(root.get("DataVersion"), Some(&Tag::Int(DATA_VERSION)));
        assert!(root.get("data").is_some());
    }

    #[test]
    fn default_scoreboard_is_not_written_until_a_file_exists() {
        let world = TempWorld::new();
        let store = world.store();
        let default = Scoreboard::default();
        store.save_scoreboard(&default).unwrap();
        let path = store.scoreboard_path();
        assert!(!path.exists());

        let mut populated = Scoreboard::default();
        populated.add_objective(
            "kills",
            "dummy",
            crate::scoreboard::literal("Kills"),
            RenderType::Integer,
            false,
            None,
        );
        store.save_scoreboard(&populated).unwrap();
        store.save_scoreboard(&default).unwrap();
        assert!(path.exists());
        assert_eq!(store.load_scoreboard().unwrap(), default);
    }

    #[test]
    fn border_round_trips_per_dimension_and_omits_defaults() {
        let world = TempWorld::new();
        let store = world.store();
        let overworld = "minecraft:overworld";
        let nether = "minecraft:the_nether";
        assert_eq!(
            store.load_world_border(overworld).unwrap(),
            WorldBorder::default()
        );
        assert_eq!(
            store.load_world_border(nether).unwrap(),
            WorldBorder::default()
        );
        store
            .save_world_border(overworld, &WorldBorder::default())
            .unwrap();
        assert!(!store.world_border_path(overworld).unwrap().exists());

        let mut border = WorldBorder::default();
        border.set_center([21.5, -4.25]);
        border.set_size(MAX_SIZE / 4.0);
        store.save_world_border(nether, &border).unwrap();
        assert_eq!(store.load_world_border(nether).unwrap(), border);
        assert_eq!(
            store.load_world_border(overworld).unwrap(),
            WorldBorder::default()
        );
        let root = read_dat(&store.world_border_path(nether).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(root.get("DataVersion"), Some(&Tag::Int(DATA_VERSION)));
    }

    #[test]
    fn only_overworld_uses_the_legacy_root_border_file() {
        let world = TempWorld::new();
        let store = world.store();
        let mut border = WorldBorder::default();
        border.set_center([7.0, 11.0]);
        write_dat(
            &world.0.join("data/minecraft/world_border.dat"),
            &Tag::compound([
                ("data", border.pack()),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
        .unwrap();

        assert_eq!(store.load_world_border("overworld").unwrap(), border);
        assert_eq!(
            store.load_world_border("minecraft:the_end").unwrap(),
            WorldBorder::default()
        );
    }

    #[test]
    fn dimension_identifiers_cannot_escape_the_world_directory() {
        let world = TempWorld::new();
        let store = world.store();
        for id in [
            "",
            "minecraft:",
            "../outside",
            "minecraft:../outside",
            "minecraft:./outside",
            "minecraft:a//b",
            "/tmp/world",
            "minecraft:a/../../b",
        ] {
            assert_eq!(
                store.world_border_path(id).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                store
                    .save_world_border(id, &WorldBorder::default())
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}

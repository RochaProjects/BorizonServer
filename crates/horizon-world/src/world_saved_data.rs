//! Server-owned local storage for vanilla world-global and per-dimension SavedData.
//!
//! This adapter owns file paths and the SavedData NBT envelope only. The server
//! runtime remains responsible for deciding when a world/level is ready to load
//! or save, and for excluding remote worlds from local persistence.
use crate::{
    DATA_VERSION,
    datafix::{
        files::{current_world_gen_settings, read_dat, write_dat},
        schema::Ty,
        update_saved_data,
    },
    gamerules::GameRules,
    identifier::Identifier,
    nbt::{Compound, JavaString, Tag},
    scoreboard::Scoreboard,
    weather::Weather,
    world_border::WorldBorder,
};
use std::{
    io,
    path::{Component as PathComponent, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// The local SavedData files belonging to one vanilla world directory.
#[derive(Clone, Debug)]
pub struct WorldSavedData {
    world_root: PathBuf,
}

/// World-generation seed and shared spawn read from the vanilla world files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldBasics {
    pub seed: Option<i64>,
    pub spawn: Option<[i32; 3]>,
}

impl WorldSavedData {
    pub fn new(world_root: impl Into<PathBuf>) -> Self {
        Self {
            world_root: world_root.into(),
        }
    }

    /// DedicatedServer's level.dat fallback: restore a readable `level.dat_old`
    /// only when the current file is missing or unreadable, keeping the broken
    /// file under a timestamped name as the vanilla loader does.
    pub fn restore_level_dat_old(&self) -> io::Result<bool> {
        let current = self.world_root.join("level.dat");
        if read_dat(&current).is_ok_and(|data| data.is_some()) {
            return Ok(false);
        }
        let old = self.world_root.join("level.dat_old");
        if !read_dat(&old).is_ok_and(|data| data.is_some()) {
            return Ok(false);
        }
        if current.exists() {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let corrupted = self.world_root.join(format!("level.dat_corrupted_{stamp}"));
            std::fs::rename(&current, &corrupted)?;
            if let Err(error) = std::fs::rename(&old, &current) {
                let _ = std::fs::rename(corrupted, &current);
                return Err(error);
            }
        } else {
            std::fs::rename(old, current)?;
        }
        Ok(true)
    }

    /// Read the seed and spawn exactly where 26.3 stores them. New worlds may
    /// have neither yet and use server.properties until their first save.
    pub fn load_world_basics(&self) -> io::Result<WorldBasics> {
        let root = read_dat(&self.world_root.join("level.dat"))?;
        let data = root.as_ref().and_then(|root| root.get("Data"));
        let settings = current_world_gen_settings(&self.world_root)?;
        let seed = settings
            .as_ref()
            .and_then(|settings| integer(settings.get("seed")))
            .or_else(|| {
                data.and_then(|data| {
                    integer(
                        data.get("WorldGenSettings")
                            .and_then(|settings| settings.get("seed")),
                    )
                })
            });
        let spawn =
            data.and_then(
                |data| match data.get("spawn").and_then(|spawn| spawn.get("pos")) {
                    Some(Tag::IntArray(position)) if position.len() == 3 => {
                        Some([position[0], position[1], position[2]])
                    }
                    _ if data.get("SpawnX").is_some() => Some([
                        integer(data.get("SpawnX"))? as i32,
                        integer(data.get("SpawnY"))? as i32,
                        integer(data.get("SpawnZ"))? as i32,
                    ]),
                    _ => None,
                },
            );
        Ok(WorldBasics { seed, spawn })
    }

    /// Persist the 26.3 world-generation seed and shared Overworld spawn while
    /// preserving unrelated saved generator dimensions and level.dat fields.
    pub fn save_world_basics(
        &self,
        seed: i64,
        spawn: [i32; 3],
        level_name: &str,
        game_mode: i32,
        difficulty: &str,
        hardcore: bool,
    ) -> io::Result<()> {
        let settings = current_world_gen_settings(&self.world_root)?
            .unwrap_or_else(|| default_noise_world_settings(seed));
        let mut settings = settings;
        if let Tag::Compound(fields) = &mut settings {
            fields.insert("seed".into(), Tag::Long(seed));
        }
        let settings_path = self
            .world_root
            .join("data/minecraft/world_gen_settings.dat");
        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_saved_data(&settings_path, settings)?;

        let path = self.world_root.join("level.dat");
        let had_level_file = path.exists();
        let mut root = read_dat(&path)?.unwrap_or_else(empty_compound);
        if !matches!(root, Tag::Compound(_)) {
            root = empty_compound();
        }
        let mut data = root.get("Data").cloned().unwrap_or_else(empty_compound);
        if !matches!(data, Tag::Compound(_)) {
            data = empty_compound();
        }
        if let Tag::Compound(fields) = &mut data {
            if !had_level_file {
                fields.insert("DataVersion".into(), Tag::Int(crate::DATA_VERSION));
            }
            fields.insert(
                "Version".into(),
                Tag::compound([
                    (
                        "Name",
                        Tag::String(JavaString::from(horizon_core::version::VANILLA_VERSION)),
                    ),
                    ("Id", Tag::Int(crate::DATA_VERSION)),
                    ("Snapshot", Tag::Byte(0)),
                    ("Series", Tag::String(JavaString::from("main"))),
                ]),
            );
            fields.insert("version".into(), Tag::Int(19_133));
            fields.insert(
                "LevelName".into(),
                Tag::String(JavaString::from(level_name)),
            );
            fields.insert("GameType".into(), Tag::Int(game_mode.clamp(0, 3)));
            fields.insert(
                "LastPlayed".into(),
                Tag::Long(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64,
                ),
            );
            let mut history = match fields.get(&JavaString::from("version_history")) {
                Some(Tag::List(items)) => items.clone(),
                _ => Vec::new(),
            };
            let current_version = Tag::Int(crate::DATA_VERSION);
            if history.last() != Some(&current_version) {
                history.push(current_version);
            }
            fields.insert("version_history".into(), Tag::List(history));
            let mut brands = match fields.get(&JavaString::from("ServerBrands")) {
                Some(Tag::List(items)) => items.clone(),
                _ => Vec::new(),
            };
            let horizon_brand = Tag::String(JavaString::from("horizon"));
            if !brands.contains(&horizon_brand) {
                brands.push(horizon_brand);
            }
            fields.insert("ServerBrands".into(), Tag::List(brands));
            let mut difficulty_settings = fields
                .get(&JavaString::from("difficulty_settings"))
                .cloned()
                .unwrap_or_else(empty_compound);
            if !matches!(difficulty_settings, Tag::Compound(_)) {
                difficulty_settings = empty_compound();
            }
            if let Tag::Compound(settings) = &mut difficulty_settings {
                settings.insert(
                    "difficulty".into(),
                    Tag::String(JavaString::from(difficulty)),
                );
                settings.insert("hardcore".into(), Tag::Byte(i8::from(hardcore)));
                settings.entry("locked".into()).or_insert(Tag::Byte(0));
            }
            fields.insert("difficulty_settings".into(), difficulty_settings);
            fields.insert(
                "spawn".into(),
                Tag::compound([
                    ("pos", Tag::IntArray(spawn.to_vec())),
                    (
                        "dimension",
                        Tag::String(JavaString::from("minecraft:overworld")),
                    ),
                    ("yaw", Tag::Float(0.0_f32.to_bits())),
                    ("pitch", Tag::Float(0.0_f32.to_bits())),
                ]),
            );
        }
        if let Tag::Compound(fields) = &mut root {
            fields.insert("Data".into(), data);
        }
        write_dat(&path, &root)
    }

    /// Load `Data.Time`, vanilla's world game-time counter. A missing or
    /// non-integral field defaults to zero; legacy `Data.DayTime` is not used
    /// here because 26.3 stores the day clock separately in `world_clocks.dat`.
    pub fn load_game_time(&self) -> io::Result<i64> {
        Ok(self
            .read_legacy_level_data()?
            .and_then(|data| integer(data.get("Time")))
            .unwrap_or(0))
    }

    /// Update only vanilla `Data.Time` in `level.dat`, preserving all other
    /// root and Data fields. This does not write a legacy `DayTime` key.
    pub fn save_game_time(&self, game_time: i64) -> io::Result<()> {
        let path = self.world_root.join("level.dat");
        let mut root = read_dat(&path)?.unwrap_or_else(empty_compound);
        if !matches!(root, Tag::Compound(_)) {
            root = empty_compound();
        }
        let mut data = root.get("Data").cloned().unwrap_or_else(empty_compound);
        if !matches!(data, Tag::Compound(_)) {
            data = empty_compound();
        }
        if let Tag::Compound(fields) = &mut data {
            fields.insert("Time".into(), Tag::Long(game_time));
        }
        if let Tag::Compound(fields) = &mut root {
            fields.insert("Data".into(), data);
        }
        write_dat(&path, &root)
    }

    /// The world-global `data/minecraft/scoreboard.dat`.
    pub fn scoreboard_path(&self) -> PathBuf {
        self.world_root.join("data/minecraft/scoreboard.dat")
    }

    /// The world-global `data/minecraft/game_rules.dat`.
    pub fn game_rules_path(&self) -> PathBuf {
        self.world_root.join("data/minecraft/game_rules.dat")
    }

    /// The world-global `data/minecraft/weather.dat`.
    pub fn weather_path(&self) -> PathBuf {
        self.world_root.join("data/minecraft/weather.dat")
    }

    /// The world-global `data/minecraft/world_clocks.dat`.
    pub fn world_clocks_path(&self) -> PathBuf {
        self.world_root.join("data/minecraft/world_clocks.dat")
    }

    /// Load vanilla total-tick counters keyed by dimension identifier.
    /// When the Overworld entry is absent, `Data.DayTime` remains a read-only
    /// fallback for legacy saves; other saved dimensions are preserved.
    pub fn load_world_clocks(&self) -> io::Result<WorldClocks> {
        let data = read_saved_data(&self.world_clocks_path(), Ty::SavedWorldClocks)?
            .unwrap_or_else(empty_compound);
        let mut clocks = WorldClocks { data };
        if clocks.total_ticks("minecraft:overworld").is_none()
            && let Some(level) = self.read_legacy_level_data()?
            && let Some(day_time) = integer(level.get("DayTime"))
        {
            clocks.set_total_ticks("minecraft:overworld", day_time);
        }
        Ok(clocks)
    }

    /// Save per-dimension counters with the current data version.
    pub fn save_world_clocks(&self, clocks: &WorldClocks) -> io::Result<()> {
        write_saved_data(&self.world_clocks_path(), clocks.data.clone())
    }

    /// Load vanilla weather timers and targets, falling back field-by-field to
    /// pre-SavedData `level.dat` keys. Fade levels are initialized to targets.
    pub fn load_weather(&self) -> io::Result<Weather> {
        let data =
            read_saved_data(&self.weather_path(), Ty::SavedWeather)?.unwrap_or_else(empty_compound);
        let legacy = if [
            "clear_weather_time",
            "rain_time",
            "thunder_time",
            "raining",
            "thundering",
        ]
        .iter()
        .any(|key| integer(data.get(key)).is_none())
        {
            self.read_legacy_level_data()?
        } else {
            None
        };
        let number = |key: &str, old_key: &str| {
            integer(data.get(key))
                .or_else(|| legacy.as_ref().and_then(|old| integer(old.get(old_key))))
                .unwrap_or(0)
        };
        let raining = number("raining", "raining") != 0;
        let thundering = number("thundering", "thundering") != 0;
        Ok(Weather {
            clear_time: number("clear_weather_time", "clearWeatherTime") as i32,
            rain_time: number("rain_time", "rainTime") as i32,
            thunder_time: number("thunder_time", "thunderTime") as i32,
            raining,
            thundering,
            rain_level: if raining { (1.0, 1.0) } else { (0.0, 0.0) },
            thunder_level: if thundering { (1.0, 1.0) } else { (0.0, 0.0) },
        })
    }

    /// Save the world-global vanilla weather fields with the current data version.
    /// Interpolated fade levels are runtime state and are not serialized.
    pub fn save_weather(&self, weather: &Weather) -> io::Result<()> {
        write_saved_data(
            &self.weather_path(),
            Tag::compound([
                ("clear_weather_time", Tag::Int(weather.clear_time)),
                ("rain_time", Tag::Int(weather.rain_time)),
                ("thunder_time", Tag::Int(weather.thunder_time)),
                ("raining", Tag::Byte(i8::from(weather.raining))),
                ("thundering", Tag::Byte(i8::from(weather.thundering))),
            ]),
        )
    }

    /// Load game rules for this world's enabled feature set.
    pub fn load_game_rules(&self, features: &[String]) -> io::Result<GameRules> {
        let data = read_saved_data(&self.game_rules_path(), Ty::SavedGameRules)?
            .unwrap_or_else(empty_compound);
        let root = Tag::compound([("data", data)]);
        Ok(GameRules::from_nbt_with_features(&root, features))
    }

    /// Save game rules with the current data version in vanilla SavedData form.
    pub fn save_game_rules(&self, rules: &GameRules) -> io::Result<()> {
        let root = rules.to_nbt().tag;
        let data = root.get("data").cloned().unwrap_or_else(empty_compound);
        write_saved_data(&self.game_rules_path(), data)
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

    /// Vanilla Anvil region directory for one dimension. The Overworld stores
    /// chunks under the world root; other dimensions use `dimensions/<id>/region`.
    pub fn region_directory(&self, dimension: &str) -> io::Result<PathBuf> {
        let id = parse_dimension(dimension)?;
        if is_overworld(&id) {
            Ok(self.world_root.join("region"))
        } else {
            Ok(self
                .world_root
                .join("dimensions")
                .join(id.namespace())
                .join(id.path())
                .join("region"))
        }
    }

    fn read_legacy_level_data(&self) -> io::Result<Option<Tag>> {
        let Some(root) = read_dat(&self.world_root.join("level.dat"))? else {
            return Ok(None);
        };
        Ok(root.get("Data").cloned())
    }
}

/// Saved total-tick counters for dimensions in one vanilla world.
///
/// Unknown fields and dimension entries remain opaque and are retained when a
/// known counter is updated, so newer SavedData survives an older server round-trip.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldClocks {
    data: Tag,
}

impl Default for WorldClocks {
    fn default() -> Self {
        Self {
            data: empty_compound(),
        }
    }
}

impl WorldClocks {
    /// The saved total ticks for a dimension, if that entry has a numeric value.
    pub fn total_ticks(&self, dimension: &str) -> Option<i64> {
        integer(self.data.get(dimension)?.get("total_ticks"))
    }

    /// Set one dimension's vanilla total-tick counter without discarding
    /// unrelated dimensions or unknown fields in its entry.
    pub fn set_total_ticks(&mut self, dimension: impl Into<String>, total_ticks: i64) {
        let key = dimension.into();
        if !matches!(&self.data, Tag::Compound(_)) {
            self.data = empty_compound();
        }
        let Tag::Compound(data) = &mut self.data else {
            unreachable!()
        };
        let entry = data
            .entry(JavaString::from(key.as_str()))
            .or_insert_with(empty_compound);
        if let Tag::Compound(fields) = entry {
            fields.insert("total_ticks".into(), Tag::Long(total_ticks));
        } else {
            *entry = Tag::compound([("total_ticks", Tag::Long(total_ticks))]);
        }
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

fn integer(tag: Option<&Tag>) -> Option<i64> {
    match tag? {
        Tag::Byte(value) => Some(i64::from(*value)),
        Tag::Short(value) => Some(i64::from(*value)),
        Tag::Int(value) => Some(i64::from(*value)),
        Tag::Long(value) => Some(*value),
        _ => None,
    }
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

fn default_noise_world_settings(seed: i64) -> Tag {
    let string = |value: &str| Tag::String(JavaString::from(value));
    let preset = |value: &str| {
        Tag::compound([
            ("type", string("minecraft:multi_noise")),
            ("preset", string(value)),
        ])
    };
    let dimension = |kind: &str, settings: &str, biome_source: Tag| {
        Tag::compound([
            ("type", string(kind)),
            (
                "generator",
                Tag::compound([
                    ("type", string("minecraft:noise")),
                    ("settings", string(settings)),
                    ("biome_source", biome_source),
                ]),
            ),
        ])
    };
    Tag::compound([
        ("seed", Tag::Long(seed)),
        ("generate_structures", Tag::Byte(1)),
        ("bonus_chest", Tag::Byte(0)),
        (
            "dimensions",
            Tag::compound([
                (
                    "minecraft:overworld",
                    dimension(
                        "minecraft:overworld",
                        "minecraft:overworld",
                        preset("minecraft:overworld"),
                    ),
                ),
                (
                    "minecraft:the_nether",
                    dimension(
                        "minecraft:the_nether",
                        "minecraft:nether",
                        preset("minecraft:nether"),
                    ),
                ),
                (
                    "minecraft:the_end",
                    dimension(
                        "minecraft:the_end",
                        "minecraft:end",
                        Tag::compound([("type", string("minecraft:the_end"))]),
                    ),
                ),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        gamerules::{MINECART_IMPROVEMENTS, Value},
        scoreboard::RenderType,
        world_border::MAX_SIZE,
    };
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

        fn write_legacy_level(&self, data: Tag) {
            write_dat(&self.0.join("level.dat"), &Tag::compound([("Data", data)])).unwrap();
        }
    }

    impl Drop for TempWorld {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn level_dat_game_time_round_trips_without_overwriting_other_vanilla_fields() {
        let world = TempWorld::new();
        world.write_legacy_level(Tag::compound([
            ("Time", Tag::Long(1234)),
            ("DayTime", Tag::Long(24_000)),
            ("FutureField", Tag::Long(99)),
        ]));
        let store = world.store();
        assert_eq!(store.load_game_time().unwrap(), 1234);
        store.save_game_time(5678).unwrap();

        let root = read_dat(&world.0.join("level.dat")).unwrap().unwrap();
        let data = root.get("Data").unwrap();
        assert_eq!(data.get("Time"), Some(&Tag::Long(5678)));
        assert_eq!(data.get("DayTime"), Some(&Tag::Long(24_000)));
        assert_eq!(data.get("FutureField"), Some(&Tag::Long(99)));
        assert_eq!(store.load_game_time().unwrap(), 5678);
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
    fn game_rules_round_trip_as_feature_aware_world_saved_data() {
        let world = TempWorld::new();
        let store = world.store();
        let features = vec![MINECART_IMPROVEMENTS.to_owned()];
        let mut rules = GameRules::with_features(&features);
        assert!(rules.set("keep_inventory", Value::Bool(true)));
        assert!(rules.set("max_minecart_speed", Value::Int(24)));

        store.save_game_rules(&rules).unwrap();
        assert_eq!(store.load_game_rules(&features).unwrap(), rules);
        assert_eq!(
            store
                .load_game_rules(&[])
                .unwrap()
                .get("max_minecart_speed"),
            None
        );

        let root = read_dat(&store.game_rules_path()).unwrap().unwrap();
        assert_eq!(root.get("DataVersion"), Some(&Tag::Int(DATA_VERSION)));
        assert_eq!(
            root.get("data")
                .and_then(|data| data.get("minecraft:keep_inventory")),
            Some(&Tag::Byte(1))
        );
    }

    #[test]
    fn weather_round_trips_as_world_global_saved_data() {
        let world = TempWorld::new();
        let store = world.store();
        let weather = Weather {
            clear_time: 31,
            rain_time: 1200,
            thunder_time: 2400,
            raining: true,
            thundering: false,
            rain_level: (0.25, 0.75),
            thunder_level: (0.1, 0.6),
        };

        store.save_weather(&weather).unwrap();

        let loaded = store.load_weather().unwrap();
        assert_eq!(loaded.clear_time, weather.clear_time);
        assert_eq!(loaded.rain_time, weather.rain_time);
        assert_eq!(loaded.thunder_time, weather.thunder_time);
        assert_eq!(loaded.raining, weather.raining);
        assert_eq!(loaded.thundering, weather.thundering);
        assert_eq!(loaded.rain_level, (1.0, 1.0));
        assert_eq!(loaded.thunder_level, (0.0, 0.0));

        let root = read_dat(&store.weather_path()).unwrap().unwrap();
        assert_eq!(root.get("DataVersion"), Some(&Tag::Int(DATA_VERSION)));
        assert_eq!(
            root.get("data")
                .and_then(|data| data.get("clear_weather_time")),
            Some(&Tag::Int(31))
        );
        assert_eq!(
            root.get("data").and_then(|data| data.get("raining")),
            Some(&Tag::Byte(1))
        );
    }

    #[test]
    fn missing_weather_saved_data_uses_vanilla_defaults() {
        let world = TempWorld::new();
        let store = world.store();

        assert_eq!(store.load_weather().unwrap(), Weather::default());
        assert!(!store.weather_path().exists());
    }

    #[test]
    fn legacy_level_data_fills_missing_weather_and_overworld_clock() {
        let world = TempWorld::new();
        let store = world.store();
        world.write_legacy_level(Tag::compound([
            ("DataVersion", Tag::Int(4000)),
            ("DayTime", Tag::Long(34_000)),
            ("clearWeatherTime", Tag::Int(31)),
            ("rainTime", Tag::Int(47)),
            ("thunderTime", Tag::Int(59)),
            ("raining", Tag::Byte(1)),
            ("thundering", Tag::Byte(0)),
        ]));
        write_saved_data(
            &store.world_clocks_path(),
            Tag::compound([(
                "minecraft:the_end",
                Tag::compound([("total_ticks", Tag::Long(1234))]),
            )]),
        )
        .unwrap();

        let weather = store.load_weather().unwrap();
        assert_eq!(weather.clear_time, 31);
        assert_eq!(weather.rain_time, 47);
        assert_eq!(weather.thunder_time, 59);
        assert!(weather.raining);
        assert!(!weather.thundering);
        assert_eq!(weather.rain_level, (1.0, 1.0));
        assert_eq!(weather.thunder_level, (0.0, 0.0));

        let clocks = store.load_world_clocks().unwrap();
        assert_eq!(clocks.total_ticks("minecraft:overworld"), Some(34_000));
        assert_eq!(clocks.total_ticks("minecraft:the_end"), Some(1234));
        assert!(!store.weather_path().exists());
    }

    #[test]
    fn saved_weather_fields_override_legacy_values_individually() {
        let world = TempWorld::new();
        let store = world.store();
        world.write_legacy_level(Tag::compound([
            ("clearWeatherTime", Tag::Int(31)),
            ("rainTime", Tag::Int(47)),
            ("thunderTime", Tag::Int(59)),
            ("raining", Tag::Byte(1)),
            ("thundering", Tag::Byte(1)),
        ]));
        write_saved_data(
            &store.weather_path(),
            Tag::compound([
                ("clear_weather_time", Tag::Int(9)),
                ("raining", Tag::Byte(0)),
            ]),
        )
        .unwrap();

        let weather = store.load_weather().unwrap();
        assert_eq!(weather.clear_time, 9);
        assert_eq!(weather.rain_time, 47);
        assert_eq!(weather.thunder_time, 59);
        assert!(!weather.raining);
        assert!(weather.thundering);
        assert_eq!(weather.rain_level, (0.0, 0.0));
        assert_eq!(weather.thunder_level, (1.0, 1.0));
    }

    #[test]
    fn world_clocks_round_trip_per_dimension_and_preserve_unknown_data() {
        let world = TempWorld::new();
        let store = world.store();
        world.write_legacy_level(Tag::compound([("DayTime", Tag::Long(7000))]));
        write_dat(
            &store.world_clocks_path(),
            &Tag::compound([
                (
                    "data",
                    Tag::compound([
                        (
                            "minecraft:overworld",
                            Tag::compound([
                                ("total_ticks", Tag::Long(9001)),
                                ("future_field", Tag::Byte(7)),
                            ]),
                        ),
                        (
                            "minecraft:the_end",
                            Tag::compound([("total_ticks", Tag::Long(1234))]),
                        ),
                        (
                            "example:extra/dimension",
                            Tag::compound([("future_counter", Tag::Int(55))]),
                        ),
                    ]),
                ),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
        .unwrap();

        let mut clocks = store.load_world_clocks().unwrap();
        assert_eq!(clocks.total_ticks("minecraft:overworld"), Some(9001));
        assert_eq!(clocks.total_ticks("minecraft:the_end"), Some(1234));
        assert_eq!(clocks.total_ticks("example:extra/dimension"), None);
        clocks.set_total_ticks("minecraft:overworld", 9002);
        clocks.set_total_ticks("minecraft:the_nether", 77);
        store.save_world_clocks(&clocks).unwrap();

        let root = read_dat(&store.world_clocks_path()).unwrap().unwrap();
        assert_eq!(root.get("DataVersion"), Some(&Tag::Int(DATA_VERSION)));
        let data = root.get("data").unwrap();
        assert_eq!(clocks.total_ticks("minecraft:overworld"), Some(9002));
        assert_eq!(
            data.get("minecraft:overworld")
                .and_then(|entry| entry.get("future_field")),
            Some(&Tag::Byte(7))
        );
        assert_eq!(
            data.get("minecraft:the_end")
                .and_then(|entry| entry.get("total_ticks")),
            Some(&Tag::Long(1234))
        );
        assert_eq!(
            data.get("example:extra/dimension")
                .and_then(|entry| entry.get("future_counter")),
            Some(&Tag::Int(55))
        );
        assert_eq!(
            store
                .load_world_clocks()
                .unwrap()
                .total_ticks("minecraft:the_nether"),
            Some(77)
        );
    }

    #[test]
    fn missing_world_clocks_saved_data_is_empty() {
        let world = TempWorld::new();
        let store = world.store();

        assert_eq!(store.load_world_clocks().unwrap(), WorldClocks::default());
        assert!(!store.world_clocks_path().exists());
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
    fn default_border_save_leaves_existing_file_unchanged_like_reference() {
        let world = TempWorld::new();
        let store = world.store();
        let dimension = "minecraft:overworld";
        let mut saved = WorldBorder::default();
        saved.set_center([13.5, -8.25]);
        store.save_world_border(dimension, &saved).unwrap();
        let path = store.world_border_path(dimension).unwrap();
        let before = fs::read(&path).unwrap();

        store
            .save_world_border(dimension, &WorldBorder::default())
            .unwrap();

        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(store.load_world_border(dimension).unwrap(), saved);
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
        assert_eq!(
            store.region_directory("minecraft:overworld").unwrap(),
            world.0.join("region")
        );
        assert_eq!(
            store.region_directory("minecraft:the_nether").unwrap(),
            world.0.join("dimensions/minecraft/the_nether/region")
        );
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
                store.region_directory(id).unwrap_err().kind(),
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

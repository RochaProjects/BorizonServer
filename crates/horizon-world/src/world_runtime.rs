//! Local vanilla world owner joining world-global SavedData to resident dimensions.
//!
//! This is not the dedicated server engine: callers still drive ticks, choose
//! dimension readiness, own region files and persist dirty chunk snapshots. The
//! owner centralizes local SavedData loading/saving and per-dimension borders and
//! total-tick counters without carrying client or remote-world state.
use crate::{
    gamerules::GameRules,
    level::{LevelAccess, ResidentDimension},
    scoreboard::Scoreboard,
    weather::Weather,
    world_border::WorldBorder,
    world_saved_data::{WorldClocks, WorldSavedData},
};
use std::{collections::BTreeMap, io, path::Path};

/// Runtime state for one registered dimension in a local vanilla world.
pub struct WorldDimension<'registry> {
    resident: ResidentDimension<'registry>,
    border: WorldBorder,
    total_ticks: i64,
}
impl<'registry> WorldDimension<'registry> {
    pub fn resident(&self) -> &ResidentDimension<'registry> {
        &self.resident
    }

    pub fn resident_mut(&mut self) -> &mut ResidentDimension<'registry> {
        &mut self.resident
    }

    pub fn border(&self) -> &WorldBorder {
        &self.border
    }

    pub fn border_mut(&mut self) -> &mut WorldBorder {
        &mut self.border
    }

    pub fn total_ticks(&self) -> i64 {
        self.total_ticks
    }

    /// Update the clock tracked by `world_clocks.dat`; the server tick loop must
    /// call this according to that dimension's vanilla tick policy.
    pub fn set_total_ticks(&mut self, ticks: i64) {
        self.total_ticks = ticks;
        let day_time = self.resident.level().day_time();
        self.resident.set_times(ticks, day_time);
    }

    /// Advance game time once and optionally advance day time, returning both
    /// clocks. The owning server applies the vanilla `advance_time` rule here.
    pub fn advance_time(&mut self, advance_day_time: bool) -> (i64, i64) {
        let (game_time, day_time) = self.resident.advance_time(advance_day_time);
        self.total_ticks = game_time;
        (game_time, day_time)
    }
}

/// World-global vanilla state and the dimensions currently owned by this process.
pub struct LocalWorldRuntime<'registry> {
    saved: WorldSavedData,
    scoreboard: Scoreboard,
    game_rules: GameRules,
    weather: Weather,
    clocks: WorldClocks,
    dimensions: BTreeMap<String, WorldDimension<'registry>>,
}
impl<'registry> LocalWorldRuntime<'registry> {
    /// Open the local SavedData files. The path is always treated as local storage;
    /// this runtime has no remote-world persistence mode.
    pub fn open(root: impl AsRef<Path>, enabled_features: &[String]) -> io::Result<Self> {
        let saved = WorldSavedData::new(root.as_ref());
        Ok(Self {
            scoreboard: saved.load_scoreboard()?,
            game_rules: saved.load_game_rules(enabled_features)?,
            weather: saved.load_weather()?,
            clocks: saved.load_world_clocks()?,
            saved,
            dimensions: BTreeMap::new(),
        })
    }

    pub fn scoreboard(&self) -> &Scoreboard {
        &self.scoreboard
    }

    pub fn scoreboard_mut(&mut self) -> &mut Scoreboard {
        &mut self.scoreboard
    }

    pub fn game_rules(&self) -> &GameRules {
        &self.game_rules
    }

    pub fn game_rules_mut(&mut self) -> &mut GameRules {
        &mut self.game_rules
    }

    pub fn weather(&self) -> &Weather {
        &self.weather
    }

    pub fn weather_mut(&mut self) -> &mut Weather {
        &mut self.weather
    }

    pub fn dimension(&self, key: &str) -> Option<&WorldDimension<'registry>> {
        self.dimensions.get(key)
    }

    pub fn dimension_mut(&mut self, key: &str) -> Option<&mut WorldDimension<'registry>> {
        self.dimensions.get_mut(key)
    }

    pub fn dimensions(&self) -> impl Iterator<Item = (&str, &WorldDimension<'registry>)> {
        self.dimensions
            .iter()
            .map(|(key, dimension)| (key.as_str(), dimension))
    }

    /// Register an empty resident dimension before loading its chunks, so restored
    /// clocks are applied before tick deadlines are unpacked. Loads border and
    /// clock from SavedData; duplicate keys leave the runtime unchanged.
    pub fn add_dimension(&mut self, resident: ResidentDimension<'registry>) -> io::Result<bool> {
        let key = resident.key().to_owned();
        if self.dimensions.contains_key(&key) {
            return Ok(false);
        }
        if resident.level().loaded_chunk_count() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "register the empty dimension before loading resident chunks",
            ));
        }
        let border = self.saved.load_world_border(&key)?;
        let total_ticks = self.clocks.total_ticks(&key).unwrap_or(0);
        let day_time = resident.level().day_time();
        let mut resident = resident;
        resident.set_times(total_ticks, day_time);
        self.dimensions.insert(
            key,
            WorldDimension {
                resident,
                border,
                total_ticks,
            },
        );
        Ok(true)
    }

    /// Persist world-global SavedData and the borders/clocks of registered
    /// dimensions. Dirty chunks remain the resident dimensions' responsibility;
    /// callers must save and sync those region records separately.
    pub fn save_world_data(&mut self) -> io::Result<()> {
        for (key, dimension) in &self.dimensions {
            self.clocks
                .set_total_ticks(key.clone(), dimension.total_ticks);
        }
        self.saved.save_scoreboard(&self.scoreboard)?;
        self.saved.save_game_rules(&self.game_rules)?;
        self.saved.save_weather(&self.weather)?;
        self.saved.save_world_clocks(&self.clocks)?;
        for (key, dimension) in &self.dimensions {
            self.saved.save_world_border(key, &dimension.border)?;
        }
        Ok(())
    }
}

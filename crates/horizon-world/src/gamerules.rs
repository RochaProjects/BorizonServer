//! Game rules (26.3 `GameRules`): the registered rules with their category,
//! type, default and bounds, a value map, and persistence in the world's
//! `data/minecraft/game_rules.dat` saved-data file.
use crate::world_saved_data::WorldSavedData;
use crate::{
    DATA_VERSION,
    nbt::{JavaString, NamedTag, Tag},
};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Bool(bool),
    Int(i32),
}
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
        }
    }
}
pub struct Rule {
    pub name: &'static str,
    pub category: &'static str,
    pub default: Value,
    pub min: i32,
    pub max: i32,
}
const fn b(name: &'static str, category: &'static str, default: bool) -> Rule {
    Rule {
        name,
        category,
        default: Value::Bool(default),
        min: 0,
        max: 0,
    }
}
const fn i(name: &'static str, category: &'static str, default: i32, min: i32, max: i32) -> Rule {
    Rule {
        name,
        category,
        default: Value::Int(default),
        min,
        max,
    }
}
/// Every rule available without experimental features, in registration order.
pub const RULES: &[Rule] = &[
    b("advance_time", "updates", true),
    b("advance_weather", "updates", true),
    b("allow_entering_nether_using_portals", "misc", true),
    b("block_drops", "drops", true),
    b("block_explosion_drop_decay", "drops", true),
    b("command_blocks_work", "misc", true),
    b("command_block_output", "chat", true),
    b("drowning_damage", "player", true),
    b("elytra_movement_check", "player", true),
    b("ender_pearls_vanish_on_death", "player", true),
    b("entity_drops", "drops", true),
    b("fall_damage", "player", true),
    b("fire_damage", "player", true),
    i(
        "fire_spread_radius_around_player",
        "updates",
        128,
        -1,
        i32::MAX,
    ),
    b("forgive_dead_players", "mobs", true),
    b("freeze_damage", "player", true),
    b("global_sound_events", "misc", true),
    b("immediate_respawn", "player", false),
    b("keep_inventory", "player", false),
    b("lava_source_conversion", "updates", false),
    b("limited_crafting", "player", false),
    b("locator_bar", "player", true),
    b("log_admin_commands", "chat", true),
    i("max_block_modifications", "misc", 32768, 1, i32::MAX),
    i("max_command_forks", "misc", 65536, 0, i32::MAX),
    i("max_command_sequence_length", "misc", 65536, 0, i32::MAX),
    i("max_entity_cramming", "mobs", 24, 0, i32::MAX),
    i("max_snow_accumulation_height", "updates", 1, 0, 8),
    b("mob_drops", "drops", true),
    b("mob_explosion_drop_decay", "drops", true),
    b("mob_griefing", "mobs", true),
    b("natural_health_regeneration", "player", true),
    b("player_movement_check", "player", true),
    i(
        "players_nether_portal_creative_delay",
        "player",
        0,
        0,
        i32::MAX,
    ),
    i(
        "players_nether_portal_default_delay",
        "player",
        80,
        0,
        i32::MAX,
    ),
    i("players_sleeping_percentage", "player", 100, 0, i32::MAX),
    b("projectiles_can_break_blocks", "drops", true),
    b("pvp", "player", true),
    b("raids", "mobs", true),
    i("random_tick_speed", "updates", 3, 0, i32::MAX),
    b("reduced_debug_info", "misc", false),
    i("respawn_radius", "player", 10, 0, i32::MAX),
    b("send_command_feedback", "chat", true),
    b("show_advancement_messages", "chat", true),
    b("show_death_messages", "chat", true),
    b("spawner_blocks_work", "misc", true),
    b("spawn_mobs", "spawning", true),
    b("spawn_monsters", "spawning", true),
    b("spawn_patrols", "spawning", true),
    b("spawn_phantoms", "spawning", true),
    b("spawn_wandering_traders", "spawning", true),
    b("spawn_wardens", "spawning", true),
    b("spectators_generate_chunks", "player", true),
    b("spread_vines", "updates", true),
    b("tnt_explodes", "misc", true),
    b("tnt_explosion_drop_decay", "drops", false),
    b("universal_anger", "mobs", false),
    b("water_source_conversion", "updates", true),
];
/// GameRules.MAX_MINECART_SPEED (26.3), available with minecart improvements.
pub const MAX_MINECART_SPEED: Rule = i("max_minecart_speed", "misc", 8, 1, 1000);
pub const MINECART_IMPROVEMENTS: &str = "minecraft:minecart_improvements";

pub fn available_rules(minecart_improvements: bool) -> impl Iterator<Item = &'static Rule> {
    RULES
        .iter()
        .chain(minecart_improvements.then_some(&MAX_MINECART_SPEED))
}

pub fn rule(name: &str) -> Option<&'static Rule> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    available_rules(true).find(|r| r.name == name)
}

/// Current values (defaults unless set).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameRules {
    values: BTreeMap<&'static str, Value>,
    minecart_improvements: bool,
}
impl GameRules {
    pub fn with_features(features: &[String]) -> Self {
        Self {
            minecart_improvements: features.iter().any(|f| f == MINECART_IMPROVEMENTS),
            ..Self::default()
        }
    }
    /// GameRules.copy(enabledFeatures): keep available values, add defaults for
    /// newly enabled rules and discard rules absent from the target feature set.
    pub fn copy_with_features(&self, features: &[String]) -> Self {
        let mut rules = Self::with_features(features);
        for (&name, &value) in &self.values {
            rules.set(name, value);
        }
        rules
    }
    pub fn available_rules(&self) -> impl Iterator<Item = &'static Rule> + use<> {
        available_rules(self.minecart_improvements)
    }
    pub fn get(&self, name: &str) -> Option<Value> {
        let r = rule(name)?;
        if r.name == MAX_MINECART_SPEED.name && !self.minecart_improvements {
            return None;
        }
        Some(self.values.get(r.name).copied().unwrap_or(r.default))
    }
    pub fn bool(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Value::Bool(true)))
    }
    /// Parse a network rule value, retaining type and range validation.
    pub fn set_string(&mut self, name: &str, text: &str) -> bool {
        let value = match self.get(name) {
            Some(Value::Bool(_)) => text.parse::<bool>().ok().map(Value::Bool),
            Some(Value::Int(_)) => text.parse::<i32>().ok().map(Value::Int),
            None => None,
        };
        value.is_some_and(|v| self.set(name, v))
    }
    pub fn int(&self, name: &str) -> i32 {
        match self.get(name) {
            Some(Value::Int(v)) => v,
            _ => 0,
        }
    }
    /// Set a rule; false if the name or value type is wrong.
    pub fn set(&mut self, name: &str, value: Value) -> bool {
        let Some(r) = rule(name).filter(|r| self.get(r.name).is_some()) else {
            return false;
        };
        let ok = match (r.default, value) {
            (Value::Bool(_), Value::Bool(_)) => true,
            (Value::Int(_), Value::Int(v)) => (r.min..=r.max).contains(&v),
            _ => false,
        };
        if ok && value == r.default {
            self.values.remove(r.name);
        } else if ok {
            self.values.insert(r.name, value);
        }
        ok
    }
    /// Every rule's value by name without namespace, as `save::Level` holds
    /// them (booleans as 0 or 1).
    pub fn by_name(&self) -> BTreeMap<String, i32> {
        self.available_rules()
            .map(|r| {
                let v = match self.values.get(r.name).copied().unwrap_or(r.default) {
                    Value::Bool(b) => i32::from(b),
                    Value::Int(i) => i,
                };
                (r.name.to_owned(), v)
            })
            .collect()
    }
    /// GameRuleMap codec: `{data: {"minecraft:<rule>": value...}, DataVersion}`.
    pub fn to_nbt(&self) -> NamedTag {
        let data: Vec<(JavaString, Tag)> = self
            .available_rules()
            .map(|r| {
                let v = self.values.get(r.name).copied().unwrap_or(r.default);
                let tag = match v {
                    Value::Bool(b) => Tag::Byte(i8::from(b)),
                    Value::Int(i) => Tag::Int(i),
                };
                (
                    JavaString::from(format!("minecraft:{}", r.name).as_str()),
                    tag,
                )
            })
            .collect();
        NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("data", Tag::compound(data)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        }
    }
    pub fn from_nbt(root: &Tag) -> Self {
        Self::from_nbt_with_features(root, &[])
    }
    pub fn from_nbt_with_features(root: &Tag, features: &[String]) -> Self {
        let mut rules = Self::with_features(features);
        let Some(data) = root.get("data") else {
            return rules;
        };
        for r in rules.available_rules() {
            let key = format!("minecraft:{}", r.name);
            let value = data.get(&key).or_else(|| data.get(r.name));
            match (r.default, value) {
                (Value::Bool(_), Some(Tag::Byte(v))) => {
                    rules.set(r.name, Value::Bool(*v != 0));
                }
                (Value::Int(_), Some(Tag::Int(v))) => {
                    rules.set(r.name, Value::Int(*v));
                }
                _ => {}
            }
        }
        rules
    }
    pub fn load(world: &Path) -> Self {
        Self::load_with_features(world, &[])
    }
    pub fn load_with_features(world: &Path, features: &[String]) -> Self {
        WorldSavedData::new(world)
            .load_game_rules(features)
            .unwrap_or_else(|_| Self::with_features(features))
    }
    pub fn save(&self, world: &Path) -> std::io::Result<()> {
        WorldSavedData::new(world).save_game_rules(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct RuleLevel {
        rules: GameRules,
        random: horizon_core::random::LegacyRandom,
    }

    impl crate::level::LevelAccess for RuleLevel {
        fn block_state(
            &self,
            _pos: horizon_core::position::BlockPos,
        ) -> crate::registry::BlockStateId {
            crate::registry::BlockStateId(0)
        }
        fn set_raw(
            &mut self,
            _pos: horizon_core::position::BlockPos,
            _state: crate::registry::BlockStateId,
        ) -> Option<crate::registry::BlockStateId> {
            None
        }
        fn is_loaded(&self, _pos: horizon_core::position::BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            0
        }
        fn game_rule_catalog(&self) -> &GameRules {
            &self.rules
        }
    }

    #[test]
    fn catalog_names_are_unique_and_integer_defaults_are_in_bounds() {
        let mut names = std::collections::BTreeMap::new();
        for r in RULES.iter().chain([&MAX_MINECART_SPEED]) {
            assert!(
                names.insert(r.name, ()).is_none(),
                "duplicate rule: {}",
                r.name
            );
            if let Value::Int(default) = r.default {
                assert!(
                    (r.min..=r.max).contains(&default),
                    "bad default: {}",
                    r.name
                );
            }
        }
    }

    #[test]
    fn network_values_validate_rule_names_types_and_bounds() {
        let mut rules = GameRules::default();
        assert!(rules.set_string("minecraft:keep_inventory", "true"));
        assert!(rules.bool("keep_inventory"));
        assert!(!rules.set_string("minecraft:keep_inventory", "1"));
        assert!(rules.bool("keep_inventory"));
        assert!(rules.set_string("minecraft:random_tick_speed", "7"));
        assert_eq!(rules.int("random_tick_speed"), 7);
        assert!(!rules.set_string("minecraft:random_tick_speed", "-1"));
        assert!(!rules.set_string("minecraft:random_tick_speed", "999999999999"));
        assert!(!rules.set_string("minecraft:unknown", "true"));
        assert!(!rules.set_string("other:keep_inventory", "false"));
        assert_eq!(rules.int("random_tick_speed"), 7);
    }
    #[test]
    fn defaults_bounds_and_round_trip() {
        let mut rules = GameRules::default();
        assert!(rules.bool("advance_time"));
        assert_eq!(rules.int("random_tick_speed"), 3);
        assert!(!rules.set("random_tick_speed", Value::Bool(true)));
        assert!(!rules.set("max_snow_accumulation_height", Value::Int(9)));
        assert!(rules.set("keep_inventory", Value::Bool(true)));
        assert!(rules.set("minecraft:random_tick_speed", Value::Int(10)));
        let back = GameRules::from_nbt(&rules.to_nbt().tag);
        assert_eq!(back, rules);
        assert!(back.bool("keep_inventory"));
        let dir = std::env::temp_dir().join(format!("horizon-gamerules-{}", std::process::id()));
        rules.save(&dir).unwrap();
        assert_eq!(GameRules::load(&dir), rules);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn experimental_rule_is_gated_and_copied_with_features() {
        let mut rules = GameRules::default();
        assert_eq!(rules.get("max_minecart_speed"), None);
        let feature = vec![MINECART_IMPROVEMENTS.to_owned()];
        rules = rules.copy_with_features(&feature);
        assert_eq!(rules.get("max_minecart_speed"), Some(Value::Int(8)));
        assert!(rules.set_string("max_minecart_speed", "24"));
        assert_eq!(rules.int("max_minecart_speed"), 24);
        let encoded = rules.to_nbt();
        assert_eq!(
            GameRules::from_nbt_with_features(&encoded.tag, &feature).int("max_minecart_speed"),
            24
        );
        assert_eq!(
            GameRules::from_nbt(&encoded.tag).get("max_minecart_speed"),
            None
        );
        assert_eq!(
            rules.copy_with_features(&[]).get("max_minecart_speed"),
            None
        );
    }

    #[test]
    fn level_projection_uses_catalog_defaults_and_overrides() {
        let mut rules = GameRules::default();
        assert_eq!(
            crate::level::GameRules::from(&rules),
            crate::level::GameRules::default()
        );
        assert!(rules.set("lava_source_conversion", Value::Bool(true)));
        assert!(rules.set("random_tick_speed", Value::Int(11)));
        assert!(rules.set("block_drops", Value::Bool(false)));
        let projected = crate::level::GameRules::from(&rules);
        assert!(projected.lava_source_conversion);
        assert_eq!(projected.random_tick_speed, 11);
        assert!(!projected.block_drops);
        assert!(projected.mob_griefing);
    }

    #[test]
    fn level_access_projects_the_host_catalog() {
        let mut rules = GameRules::default();
        assert!(rules.set("random_tick_speed", Value::Int(9)));
        assert!(rules.set("spread_vines", Value::Bool(false)));
        let level = RuleLevel {
            rules,
            random: horizon_core::random::LegacyRandom::new(1),
        };
        let projected = crate::level::LevelAccess::game_rules(&level);
        assert_eq!(projected.random_tick_speed, 9);
        assert!(!projected.spread_vines);
    }

    #[test]
    fn invalid_saved_file_falls_back_to_defaults() {
        let dir =
            std::env::temp_dir().join(format!("horizon-gamerules-invalid-{}", std::process::id()));
        let path = dir.join("data/minecraft/game_rules.dat");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not gzip data").unwrap();
        assert_eq!(GameRules::load(&dir), GameRules::default());
        let _ = fs::remove_dir_all(dir);
    }
}

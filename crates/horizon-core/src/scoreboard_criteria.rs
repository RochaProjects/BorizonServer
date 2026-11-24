//! Vanilla scoreboard criterion and statistic identifiers, independent of scoreboard state.

pub const CRITERIA_DUMMY: &str = "dummy";
pub const CRITERIA_TRIGGER: &str = "trigger";
pub const CRITERIA_DEATH_COUNT: &str = "deathCount";
pub const CRITERIA_PLAYER_KILLS: &str = "playerKillCount";
pub const CRITERIA_TOTAL_KILLS: &str = "totalKillCount";
pub const CRITERIA_HEALTH: &str = "health";
pub const CRITERIA_FOOD: &str = "food";
pub const CRITERIA_AIR: &str = "air";
pub const CRITERIA_ARMOR: &str = "armor";
pub const CRITERIA_XP: &str = "xp";
pub const CRITERIA_LEVEL: &str = "level";

/// `ObjectiveCriteria.getCustomCriteriaNames`, excluding per-team-color criteria.
pub const CUSTOM_CRITERIA: [&str; 11] = [
    CRITERIA_DUMMY,
    CRITERIA_TRIGGER,
    CRITERIA_DEATH_COUNT,
    CRITERIA_PLAYER_KILLS,
    CRITERIA_TOTAL_KILLS,
    CRITERIA_HEALTH,
    CRITERIA_FOOD,
    CRITERIA_AIR,
    CRITERIA_ARMOR,
    CRITERIA_XP,
    CRITERIA_LEVEL,
];

/// Vanilla custom statistic IDs in registration order.
pub const CUSTOM_STATS: &[&str] = &[
    "leave_game",
    "play_time",
    "total_world_time",
    "time_since_death",
    "time_since_rest",
    "sneak_time",
    "walk_one_cm",
    "crouch_one_cm",
    "sprint_one_cm",
    "walk_on_water_one_cm",
    "fall_one_cm",
    "climb_one_cm",
    "fly_one_cm",
    "walk_under_water_one_cm",
    "minecart_one_cm",
    "boat_one_cm",
    "pig_one_cm",
    "happy_ghast_one_cm",
    "horse_one_cm",
    "aviate_one_cm",
    "swim_one_cm",
    "strider_one_cm",
    "nautilus_one_cm",
    "jump",
    "drop",
    "damage_dealt",
    "damage_dealt_absorbed",
    "damage_dealt_resisted",
    "damage_taken",
    "damage_blocked_by_shield",
    "damage_absorbed",
    "damage_resisted",
    "deaths",
    "mob_kills",
    "animals_bred",
    "player_kills",
    "fish_caught",
    "talked_to_villager",
    "traded_with_villager",
    "eat_cake_slice",
    "fill_cauldron",
    "use_cauldron",
    "clean_armor",
    "clean_banner",
    "clean_shulker_box",
    "interact_with_brewingstand",
    "interact_with_beacon",
    "inspect_dropper",
    "inspect_hopper",
    "inspect_dispenser",
    "play_noteblock",
    "tune_noteblock",
    "pot_flower",
    "trigger_trapped_chest",
    "open_enderchest",
    "enchant_item",
    "play_record",
    "interact_with_furnace",
    "interact_with_crafting_table",
    "open_chest",
    "sleep_in_bed",
    "sleep_in_straw_bed",
    "open_shulker_box",
    "open_barrel",
    "interact_with_blast_furnace",
    "interact_with_smoker",
    "interact_with_lectern",
    "interact_with_campfire",
    "interact_with_cartography_table",
    "interact_with_loom",
    "interact_with_stonecutter",
    "bell_ring",
    "raid_trigger",
    "raid_win",
    "interact_with_anvil",
    "interact_with_grindstone",
    "target_hit",
    "interact_with_smithing_table",
];

/// Vanilla statistic categories in registration order.
pub const STAT_TYPES: &[&str] = &[
    "mined",
    "crafted",
    "used",
    "broken",
    "picked_up",
    "dropped",
    "killed",
    "killed_by",
    "custom",
];

/// Builds the vanilla criterion ID for a statistic type and registry key.
pub fn stat_criteria(stat_type: &str, key: &str) -> String {
    format!(
        "minecraft.{}:{}",
        stat_type.trim_start_matches("minecraft:"),
        key.replace(':', ".")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalogs_have_expected_sizes_and_no_duplicate_stats() {
        assert_eq!(CUSTOM_CRITERIA.len(), 11);
        assert_eq!(CUSTOM_STATS.len(), 78);
        assert_eq!(STAT_TYPES.len(), 9);
        assert_eq!(
            CUSTOM_STATS.iter().copied().collect::<BTreeSet<_>>().len(),
            CUSTOM_STATS.len()
        );
    }

    #[test]
    fn stat_criteria_matches_vanilla_identifier_format() {
        assert_eq!(
            stat_criteria("mined", "minecraft:stone"),
            "minecraft.mined:minecraft.stone"
        );
        assert_eq!(
            stat_criteria("minecraft:custom", "minecraft:play_time"),
            "minecraft.custom:minecraft.play_time"
        );
    }
}

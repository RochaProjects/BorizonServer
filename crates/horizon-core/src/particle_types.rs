//! Vanilla particle registry paths used to validate and suggest `/particle` arguments.
//!
//! This is deliberately only an ordered name catalog. Per-particle client implementation,
//! sprite, transparency, and render-layer flags remain outside the server-owned core.
pub const VANILLA_PARTICLE_TYPES: &[&str] = &[
    "block",
    "block_crumble",
    "trail",
    "dust_pillar",
    "falling_dust",
    "smoke",
    "large_smoke",
    "white_smoke",
    "flame",
    "soul_fire_flame",
    "copper_fire_flame",
    "small_flame",
    "lava",
    "dripping_water",
    "falling_water",
    "dripping_lava",
    "falling_lava",
    "landing_lava",
    "dripping_honey",
    "falling_honey",
    "landing_honey",
    "falling_nectar",
    "falling_spore_blossom",
    "spore_blossom_air",
    "dripping_obsidian_tear",
    "falling_obsidian_tear",
    "landing_obsidian_tear",
    "dripping_dripstone_water",
    "falling_dripstone_water",
    "dripping_dripstone_lava",
    "falling_dripstone_lava",
    "splash",
    "rain",
    "bubble",
    "sulfur_bubbles",
    "noxious_gas",
    "noxious_gas_cloud",
    "geyser",
    "geyser_base",
    "geyser_poof",
    "geyser_plume",
    "bubble_pop",
    "bubble_column_up",
    "current_down",
    "underwater",
    "portal",
    "reverse_portal",
    "enchant",
    "nautilus",
    "vault_connection",
    "crit",
    "enchanted_hit",
    "damage_indicator",
    "heart",
    "angry_villager",
    "happy_villager",
    "composter",
    "mycelium",
    "dolphin",
    "egg_crack",
    "poof",
    "explosion",
    "explosion_emitter",
    "campfire_cosy_smoke",
    "campfire_signal_smoke",
    "note",
    "dust",
    "totem_of_undying",
    "firework",
    "end_rod",
    "ash",
    "white_ash",
    "crimson_spore",
    "warped_spore",
    "cherry_leaves",
    "pale_oak_leaves",
    "tinted_leaves",
    "effect",
    "instant_effect",
    "entity_effect",
    "witch",
    "infested",
    "cloud",
    "sneeze",
    "snowflake",
    "soul",
    "sculk_soul",
    "dragon_breath",
    "gust",
    "small_gust",
    "gust_emitter_large",
    "gust_emitter_small",
    "dust_color_transition",
    "sculk_charge",
    "sculk_charge_pop",
    "shriek",
    "sonic_boom",
    "vibration",
    "sweep_attack",
    "glow",
    "wax_on",
    "wax_off",
    "electric_spark",
    "scrape",
    "squid_ink",
    "glow_squid_ink",
    "item",
    "item_slime",
    "item_snowball",
    "item_cobweb",
    "sulfur_cube_goo",
    "flash",
    "firefly",
    "trial_spawner_detection",
    "trial_spawner_detection_ominous",
    "trial_omen",
    "raid_omen",
    "ominous_spawning",
    "spit",
    "fishing",
    "dust_plume",
    "red_poplar_leaves",
    "orange_poplar_leaves",
    "yellow_poplar_leaves",
    "pause_mob_growth",
    "reset_mob_growth",
    "block_marker",
];

/// Return the vanilla path when `name` is a known bare or `minecraft:` ID.
pub fn from_name(name: &str) -> Option<&'static str> {
    let path = name.strip_prefix("minecraft:").unwrap_or(name);
    VANILLA_PARTICLE_TYPES
        .iter()
        .copied()
        .find(|candidate| *candidate == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_matches_vanilla_order_and_has_no_duplicates() {
        assert_eq!(VANILLA_PARTICLE_TYPES.len(), 127);
        assert_eq!(VANILLA_PARTICLE_TYPES.first(), Some(&"block"));
        assert_eq!(VANILLA_PARTICLE_TYPES.last(), Some(&"block_marker"));
        assert_eq!(
            VANILLA_PARTICLE_TYPES
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            VANILLA_PARTICLE_TYPES.len()
        );
    }

    #[test]
    fn lookup_accepts_bare_and_minecraft_ids_only() {
        assert_eq!(from_name("block"), Some("block"));
        assert_eq!(from_name("minecraft:gust"), Some("gust"));
        assert_eq!(from_name("example:gust"), None);
        assert_eq!(from_name("minecraft:not_a_particle"), None);
    }
}

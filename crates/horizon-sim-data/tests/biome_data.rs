use horizon_sim_data::biome::Biomes;
use serde_json::json;

#[test]
fn biome_packs_keep_builtin_ids_and_expose_server_spawn_and_temperature_data() {
    let base = Biomes::from_jsons(&[
        ("minecraft:meadow", json!({"temperature": 0.8})),
        ("minecraft:plains", json!({"temperature": 0.8})),
    ]);
    let replacement = json!({
        "temperature": -0.5,
        "has_precipitation": true,
        "features": [["minecraft:ore_iron_upper"]],
        "attributes": {
            "minecraft:gameplay/natural_mob_spawns": {
                "argument": {
                    "spawns_by_category": {
                        "monster": [{
                            "type": "minecraft:zombie",
                            "weight": 80,
                            "count": {
                                "type": "minecraft:uniform",
                                "min_inclusive": 1,
                                "max_inclusive": 4
                            }
                        }],
                        "creature": [{
                            "type": "minecraft:sheep",
                            "weight": 12,
                            "count": 4
                        }]
                    },
                    "spawn_costs": {
                        "minecraft:zombie": {
                            "energy_budget": 1.0,
                            "charge": 0.2
                        }
                    }
                }
            }
        }
    });
    let added = json!({"temperature": 0.4});
    let datapack = base.with_packs([("minecraft:plains", &replacement), ("pack:extra", &added)]);

    assert_eq!(datapack.id("minecraft:meadow"), 0);
    assert_eq!(datapack.id("minecraft:plains"), 1);
    assert_eq!(datapack.id("pack:extra"), 2);

    let plains = datapack.get(datapack.id("minecraft:plains"));
    assert_eq!(plains.temperature, -0.5);
    assert!(!plains.frozen);
    assert_eq!(plains.features, [vec!["minecraft:ore_iron_upper"]]);
    let zombies = &plains.spawns["monster"];
    assert_eq!(zombies.len(), 1);
    assert_eq!(zombies[0].entity, "minecraft:zombie");
    assert_eq!(
        (zombies[0].min, zombies[0].max, zombies[0].constant),
        (1, 4, false)
    );
    let sheep = &plains.spawns["creature"];
    assert_eq!(
        (sheep[0].min, sheep[0].max, sheep[0].constant),
        (4, 4, true)
    );
    assert_eq!(plains.spawn_costs["minecraft:zombie"].energy_budget, 1.0);
    assert_eq!(plains.spawn_costs["minecraft:zombie"].charge, 0.2);
    assert_eq!(plains.snows_at(0, 100, 0), Some(true));
}

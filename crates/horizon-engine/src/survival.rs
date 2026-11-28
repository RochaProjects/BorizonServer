//! Dedicated-player hunger hooks backed by the server-safe world contracts.

use horizon_core::geometry::Vec3;
use horizon_world::{
    attributes::entity::{Catalog, EntityAttributes},
    combat_rules::ArmorWorld,
    damage_immunity::{self, DamageRule, ImmunityTag, ImmunityWorld},
    damage_reduction::Bypass,
    damage_source::{DamageSource, DamageTypeId, Scaling},
    food::{Difficulty, FoodData, FoodWorld},
    living_damage::DamageStat,
    living_hurt::{HurtState, HurtTag, HurtWorld},
    movement_statistics::{self, MovementStat, MovementStatisticsWorld},
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    player_hurt::{CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld},
    server_player_combat::{CombatEnvironment, ServerPlayerCombat},
    server_player_damage::ServerDamageEnvironment,
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
use std::{convert::Infallible, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct PlayerHealth {
    attributes: EntityAttributes,
    data: SyncedFloats,
    ids: PlayerFloatIds,
    hurt: HurtState<DamageTypeId>,
    dead: bool,
    death_message: Option<String>,
}

impl PlayerHealth {
    pub(crate) fn new(
        catalog: Arc<Catalog>,
        ids: PlayerFloatIds,
        initial_health: f32,
    ) -> Result<Self, String> {
        let attributes = catalog
            .create("minecraft:player")
            .ok_or("attributes.json has no minecraft:player defaults")?;
        let data = SyncedFloats::new(&[
            FloatValue {
                id: ids.health,
                value: initial_health,
            },
            FloatValue {
                id: ids.absorption,
                value: 0.0,
            },
        ])
        .map_err(|error| format!("initialize player health data: {error}"))?;
        let mut player = Self {
            attributes,
            data,
            ids,
            hurt: HurtState {
                no_action_time: 0,
                cooldown: 0,
                last_hurt: 0.0,
                hurt_duration: 0,
                hurt_time: 0,
                last_source: None,
                last_stamp: 0,
            },
            dead: initial_health <= 0.0,
            death_message: None,
        };
        player.set_health(initial_health)?;
        Ok(player)
    }

    pub(crate) fn health(&self) -> f32 {
        self.data
            .get(self.ids.health)
            .expect("validated player health data ID")
    }

    fn set_health(&mut self, value: f32) -> Result<(), String> {
        let mut observer = NoopHealthObserver;
        let mut health = PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut self.data,
            self.ids,
            self.dead,
            &mut observer,
        )
        .map_err(|error| format!("bind player health: {error}"))?;
        health
            .set_health(value)
            .map_err(|error| format!("set player health: {error}"))
    }

    fn heal(&mut self, amount: f32) -> Result<(), String> {
        let mut observer = NoopHealthObserver;
        let mut health = PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut self.data,
            self.ids,
            self.dead,
            &mut observer,
        )
        .map_err(|error| format!("bind player health: {error}"))?;
        health
            .heal(amount)
            .map_err(|error| format!("heal player: {error}"))
    }

    fn is_hurt(&mut self) -> Result<bool, String> {
        let mut observer = NoopHealthObserver;
        let mut health = PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut self.data,
            self.ids,
            self.dead,
            &mut observer,
        )
        .map_err(|error| format!("bind player health: {error}"))?;
        health
            .is_hurt()
            .map_err(|error| format!("read player health: {error}"))
    }

    pub(crate) fn is_dead(&self) -> bool {
        self.dead
    }

    pub(crate) fn death_message(&self) -> &str {
        self.death_message
            .as_deref()
            .unwrap_or("death.attack.generic")
    }

    pub(crate) fn respawn(&mut self, health: f32) -> Result<(), String> {
        self.dead = false;
        self.death_message = None;
        self.hurt = HurtState {
            no_action_time: 0,
            cooldown: 0,
            last_hurt: 0.0,
            hurt_duration: 0,
            hurt_time: 0,
            last_source: None,
            last_stamp: 0,
        };
        self.set_health(health)
    }
}

struct NoopHealthObserver;

impl Observer for NoopHealthObserver {
    type Error = Infallible;

    fn updated(&mut self, _: &mut SyncedFloats, _: u8) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct FoodTick<'a> {
    health: &'a mut PlayerHealth,
    damage_types: &'a horizon_world::damage_source::Catalog,
    difficulty: Difficulty,
    natural_regeneration: bool,
    invulnerable: bool,
    game_time: i64,
}

impl FoodWorld for FoodTick<'_> {
    type Error = String;
    type Level = ();

    fn level(&mut self, _: &mut FoodData) -> Result<Self::Level, Self::Error> {
        Ok(())
    }

    fn difficulty(&mut self, _: Self::Level, _: &mut FoodData) -> Result<Difficulty, Self::Error> {
        Ok(self.difficulty)
    }

    fn natural_regeneration(
        &mut self,
        _: Self::Level,
        _: &mut FoodData,
    ) -> Result<bool, Self::Error> {
        Ok(self.natural_regeneration)
    }

    fn is_hurt(&mut self, _: &mut FoodData) -> Result<bool, Self::Error> {
        self.health.is_hurt()
    }

    fn heal(&mut self, _: &mut FoodData, amount: f32) -> Result<(), Self::Error> {
        self.health.heal(amount)
    }

    fn health(&mut self, _: &mut FoodData) -> Result<f32, Self::Error> {
        Ok(self.health.health())
    }

    fn hurt_starvation(
        &mut self,
        _: Self::Level,
        food: &mut FoodData,
        amount: f32,
    ) -> Result<(), Self::Error> {
        if self.invulnerable || self.health.dead {
            return Ok(());
        }
        let id = self
            .damage_types
            .id("minecraft:starve")
            .ok_or("damage catalog has no minecraft:starve")?;
        let source = self
            .damage_types
            .source::<()>(id, None, None, None)
            .ok_or("damage catalog cannot construct minecraft:starve")?;
        let PlayerHealth {
            attributes,
            data,
            ids,
            hurt,
            dead,
            death_message,
            ..
        } = &mut *self.health;
        let mut env = StarvationDamage {
            source,
            dead,
            death_message,
            invulnerable: false,
            difficulty: self.difficulty,
            game_time: self.game_time,
        };
        let mut combat = ServerPlayerCombat::new(attributes, data, food, *ids, &mut env)
            .map_err(|error| format!("initialize starvation combat: {error}"))?;
        combat
            .hurt(hurt, amount)
            .map(|_| ())
            .map_err(|error| format!("apply starvation damage: {error}"))?;
        Ok(())
    }
}

/// The starvation hit uses the same server-player hurt pipeline as other
/// vanilla damage. Equipment/effects/entities are absent from this runtime
/// slice, so their adapters report the empty-world values explicitly.
struct StarvationDamage<'a> {
    source: DamageSource<'a, ()>,
    dead: &'a mut bool,
    death_message: &'a mut Option<String>,
    invulnerable: bool,
    difficulty: Difficulty,
    game_time: i64,
}

impl Observer for StarvationDamage<'_> {
    type Error = Infallible;
    fn updated(&mut self, _: &mut SyncedFloats, _: u8) -> Result<(), Self::Error> {
        Ok(())
    }
}
impl ArmorWorld for StarvationDamage<'_> {
    type Error = Infallible;
    type Weapon = ();
    type ServerLevel = ();
    fn weapon(&mut self) -> Result<Option<()>, Self::Error> {
        Ok(None)
    }
    fn server_level(&mut self) -> Result<Option<()>, Self::Error> {
        Ok(Some(()))
    }
    fn modify_armor_effectiveness(&mut self, _: (), _: (), value: f32) -> Result<f32, Self::Error> {
        Ok(value)
    }
}
impl ServerDamageEnvironment for StarvationDamage<'_> {
    type Attacker = ();
    fn invulnerable(&mut self) -> Result<bool, Infallible> {
        Ok(damage_immunity::server_player_immunity(self).unwrap_or(true))
    }
    fn bypasses(&mut self, kind: Bypass) -> Result<bool, Infallible> {
        Ok(self.source.bypasses(kind))
    }
    fn hurt_armor(&mut self, _: &mut EntityAttributes, _: f32) -> Result<(), Infallible> {
        Ok(())
    }
    fn has_resistance(&mut self) -> Result<bool, Infallible> {
        Ok(false)
    }
    fn resistance_amplifier(&mut self) -> Result<i32, Infallible> {
        Ok(0)
    }
    fn award_resisted(&mut self, _: i32) -> Result<(), Infallible> {
        Ok(())
    }
    fn attacker(&mut self) -> Result<Option<()>, Infallible> {
        Ok(None)
    }
    fn award_dealt_resisted(&mut self, _: i32) -> Result<(), Infallible> {
        Ok(())
    }
    fn damage_protection(&mut self, _: ()) -> Result<f32, Infallible> {
        Ok(0.0)
    }
    fn award_self(&mut self, _: DamageStat, _: i32) -> Result<(), Infallible> {
        Ok(())
    }
    fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> Result<(), Infallible> {
        Ok(())
    }
    fn source_food_exhaustion(&mut self) -> Result<f32, Infallible> {
        Ok(self.source.definition().exhaustion)
    }
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    fn level_is_client(&mut self) -> Result<bool, Infallible> {
        Ok(false)
    }
    fn record_damage(&mut self, _: f32) -> Result<(), Infallible> {
        Ok(())
    }
    fn damage_event(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}
impl CombatEnvironment for StarvationDamage<'_> {
    fn dead_flag(&self) -> bool {
        *self.dead
    }
}
impl ImmunityWorld for StarvationDamage<'_> {
    type Error = Infallible;
    fn removed(&self) -> bool {
        false
    }
    fn permanently_invulnerable(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn temporarily_invulnerable(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn tag(&mut self, tag: ImmunityTag) -> Result<bool, Self::Error> {
        Ok(self.source.immunity_tag(tag))
    }
    fn creative_source(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn fire_immune(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn fall_immune(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn enchantment_immunity(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn damage_rule(&mut self, rule: DamageRule) -> Result<bool, Self::Error> {
        Ok(match rule {
            DamageRule::Drowning | DamageRule::Fall | DamageRule::Fire | DamageRule::Freeze => true,
        })
    }
    fn changing_dimension(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn ender_pearl(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn client_loaded(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
}
impl HurtWorld for StarvationDamage<'_> {
    type Error = Infallible;
    type Source = DamageTypeId;
    type UseItem = ();
    type Blocking = ();
    type Sound = ();
    fn source(&self) -> DamageTypeId {
        self.source.type_id()
    }
    fn is_server_player(&self) -> bool {
        true
    }
    fn invulnerable(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<bool, Self::Error> {
        Ok(damage_immunity::server_player_immunity(self).unwrap_or(true))
    }
    fn dead_or_dying(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<bool, Self::Error> {
        Ok(*self.dead)
    }
    fn tag(&mut self, _: &mut HurtState<DamageTypeId>, tag: HurtTag) -> Result<bool, Self::Error> {
        Ok(self.source.hurt_tag(tag))
    }
    fn fire_resistance(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn sleeping(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn stop_sleeping(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn use_item(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn apply_blocking(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: f32,
    ) -> Result<f32, Self::Error> {
        Ok(0.0)
    }
    fn freeze_hurts_extra(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn helmet_empty(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn hurt_helmet(&mut self, _: &mut HurtState<DamageTypeId>, _: f32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn actually_hurt(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: f32,
    ) -> Result<(), Self::Error> {
        unreachable!("ServerPlayerCombat owns inner damage")
    }
    fn resolve_mob(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn resolve_player(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn blocking_component(&mut self, _: ()) -> Option<()> {
        None
    }
    fn on_blocked(&mut self, _: &mut HurtState<DamageTypeId>, _: ()) -> Result<(), Self::Error> {
        Ok(())
    }
    fn broadcast_damage(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn mark_hurt(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn knockback(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: f32,
        _: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn death_protection(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn death_sound(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn make_sound(&mut self, _: &mut HurtState<DamageTypeId>, _: ()) -> Result<(), Self::Error> {
        Ok(())
    }
    fn secondary_sound(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn die(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        *self.dead = true;
        *self.death_message = Some(format!(
            "death.attack.{}",
            self.source.definition().message_id
        ));
        Ok(())
    }
    fn hurt_sound(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn game_time(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<i64, Self::Error> {
        Ok(self.game_time)
    }
    fn effects_on_hurt(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: f32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn player_hurt_criterion(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: f32,
        _: f32,
        _: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn award_blocked(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: i32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn attacker_criterion(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: f32,
        _: f32,
        _: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
impl PlayerHurtWorld for StarvationDamage<'_> {
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    fn bypasses_invulnerability(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
    ) -> Result<bool, Self::Error> {
        Ok(self.source.in_tag("minecraft:bypasses_invulnerability"))
    }
    fn remove_shoulders(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<(), Self::Error> {
        Ok(())
    }
    fn scales_with_difficulty(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
    ) -> Result<bool, Self::Error> {
        Ok(!matches!(self.source.definition().scaling, Scaling::Never))
    }
    fn difficulty(&mut self, _: &mut HurtState<DamageTypeId>) -> Result<Difficulty, Self::Error> {
        Ok(self.difficulty)
    }
}
impl ServerPlayerHurtWorld for StarvationDamage<'_> {
    type Player = ();
    type Arrow = ();
    fn causing_entity(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
    ) -> Result<CausingEntity<(), ()>, Self::Error> {
        Ok(CausingEntity::Other)
    }
    fn arrow_player_owner(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: (),
    ) -> Result<Option<()>, Self::Error> {
        Ok(None)
    }
    fn can_harm_player(
        &mut self,
        _: &mut HurtState<DamageTypeId>,
        _: (),
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub(crate) fn tick_food(
    health: &mut PlayerHealth,
    damage_types: &horizon_world::damage_source::Catalog,
    food: &mut FoodData,
    difficulty: Difficulty,
    natural_regeneration: bool,
    invulnerable: bool,
    server_tick: u64,
) -> Result<(), String> {
    // ServerPlayer.tickRegeneration runs before the ordinary FoodData tick.
    if difficulty == Difficulty::Peaceful && natural_regeneration && server_tick.is_multiple_of(20)
    {
        if health.is_hurt()? {
            health.heal(1.0)?;
        }
        if food.saturation < 20.0 {
            food.saturation += 1.0;
        }
    }
    if server_tick.is_multiple_of(10) && food.needs_food() {
        food.food = food.food.wrapping_add(1);
    }

    let mut world = FoodTick {
        health,
        damage_types,
        difficulty,
        natural_regeneration,
        invulnerable,
        game_time: server_tick as i64,
    };
    food.tick(&mut world)
}

struct MovementFood<'a> {
    food: &'a mut FoodData,
    grounded: bool,
    sprinting: bool,
    crouching: bool,
}

impl MovementStatisticsWorld for MovementFood<'_> {
    type Error = Infallible;

    fn passenger(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn swimming(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn eyes_in_water(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn in_water(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn on_climbable(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn on_ground(&mut self) -> Result<bool, Self::Error> {
        Ok(self.grounded)
    }
    fn sprinting(&mut self) -> Result<bool, Self::Error> {
        Ok(self.sprinting)
    }
    fn crouching(&mut self) -> Result<bool, Self::Error> {
        Ok(self.crouching)
    }
    fn fall_flying(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn award(&mut self, _: MovementStat, _: i32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn exhaust(&mut self, amount: f32) -> Result<(), Self::Error> {
        self.food.add_exhaustion(amount);
        Ok(())
    }
}

pub(crate) fn accepted_movement(
    food: &mut FoodData,
    from: [f64; 3],
    to: [f64; 3],
    grounded: bool,
    sprinting: bool,
    crouching: bool,
    invulnerable: bool,
) {
    if invulnerable {
        return;
    }
    let mut world = MovementFood {
        food,
        grounded,
        sprinting,
        crouching,
    };
    let delta = Vec3::new(to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    match movement_statistics::check(&mut world, delta) {
        Ok(()) => {}
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use super::PlayerHealth;
    use horizon_world::{attributes::entity::Catalog, player_health::PlayerFloatIds};
    use serde_json::json;
    use std::sync::Arc;

    fn bits(value: f64) -> String {
        format!("{:016x}", value.to_bits())
    }

    fn catalog(max_health: f64) -> Arc<Catalog> {
        let attributes = ["minecraft:max_health", "minecraft:max_absorption"]
            .into_iter()
            .map(|id| {
                let default = if id == "minecraft:max_health" {
                    max_health
                } else {
                    0.0
                };
                json!({
                    "id": id,
                    "default": bits(default),
                    "syncable": true,
                    "min": bits(0.0),
                    "max": bits(1024.0)
                })
            })
            .collect::<Vec<_>>();
        let entity_attributes = json!({
            "minecraft:max_health": bits(max_health),
            "minecraft:max_absorption": bits(0.0)
        });
        Arc::new(
            Catalog::from_json(&json!({
                "data_version": horizon_world::DATA_VERSION,
                "attributes": attributes,
                "entities": { "minecraft:player": entity_attributes }
            }))
            .unwrap(),
        )
    }

    #[test]
    fn player_health_clamps_and_heals_through_registered_attributes() {
        let ids = PlayerFloatIds {
            health: 9,
            absorption: 17,
        };
        let mut player = PlayerHealth::new(catalog(40.0), ids, 60.0).unwrap();
        assert_eq!(player.health(), 40.0);

        player.set_health(38.0).unwrap();
        player.heal(10.0).unwrap();
        assert_eq!(player.health(), 40.0);
    }
}

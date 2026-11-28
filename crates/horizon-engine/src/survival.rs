//! Dedicated-player hunger hooks backed by the server-safe world contracts.

use horizon_core::geometry::Vec3;
use horizon_world::{
    food::{Difficulty, FoodData, FoodWorld},
    movement_statistics::{self, MovementStat, MovementStatisticsWorld},
};
use std::convert::Infallible;

struct FoodTick<'a> {
    health: &'a mut f32,
    max_health: f32,
    difficulty: Difficulty,
    natural_regeneration: bool,
    invulnerable: bool,
}

impl FoodWorld for FoodTick<'_> {
    type Error = Infallible;
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
        Ok(*self.health < self.max_health)
    }

    fn heal(&mut self, _: &mut FoodData, amount: f32) -> Result<(), Self::Error> {
        *self.health = (*self.health + amount).min(self.max_health);
        Ok(())
    }

    fn health(&mut self, _: &mut FoodData) -> Result<f32, Self::Error> {
        Ok(*self.health)
    }

    fn hurt_starvation(
        &mut self,
        _: Self::Level,
        _: &mut FoodData,
        amount: f32,
    ) -> Result<(), Self::Error> {
        // Starvation damage bypasses armor in vanilla. PlayerCombat's full
        // hurt/death owner is still pending; preserve the health loss here.
        if !self.invulnerable {
            *self.health = (*self.health - amount).max(0.0);
        }
        Ok(())
    }
}

pub(crate) fn tick_food(
    health: &mut f32,
    food: &mut FoodData,
    difficulty: Difficulty,
    natural_regeneration: bool,
    invulnerable: bool,
    server_tick: u64,
    max_health: f32,
) {
    // ServerPlayer.tickRegeneration runs before the ordinary FoodData tick.
    if difficulty == Difficulty::Peaceful && natural_regeneration && server_tick.is_multiple_of(20)
    {
        if *health < max_health {
            *health = (*health + 1.0).min(max_health);
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
        max_health,
        difficulty,
        natural_regeneration,
        invulnerable,
    };
    match food.tick(&mut world) {
        Ok(()) => {}
        Err(never) => match never {},
    }
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

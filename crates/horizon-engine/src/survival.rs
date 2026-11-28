//! Dedicated-player hunger hooks backed by the server-safe world contracts.

use horizon_core::geometry::Vec3;
use horizon_world::{
    food::{Difficulty, FoodData, FoodWorld},
    movement_statistics::{self, MovementStat, MovementStatisticsWorld},
};
use std::convert::Infallible;

struct FoodTick<'a> {
    health: &'a mut f32,
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
        Ok(*self.health < 20.0)
    }

    fn heal(&mut self, _: &mut FoodData, amount: f32) -> Result<(), Self::Error> {
        *self.health = (*self.health + amount).min(20.0);
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
) {
    let mut world = FoodTick {
        health,
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

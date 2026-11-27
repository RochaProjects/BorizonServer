//! Server-side vanilla block-mining math shared by headless PLAY owners.
//!
//! Ported from `origin/main` `horizon-items/src/mining.rs` and
//! `horizon-client/src/lan/guest.rs::within_block_range`. The item/tool
//! lookup remains an input owned by the caller; this module does not invent it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DestroySpeedModifiers {
    pub mining_efficiency: f32,
    pub effects: f32,
    pub block_break_speed: f32,
    pub submerged_mining_speed: Option<f32>,
    pub on_ground: bool,
}
impl Default for DestroySpeedModifiers {
    fn default() -> Self {
        Self {
            mining_efficiency: 0.0,
            effects: 1.0,
            block_break_speed: 1.0,
            submerged_mining_speed: None,
            on_ground: true,
        }
    }
}
/// Port of `Player.getDestroySpeed` after the held item's speed has been read.
pub fn destroy_speed(tool_speed: f32, player: &DestroySpeedModifiers) -> f32 {
    let mut speed = tool_speed;
    if speed > 1.0 {
        speed += player.mining_efficiency;
    }
    speed *= player.effects;
    speed *= player.block_break_speed;
    if let Some(submerged) = player.submerged_mining_speed {
        speed *= submerged;
    }
    if !player.on_ground {
        speed /= 5.0;
    }
    speed
}
/// Port of `BlockBehaviour.getDestroyProgress`.
pub fn destroy_progress(hardness: f32, harvest: bool, destroy_speed: f32) -> f32 {
    if hardness == -1.0 {
        return 0.0;
    }
    destroy_speed / hardness / if harvest { 30.0 } else { 100.0 }
}
/// Port of `Player.isWithinBlockInteractionRange`: squared distance from the
/// eye to the nearest point of the target block's unit AABB is `< range²`.
pub fn within_block_range(eye: [f64; 3], range: f64, pos: [i32; 3]) -> bool {
    (0..3)
        .map(|axis| {
            let lo = f64::from(pos[axis]);
            let hi = lo + 1.0;
            let delta = if eye[axis] < lo {
                lo - eye[axis]
            } else if eye[axis] > hi {
                eye[axis] - hi
            } else {
                0.0
            };
            delta * delta
        })
        .sum::<f64>()
        < range * range
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_destroy_progress_divisors_and_unbreakable_hardness() {
        assert_eq!(destroy_progress(1.5, true, 1.0), 1.0 / 1.5 / 30.0);
        assert_eq!(destroy_progress(1.5, false, 1.0), 1.0 / 1.5 / 100.0);
        assert_eq!(destroy_progress(-1.0, true, 100.0), 0.0);
    }
    #[test]
    fn reference_destroy_speed_applies_efficiency_effects_water_and_air_penalty() {
        let grounded = DestroySpeedModifiers {
            mining_efficiency: 2.0,
            effects: 1.5,
            block_break_speed: 0.5,
            submerged_mining_speed: Some(0.2),
            on_ground: true,
        };
        assert_eq!(destroy_speed(3.0, &grounded), 0.75);
        let airborne = DestroySpeedModifiers {
            on_ground: false,
            ..grounded
        };
        assert_eq!(destroy_speed(3.0, &airborne), 0.15);
    }
    #[test]
    fn reference_block_reach_measures_to_block_box_not_its_center() {
        assert!(within_block_range([0.5, 1.0, 0.5], 4.5, [4, 1, 0]));
        assert!(!within_block_range([0.5, 1.0, 0.5], 4.5, [6, 1, 0]));
        assert!(!within_block_range([0.5, 1.0, 0.5], 0.5, [1, 1, 0]));
    }
}

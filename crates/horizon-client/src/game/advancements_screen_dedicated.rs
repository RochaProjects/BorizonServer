//! Dedicated-server stub for the client-only Advancements screen.
//!
//! A dedicated server has no local screen to draw or interact with. Keep the
//! same method surface so shared game/control code can compile, but leave the
//! client UI implementation out of the dedicated build.

use super::Game;

#[derive(Default)]
pub(super) struct AdvUi;

impl Game<'_> {
    pub(super) fn open_advancements(&mut self, _from_pause: bool) {}

    pub(super) fn close_advancements(&mut self) {}

    pub(super) fn draw_advancements(&mut self, _batch: &mut horizon_client::hud::HudBatch) {}

    pub(super) fn advancements_mouse(&mut self, _pressed: bool, _size: [u32; 2]) {}

    pub(super) fn advancements_drag(&mut self) {}

    pub(super) fn advancements_scroll(&mut self, _x: f32, _y: f32) {}
}

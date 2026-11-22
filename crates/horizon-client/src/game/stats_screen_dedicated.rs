//! Dedicated-server stub for the client-only statistics screen.
//!
//! Statistics data and persistence remain in the shared game runtime; only
//! the local table, sorting, scrolling, and rendering UI are omitted here.

use super::Game;
use winit::keyboard::KeyCode;

#[derive(Default)]
pub(super) struct StatsUi;

impl Game<'_> {
    pub(super) fn open_stats(&mut self) {}

    pub(super) fn draw_stats(&mut self, _batch: &mut horizon_client::hud::HudBatch) {}

    pub(super) fn stats_key(&mut self, _key: KeyCode) {}

    pub(super) fn stats_click(&mut self, _size: [u32; 2]) {}

    pub(super) fn stats_scroll(&mut self, _amount: f32) {}

    pub(super) fn stats_press_scrollbar(&mut self, _size: [u32; 2]) -> bool {
        false
    }

    pub(super) fn stats_drag(&mut self) {}

    pub(super) fn stats_release(&mut self) {}
}

//! Horizon cosmetics are a client feature and are not loaded by dedicated servers.
use super::Game;

pub(super) fn enabled() -> bool {
    false
}

pub(super) fn register() -> Result<(), horizon_hooks::TooLate> {
    Ok(())
}

impl Game<'_> {
    pub(super) fn cosmetics_say(&mut self) {}

    pub(super) fn cosmetics_frame(&mut self) {}

    pub(in crate::game) fn tick_emotes(&mut self) {}

    pub(in crate::game) fn open_emote_wheel(&mut self) {}

    pub(in crate::game) fn close_emote_wheel(&mut self, _play: Option<usize>) {}

    pub(in crate::game) fn draw_emote_wheel(&mut self, _batch: &mut horizon_client::hud::HudBatch) {
    }

    pub(in crate::game) fn emote_wheel_event(
        &mut self,
        _event: &winit::event::WindowEvent,
    ) -> bool {
        false
    }
}

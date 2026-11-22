//! No-op title presentation surface for the dedicated-server target.
use super::*;

impl Title {
    pub fn draw(
        &self,
        _atlas: &HudAtlas,
        _batch: &mut HudBatch,
        _cursor: (f32, f32),
        _hide_splash: bool,
    ) {
    }

    /// Dedicated servers do not render title-screen item previews.
    pub fn draw_with_items(
        &self,
        _atlas: &HudAtlas,
        _items: Option<&crate::item_render::ItemRenderer>,
        _batch: &mut HudBatch,
        _cursor: (f32, f32),
        _hide_splash: bool,
        _hide_server_address: bool,
    ) {
    }
}

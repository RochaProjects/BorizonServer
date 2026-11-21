//! Pure data types shared between block-entity simulation and rendering:
//! dye colours, animation state, and the enchanting-table book.
//! No GPU, shader, or window dependencies.

use glam::Mat4;

/// Minecraft dye colour names and their ARGB hex values.
pub const DYES: [(&str, u32); 16] = [
    ("white", 0xF9FFFE),
    ("orange", 0xF9801D),
    ("magenta", 0xC74EBD),
    ("light_blue", 0x3AB3DA),
    ("yellow", 0xFED83D),
    ("lime", 0x80C71F),
    ("pink", 0xF38BAA),
    ("gray", 0x474F52),
    ("light_gray", 0x9D9D97),
    ("cyan", 0x169C9C),
    ("purple", 0x8932B8),
    ("blue", 0x3C44AA),
    ("brown", 0x835432),
    ("green", 0x5E7C16),
    ("red", 0xB02E26),
    ("black", 0x1D1D21),
];

/// `EnchantingTableBlockEntity` book fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnchantingBook {
    pub time: i32,
    pub flip: f32,
    pub o_flip: f32,
    pub flip_t: f32,
    pub flip_a: f32,
    pub open: f32,
    pub o_open: f32,
    pub rot: f32,
    pub o_rot: f32,
    pub t_rot: f32,
}

/// Per-block-entity animation state read by the renderer each frame.
#[derive(Default)]
pub struct BlockAnim {
    /// `ShulkerBoxBlockEntity` progress and progressOld.
    pub shulker: (f32, f32),
    /// `BellBlockEntity.ticks` and the click direction while `shaking`.
    pub bell_ticks: i32,
    pub bell_shake: Option<&'static str>,
    /// `ConduitBlockEntity` tickCount, isActive, isHunting and activeRotation.
    pub conduit: (i32, bool, bool, f32),
    /// `SkullBlockEntity` isAnimating and animationTickCount.
    pub skull: (bool, i32),
    /// `DecoratedPotBlockEntity` decorations as item ids: back, left, right, front.
    pub sherds: [Option<String>; 4],
    /// `DecoratedPotRenderState` wobble style (positive) and progress.
    pub wobble: Option<(bool, f32)>,
    /// `EnchantingTableBlockEntity` book fields.
    pub book: EnchantingBook,
    /// `TheEndGatewayBlockEntity` age and teleportCooldown.
    pub gateway: (i64, i32),
}

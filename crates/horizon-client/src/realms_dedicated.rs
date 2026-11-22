//! Dedicated-server compatibility surface for the title code.
//!
//! Realms is an interactive client service. The dedicated server never opens this
//! screen and cannot initiate Realms connections; keep only the types and inert
//! methods needed by shared title code and the (disabled) pending-pack lookup.
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct RealmServer;

#[derive(Clone, Debug)]
pub struct CreateWorldRequest {
    pub server: RealmServer,
    pub slot: i32,
    pub kind: ResetKind,
    pub creation: Option<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetKind {
    Reset,
    Empty,
    NewRealm,
}

#[derive(Clone, Debug)]
pub struct CreatedWorld {
    pub level: crate::save::Level,
    pub world_gen: crate::save::WorldGen,
    pub flat_settings: Option<horizon_world::nbt::Tag>,
    pub single_biome: Option<String>,
    pub data_packs: (Vec<String>, Vec<String>),
    pub pack_dir: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    None,
    Back,
    Connect(String),
    OpenUri(String),
    Copy(String),
    CreateWorld(CreateWorldRequest),
}

#[derive(Clone, Debug, Default)]
pub struct Screen;

impl Screen {
    pub fn with_saves(_saves: Option<PathBuf>) -> Self {
        Self
    }

    pub fn from_title(_saves: Option<PathBuf>) -> Self {
        Self
    }

    pub fn cancel_create_world(&mut self, _request: CreateWorldRequest) {}

    pub fn start_create_world_upload(
        &mut self,
        _request: CreateWorldRequest,
        _world: CreatedWorld,
    ) {
    }

    pub fn drag(&mut self, _cursor: (f32, f32), _size: (f32, f32)) {}

    pub fn release(&mut self) {}

    pub fn wheel(&mut self, _delta: f32, _cursor: (f32, f32), _size: (f32, f32)) {}

    pub fn poll(&mut self) -> Outcome {
        Outcome::None
    }

    pub fn draw(
        &self,
        _batch: &mut crate::hud::HudBatch,
        _atlas: &crate::hud::HudAtlas,
        _caret: &mut crate::ui::Caret,
        _started: Instant,
        _cursor: (f32, f32),
        _w: f32,
        _h: f32,
        _list_focused: bool,
    ) {
    }

    pub fn draw_header_badges(
        &self,
        _atlas: &crate::hud::HudAtlas,
        _batch: &mut crate::hud::HudBatch,
        _w: f32,
    ) {
    }

    pub fn draw_tooltip(
        &self,
        _atlas: &crate::hud::HudAtlas,
        _batch: &mut crate::hud::HudBatch,
        _cursor: (f32, f32),
        _size: (f32, f32),
    ) {
    }

    pub fn tab_key(
        &mut self,
        _key: winit::keyboard::KeyCode,
        _ctrl: bool,
        _shift: bool,
    ) -> Option<bool> {
        None
    }

    pub fn click(&mut self, _hit: Option<u8>, _x: f32, _y: f32, _size: (f32, f32)) -> Outcome {
        Outcome::None
    }

    pub fn take_list_click(&mut self) -> bool {
        false
    }

    pub fn focused_text(&mut self) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        None
    }

    pub fn focused_box(&self, _size: (f32, f32)) -> Option<[f32; 4]> {
        None
    }

    pub fn list_stop(&self) -> bool {
        false
    }

    pub fn pdp_arrow(&mut self, _up: bool) -> bool {
        false
    }

    pub fn main_arrow(&mut self, _up: bool) -> bool {
        false
    }

    pub fn activate_selected(&mut self) -> Outcome {
        Outcome::None
    }

    pub fn escape(&mut self) -> bool {
        true
    }

    pub fn narration(&self) -> Option<String> {
        None
    }

    pub fn buttons(
        &self,
        _w: f32,
        _h: f32,
    ) -> Vec<(f32, f32, f32, String, bool, u8, Option<(&'static str, f32)>)> {
        Vec::new()
    }
}

/// A dedicated server has no client Realms session that could provide a pack.
pub fn take_pending_pack(_address: &str) -> Option<RealmsPack> {
    None
}

#[derive(Clone, Debug)]
pub struct RealmsPack {
    pub address: String,
    pub id: u128,
    pub url: String,
    pub hash: String,
}

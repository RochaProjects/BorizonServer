//! Player-list data types and server-safe helpers.
//! TabList::layout (needs Font) lives in horizon-client-render's tab_list.rs.
use crate::{
    hud_layout::{Blend, GuiDraw, WHITE},
    java_text::float_to_string,
    scoreboard::{Comp, NumberFormat, Objective, RenderType, Scoreboard, comp_text},
    text::Text,
};
use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

/// `GameType.SPECTATOR` id.
pub const SPECTATOR: i32 = 3;
/// `PlayerTabOverlay.MAX_ROWS_PER_COL`.
pub const MAX_ROWS_PER_COL: i32 = 20;

/// One `PlayerInfo` the list can show.
#[derive(Clone, Debug, PartialEq)]
pub struct TabPlayer {
    pub uuid: u128,
    pub name: String,
    pub game_mode: i32,
    pub latency: i32,
    pub display_name: Option<Comp>,
    pub list_order: i32,
    pub show_hat: bool,
    pub skin: String,
}
impl TabPlayer {
    pub fn new(uuid: u128, name: impl Into<String>) -> Self {
        Self {
            uuid,
            name: name.into(),
            game_mode: 0,
            latency: 0,
            display_name: None,
            list_order: 0,
            show_hat: true,
            skin: default_skin(uuid).to_owned(),
        }
    }
}

pub fn default_skin(uuid: u128) -> &'static str {
    const SKINS: [&str; 18] = [
        "entity/player/slim/alex",
        "entity/player/slim/ari",
        "entity/player/slim/efe",
        "entity/player/slim/kai",
        "entity/player/slim/makena",
        "entity/player/slim/noor",
        "entity/player/slim/steve",
        "entity/player/slim/sunny",
        "entity/player/slim/zuri",
        "entity/player/wide/alex",
        "entity/player/wide/ari",
        "entity/player/wide/efe",
        "entity/player/wide/kai",
        "entity/player/wide/makena",
        "entity/player/wide/noor",
        "entity/player/wide/steve",
        "entity/player/wide/sunny",
        "entity/player/wide/zuri",
    ];
    let hilo = ((uuid >> 64) as u64) ^ (uuid as u64);
    let hash = ((hilo >> 32) as i32) ^ (hilo as i32);
    SKINS[hash.rem_euclid(SKINS.len() as i32) as usize]
}

pub fn face(
    out: &mut Vec<GuiDraw>,
    skin: &str,
    x: i32,
    y: i32,
    size: i32,
    hat: bool,
    flip: bool,
    color: u32,
) {
    let (v, height) = if flip { (16.0, -8) } else { (8.0, 8) };
    for u in [8.0, 40.0].into_iter().take(if hat { 2 } else { 1 }) {
        out.push(GuiDraw::Blit {
            texture: Cow::Owned(skin.to_owned()),
            blend: Blend::Textured,
            x,
            y,
            w: size,
            h: size,
            uv: [u, v],
            src: [8, height],
            tex: [64, 64],
            color,
        });
    }
}

pub fn visible(
    hud_shown: bool,
    key_down: bool,
    local_server: bool,
    listed: usize,
    objective: bool,
) -> bool {
    hud_shown && key_down && !(local_server && listed <= 1 && !objective)
}

pub fn row_background(background_for_chat_only: bool, text_background_opacity: f64) -> u32 {
    if background_for_chat_only {
        0x20FF_FFFF
    } else {
        ((text_background_opacity as f32 * 255.0).floor() as i32 as u32 & 0xFF) << 24
    }
}

pub fn sorted_players<'a>(players: &'a [TabPlayer], board: &Scoreboard) -> Vec<&'a TabPlayer> {
    let mut out: Vec<_> = players
        .iter()
        .map(|p| (p, board.players_team(&p.name).map_or("", |t| t.name.as_str())))
        .collect();
    out.sort_by(|(a, team_a), (b, team_b)| {
        a.list_order
            .wrapping_neg()
            .cmp(&b.list_order.wrapping_neg())
            .then_with(|| (a.game_mode == SPECTATOR).cmp(&(b.game_mode == SPECTATOR)))
            .then_with(|| compare_utf16(team_a, team_b))
            .then_with(|| compare_ignore_case(&a.name, &b.name))
    });
    out.truncate(80);
    out.into_iter().map(|(p, _)| p).collect()
}

pub fn narration(players: &[TabPlayer], board: &Scoreboard) -> Text {
    Text::translate("multiplayer.player.list.narration", &[listed_names(players, board)])
}

fn listed_names(players: &[TabPlayer], board: &Scoreboard) -> Text {
    sorted_players(players, board)
        .into_iter()
        .enumerate()
        .fold(Text::empty(), |names, (i, p)| {
            let names = if i == 0 { names } else { names.append(Text::literal(", ")) };
            names.append(name_for_display(p, board))
        })
}

fn italic_root(c: &Comp) -> Comp {
    use serde_json::Value;
    match c {
        Value::Object(o) => {
            let mut o = o.clone();
            o.insert("italic".into(), Value::Bool(true));
            Value::Object(o)
        }
        Value::Array(list) if !list.is_empty() => {
            let mut list = list.clone();
            list[0] = italic_root(&list[0]);
            Value::Array(list)
        }
        Value::Array(_) => serde_json::json!({"text": "", "italic": true}),
        other => {
            let text = match other {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                v => v.to_string(),
            };
            serde_json::json!({"text": text, "italic": true})
        }
    }
}

pub fn name_for_display(p: &TabPlayer, board: &Scoreboard) -> Text {
    let spectator = p.game_mode == SPECTATOR;
    let name = match &p.display_name {
        Some(d) if spectator => return comp_text(&italic_root(d)),
        Some(d) => return comp_text(d),
        None => match board.players_team(&p.name) {
            Some(team) => team.formatted_name(Text::literal(p.name.clone())),
            None => Text::literal(p.name.clone()),
        },
    };
    if spectator { name.italic() } else { name }
}

pub fn ping_sprite(latency: i32) -> &'static str {
    match latency {
        i32::MIN..=-1 => "icon/ping_unknown",
        0..150 => "icon/ping_5",
        150..300 => "icon/ping_4",
        300..600 => "icon/ping_3",
        600..1000 => "icon/ping_2",
        _ => "icon/ping_1",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HealthState {
    last_value: i32,
    displayed_value: i32,
    last_update_tick: i64,
    blink_until_tick: i64,
}
impl HealthState {
    fn new(value: i32) -> Self {
        Self { last_value: value, displayed_value: value, last_update_tick: 0, blink_until_tick: 0 }
    }
    fn update(&mut self, value: i32, tick: i64) {
        if value != self.last_value {
            let duration = if value < self.last_value { 20 } else { 10 };
            self.blink_until_tick = tick + duration;
            self.last_value = value;
            self.last_update_tick = tick;
        }
        if tick - self.last_update_tick > 20 {
            self.displayed_value = value;
        }
    }
    fn is_blinking(&self, tick: i64) -> bool {
        self.blink_until_tick > tick && (self.blink_until_tick - tick) % 6 >= 3
    }
}

/// A mark drawn after a player's head in the tab list.
pub type NameMark = &'static [(u8, u8, u8, u8, u32)];

pub struct TabInputs<'a> {
    pub players: &'a [TabPlayer],
    pub board: &'a Scoreboard,
    pub objective: Option<&'a Objective>,
    pub screen_width: i32,
    pub online_mode: bool,
    pub gui_ticks: i64,
    pub background: u32,
    pub upside_down: &'a dyn Fn(u128) -> bool,
    pub mark: &'a dyn Fn(u128) -> Option<NameMark>,
}

/// `PlayerTabOverlay`: the header, footer, visibility and heart states.
/// `TabList::layout` lives in the render crate (needs `Font`).
#[derive(Clone, Debug, Default)]
pub struct TabList {
    pub header: Option<Text>,
    pub footer: Option<Text>,
    pub(crate) visible: bool,
    pub(crate) health: HashMap<u128, HealthState>,
    pub(crate) order: Option<(Vec<TabPlayer>, Scoreboard, Vec<u128>)>,
}

impl TabList {
    pub fn set_visible(&mut self, visible: bool) -> bool {
        if self.visible != visible {
            self.health.clear();
            self.visible = visible;
            return visible;
        }
        false
    }
    pub fn reset(&mut self) {
        self.header = None;
        self.footer = None;
    }
    pub fn displayed_ids(&mut self, players: &[TabPlayer], board: &Scoreboard) -> &[u128] {
        if self.order.as_ref().is_none_or(|(old, previous, _)| {
            old.len() != players.len()
                || old.iter().zip(players).any(|(a, b)| {
                    a.uuid != b.uuid || a.name != b.name
                        || a.list_order != b.list_order || a.game_mode != b.game_mode
                })
                || previous != board
        }) {
            let ids = sorted_players(players, board).iter().map(|p| p.uuid).collect();
            self.order = Some((players.to_vec(), board.clone(), ids));
        }
        &self.order.as_ref().unwrap().2
    }
}

fn compare_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
fn compare_ignore_case(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

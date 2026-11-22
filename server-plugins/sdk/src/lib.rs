//! Idiomatic Rust wrappers for the Horizon server plugin ABI v3.
//!
//! The SDK keeps the existing C ABI unchanged. Plugins remain trusted native
//! code and callbacks execute synchronously on the server thread.

use std::{collections::BTreeMap, ffi::c_void, fmt, slice, str};

use horizon_server_plugin_api::{self as abi, result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub use horizon_server_plugin_api::{
    ABI_VERSION_V3, COMMAND_COMPLETION_PREFIX, HostApiV3, PluginV3,
};
pub use horizon_server_plugin_api::{event, operation};

/// Maximum JSON request/response size supported by the current ABI v3 host.
pub const MAX_JSON_BYTES: usize = 65_536;
/// Maximum command line size accepted by the host for a dynamic completion request.
pub const MAX_COMMAND_COMPLETION_BYTES: usize = 512;
/// Maximum plain-item deltas accepted by one ABI v2 economy transaction.
pub const MAX_TRANSACTION_ITEMS: usize = 16;

/// Return the command line when `line` is a host-issued dynamic completion
/// callback, or `None` for an ordinary command or an oversized request.
/// The returned slice borrows the input and includes the partial final token.
pub fn command_completion_input(line: &str) -> Option<&str> {
    let input = line.strip_prefix(abi::COMMAND_COMPLETION_PREFIX)?;
    (input.len() <= MAX_COMMAND_COMPLETION_BYTES).then_some(input)
}

/// UUID represented in the same high/low 64-bit form used by the C ABI.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PlayerId(u128);

impl PlayerId {
    /// The all-zero UUID, used for operations that are not associated with a player.
    pub const NONE: Self = Self(0);

    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }

    pub const fn from_parts(high: u64, low: u64) -> Self {
        Self(((high as u128) << 64) | low as u128)
    }

    pub const fn parts(self) -> (u64, u64) {
        ((self.0 >> 64) as u64, self.0 as u64)
    }
}

/// Vanilla player game mode understood by the current server operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    /// Numeric ID used by Horizon's existing JSON operation contract.
    pub const fn as_id(self) -> i32 {
        match self {
            Self::Survival => 0,
            Self::Creative => 1,
            Self::Adventure => 2,
            Self::Spectator => 3,
        }
    }
}

/// Text accepted by Horizon's component-aware chat and sidebar operations.
/// Plain strings retain the host's legacy formatting support; `Json` passes
/// a Minecraft text component value through to the host.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TextComponent {
    Text(String),
    Json(Value),
}

impl TextComponent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub fn json(component: Value) -> Self {
        Self::Json(component)
    }
}

/// One scoreboard sidebar row. `right` is optional and may be used as a short score/value.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SidebarLine {
    pub left: TextComponent,
    pub right: Option<TextComponent>,
}

impl SidebarLine {
    pub fn new(left: impl Into<String>) -> Self {
        Self {
            left: TextComponent::text(left),
            right: None,
        }
    }

    pub fn with_right(mut self, right: impl Into<String>) -> Self {
        self.right = Some(TextComponent::text(right));
        self
    }

    pub fn with_components(mut self, left: TextComponent, right: Option<TextComponent>) -> Self {
        self.left = left;
        self.right = right;
        self
    }
}

/// One item placed in a plugin-owned virtual chest menu.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuItemSpec {
    pub slot: u8,
    pub item: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<TextComponent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lore: Vec<TextComponent>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub components: BTreeMap<String, Value>,
}

impl MenuItemSpec {
    pub fn new(slot: u8, item: impl Into<String>) -> Self {
        Self {
            slot,
            item: item.into(),
            count: None,
            name: None,
            lore: Vec::new(),
            components: BTreeMap::new(),
        }
    }

    pub fn count(mut self, count: u8) -> Self {
        self.count = Some(count);
        self
    }

    pub fn name(mut self, name: TextComponent) -> Self {
        self.name = Some(name);
        self
    }

    pub fn lore(mut self, lore: Vec<TextComponent>) -> Self {
        self.lore = lore;
        self
    }

    pub fn component(mut self, key: impl Into<String>, value: Value) -> Self {
        self.components.insert(key.into(), value);
        self
    }
}

/// A virtual chest menu shown to one player.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuSpec {
    pub rows: u8,
    pub title: TextComponent,
    pub items: Vec<MenuItemSpec>,
}

impl MenuSpec {
    pub fn new(title: TextComponent) -> Self {
        Self {
            rows: 3,
            title,
            items: Vec::new(),
        }
    }

    pub fn rows(mut self, rows: u8) -> Self {
        self.rows = rows;
        self
    }

    pub fn item(mut self, item: MenuItemSpec) -> Self {
        self.items.push(item);
        self
    }
}

/// A resource pack to offer to one online player.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourcePackSpec {
    pub id: u128,
    pub url: String,
    pub sha1: Option<String>,
    pub prompt: Option<TextComponent>,
    pub required: bool,
}

impl ResourcePackSpec {
    pub fn new(id: u128, url: impl Into<String>) -> Self {
        Self {
            id,
            url: url.into(),
            sha1: None,
            prompt: None,
            required: false,
        }
    }

    pub fn sha1(mut self, sha1: impl Into<String>) -> Self {
        self.sha1 = Some(sha1.into());
        self
    }

    pub fn prompt(mut self, prompt: TextComponent) -> Self {
        self.prompt = Some(prompt);
        self
    }

    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }
}

/// Outcome of a resource-pack batch; the host may have sent a prefix before failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourcePackBatchResult {
    pub sent: usize,
    pub complete: bool,
    pub error: Option<String>,
}

/// Inventory view supported by the current server-plugin operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryViewKind {
    Player,
    Ender,
}

impl InventoryViewKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Ender => "ender",
        }
    }

    const fn slot_count(self) -> usize {
        match self {
            Self::Player => 41,
            Self::Ender => 27,
        }
    }

    const fn default_rows(self) -> u8 {
        match self {
            Self::Player => 6,
            Self::Ender => 3,
        }
    }
}

/// Configuration for opening another player's live or saved inventory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryViewSpec {
    pub target: PlayerId,
    pub kind: InventoryViewKind,
    pub title: String,
    /// Host clamps explicit values to 1–6 rows; the SDK validates the effective menu size.
    pub rows: Option<u8>,
    /// Menu slot to inventory slot mapping; `None` entries are blank plugin-owned slots.
    pub layout: Option<Vec<Option<usize>>>,
}

impl InventoryViewSpec {
    pub fn new(target: PlayerId, kind: InventoryViewKind, title: impl Into<String>) -> Self {
        Self {
            target,
            kind,
            title: title.into(),
            rows: None,
            layout: None,
        }
    }

    pub fn rows(mut self, rows: u8) -> Self {
        self.rows = Some(rows);
        self
    }

    pub fn layout(mut self, layout: Vec<Option<usize>>) -> Self {
        self.layout = Some(layout);
        self
    }
}

/// Horizontal alignment for a vanilla `text_display`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextDisplayAlignment {
    Left,
    #[default]
    Center,
    Right,
}
impl TextDisplayAlignment {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// Billboard behavior for a vanilla `text_display`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextDisplayBillboard {
    Fixed,
    Vertical,
    Horizontal,
    #[default]
    Center,
}
impl TextDisplayBillboard {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Center => "center",
        }
    }
}

/// Replacement specification for a persistent vanilla text display.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDisplaySpec {
    pub uuid: u128,
    pub dimension: String,
    pub position: [f64; 3],
    pub text: TextComponent,
    pub line_width: u16,
    pub background: u32,
    pub view_range: f32,
    pub shadow: bool,
    pub see_through: bool,
    pub alignment: TextDisplayAlignment,
    pub billboard: TextDisplayBillboard,
}
impl TextDisplaySpec {
    pub fn new(
        uuid: u128,
        dimension: impl Into<String>,
        position: [f64; 3],
        text: TextComponent,
    ) -> Self {
        Self {
            uuid,
            dimension: dimension.into(),
            position,
            text,
            line_width: 200,
            background: 0x4000_0000,
            view_range: 1.0,
            shadow: false,
            see_through: false,
            alignment: TextDisplayAlignment::Center,
            billboard: TextDisplayBillboard::Center,
        }
    }
    pub fn line_width(mut self, width: u16) -> Self {
        self.line_width = width;
        self
    }
    pub fn background(mut self, argb: u32) -> Self {
        self.background = argb;
        self
    }
    pub fn view_range(mut self, range: f32) -> Self {
        self.view_range = range;
        self
    }
    pub fn shadow(mut self, enabled: bool) -> Self {
        self.shadow = enabled;
        self
    }
    pub fn see_through(mut self, enabled: bool) -> Self {
        self.see_through = enabled;
        self
    }
    pub fn alignment(mut self, alignment: TextDisplayAlignment) -> Self {
        self.alignment = alignment;
        self
    }
    pub fn billboard(mut self, billboard: TextDisplayBillboard) -> Self {
        self.billboard = billboard;
        self
    }
}

/// One-shot text display creation spec for the host operation that returns a runtime entity ID.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDisplayCreateSpec {
    pub dimension: String,
    pub position: [f64; 3],
    pub text: TextComponent,
    pub line_width: u16,
    pub background: u32,
    pub view_range: f32,
    pub shadow: bool,
    pub see_through: bool,
    pub alignment: TextDisplayAlignment,
    pub billboard: TextDisplayBillboard,
}

impl TextDisplayCreateSpec {
    pub fn new(dimension: impl Into<String>, position: [f64; 3], text: TextComponent) -> Self {
        Self {
            dimension: dimension.into(),
            position,
            text,
            line_width: 200,
            background: 0x4000_0000,
            view_range: 1.0,
            shadow: false,
            see_through: false,
            alignment: TextDisplayAlignment::Center,
            billboard: TextDisplayBillboard::Center,
        }
    }
    pub fn line_width(mut self, width: u16) -> Self {
        self.line_width = width;
        self
    }
    pub fn background(mut self, argb: u32) -> Self {
        self.background = argb;
        self
    }
    pub fn view_range(mut self, range: f32) -> Self {
        self.view_range = range;
        self
    }
    pub fn shadow(mut self, enabled: bool) -> Self {
        self.shadow = enabled;
        self
    }
    pub fn see_through(mut self, enabled: bool) -> Self {
        self.see_through = enabled;
        self
    }
    pub fn alignment(mut self, alignment: TextDisplayAlignment) -> Self {
        self.alignment = alignment;
        self
    }
    pub fn billboard(mut self, billboard: TextDisplayBillboard) -> Self {
        self.billboard = billboard;
        self
    }
}

/// Host IDs returned when a text display is upserted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextDisplayResult {
    pub uuid: u128,
    pub entity_id: u32,
}

/// Replacement specification for a persistent vanilla `item_display`.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemDisplaySpec {
    pub uuid: u128,
    pub dimension: String,
    pub position: [f64; 3],
    pub item_model: String,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}
impl ItemDisplaySpec {
    pub fn new(
        uuid: u128,
        dimension: impl Into<String>,
        position: [f64; 3],
        item_model: impl Into<String>,
    ) -> Self {
        Self {
            uuid,
            dimension: dimension.into(),
            position,
            item_model: item_model.into(),
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
    pub fn translation(mut self, value: [f32; 3]) -> Self {
        self.translation = value;
        self
    }
    pub fn rotation(mut self, quaternion: [f32; 4]) -> Self {
        self.rotation = quaternion;
        self
    }
    pub fn scale(mut self, value: [f32; 3]) -> Self {
        self.scale = value;
        self
    }
}

/// A dimension-scoped runtime entity ID returned by entity creation operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntityRef {
    pub dimension: String,
    pub entity_id: u32,
}
impl EntityRef {
    pub fn new(dimension: impl Into<String>, entity_id: u32) -> Self {
        Self {
            dimension: dimension.into(),
            entity_id,
        }
    }
}

/// Generic native entity spawn options supported by Horizon's plugin host.
#[derive(Clone, Debug, PartialEq)]
pub struct EntitySpawnSpec {
    pub uuid: u128,
    pub dimension: String,
    pub entity_type: String,
    pub position: [f64; 3],
    pub no_ai: Option<bool>,
    pub invulnerable: Option<bool>,
    pub silent: Option<bool>,
    pub persistent: Option<bool>,
    pub name_visible: Option<bool>,
    pub custom_name: Option<String>,
    pub interaction_width: Option<f32>,
    pub interaction_height: Option<f32>,
    pub interaction_response: Option<bool>,
}
impl EntitySpawnSpec {
    pub fn new(
        uuid: u128,
        dimension: impl Into<String>,
        entity_type: impl Into<String>,
        position: [f64; 3],
    ) -> Self {
        Self {
            uuid,
            dimension: dimension.into(),
            entity_type: entity_type.into(),
            position,
            no_ai: None,
            invulnerable: None,
            silent: None,
            persistent: None,
            name_visible: None,
            custom_name: None,
            interaction_width: None,
            interaction_height: None,
            interaction_response: None,
        }
    }
    pub fn no_ai(mut self, value: bool) -> Self {
        self.no_ai = Some(value);
        self
    }
    pub fn invulnerable(mut self, value: bool) -> Self {
        self.invulnerable = Some(value);
        self
    }
    pub fn silent(mut self, value: bool) -> Self {
        self.silent = Some(value);
        self
    }
    pub fn persistent(mut self, value: bool) -> Self {
        self.persistent = Some(value);
        self
    }
    pub fn name_visible(mut self, value: bool) -> Self {
        self.name_visible = Some(value);
        self
    }
    pub fn custom_name(mut self, value: impl Into<String>) -> Self {
        self.custom_name = Some(value.into());
        self
    }
    pub fn interaction_width(mut self, value: f32) -> Self {
        self.interaction_width = Some(value);
        self
    }
    pub fn interaction_height(mut self, value: f32) -> Self {
        self.interaction_height = Some(value);
        self
    }
    pub fn interaction_response(mut self, value: bool) -> Self {
        self.interaction_response = Some(value);
        self
    }
}

/// Stable UUID and runtime ID returned by `entity.spawn`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntitySpawnResult {
    pub uuid: u128,
    pub entity: EntityRef,
}

/// Prefix/suffix shown above a player's head, when explicitly supplied.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct NameTagStyle {
    pub prefix: Option<TextComponent>,
    pub suffix: Option<TextComponent>,
}

/// Display fields sent together because the host clears omitted chat/tab names.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerDisplaySpec {
    pub player: PlayerId,
    /// `None` clears the chat display name; this field is always sent.
    pub chat: Option<TextComponent>,
    /// `None` clears the player-list name; this field is always sent.
    pub tab: Option<TextComponent>,
    /// `None` leaves the nametag style untouched; `Some` sends both fields, and each `None` clears that side.
    pub nametag: Option<NameTagStyle>,
}

/// Per-field outcome of `player.display.set`, including partial application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerDisplayResult {
    pub applied: bool,
    pub chat: bool,
    pub tab: bool,
    pub nametag: bool,
}

/// Errors returned while calling the host's JSON operation bridge.
#[derive(Debug)]
pub enum CallError {
    FunctionUnavailable,
    RequestTooLarge { actual: usize, maximum: usize },
    ResponseTooLarge { required: usize, maximum: usize },
    HostStatus(i32),
    Serialization(serde_json::Error),
    InvalidResponse(serde_json::Error),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FunctionUnavailable => f.write_str("host does not provide ABI v3 call_json"),
            Self::RequestTooLarge { actual, maximum } => {
                write!(f, "JSON request is {actual} bytes; maximum is {maximum}")
            }
            Self::ResponseTooLarge { required, maximum } => {
                write!(
                    f,
                    "JSON response requires {required} bytes; maximum is {maximum}"
                )
            }
            Self::HostStatus(status) => write!(f, "host returned ABI status {status}"),
            Self::Serialization(error) => write!(f, "could not serialize JSON request: {error}"),
            Self::InvalidResponse(error) => write!(f, "host returned invalid JSON: {error}"),
        }
    }
}

impl std::error::Error for CallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialization(error) | Self::InvalidResponse(error) => Some(error),
            _ => None,
        }
    }
}

/// Failure returned by a typed SDK helper when the host accepts the ABI call
/// but rejects the requested operation.
#[derive(Debug)]
pub enum OperationError {
    Call(CallError),
    Rejected(String),
    InvalidResponse,
    InvalidArgument(&'static str),
}

/// Plain-item delta included in an ABI v2 balance/inventory transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemDeltaSpec {
    pub item: String,
    /// Positive adds items; negative removes them. Zero is invalid.
    pub delta: i32,
}
impl ItemDeltaSpec {
    pub fn new(item: impl Into<String>, delta: i32) -> Self {
        Self {
            item: item.into(),
            delta,
        }
    }
}

/// Failures returned by ABI v2 economy functions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EconomyError {
    FunctionUnavailable,
    InvalidCurrency,
    InsufficientBalance,
    ItemsRejected,
    StorageError,
    InvalidRequest,
    UnexpectedStatus(i32),
}
impl fmt::Display for EconomyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FunctionUnavailable => f.write_str("economy function is unavailable"),
            Self::InvalidCurrency => f.write_str("invalid currency ID"),
            Self::InsufficientBalance => f.write_str("insufficient balance"),
            Self::ItemsRejected => f.write_str("inventory transaction was rejected"),
            Self::StorageError => f.write_str("balance storage is unavailable or failed"),
            Self::InvalidRequest => f.write_str("host rejected the transaction request"),
            Self::UnexpectedStatus(status) => write!(f, "unexpected economy status {status}"),
        }
    }
}
impl std::error::Error for EconomyError {}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call(error) => write!(f, "host call failed: {error}"),
            Self::Rejected(reason) => write!(f, "host rejected operation: {reason}"),
            Self::InvalidResponse => f.write_str("host returned an unexpected operation response"),
            Self::InvalidArgument(reason) => write!(f, "invalid SDK argument: {reason}"),
        }
    }
}

impl std::error::Error for OperationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Call(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CallError> for OperationError {
    fn from(error: CallError) -> Self {
        Self::Call(error)
    }
}

/// Declarative data for registering one literal command during plugin startup.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec<'a> {
    usage: &'a str,
    default_level: u8,
    permission: Option<&'a str>,
}

/// Opaque identifier returned by the host when scheduling a server-tick task.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TaskId(u64);

impl TaskId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Typed view of a scheduled callback delivered through `event::SERVER_TASK`.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerTaskEvent {
    pub task_id: TaskId,
    /// Task payload chosen by the plugin when scheduling it.
    pub data: Value,
}

/// Known resource-pack status codes in Horizon's current protocol.
/// Unknown codes remain available through `ServerNotificationEvent::ResourcePackStatus::status`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourcePackStatusCode {
    /// Protocol code 0.
    SuccessfullyLoaded,
    /// Protocol code 1.
    Declined,
    /// Protocol code 2.
    FailedDownload,
    /// Protocol code 3.
    Accepted,
    /// Protocol code 4.
    Downloaded,
    /// Protocol code 5.
    InvalidUrl,
    /// Protocol code 6.
    FailedReload,
    /// Protocol code 7.
    Discarded,
}

impl ResourcePackStatusCode {
    /// Decode a code known to the current host, preserving forward compatibility via `None`.
    pub const fn from_raw(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::SuccessfullyLoaded),
            1 => Some(Self::Declined),
            2 => Some(Self::FailedDownload),
            3 => Some(Self::Accepted),
            4 => Some(Self::Downloaded),
            5 => Some(Self::InvalidUrl),
            6 => Some(Self::FailedReload),
            7 => Some(Self::Discarded),
            _ => None,
        }
    }

    /// Return the current host's protocol code for this known status.
    pub const fn as_raw(self) -> i32 {
        match self {
            Self::SuccessfullyLoaded => 0,
            Self::Declined => 1,
            Self::FailedDownload => 2,
            Self::Accepted => 3,
            Self::Downloaded => 4,
            Self::InvalidUrl => 5,
            Self::FailedReload => 6,
            Self::Discarded => 7,
        }
    }
}

/// Position and rotation captured by the host in a player event.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[non_exhaustive]
pub struct PlayerEventLocation {
    pub dimension: String,
    pub position: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
}

/// Typed read-only view of event payloads confirmed to be emitted by the host.
/// The decoder itself never cancels an event; `BLOCK_BREAK` and
/// `PLAYER_DEATH` can be vetoed by returning true from the generic callback.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ServerNotificationEvent {
    PlayerJoined {
        player: PlayerId,
        name: String,
        dimension: String,
    },
    PlayerQuit {
        player: PlayerId,
        name: String,
        last_location: Option<PlayerEventLocation>,
    },
    /// A player death attempt; returning true from the generic callback keeps them at 1 health.
    PlayerDeathAttempt {
        player: PlayerId,
        name: String,
        damage_type: String,
        /// Host entity ID; zero is used for no attacker or a player attacker.
        attacker_entity_id: u32,
        location: PlayerEventLocation,
    },
    /// A block break after permission checks but before destruction; callback true vetoes it.
    BlockBreakAttempt {
        player: PlayerId,
        name: String,
        dimension: String,
        position: [i32; 3],
        block_state_id: u32,
    },
    /// A post-placement notification; it cannot cancel the completed placement.
    BlockPlaced {
        player: PlayerId,
        name: String,
        dimension: String,
        position: [i32; 3],
        block_state_id: u32,
    },
    MenuClosed {
        player: PlayerId,
        dimension: String,
        window_id: i32,
    },
    /// Chat delivered after broadcast; the callback cannot cancel or rewrite it.
    ChatAfterBroadcast {
        player: PlayerId,
        name: String,
        message: String,
    },
    DimensionChanged {
        player: PlayerId,
        name: String,
        from: String,
        to: String,
    },
    Teleported {
        player: PlayerId,
        name: String,
        dimension: String,
        from: [f64; 3],
        to: [f64; 3],
    },
    ResourcePackStatus {
        player: PlayerId,
        name: String,
        pack_id: String,
        /// Raw status code emitted by the host; no semantic mapping is assumed.
        status: i32,
    },
}

impl ServerNotificationEvent {
    /// Whether the current host consumes the generic callback's `true` return
    /// for this event kind. This is a host capability, not a property inferred
    /// from the event's name or payload.
    pub const fn is_cancellable(&self) -> bool {
        matches!(
            self,
            Self::BlockBreakAttempt { .. } | Self::PlayerDeathAttempt { .. }
        )
    }
}

/// Decode a known, emitted notification event from the generic callback JSON.
/// Unknown event kinds return `Ok(None)`; malformed known payloads return an
/// error. This decoder does not imply cancellation or mutation support.
pub fn decode_server_notification_event(
    kind: u32,
    payload: &Value,
) -> Result<Option<ServerNotificationEvent>, OperationError> {
    if !matches!(
        kind,
        event::PLAYER_JOIN
            | event::PLAYER_QUIT
            | event::PLAYER_DEATH
            | event::BLOCK_BREAK
            | event::BLOCK_PLACE
            | event::MENU_CLOSED
            | event::CHAT
            | event::PLAYER_CHANGED_DIMENSION
            | event::PLAYER_TELEPORT
            | event::RESOURCE_PACK_STATUS
    ) {
        return Ok(None);
    }
    let data = payload.get("data").ok_or(OperationError::InvalidResponse)?;
    let player = payload
        .get("player")
        .and_then(Value::as_str)
        .filter(|value| value.len() == 32)
        .and_then(|value| u128::from_str_radix(value, 16).ok())
        .map(PlayerId::from_u128)
        .ok_or(OperationError::InvalidResponse)?;
    if kind == event::PLAYER_JOIN || kind == event::PLAYER_QUIT {
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        if kind == event::PLAYER_JOIN {
            if !data.is_null() {
                return Err(OperationError::InvalidResponse);
            }
            let dimension = payload
                .get("dimension")
                .and_then(Value::as_str)
                .ok_or(OperationError::InvalidResponse)?
                .to_owned();
            return Ok(Some(ServerNotificationEvent::PlayerJoined {
                player,
                name,
                dimension,
            }));
        }
        let last_location = match data.get("last_location") {
            Some(Value::Null) => None,
            Some(location) => Some(decode_player_event_location(location)?),
            None => return Err(OperationError::InvalidResponse),
        };
        return Ok(Some(ServerNotificationEvent::PlayerQuit {
            player,
            name,
            last_location,
        }));
    }
    if kind == event::PLAYER_DEATH {
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        let damage_type = data
            .get("damage_type")
            .and_then(Value::as_str)
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        let attacker_entity_id = data
            .get("attacker_entity_id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or(OperationError::InvalidResponse)?;
        let location = decode_player_event_location(
            data.get("location")
                .ok_or(OperationError::InvalidResponse)?,
        )?;
        return Ok(Some(ServerNotificationEvent::PlayerDeathAttempt {
            player,
            name,
            damage_type,
            attacker_entity_id,
            location,
        }));
    }
    if kind == event::BLOCK_BREAK || kind == event::BLOCK_PLACE {
        if !data.is_null() {
            return Err(OperationError::InvalidResponse);
        }
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        let dimension = payload
            .get("dimension")
            .and_then(Value::as_str)
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        let position = decode_event_position(
            payload
                .get("position")
                .ok_or(OperationError::InvalidResponse)?,
        )?;
        let block_state_id = payload
            .get("data_id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or(OperationError::InvalidResponse)?;
        return Ok(Some(if kind == event::BLOCK_BREAK {
            ServerNotificationEvent::BlockBreakAttempt {
                player,
                name,
                dimension,
                position,
                block_state_id,
            }
        } else {
            ServerNotificationEvent::BlockPlaced {
                player,
                name,
                dimension,
                position,
                block_state_id,
            }
        }));
    }
    if kind == event::MENU_CLOSED {
        let dimension = payload
            .get("dimension")
            .and_then(Value::as_str)
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        let window_id = payload
            .get("window_id")
            .and_then(Value::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .ok_or(OperationError::InvalidResponse)?;
        if !data.is_null() {
            return Err(OperationError::InvalidResponse);
        }
        return Ok(Some(ServerNotificationEvent::MenuClosed {
            player,
            dimension,
            window_id,
        }));
    }
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .ok_or(OperationError::InvalidResponse)?
        .to_owned();
    if kind == event::RESOURCE_PACK_STATUS {
        let pack_id = data
            .get("pack_id")
            .and_then(Value::as_str)
            .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        let status = data
            .get("status")
            .and_then(Value::as_i64)
            .and_then(|status| i32::try_from(status).ok())
            .ok_or(OperationError::InvalidResponse)?;
        return Ok(Some(ServerNotificationEvent::ResourcePackStatus {
            player,
            name,
            pack_id,
            status,
        }));
    }
    let event = match kind {
        event::CHAT => {
            if data.get("phase").and_then(Value::as_str) != Some("after_broadcast") {
                return Err(OperationError::InvalidResponse);
            }
            ServerNotificationEvent::ChatAfterBroadcast {
                player,
                name,
                message: data
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or(OperationError::InvalidResponse)?
                    .to_owned(),
            }
        }
        event::PLAYER_CHANGED_DIMENSION => ServerNotificationEvent::DimensionChanged {
            player,
            name,
            from: data
                .get("from")
                .and_then(Value::as_str)
                .ok_or(OperationError::InvalidResponse)?
                .to_owned(),
            to: data
                .get("to")
                .and_then(Value::as_str)
                .ok_or(OperationError::InvalidResponse)?
                .to_owned(),
        },
        event::PLAYER_TELEPORT => ServerNotificationEvent::Teleported {
            player,
            name,
            dimension: data
                .get("dimension")
                .and_then(Value::as_str)
                .ok_or(OperationError::InvalidResponse)?
                .to_owned(),
            from: decode_event_vector(data.get("from").ok_or(OperationError::InvalidResponse)?)?,
            to: decode_event_vector(data.get("to").ok_or(OperationError::InvalidResponse)?)?,
        },
        _ => return Ok(None),
    };
    Ok(Some(event))
}

fn decode_event_position(value: &Value) -> Result<[i32; 3], OperationError> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or(OperationError::InvalidResponse)?;
    let mut result = [0; 3];
    for (index, value) in values.iter().enumerate() {
        result[index] = value
            .as_i64()
            .and_then(|number| i32::try_from(number).ok())
            .ok_or(OperationError::InvalidResponse)?;
    }
    Ok(result)
}

fn decode_player_event_location(value: &Value) -> Result<PlayerEventLocation, OperationError> {
    let dimension = value
        .get("dimension")
        .and_then(Value::as_str)
        .ok_or(OperationError::InvalidResponse)?
        .to_owned();
    let position = decode_event_vector(
        value
            .get("position")
            .ok_or(OperationError::InvalidResponse)?,
    )?;
    let yaw = value
        .get("yaw")
        .and_then(Value::as_f64)
        .filter(|angle| angle.is_finite())
        .ok_or(OperationError::InvalidResponse)?;
    let pitch = value
        .get("pitch")
        .and_then(Value::as_f64)
        .filter(|angle| angle.is_finite())
        .ok_or(OperationError::InvalidResponse)?;
    Ok(PlayerEventLocation {
        dimension,
        position,
        yaw,
        pitch,
    })
}

fn decode_event_vector(value: &Value) -> Result<[f64; 3], OperationError> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or(OperationError::InvalidResponse)?;
    let mut result = [0.0; 3];
    for (index, value) in values.iter().enumerate() {
        result[index] = value
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or(OperationError::InvalidResponse)?;
    }
    Ok(result)
}

/// One page from a plugin storage key listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoragePage {
    pub keys: Vec<String>,
    /// Cursor to pass as `after` for the next lexicographic page.
    pub next: Option<String>,
}

/// Point-in-time view of dimensions known to the world save and those loaded in memory.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct WorldDimensions {
    /// Dimension IDs known to the current save.
    pub dimensions: Vec<String>,
    /// Dimension IDs currently loaded into server memory.
    pub loaded_dimensions: Vec<String>,
}

/// Result of loading a saved dimension, including its optional chunk ticket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DimensionLoadResult {
    pub dimension: String,
    pub loaded: bool,
    pub pinned: bool,
}

/// World-generation preset accepted by Horizon's custom-dimension creator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CustomDimensionPreset {
    Overworld,
    Nether,
    End,
    Flat,
}
impl CustomDimensionPreset {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Overworld => "overworld",
            Self::Nether => "nether",
            Self::End => "end",
            Self::Flat => "flat",
        }
    }
}

/// Result of creating a custom dimension and preparing its generator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomDimensionCreateResult {
    pub dimension: String,
    pub preset: CustomDimensionPreset,
    /// The host attempts to close the newly prepared dimension; false means it remained loaded.
    pub loaded: bool,
}

/// Player-data categories accepted by Horizon's two-phase reset operation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PlayerDataCategory {
    /// Player advancement progress and related files.
    Advancements,
    /// Vanilla player statistics.
    Stats,
    /// Serialized player data; the host requires an empty server to reset this category.
    PlayerData,
}
impl PlayerDataCategory {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Advancements => "advancements",
            Self::Stats => "stats",
            Self::PlayerData => "playerdata",
        }
    }
}

/// Short-lived preview token returned by `prepare_player_data_reset`.
///
/// The token is private and this value is deliberately not `Clone`; pass the
/// plan by value to commit it at most once through the typed SDK helper.
pub struct PlayerDataResetPlan {
    token: String,
    expires_in_seconds: u64,
    categories: Vec<PlayerDataCategory>,
    affected_paths: Vec<String>,
    backup_required: bool,
    requires_force: bool,
}
impl PlayerDataResetPlan {
    /// Seconds until the host expires this prepared plan (currently 60).
    pub const fn expires_in_seconds(&self) -> u64 {
        self.expires_in_seconds
    }
    /// Normalized set of categories that will be affected by commit.
    pub fn categories(&self) -> &[PlayerDataCategory] {
        &self.categories
    }
    /// Existing world-relative paths identified by the host during preparation.
    pub fn affected_paths(&self) -> &[String] {
        &self.affected_paths
    }
    /// Exact `backup_required` flag returned by the host preview; commit still takes an explicit choice.
    pub const fn backup_required(&self) -> bool {
        self.backup_required
    }
    /// Whether online sessions prevent committing the selected categories.
    pub const fn requires_force(&self) -> bool {
        self.requires_force
    }
}
impl fmt::Debug for PlayerDataResetPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayerDataResetPlan")
            .field("token", &"<private>")
            .field("expires_in_seconds", &self.expires_in_seconds)
            .field("categories", &self.categories)
            .field("affected_paths", &self.affected_paths)
            .field("backup_required", &self.backup_required)
            .field("requires_force", &self.requires_force)
            .finish()
    }
}

/// Summary of successfully removed player-data paths and the optional backup location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerDataResetResult {
    pub categories: Vec<PlayerDataCategory>,
    pub backup: Option<String>,
    pub removed_paths: Vec<String>,
}

/// A safe position found in an already loaded chunk of a player's current dimension.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SafeLocation {
    pub dimension: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// Read-only state of the server maintenance gate and admitted player sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
pub struct ServerMaintenanceStatus {
    /// Whether the server currently refuses new connections for maintenance.
    pub active: bool,
    /// Number of currently admitted player sessions.
    pub online: u64,
}

/// Outcome of beginning maintenance; connected sessions are kicked by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceBeginResult {
    pub online_after_kick_requests: u64,
    pub kicked: u64,
}

/// Outcome of ending maintenance; it can only end once no sessions remain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaintenanceEndResult {
    Ended,
    PlayersOnline(u64),
}

/// Snapshot of all saved vanilla statistics for one online player.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct PlayerStatistics {
    /// Statistic type IDs mapped to statistic IDs and their signed counts.
    pub stats: BTreeMap<String, BTreeMap<String, i32>>,
    /// Minecraft data version of the serialized statistics record.
    #[serde(rename = "DataVersion")]
    pub data_version: i32,
}

/// A vanilla gamerule value, whose type depends on the selected rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GameRuleValue {
    Bool(bool),
    Int(i32),
}

/// Severity accepted by the server's bounded plugin log operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerLogLevel {
    Info,
    Warn,
    Error,
}

impl ServerLogLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Read-only snapshot returned for a player who is online when queried.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PlayerSnapshot {
    /// Player UUID, decoded from the host's 32-character hexadecimal form.
    #[serde(rename = "uuid", deserialize_with = "deserialize_player_id")]
    pub id: PlayerId,
    /// Current player name.
    pub name: String,
    /// Namespaced world or dimension ID.
    pub dimension: String,
    /// Position in world coordinates, ordered `[x, y, z]`.
    pub position: [f64; 3],
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Vanilla game mode ID: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Operator permission level, from 0 through 4.
    pub permission_level: u8,
    /// Current health points.
    pub health: f32,
}

/// Result of dispatching a command as an online player, preserving command feedback components.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerCommandResult {
    /// False means the command did not run (for example, player is offline or not in a valid turn).
    pub ran: bool,
    /// Vanilla/host command result value.
    pub result: i32,
    /// Structured feedback components captured while running the command.
    pub output: Vec<Value>,
}

impl<'a> CommandSpec<'a> {
    /// Create a command with the host's default operator level of zero.
    pub const fn new(usage: &'a str) -> Self {
        Self {
            usage,
            default_level: 0,
            permission: None,
        }
    }

    /// Set the fallback operator level, clamped to the host's supported range 0–4.
    pub const fn default_level(mut self, level: u8) -> Self {
        self.default_level = if level > 4 { 4 } else { level };
        self
    }

    /// Associate a named permission node, resolved by ABI v3 permission callbacks.
    /// When present, the host uses this decision instead of `default_level`;
    /// the operator level is only the fallback when no node is configured.
    /// Empty nodes and nodes longer than 128 UTF-8 bytes are rejected locally by
    /// [`PluginApi::register_command`].
    pub const fn permission(mut self, node: &'a str) -> Self {
        self.permission = Some(node);
        self
    }
}

/// Borrowed access to host operations, valid only for the current callback.
/// Do not store this value or use it from another thread.
#[derive(Clone, Copy)]
pub struct PluginApi<'a> {
    host: &'a HostApiV3,
    plugin_id: &'static str,
}

impl<'a> PluginApi<'a> {
    fn new(host: &'a HostApiV3, plugin_id: &'static str) -> Self {
        Self { host, plugin_id }
    }

    /// Read one player's balance using the stable ABI v2 economy callback.
    pub fn get_balance(&self, player: PlayerId, currency: &str) -> Result<i64, EconomyError> {
        validate_currency(currency)?;
        let function = self
            .host
            .common
            .get_balance
            .ok_or(EconomyError::FunctionUnavailable)?;
        let (high, low) = player.parts();
        let mut balance = 0i64;
        // SAFETY: currency and output stay valid for this synchronous ABI call.
        let status = unsafe {
            function(
                self.host.common.context,
                high,
                low,
                currency.as_ptr(),
                currency.len(),
                &mut balance,
            )
        };
        match status {
            result::OK => Ok(balance),
            result::STORAGE_ERROR => Err(EconomyError::StorageError),
            0 => Err(EconomyError::InvalidCurrency),
            other => Err(EconomyError::UnexpectedStatus(other)),
        }
    }

    /// Atomically apply a balance delta and up to 16 plain-item deltas.
    ///
    /// Pass an empty currency and zero balance delta for an item-only exchange.
    /// The host requires the player online and outside their active command turn;
    /// inventory effects and balance persistence are coordinated on the server
    /// thread but are not a cross-file crash-atomic save transaction.
    pub fn transact(
        &self,
        player: PlayerId,
        currency: &str,
        balance_delta: i64,
        items: &[ItemDeltaSpec],
    ) -> Result<(), EconomyError> {
        if currency.is_empty() {
            if balance_delta != 0 {
                return Err(EconomyError::InvalidRequest);
            }
        } else {
            validate_currency(currency)?;
        }
        if items.len() > MAX_TRANSACTION_ITEMS {
            return Err(EconomyError::InvalidRequest);
        }
        for item in items {
            if item.delta == 0 || !valid_economy_item_id(&item.item) {
                return Err(EconomyError::InvalidRequest);
            }
        }
        let function = self
            .host
            .common
            .transact
            .ok_or(EconomyError::FunctionUnavailable)?;
        let mut raw_items = [abi::ItemDeltaV2 {
            item: std::ptr::null(),
            item_len: 0,
            delta: 0,
        }; MAX_TRANSACTION_ITEMS];
        for (raw, item) in raw_items.iter_mut().zip(items) {
            *raw = abi::ItemDeltaV2 {
                item: item.item.as_ptr(),
                item_len: item.item.len(),
                delta: item.delta,
            };
        }
        let (high, low) = player.parts();
        // SAFETY: raw_items borrow item strings retained in `items`; both arrays
        // remain alive for the complete synchronous host call.
        let status = unsafe {
            function(
                self.host.common.context,
                high,
                low,
                if currency.is_empty() {
                    std::ptr::null()
                } else {
                    currency.as_ptr()
                },
                currency.len(),
                balance_delta,
                if items.is_empty() {
                    std::ptr::null()
                } else {
                    raw_items.as_ptr()
                },
                items.len(),
            )
        };
        match status {
            result::OK => Ok(()),
            result::INSUFFICIENT_BALANCE => Err(EconomyError::InsufficientBalance),
            result::ITEMS_REJECTED => Err(EconomyError::ItemsRejected),
            result::STORAGE_ERROR => Err(EconomyError::StorageError),
            result::INVALID_REQUEST => Err(EconomyError::InvalidRequest),
            other => Err(EconomyError::UnexpectedStatus(other)),
        }
    }

    /// Call a named ABI v3 operation. `data` is serialized inside the standard
    /// `{ "operation": ..., "data": ... }` envelope. The response is JSON;
    /// operation-level failures are represented by the host's response object.
    pub fn call<T: Serialize + ?Sized>(
        &self,
        player: PlayerId,
        operation: &str,
        data: &T,
    ) -> Result<Value, CallError> {
        let mut request = Vec::new();
        let mut response = Vec::new();
        self.call_with_buffers(player, operation, data, &mut request, &mut response)
    }

    /// Call an ABI v3 operation while reusing the caller-owned response buffer.
    /// The SDK ensures its capacity is at least `MAX_JSON_BYTES`, so this call
    /// retains the same response limit as [`PluginApi::call`]. Keep one buffer
    /// in plugin state and reuse it for frequent operations to avoid allocating
    /// a fresh 64 KiB response allocation on every call.
    pub fn call_with_response_buffer<T: Serialize + ?Sized>(
        &self,
        player: PlayerId,
        operation: &str,
        data: &T,
        response: &mut Vec<u8>,
    ) -> Result<Value, CallError> {
        let mut request = Vec::new();
        self.call_with_buffers(player, operation, data, &mut request, response)
    }

    /// Call an ABI v3 operation while reusing caller-owned request and response
    /// buffers. The SDK ensures the response buffer has at least
    /// `MAX_JSON_BYTES` capacity; the request buffer grows only as required by
    /// its serialized envelope. Retaining both buffers in plugin state avoids
    /// repeated byte-buffer allocations for frequent calls. JSON value
    /// serialization/deserialization may still allocate their own data. The
    /// request buffer is cleared before serialization and the response buffer
    /// before entering the host callback.
    pub fn call_with_buffers<T: Serialize + ?Sized>(
        &self,
        player: PlayerId,
        operation: &str,
        data: &T,
        request: &mut Vec<u8>,
        response: &mut Vec<u8>,
    ) -> Result<Value, CallError> {
        #[derive(Serialize)]
        struct Request<'a, T: ?Sized> {
            operation: &'a str,
            data: &'a T,
        }

        request.clear();
        serde_json::to_writer(&mut *request, &Request { operation, data })
            .map_err(CallError::Serialization)?;
        if request.len() > MAX_JSON_BYTES {
            return Err(CallError::RequestTooLarge {
                actual: request.len(),
                maximum: MAX_JSON_BYTES,
            });
        }
        let call_json = self.host.call_json.ok_or(CallError::FunctionUnavailable)?;
        let (high, low) = player.parts();
        response.clear();
        response.reserve(MAX_JSON_BYTES);
        let mut response_len = 0usize;
        // SAFETY: request and response buffers stay alive for this synchronous
        // call, their lengths match their allocations, and response_len is valid.
        let status = unsafe {
            call_json(
                self.host.context,
                high,
                low,
                request.as_ptr(),
                request.len(),
                response.as_mut_ptr(),
                MAX_JSON_BYTES,
                &mut response_len,
            )
        };
        if status == result::BUFFER_TOO_SMALL {
            return Err(CallError::ResponseTooLarge {
                required: response_len,
                maximum: MAX_JSON_BYTES,
            });
        }
        if status != result::OK {
            return Err(CallError::HostStatus(status));
        }
        if response_len > MAX_JSON_BYTES {
            return Err(CallError::ResponseTooLarge {
                required: response_len,
                maximum: MAX_JSON_BYTES,
            });
        }
        // SAFETY: the host reports the number of bytes it wrote, bounded by
        // the capacity passed above; those bytes were initialized by the host.
        unsafe { response.set_len(response_len) };
        serde_json::from_slice(response).map_err(CallError::InvalidResponse)
    }

    /// Register a command during `Plugin::register_commands`.
    pub fn register_command(&self, spec: CommandSpec<'_>) -> Result<(), OperationError> {
        if spec
            .permission
            .is_some_and(|node| node.is_empty() || node.len() > 128)
        {
            return Err(OperationError::InvalidArgument(
                "permission node must be 1-128 bytes",
            ));
        }
        #[derive(Serialize)]
        struct Registration<'a> {
            usage: &'a str,
            default_level: u8,
            #[serde(skip_serializing_if = "Option::is_none")]
            permission: Option<&'a str>,
        }

        let response = self.call(
            PlayerId::NONE,
            operation::COMMAND_REGISTER,
            &Registration {
                usage: spec.usage,
                default_level: spec.default_level,
                permission: spec.permission,
            },
        )?;
        check_operation(response).map(|_| ())
    }

    /// Add one dynamic command completion while the host is invoking the plugin
    /// command callback for a completion request. The host rejects calls outside
    /// that temporary suggestions context.
    pub fn add_command_suggestion(
        &self,
        player: PlayerId,
        value: &str,
    ) -> Result<(), OperationError> {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(OperationError::InvalidArgument(
                "suggestion must be 1-256 bytes without control characters",
            ));
        }
        let response = self.call(
            player,
            operation::COMMAND_SUGGESTION_ADD,
            &serde_json::json!({
                "value": value,
            }),
        )?;
        check_operation(response).map(|_| ())
    }

    /// Run a command under an online player's identity and permissions.
    ///
    /// This may mutate the world or server exactly as the player command would;
    /// use only for commands the player is authorized to execute. The host only
    /// runs it in a valid player/server command context and returns `ran: false`
    /// otherwise. A leading slash is accepted by the host.
    pub fn execute_player_command(
        &self,
        player: PlayerId,
        command: &str,
    ) -> Result<PlayerCommandResult, OperationError> {
        if command.trim().is_empty() || command.len() > 2_048 {
            return Err(OperationError::InvalidArgument(
                "player command must be non-empty and at most 2048 bytes",
            ));
        }
        let response = self.call(
            PlayerId::NONE,
            operation::PLAYER_COMMAND_EXECUTE,
            &serde_json::json!({
                "uuid": format!("{:032x}", player.as_u128()),
                "command": command,
            }),
        )?;
        let ran = response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)?;
        let result = response
            .get("result")
            .and_then(Value::as_i64)
            .and_then(|result| i32::try_from(result).ok())
            .ok_or(OperationError::InvalidResponse)?;
        let output = response
            .get("output")
            .and_then(Value::as_array)
            .cloned()
            .ok_or(OperationError::InvalidResponse)?;
        Ok(PlayerCommandResult {
            ran,
            result,
            output,
        })
    }

    /// Send plain text to one online player as a structured Minecraft component.
    pub fn send_message(&self, player: PlayerId, text: &str) -> Result<(), OperationError> {
        let response = self.call(
            player,
            operation::CHAT_SEND,
            &serde_json::json!({"component": {"text": text}}),
        )?;
        check_operation(response).map(|_| ())
    }

    /// Open a plugin-owned virtual chest menu for the callback player.
    ///
    /// The returned window ID is used in later `menu_click` callbacks. This
    /// validates the host's 1–6 row, 54-item, slot, count and component-key
    /// limits before making the synchronous host call.
    pub fn open_menu(
        &self,
        player: PlayerId,
        menu: &MenuSpec,
    ) -> Result<Option<i32>, OperationError> {
        if !(1..=6).contains(&menu.rows) {
            return Err(OperationError::InvalidArgument(
                "menu rows must be between 1 and 6",
            ));
        }
        if menu.items.len() > 54 {
            return Err(OperationError::InvalidArgument(
                "menu supports at most 54 item entries",
            ));
        }
        let slot_count = usize::from(menu.rows) * 9;
        let mut seen = vec![false; slot_count];
        for item in &menu.items {
            if usize::from(item.slot) >= slot_count {
                return Err(OperationError::InvalidArgument(
                    "menu item slot exceeds the selected row capacity",
                ));
            }
            if std::mem::replace(&mut seen[usize::from(item.slot)], true) {
                return Err(OperationError::InvalidArgument(
                    "menu item slots must be unique",
                ));
            }
            if item.item.is_empty() || item.item.len() > 256 {
                return Err(OperationError::InvalidArgument(
                    "menu item ID must contain 1–256 bytes",
                ));
            }
            if item.count.is_some_and(|count| !(1..=64).contains(&count)) {
                return Err(OperationError::InvalidArgument(
                    "menu item count must be between 1 and 64",
                ));
            }
            if item.components.keys().any(|component| {
                component.len() > 128
                    || !component.contains(':')
                    || !component.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || b"_:/.-".contains(&byte)
                    })
            }) {
                return Err(OperationError::InvalidArgument(
                    "menu component keys must be namespaced lowercase IDs of at most 128 bytes",
                ));
            }
        }
        let response = self.call(player, operation::MENU_OPEN, menu)?;
        match response.get("ok").and_then(Value::as_bool) {
            Some(true) => response
                .get("window_id")
                .and_then(Value::as_i64)
                .and_then(|window| i32::try_from(window).ok())
                .map(Some)
                .ok_or(OperationError::InvalidResponse),
            Some(false) if response.get("window_id").is_some_and(Value::is_null) => Ok(None),
            _ => Err(OperationError::InvalidResponse),
        }
    }

    /// Create or replace a persistent vanilla `text_display` by stable UUID.
    ///
    /// This is an entity upsert, not an atomic in-place edit: the host removes
    /// the previous matching display before spawning its replacement. The
    /// destination dimension must already be loaded and `view_range` is 0–16.
    pub fn upsert_text_display(
        &self,
        spec: &TextDisplaySpec,
    ) -> Result<TextDisplayResult, OperationError> {
        if !spec
            .position
            .iter()
            .all(|coordinate| coordinate.is_finite())
        {
            return Err(OperationError::InvalidArgument(
                "text display coordinates must be finite",
            ));
        }
        if !(1..=2000).contains(&spec.line_width) {
            return Err(OperationError::InvalidArgument(
                "text display line width must be between 1 and 2000",
            ));
        }
        if !spec.view_range.is_finite() || !(0.0..=16.0).contains(&spec.view_range) {
            return Err(OperationError::InvalidArgument(
                "text display view range must be between 0 and 16",
            ));
        }
        if let TextComponent::Json(component) = &spec.text
            && !component.is_object()
        {
            return Err(OperationError::InvalidArgument(
                "text display JSON component must be an object",
            ));
        }
        let response = self.call(
            PlayerId::NONE,
            operation::ENTITY_DISPLAY_UPSERT,
            &serde_json::json!({
                "uuid": format!("{:032x}", spec.uuid),
                "dimension": spec.dimension,
                "position": spec.position,
                "text": spec.text,
                "line_width": spec.line_width,
                "background": spec.background,
                "view_range": spec.view_range,
                "shadow": spec.shadow,
                "see_through": spec.see_through,
                "alignment": spec.alignment.as_str(),
                "billboard": spec.billboard.as_str(),
            }),
        )?;
        let response = check_operation(response)?;
        let uuid = response
            .get("uuid")
            .and_then(Value::as_str)
            .and_then(|uuid| u128::from_str_radix(uuid, 16).ok())
            .filter(|uuid| *uuid == spec.uuid)
            .ok_or(OperationError::InvalidResponse)?;
        let entity_id = response
            .get("entity_id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or(OperationError::InvalidResponse)?;
        Ok(TextDisplayResult { uuid, entity_id })
    }

    /// Create one vanilla `text_display` and return its dimension-scoped runtime ID.
    ///
    /// Unlike [`Self::upsert_text_display`], this operation does not accept a
    /// caller UUID or replace an existing display. Retain the returned
    /// [`EntityRef`] for `remove_entity`; runtime IDs are not persistent across reloads.
    pub fn create_text_display(
        &self,
        spec: &TextDisplayCreateSpec,
    ) -> Result<EntityRef, OperationError> {
        if !spec
            .position
            .iter()
            .all(|coordinate| coordinate.is_finite())
        {
            return Err(OperationError::InvalidArgument(
                "text display coordinates must be finite",
            ));
        }
        if !(1..=2000).contains(&spec.line_width) {
            return Err(OperationError::InvalidArgument(
                "text display line width must be between 1 and 2000",
            ));
        }
        if !spec.view_range.is_finite() || !(0.0..=16.0).contains(&spec.view_range) {
            return Err(OperationError::InvalidArgument(
                "text display view range must be between 0 and 16",
            ));
        }
        if let TextComponent::Json(component) = &spec.text
            && !component.is_object()
        {
            return Err(OperationError::InvalidArgument(
                "text display JSON component must be an object",
            ));
        }
        let response = self.call(
            PlayerId::NONE,
            operation::ENTITY_TEXT_DISPLAY_CREATE,
            &serde_json::json!({
                "dimension": spec.dimension,
                "position": spec.position,
                "text": spec.text,
                "line_width": spec.line_width,
                "background": spec.background,
                "view_range": spec.view_range,
                "shadow": spec.shadow,
                "see_through": spec.see_through,
                "alignment": spec.alignment.as_str(),
                "billboard": spec.billboard.as_str(),
            }),
        )?;
        let response = check_operation(response)?;
        let entity_id = response
            .get("entity_id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or(OperationError::InvalidResponse)?;
        Ok(EntityRef::new(spec.dimension.clone(), entity_id))
    }

    /// Delete a text display by its stable UUID; returns false when it is absent.
    pub fn delete_text_display(&self, uuid: u128) -> Result<bool, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::ENTITY_DISPLAY_DELETE,
            &serde_json::json!({"uuid": format!("{uuid:032x}")}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Create or replace a vanilla item display using a Java resource-pack model key.
    ///
    /// Replacing is not atomic: the host removes the prior matching item display
    /// before spawning the new one. The destination dimension must be loaded.
    pub fn upsert_item_display(
        &self,
        spec: &ItemDisplaySpec,
    ) -> Result<TextDisplayResult, OperationError> {
        let (namespace, path) = spec.item_model.split_once(':').unwrap_or(("", ""));
        let valid_part = |part: &str, allowed: &[u8]| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || allowed.contains(&byte)
                })
        };
        if spec.item_model.len() > 256
            || !valid_part(namespace, b"_.-")
            || !valid_part(path, b"_./-")
            || path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
        {
            return Err(OperationError::InvalidArgument(
                "item model must be a valid namespaced resource key",
            ));
        }
        if !spec.position.iter().all(|value| value.is_finite()) {
            return Err(OperationError::InvalidArgument(
                "item display coordinates must be finite",
            ));
        }
        let finite_transform = spec
            .translation
            .iter()
            .chain(spec.rotation.iter())
            .chain(spec.scale.iter())
            .all(|value| value.is_finite() && value.abs() <= 1.0e6);
        let quaternion_norm = spec
            .rotation
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if !finite_transform || quaternion_norm < 1.0e-6 {
            return Err(OperationError::InvalidArgument(
                "item display transform must be finite, bounded, and have a nonzero quaternion",
            ));
        }
        let response = check_operation(self.call(
            PlayerId::NONE,
            operation::ENTITY_ITEM_DISPLAY_UPSERT,
            &serde_json::json!({
                "uuid": format!("{:032x}", spec.uuid),
                "dimension": spec.dimension,
                "position": spec.position,
                "item_model": spec.item_model,
                "translation": spec.translation,
                "rotation": spec.rotation,
                "scale": spec.scale,
            }),
        )?)?;
        let uuid = response
            .get("uuid")
            .and_then(Value::as_str)
            .and_then(|uuid| u128::from_str_radix(uuid, 16).ok())
            .filter(|uuid| *uuid == spec.uuid)
            .ok_or(OperationError::InvalidResponse)?;
        let entity_id = response
            .get("entity_id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or(OperationError::InvalidResponse)?;
        Ok(TextDisplayResult { uuid, entity_id })
    }

    /// Move a non-simulated native entity (display, marker or interaction).
    ///
    /// This does not move mobs or players. The entity must still exist in the
    /// specified loaded dimension; the host applies yaw modulo 360 and clamps
    /// pitch to -90..=90 degrees. Displays ease toward the new position using
    /// their vanilla teleport interpolation settings.
    pub fn move_entity(
        &self,
        entity: &EntityRef,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) -> Result<bool, OperationError> {
        if !position.iter().all(|coordinate| coordinate.is_finite())
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            return Err(OperationError::InvalidArgument(
                "entity position and rotation must be finite",
            ));
        }
        let response = self.call(
            PlayerId::NONE,
            operation::ENTITY_MOVE,
            &serde_json::json!({
                "dimension": entity.dimension,
                "entity_id": entity.entity_id,
                "position": position,
                "yaw": yaw,
                "pitch": pitch,
            }),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Remove a non-player entity/projectile in its specified loaded dimension.
    ///
    /// This is not a general mob lifecycle API; removal follows the host's
    /// `/kill`-like native operation and returns false when the target is absent.
    pub fn remove_entity(&self, entity: &EntityRef) -> Result<bool, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::ENTITY_REMOVE,
            &serde_json::json!({
                "dimension": entity.dimension,
                "entity_id": entity.entity_id,
            }),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Spawn a generic native entity using the host's bounded options.
    ///
    /// This creates a real entity and should not be called repeatedly from a
    /// tick callback. Entity IDs are runtime/dimension-scoped; retain the UUID
    /// for stable removal and the returned `EntityRef` for supported operations.
    pub fn spawn_entity(
        &self,
        spec: &EntitySpawnSpec,
    ) -> Result<EntitySpawnResult, OperationError> {
        let kind = &spec.entity_type;
        if kind.len() > 128
            || !kind.contains(':')
            || kind.starts_with(':')
            || kind.ends_with(':')
            || !kind.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_:/. -".contains(&byte)
            })
            || kind.contains(' ')
        {
            return Err(OperationError::InvalidArgument(
                "entity type must be a namespaced lowercase identifier of at most 128 bytes",
            ));
        }
        if !spec
            .position
            .iter()
            .all(|coordinate| coordinate.is_finite())
        {
            return Err(OperationError::InvalidArgument(
                "entity coordinates must be finite",
            ));
        }
        if spec
            .custom_name
            .as_ref()
            .is_some_and(|name| name.len() > 256)
        {
            return Err(OperationError::InvalidArgument(
                "entity custom name must be at most 256 bytes",
            ));
        }
        if kind == "minecraft:interaction"
            && [spec.interaction_width, spec.interaction_height]
                .into_iter()
                .flatten()
                .any(|size| !size.is_finite() || !(0.0..=64.0).contains(&size))
        {
            return Err(OperationError::InvalidArgument(
                "interaction width and height must be finite and between 0 and 64",
            ));
        }
        let mut data = serde_json::json!({
            "uuid": format!("{:032x}", spec.uuid),
            "dimension": spec.dimension,
            "entity_type": spec.entity_type,
            "position": spec.position,
        });
        for (key, value) in [
            ("no_ai", spec.no_ai),
            ("invulnerable", spec.invulnerable),
            ("silent", spec.silent),
            ("persistent", spec.persistent),
            ("name_visible", spec.name_visible),
            ("interaction_response", spec.interaction_response),
        ] {
            if let Some(value) = value {
                data[key] = serde_json::json!(value);
            }
        }
        if let Some(name) = &spec.custom_name {
            data["custom_name"] = serde_json::json!(name);
        }
        if let Some(width) = spec.interaction_width {
            data["interaction_width"] = serde_json::json!(width);
        }
        if let Some(height) = spec.interaction_height {
            data["interaction_height"] = serde_json::json!(height);
        }
        let response =
            check_operation(self.call(PlayerId::NONE, operation::ENTITY_SPAWN, &data)?)?;
        let uuid = response
            .get("uuid")
            .and_then(Value::as_str)
            .and_then(|uuid| u128::from_str_radix(uuid, 16).ok())
            .filter(|uuid| *uuid == spec.uuid)
            .ok_or(OperationError::InvalidResponse)?;
        let entity_id = response
            .get("entity_id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or(OperationError::InvalidResponse)?;
        Ok(EntitySpawnResult {
            uuid,
            entity: EntityRef::new(spec.dimension.clone(), entity_id),
        })
    }

    /// Remove a non-player entity by its stable UUID across loaded dimensions.
    pub fn remove_entity_by_uuid(&self, uuid: u128) -> Result<bool, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::ENTITY_REMOVE_UUID,
            &serde_json::json!({"uuid": format!("{uuid:032x}")}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Teleport an online player to a dimension and position.
    ///
    /// Returns `false` when the host cannot perform or queue the teleport,
    /// for example because the player is offline or the destination is invalid.
    /// Cross-dimension teleports may be queued by the host until its level
    /// transition phase. Non-finite coordinates or angles are rejected locally.
    pub fn teleport_player(
        &self,
        player: PlayerId,
        dimension: &str,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) -> Result<bool, OperationError> {
        if !position.iter().all(|coordinate| coordinate.is_finite())
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            return Err(OperationError::InvalidArgument(
                "teleport coordinates and angles must be finite",
            ));
        }
        let response = self.call(
            player,
            operation::PLAYER_TELEPORT,
            &serde_json::json!({
                "dimension": dimension,
                "position": position,
                "yaw": yaw,
                "pitch": pitch,
            }),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Request a vanilla game-mode change for an online player.
    ///
    /// Returns `false` when the host does not change the mode (including an
    /// offline player or a change cancelled by a mod). The host remains
    /// responsible for applying the change in the correct player turn.
    pub fn set_game_mode(&self, player: PlayerId, mode: GameMode) -> Result<bool, OperationError> {
        let response = self.call(
            player,
            operation::PLAYER_GAME_MODE_SET,
            &serde_json::json!({"mode": mode.as_id()}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Set or clear a player's chat/list display names and optionally nametag style.
    ///
    /// The host always applies both chat and tab fields: `None` clears that
    /// override. `nametag: None` leaves the nametag unchanged. The host can
    /// partially apply fields, so inspect each returned flag. It may refuse
    /// calls made while a player's state is swapped into a native hook.
    pub fn set_player_display(
        &self,
        spec: &PlayerDisplaySpec,
    ) -> Result<PlayerDisplayResult, OperationError> {
        let mut data = serde_json::json!({
            "uuid": format!("{:032x}", spec.player.as_u128()),
            "chat": &spec.chat,
            "tab": &spec.tab,
        });
        if let Some(nametag) = &spec.nametag {
            data["nametag_prefix"] = serde_json::to_value(&nametag.prefix)
                .map_err(|_| OperationError::InvalidArgument("invalid nametag prefix"))?;
            data["nametag_suffix"] = serde_json::to_value(&nametag.suffix)
                .map_err(|_| OperationError::InvalidArgument("invalid nametag suffix"))?;
        }
        let response = self.call(PlayerId::NONE, operation::PLAYER_DISPLAY_SET, &data)?;
        let flag = |key| {
            response
                .get(key)
                .and_then(Value::as_bool)
                .ok_or(OperationError::InvalidResponse)
        };
        Ok(PlayerDisplayResult {
            applied: flag("ok")?,
            chat: flag("chat")?,
            tab: flag("tab")?,
            nametag: flag("nametag")?,
        })
    }

    /// Set the callback player's sidebar using at most 15 rows.
    ///
    /// Components are serialized for the host to interpret. Returns whether
    /// the host applied the sidebar; no more than 15 rows are accepted by the
    /// server operation.
    pub fn set_sidebar(
        &self,
        player: PlayerId,
        title: &TextComponent,
        lines: &[SidebarLine],
    ) -> Result<bool, OperationError> {
        if lines.len() > 15 {
            return Err(OperationError::InvalidArgument(
                "sidebar supports at most 15 lines",
            ));
        }
        let response = self.call(
            player,
            operation::SCOREBOARD_SIDEBAR_SET,
            &serde_json::json!({"title": title, "lines": lines}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Clear the callback player's sidebar.
    pub fn clear_sidebar(&self, player: PlayerId) -> Result<bool, OperationError> {
        let response = self.call(
            player,
            operation::SCOREBOARD_SIDEBAR_CLEAR,
            &serde_json::json!({}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Open a live or saved Player/Ender inventory view for another player.
    ///
    /// The view is interactive: clicks use vanilla transfer rules and can
    /// change the target's inventory. It may be opened only outside a player's
    /// state-swapped turn. Returns `None` when the host cannot open it.
    pub fn open_inventory_view(
        &self,
        viewer: PlayerId,
        spec: &InventoryViewSpec,
    ) -> Result<Option<i32>, OperationError> {
        if viewer == spec.target {
            return Err(OperationError::InvalidArgument(
                "inventory view target must differ from viewer",
            ));
        }
        let rows = spec
            .rows
            .unwrap_or_else(|| spec.kind.default_rows())
            .clamp(1, 6);
        let menu_slots = usize::from(rows) * 9;
        let layout_len = spec
            .layout
            .as_ref()
            .map_or_else(|| spec.kind.slot_count(), Vec::len);
        if layout_len > menu_slots {
            return Err(OperationError::InvalidArgument(
                "inventory layout exceeds the menu row capacity",
            ));
        }
        if let Some(layout) = &spec.layout {
            let mut seen = vec![false; spec.kind.slot_count()];
            for slot in layout.iter().flatten() {
                let Some(previously_seen) = seen.get_mut(*slot) else {
                    return Err(OperationError::InvalidArgument(
                        "inventory layout slot is outside the selected inventory",
                    ));
                };
                if std::mem::replace(previously_seen, true) {
                    return Err(OperationError::InvalidArgument(
                        "inventory layout cannot show a slot more than once",
                    ));
                }
            }
        }
        let response = self.call(
            viewer,
            operation::INVENTORY_VIEW_OPEN,
            &serde_json::json!({
                "target": format!("{:032x}", spec.target.as_u128()),
                "kind": spec.kind.as_str(),
                "title": &spec.title,
                "rows": spec.rows,
                "layout": &spec.layout,
            }),
        )?;
        match response.get("ok").and_then(Value::as_bool) {
            Some(true) => response
                .get("window")
                .and_then(Value::as_i64)
                .and_then(|window| i32::try_from(window).ok())
                .map(Some)
                .ok_or(OperationError::InvalidResponse),
            Some(false) if response.get("window").is_some_and(Value::is_null) => Ok(None),
            _ => Err(OperationError::InvalidResponse),
        }
    }

    /// Offer a resource pack to one online player.
    ///
    /// URLs are limited by the host to 32,767 Unicode characters and SHA-1
    /// strings to 40 bytes. `false` means the host could not queue the pack.
    pub fn send_resource_pack(
        &self,
        player: PlayerId,
        pack: &ResourcePackSpec,
    ) -> Result<bool, OperationError> {
        let data = serialize_resource_pack(pack)?;
        let response = self.call(player, operation::RESOURCE_PACK_SEND, &data)?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Offer one to eight resource packs to one online player.
    ///
    /// The host sends packs in order and may return a partially completed
    /// result; inspect `sent` and `error` even when `complete` is false.
    pub fn send_resource_packs(
        &self,
        player: PlayerId,
        packs: &[ResourcePackSpec],
    ) -> Result<ResourcePackBatchResult, OperationError> {
        if !(1..=8).contains(&packs.len()) {
            return Err(OperationError::InvalidArgument(
                "resource pack batches must contain 1–8 packs",
            ));
        }
        let packs = packs
            .iter()
            .map(serialize_resource_pack)
            .collect::<Result<Vec<_>, _>>()?;
        let response = self.call(
            player,
            operation::RESOURCE_PACK_SEND_BATCH,
            &serde_json::json!({"packs": packs}),
        )?;
        let complete = response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)?;
        let sent = response
            .get("sent")
            .and_then(Value::as_u64)
            .and_then(|sent| usize::try_from(sent).ok())
            .filter(|sent| *sent <= packs.len())
            .ok_or(OperationError::InvalidResponse)?;
        if complete && sent != packs.len() {
            return Err(OperationError::InvalidResponse);
        }
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(ResourcePackBatchResult {
            sent,
            complete,
            error,
        })
    }

    /// Remove one offered resource pack, or all server packs when `id` is `None`.
    pub fn remove_resource_pack(
        &self,
        player: PlayerId,
        id: Option<u128>,
    ) -> Result<bool, OperationError> {
        let mut data = serde_json::Map::new();
        if let Some(id) = id {
            data.insert("id".into(), serde_json::json!(format!("{id:032x}")));
        }
        let response = self.call(
            player,
            operation::RESOURCE_PACK_REMOVE,
            &Value::Object(data),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Read the host's current snapshot for one online player.
    pub fn get_online_player(&self, player: PlayerId) -> Result<PlayerSnapshot, OperationError> {
        let response = self.call(player, operation::PLAYER_GET, &serde_json::json!({}))?;
        let response = check_operation(response)?;
        serde_json::from_value(response).map_err(|_| OperationError::InvalidResponse)
    }

    /// Return one point-in-time snapshot per player currently online.
    ///
    /// The returned order follows the host's player list and is not guaranteed
    /// to remain stable. Snapshots are not live player handles.
    pub fn online_players(&self) -> Result<Vec<PlayerSnapshot>, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::PLAYER_ONLINE_LIST,
            &serde_json::json!({}),
        )?;
        decode_player_snapshots(check_operation(response)?)
    }

    /// Find one currently online player by ASCII case-insensitive name.
    pub fn find_online_player_by_name(&self, name: &str) -> Result<PlayerSnapshot, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::PLAYER_LOOKUP,
            &serde_json::json!({"name": name}),
        )?;
        decode_single_player(check_operation(response)?)
    }

    /// Find one currently online player by UUID.
    pub fn find_online_player_by_id(
        &self,
        player: PlayerId,
    ) -> Result<PlayerSnapshot, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::PLAYER_LOOKUP,
            &serde_json::json!({"uuid": format!("{:032x}", player.as_u128())}),
        )?;
        decode_single_player(check_operation(response)?)
    }

    /// Read one vanilla statistic for an online player, returning zero if it
    /// has never been recorded. Identifiers are bounded to 128 UTF-8 bytes.
    pub fn online_player_stat(
        &self,
        player: PlayerId,
        stat_type: &str,
        key: &str,
    ) -> Result<i32, OperationError> {
        if stat_type.len() > 128 || key.len() > 128 {
            return Err(OperationError::InvalidArgument(
                "stat type and key must be at most 128 bytes",
            ));
        }
        let response = self.call(
            player,
            operation::PLAYER_STAT_GET,
            &serde_json::json!({"type": stat_type, "key": key}),
        )?;
        let response = check_operation(response)?;
        response
            .get("value")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(OperationError::InvalidResponse)
    }

    /// Read all saved vanilla statistics for one online player.
    ///
    /// The full snapshot is subject to the ABI v3 64 KiB response limit. Prefer
    /// `online_player_stat` for frequent or tick-driven queries.
    pub fn online_player_stats(
        &self,
        player: PlayerId,
    ) -> Result<PlayerStatistics, OperationError> {
        let response = self.call(player, operation::PLAYER_STATS_GET, &serde_json::json!({}))?;
        let response = check_operation(response)?;
        let stats = response
            .get("stats")
            .cloned()
            .ok_or(OperationError::InvalidResponse)?;
        serde_json::from_value(stats).map_err(|_| OperationError::InvalidResponse)
    }

    /// Read whether the maintenance gate is active and how many sessions are online.
    pub fn server_maintenance_status(&self) -> Result<ServerMaintenanceStatus, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::SERVER_MAINTENANCE_STATUS,
            &serde_json::json!({}),
        )?;
        serde_json::from_value(check_operation(response)?)
            .map_err(|_| OperationError::InvalidResponse)
    }

    /// Enable the maintenance gate and ask the host to kick currently connected players.
    ///
    /// This has immediate disruptive effect. `None` uses the host's default kick
    /// message; a supplied message must contain 1–256 UTF-8 bytes.
    pub fn begin_server_maintenance(
        &self,
        message: Option<&str>,
    ) -> Result<MaintenanceBeginResult, OperationError> {
        if message.is_some_and(|message| message.is_empty() || message.len() > 256) {
            return Err(OperationError::InvalidArgument(
                "maintenance message must contain 1–256 bytes",
            ));
        }
        let data = message.map_or_else(
            || serde_json::json!({}),
            |message| serde_json::json!({"message": message}),
        );
        let response = check_operation(self.call(
            PlayerId::NONE,
            operation::SERVER_MAINTENANCE_BEGIN,
            &data,
        )?)?;
        Ok(MaintenanceBeginResult {
            online_after_kick_requests: response
                .get("online")
                .and_then(Value::as_u64)
                .ok_or(OperationError::InvalidResponse)?,
            kicked: response
                .get("kicked")
                .and_then(Value::as_u64)
                .ok_or(OperationError::InvalidResponse)?,
        })
    }

    /// End maintenance only when the host reports that no player sessions remain.
    pub fn end_server_maintenance(&self) -> Result<MaintenanceEndResult, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::SERVER_MAINTENANCE_END,
            &serde_json::json!({}),
        )?;
        match response.get("ok").and_then(Value::as_bool) {
            Some(true) if response.get("active").and_then(Value::as_bool) == Some(false) => {
                Ok(MaintenanceEndResult::Ended)
            }
            Some(false)
                if response.get("error").and_then(Value::as_str) == Some("players_online") =>
            {
                response
                    .get("online")
                    .and_then(Value::as_u64)
                    .map(MaintenanceEndResult::PlayersOnline)
                    .ok_or(OperationError::InvalidResponse)
            }
            Some(false) => Err(OperationError::Rejected(
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unspecified host rejection")
                    .to_owned(),
            )),
            _ => Err(OperationError::InvalidResponse),
        }
    }

    /// List dimensions in the save and the subset currently loaded in memory.
    pub fn world_dimensions(&self) -> Result<WorldDimensions, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_DIMENSIONS_LIST,
            &serde_json::json!({}),
        )?;
        serde_json::from_value(check_operation(response)?)
            .map_err(|_| OperationError::InvalidResponse)
    }

    /// Prepare a player-data reset plan without deleting data.
    ///
    /// The host returns affected paths and a short-lived, single-use token. Use
    /// `commit_player_data_reset` only after inspecting this preview; the host
    /// expires plans after 60 seconds and may report that players must leave.
    pub fn prepare_player_data_reset(
        &self,
        categories: &[PlayerDataCategory],
    ) -> Result<PlayerDataResetPlan, OperationError> {
        if categories.is_empty() {
            return Err(OperationError::InvalidArgument(
                "player-data reset requires at least one category",
            ));
        }
        let mut requested = categories.to_vec();
        requested.sort_by_key(|category| category.as_str());
        requested.dedup();
        let category_names = requested
            .iter()
            .map(|category| category.as_str())
            .collect::<Vec<_>>();
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_PLAYER_DATA_RESET_PREPARE,
            &serde_json::json!({"categories": category_names}),
        )?;
        let response = check_operation(response)?;
        let token = response
            .get("token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or(OperationError::InvalidResponse)?
            .to_owned();
        let expires_in_seconds = response
            .get("expires_in_seconds")
            .and_then(Value::as_u64)
            .filter(|seconds| *seconds > 0)
            .ok_or(OperationError::InvalidResponse)?;
        let returned_categories = response
            .get("categories")
            .and_then(Value::as_array)
            .and_then(|values| {
                values
                    .iter()
                    .map(|value| match value.as_str()? {
                        "advancements" => Some(PlayerDataCategory::Advancements),
                        "stats" => Some(PlayerDataCategory::Stats),
                        "playerdata" => Some(PlayerDataCategory::PlayerData),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or(OperationError::InvalidResponse)?;
        if returned_categories != requested {
            return Err(OperationError::InvalidResponse);
        }
        let affected_paths = response
            .get("affected_paths")
            .and_then(Value::as_array)
            .and_then(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or(OperationError::InvalidResponse)?;
        let backup_required = response
            .get("backup_required")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)?;
        let requires_force = response
            .get("requires_force")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)?;
        Ok(PlayerDataResetPlan {
            token,
            expires_in_seconds,
            categories: returned_categories,
            affected_paths,
            backup_required,
            requires_force,
        })
    }

    /// Commit a prepared player-data reset. This is destructive and may perform
    /// large synchronous backup/deletion I/O on the server thread; review the
    /// plan first and choose whether to keep the default backup explicitly.
    /// The plan is consumed so this typed value cannot be committed twice.
    pub fn commit_player_data_reset(
        &self,
        plan: PlayerDataResetPlan,
        backup: bool,
    ) -> Result<PlayerDataResetResult, OperationError> {
        let expected_categories = plan.categories;
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_PLAYER_DATA_RESET_COMMIT,
            &serde_json::json!({"token":plan.token,"backup":backup}),
        )?;
        if response.get("ok").and_then(Value::as_bool) == Some(false) {
            let mut reason = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unspecified host rejection")
                .to_owned();
            if let Some(path) = response.get("backup").and_then(Value::as_str) {
                reason.push_str(&format!("; backup={path}"));
            }
            if let Some(detail) = response.get("detail").and_then(Value::as_str) {
                reason.push_str(&format!("; detail={detail}"));
            }
            return Err(OperationError::Rejected(reason));
        }
        let response = check_operation(response)?;
        let categories = response
            .get("categories")
            .and_then(Value::as_array)
            .and_then(|values| {
                values
                    .iter()
                    .map(|value| match value.as_str()? {
                        "advancements" => Some(PlayerDataCategory::Advancements),
                        "stats" => Some(PlayerDataCategory::Stats),
                        "playerdata" => Some(PlayerDataCategory::PlayerData),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or(OperationError::InvalidResponse)?;
        if categories != expected_categories {
            return Err(OperationError::InvalidResponse);
        }
        let backup = match response.get("backup") {
            Some(Value::Null) => None,
            Some(Value::String(path)) => Some(path.clone()),
            _ => return Err(OperationError::InvalidResponse),
        };
        let removed_paths = response
            .get("removed_paths")
            .and_then(Value::as_array)
            .and_then(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or(OperationError::InvalidResponse)?;
        Ok(PlayerDataResetResult {
            categories,
            backup,
            removed_paths,
        })
    }

    /// Create a custom dimension using one of the host's built-in generator presets.
    ///
    /// This performs filesystem and level preparation synchronously on the server
    /// thread. It is not suitable for frequent/tick-driven use; the host validates
    /// the namespaced ID, rejects existing/protected dimensions, and tries to close
    /// the prepared level before returning.
    pub fn create_custom_dimension(
        &self,
        id: &str,
        preset: CustomDimensionPreset,
    ) -> Result<CustomDimensionCreateResult, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_DIMENSION_CREATE,
            &serde_json::json!({"id": id, "preset": preset.as_str()}),
        )?;
        let response = check_operation(response)?;
        let dimension = response
            .get("dimension")
            .and_then(Value::as_str)
            .filter(|dimension| *dimension == id)
            .ok_or(OperationError::InvalidResponse)?;
        response
            .get("preset")
            .and_then(Value::as_str)
            .filter(|returned| *returned == preset.as_str())
            .ok_or(OperationError::InvalidResponse)?;
        let loaded = response
            .get("loaded")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)?;
        Ok(CustomDimensionCreateResult {
            dimension: dimension.to_owned(),
            preset,
            loaded,
        })
    }

    /// Load an existing saved dimension and optionally keep it ticking with an owner ticket.
    ///
    /// The current host pins chunk `[0, 0]`; this is a loading/ticking mechanism,
    /// not a general chunk API. The dimension must already exist in the save.
    pub fn load_dimension(
        &self,
        dimension: &str,
        ticket_owner: Option<&str>,
    ) -> Result<DimensionLoadResult, OperationError> {
        validate_dimension_ticket_owner(ticket_owner)?;
        let mut data = serde_json::json!({"dimension": dimension});
        if let Some(owner) = ticket_owner {
            data["owner"] = serde_json::json!(owner);
        }
        let response =
            check_operation(self.call(PlayerId::NONE, operation::WORLD_DIMENSION_LOAD, &data)?)?;
        let returned_dimension = response
            .get("dimension")
            .and_then(Value::as_str)
            .filter(|returned| *returned == dimension)
            .ok_or(OperationError::InvalidResponse)?;
        let loaded = response
            .get("loaded")
            .and_then(Value::as_bool)
            .filter(|loaded| *loaded)
            .ok_or(OperationError::InvalidResponse)?;
        let pinned = response
            .get("pinned")
            .and_then(Value::as_bool)
            .filter(|pinned| *pinned == ticket_owner.is_some())
            .ok_or(OperationError::InvalidResponse)?;
        Ok(DimensionLoadResult {
            dimension: returned_dimension.to_owned(),
            loaded,
            pinned,
        })
    }

    /// Release an optional owner ticket and ask the host to unload the dimension.
    ///
    /// The host may return `false` when the dimension cannot be unloaded, such
    /// as while a player or another ticket still uses it.
    pub fn unload_dimension(
        &self,
        dimension: &str,
        ticket_owner: Option<&str>,
    ) -> Result<bool, OperationError> {
        validate_dimension_ticket_owner(ticket_owner)?;
        let mut data = serde_json::json!({"dimension": dimension});
        if let Some(owner) = ticket_owner {
            data["owner"] = serde_json::json!(owner);
        }
        let response = self.call(PlayerId::NONE, operation::WORLD_DIMENSION_UNLOAD, &data)?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Find a safe location for an online player, searching loaded chunks only.
    ///
    /// `radius` must be 8–512 blocks, and optional `attempts` must be 1–256.
    /// `center` is an optional `[x, z]` block coordinate; otherwise the player's
    /// current position is used. This synchronous search never loads chunks.
    pub fn find_safe_location(
        &self,
        player: PlayerId,
        radius: u16,
        attempts: Option<u16>,
        center: Option<[i32; 2]>,
    ) -> Result<Option<SafeLocation>, OperationError> {
        if !(8..=512).contains(&radius) {
            return Err(OperationError::InvalidArgument(
                "safe-location radius must be between 8 and 512 blocks",
            ));
        }
        if attempts.is_some_and(|count| !(1..=256).contains(&count)) {
            return Err(OperationError::InvalidArgument(
                "safe-location attempts must be between 1 and 256",
            ));
        }
        let mut data = serde_json::json!({"radius": radius});
        if let Some(attempts) = attempts {
            data["attempts"] = serde_json::json!(attempts);
        }
        if let Some(center) = center {
            data["center"] = serde_json::json!(center);
        }
        let response = self.call(player, operation::WORLD_LOCATION_FIND_SAFE, &data)?;
        if response.get("error").and_then(Value::as_str) == Some("no_safe_loaded_location") {
            if response.get("search_scope").and_then(Value::as_str) != Some("loaded_chunks") {
                return Err(OperationError::InvalidResponse);
            }
            return Ok(None);
        }
        let response = check_operation(response)?;
        if response.get("search_scope").and_then(Value::as_str) != Some("loaded_chunks") {
            return Err(OperationError::InvalidResponse);
        }
        let location = response
            .get("location")
            .cloned()
            .ok_or(OperationError::InvalidResponse)?;
        serde_json::from_value(location)
            .map(Some)
            .map_err(|_| OperationError::InvalidResponse)
    }

    /// Read one vanilla gamerule from an already loaded dimension.
    pub fn world_gamerule(
        &self,
        dimension: &str,
        rule: &str,
    ) -> Result<GameRuleValue, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_GAMERULE_GET,
            &serde_json::json!({"dimension": dimension, "rule": rule}),
        )?;
        let response = check_operation(response)?;
        let value = response
            .get("value")
            .cloned()
            .ok_or(OperationError::InvalidResponse)?;
        serde_json::from_value(value).map_err(|_| OperationError::InvalidResponse)
    }

    /// Read one effective gamerule from an existing custom dimension without loading it.
    pub fn custom_dimension_gamerule(
        &self,
        dimension: &str,
        rule: &str,
    ) -> Result<GameRuleValue, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_CUSTOM_GAMERULE_GET,
            &serde_json::json!({"dimension": dimension, "rule": rule}),
        )?;
        let response = check_operation(response)?;
        let value = response
            .get("value")
            .cloned()
            .ok_or(OperationError::InvalidResponse)?;
        serde_json::from_value(value).map_err(|_| OperationError::InvalidResponse)
    }

    /// Set one vanilla gamerule in an already loaded dimension.
    ///
    /// Only the existing bool/int type accepted by the host for `rule` is
    /// valid. This mutates the world's game behavior and returns `false` when
    /// the dimension is not loaded or the host rejects the rule/value pair.
    pub fn set_world_gamerule(
        &self,
        dimension: &str,
        rule: &str,
        value: GameRuleValue,
    ) -> Result<bool, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_GAMERULE_SET,
            &serde_json::json!({"dimension": dimension, "rule": rule, "value": value}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Set an override for one rule in an existing custom dimension.
    ///
    /// The custom dimension need not be loaded. The override is persisted by
    /// the host and only the existing bool/int type for that rule is accepted.
    pub fn set_custom_dimension_gamerule(
        &self,
        dimension: &str,
        rule: &str,
        value: GameRuleValue,
    ) -> Result<bool, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_CUSTOM_GAMERULE_SET,
            &serde_json::json!({"dimension": dimension, "rule": rule, "value": value}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Clear one custom-dimension gamerule override, restoring its effective default.
    pub fn clear_custom_dimension_gamerule(
        &self,
        dimension: &str,
        rule: &str,
    ) -> Result<bool, OperationError> {
        let response = self.call(
            PlayerId::NONE,
            operation::WORLD_CUSTOM_GAMERULE_CLEAR,
            &serde_json::json!({"dimension": dimension, "rule": rule}),
        )?;
        response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Write one server log line. Messages must contain 1–2,048 UTF-8 bytes.
    pub fn write_log(&self, level: ServerLogLevel, message: &str) -> Result<(), OperationError> {
        if message.is_empty() || message.len() > 2_048 {
            return Err(OperationError::InvalidArgument(
                "log message must contain 1–2048 bytes",
            ));
        }
        let response = self.call(
            PlayerId::NONE,
            operation::SERVER_LOG,
            &serde_json::json!({"level": level.as_str(), "message": message}),
        )?;
        check_operation(response).map(|_| ())
    }

    /// Schedule a one-shot or repeating server-thread task, delivered through
    /// this plugin's `SERVER_TASK` callback. Delays and periods are measured in
    /// server ticks and bounded by the current host to 120,000 ticks.
    pub fn schedule_task<T: Serialize + ?Sized>(
        &self,
        delay_ticks: u32,
        period_ticks: Option<u32>,
        data: &T,
    ) -> Result<TaskId, OperationError> {
        validate_plugin_id(self.plugin_id)?;
        if delay_ticks > 120_000 {
            return Err(OperationError::InvalidArgument(
                "delay exceeds 120000 ticks",
            ));
        }
        if period_ticks.is_some_and(|period| !(1..=120_000).contains(&period)) {
            return Err(OperationError::InvalidArgument(
                "period must be between 1 and 120000 ticks",
            ));
        }
        let response = self.call(
            PlayerId::NONE,
            operation::SCHEDULER_SCHEDULE,
            &serde_json::json!({
                "plugin": self.plugin_id,
                "delay_ticks": delay_ticks,
                "period_ticks": period_ticks,
                "data": data,
            }),
        )?;
        let response = check_operation(response)?;
        let id = response
            .get("task_id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<u64>().ok())
            .or_else(|| response.get("task_id").and_then(Value::as_u64))
            .filter(|id| *id != 0)
            .ok_or(OperationError::InvalidResponse)?;
        Ok(TaskId(id))
    }

    /// Cancel a task created under this plugin's ID. The host still accepts
    /// legacy raw cancellations without an owner field, but this typed helper
    /// always scopes the request to the current plugin.
    pub fn cancel_task(&self, task_id: TaskId) -> Result<bool, OperationError> {
        validate_plugin_id(self.plugin_id)?;
        let response = self.call(
            PlayerId::NONE,
            operation::SCHEDULER_CANCEL,
            &serde_json::json!({
                "plugin": self.plugin_id,
                "task_id": task_id.as_u64().to_string(),
            }),
        )?;
        let response = check_operation(response)?;
        response
            .get("cancelled")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// Decode a `SERVER_TASK` payload belonging to this plugin.
    ///
    /// Returns `Ok(None)` for a valid task owned by another plugin. Task IDs
    /// are host-global, while the SDK scheduler scopes cancellation by plugin ID.
    pub fn decode_server_task_event(
        &self,
        payload: &Value,
    ) -> Result<Option<ServerTaskEvent>, OperationError> {
        let data = payload.get("data").ok_or(OperationError::InvalidResponse)?;
        let owner = data
            .get("plugin_id")
            .and_then(Value::as_str)
            .ok_or(OperationError::InvalidResponse)?;
        if owner != self.plugin_id {
            return Ok(None);
        }
        let task_id = data
            .get("task_id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<u64>().ok())
            .or_else(|| data.get("task_id").and_then(Value::as_u64))
            .filter(|id| *id != 0)
            .map(TaskId)
            .ok_or(OperationError::InvalidResponse)?;
        let task_data = data
            .get("data")
            .cloned()
            .ok_or(OperationError::InvalidResponse)?;
        Ok(Some(ServerTaskEvent {
            task_id,
            data: task_data,
        }))
    }

    /// Read a JSON value from this plugin's persistent storage namespace.
    pub fn storage_get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, OperationError> {
        validate_plugin_id(self.plugin_id)?;
        validate_storage_key(key)?;
        let response = self.call(
            PlayerId::NONE,
            operation::PLUGIN_STORAGE_GET,
            &serde_json::json!({"plugin": self.plugin_id, "key": key}),
        )?;
        let response = check_operation(response)?;
        let value = response
            .get("value")
            .ok_or(OperationError::InvalidResponse)?;
        let found = response
            .get("found")
            .and_then(Value::as_bool)
            .unwrap_or(!value.is_null());
        if !found {
            return Ok(None);
        }
        serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| OperationError::InvalidResponse)
    }

    /// Persist a JSON-serializable value under this plugin's storage namespace.
    pub fn storage_set<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<(), OperationError> {
        validate_plugin_id(self.plugin_id)?;
        validate_storage_key(key)?;
        let response = self.call(
            PlayerId::NONE,
            operation::PLUGIN_STORAGE_SET,
            &serde_json::json!({"plugin": self.plugin_id, "key": key, "value": value}),
        )?;
        check_operation(response).map(|_| ())
    }

    /// Delete a key from this plugin's persistent storage; return whether it existed.
    pub fn storage_delete(&self, key: &str) -> Result<bool, OperationError> {
        validate_plugin_id(self.plugin_id)?;
        validate_storage_key(key)?;
        let response = self.call(
            PlayerId::NONE,
            operation::PLUGIN_STORAGE_DELETE,
            &serde_json::json!({"plugin": self.plugin_id, "key": key}),
        )?;
        let response = check_operation(response)?;
        response
            .get("removed")
            .and_then(Value::as_bool)
            .ok_or(OperationError::InvalidResponse)
    }

    /// List storage keys in lexicographic pages, optionally filtered by prefix.
    pub fn storage_list(
        &self,
        prefix: Option<&str>,
        limit: usize,
        after: Option<&str>,
    ) -> Result<StoragePage, OperationError> {
        validate_plugin_id(self.plugin_id)?;
        if let Some(prefix) = prefix {
            validate_storage_key(prefix)?;
        }
        if !(1..=256).contains(&limit) {
            return Err(OperationError::InvalidArgument(
                "storage page limit must be 1–256",
            ));
        }
        let response = self.call(
            PlayerId::NONE,
            operation::PLUGIN_STORAGE_LIST,
            &serde_json::json!({
                "plugin": self.plugin_id,
                "key": prefix,
                "limit": limit,
                "after": after,
            }),
        )?;
        let response = check_operation(response)?;
        let keys = response
            .get("keys")
            .and_then(Value::as_array)
            .ok_or(OperationError::InvalidResponse)?
            .iter()
            .map(|key| {
                key.as_str()
                    .map(str::to_owned)
                    .ok_or(OperationError::InvalidResponse)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let next = match response.get("next") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or(OperationError::InvalidResponse)?
                    .to_owned(),
            ),
        };
        Ok(StoragePage { keys, next })
    }
}

fn check_operation(response: Value) -> Result<Value, OperationError> {
    match response.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(response),
        Some(false) => Err(OperationError::Rejected(
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unspecified host rejection")
                .to_owned(),
        )),
        None => Err(OperationError::InvalidResponse),
    }
}

fn serialize_resource_pack(pack: &ResourcePackSpec) -> Result<Value, OperationError> {
    if pack.url.is_empty() || pack.url.chars().count() > 32_767 {
        return Err(OperationError::InvalidArgument(
            "resource pack URL must contain 1–32767 characters",
        ));
    }
    if pack.sha1.as_ref().is_some_and(|sha1| sha1.len() > 40) {
        return Err(OperationError::InvalidArgument(
            "resource pack SHA-1 must be at most 40 bytes",
        ));
    }
    let mut data = serde_json::json!({
        "id": format!("{:032x}", pack.id),
        "url": &pack.url,
        "required": pack.required,
    });
    if let Some(sha1) = &pack.sha1 {
        data["sha1"] = serde_json::json!(sha1);
    }
    if let Some(prompt) = &pack.prompt {
        data["prompt"] = serde_json::to_value(prompt).map_err(|_| {
            OperationError::InvalidArgument("resource pack prompt is not serializable")
        })?;
    }
    Ok(data)
}

fn decode_player_snapshots(mut response: Value) -> Result<Vec<PlayerSnapshot>, OperationError> {
    let players = response
        .as_object_mut()
        .and_then(|response| response.remove("players"))
        .ok_or(OperationError::InvalidResponse)?;
    serde_json::from_value(players).map_err(|_| OperationError::InvalidResponse)
}

fn decode_single_player(response: Value) -> Result<PlayerSnapshot, OperationError> {
    let mut players = decode_player_snapshots(response)?;
    if players.len() != 1 {
        return Err(OperationError::InvalidResponse);
    }
    players.pop().ok_or(OperationError::InvalidResponse)
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), OperationError> {
    if plugin_id.is_empty()
        || plugin_id.len() > 64
        || matches!(plugin_id, "." | "..")
        || !plugin_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    {
        return Err(OperationError::InvalidArgument("invalid plugin ID"));
    }
    Ok(())
}
fn validate_currency(currency: &str) -> Result<(), EconomyError> {
    if currency.is_empty()
        || currency.len() > 128
        || !currency.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_:.-".contains(&byte)
        })
    {
        return Err(EconomyError::InvalidCurrency);
    }
    Ok(())
}
fn valid_economy_item_id(item: &str) -> bool {
    let Some((namespace, path)) = item.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.len() <= 64
        && path.len() <= 128
        && item.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_:/.-".contains(&byte)
        })
}
fn validate_dimension_ticket_owner(owner: Option<&str>) -> Result<(), OperationError> {
    if owner.is_some_and(|owner| {
        owner.len() > 128
            || !owner.split_once(':').is_some_and(|(namespace, path)| {
                !namespace.is_empty()
                    && !path.is_empty()
                    && owner
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_:/.-".contains(&byte))
            })
    }) {
        return Err(OperationError::InvalidArgument(
            "dimension ticket owner must be a namespaced ID of at most 128 bytes",
        ));
    }
    Ok(())
}
fn validate_storage_key(key: &str) -> Result<(), OperationError> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
    {
        return Err(OperationError::InvalidArgument("invalid storage key"));
    }
    Ok(())
}

fn deserialize_player_id<'de, D>(deserializer: D) -> Result<PlayerId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.len() != 32 {
        return Err(serde::de::Error::custom(
            "player UUID must contain 32 hex digits",
        ));
    }
    u128::from_str_radix(&value, 16)
        .map(PlayerId::from_u128)
        .map_err(serde::de::Error::custom)
}

/// Result of a plugin's named permission callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    /// Grant this permission regardless of the host's operator-level fallback.
    Grant,
    /// Deny this permission regardless of the host's operator-level fallback.
    Deny,
    /// Let the host apply the registered command's operator-level fallback.
    Defer,
}

/// Rust interface implemented by a v3 native plugin.
pub trait Plugin: Default + 'static {
    /// Stable human-readable name reported by `/plugins`.
    const NAME: &'static str;
    /// Stable namespace used by scheduler tasks and persistent plugin storage.
    /// Must contain at most 64 ASCII letters, digits, `_`, `-`, or `.`.
    const ID: &'static str;
    /// Opt in to typed decoding of known server notifications. Keep this false
    /// to avoid event-specific decoding work and receive events only via `event`.
    const TYPED_NOTIFICATION_EVENTS: bool = false;

    /// Register commands during server startup, normally with
    /// [`PluginApi::register_command`] and [`CommandSpec`].
    fn register_commands(&mut self, _api: &PluginApi<'_>) {}

    /// Handle a raw command line without the leading slash. Return `true` only
    /// when this plugin handled the command.
    fn command(&mut self, _api: &PluginApi<'_>, _player: PlayerId, _line: &str) -> bool {
        false
    }

    /// Handle a click in one of this plugin's menus.
    fn menu_click(
        &mut self,
        _api: &PluginApi<'_>,
        _player: PlayerId,
        _window_id: i32,
        _slot: i32,
        _button: i32,
        _action: i32,
    ) {
    }

    /// Receive a version-extensible JSON event payload. Return `true` only to
    /// cancel when the event is cancellable; the host decides which are. In the
    /// current host, the return is consumed for `BLOCK_BREAK` and `PLAYER_DEATH`;
    /// notification-only event kinds ignore it. Results from plugins are
    /// combined with logical OR; there is no listener priority yet.
    fn event(&mut self, _api: &PluginApi<'_>, _kind: u32, _payload: &Value) -> bool {
        false
    }

    /// Handle a known notification as a typed Rust value when
    /// `TYPED_NOTIFICATION_EVENTS` is true. For decoded notification kinds,
    /// this replaces `event`; unknown or malformed notifications still fall
    /// back to the raw JSON callback. Return `true` only to veto a currently
    /// cancellable event (`BLOCK_BREAK` or `PLAYER_DEATH`).
    fn notification_event(
        &mut self,
        _api: &PluginApi<'_>,
        _event: &ServerNotificationEvent,
    ) -> bool {
        false
    }

    /// Resolve a named permission. `Defer` uses the command's host fallback.
    fn permission(
        &mut self,
        _player: PlayerId,
        _node: &str,
        _default_level: u8,
    ) -> PermissionDecision {
        PermissionDecision::Defer
    }
}

#[doc(hidden)]
pub const fn descriptor<P: Plugin>() -> PluginV3 {
    PluginV3 {
        abi_version: ABI_VERSION_V3,
        name: P::NAME.as_ptr(),
        name_len: P::NAME.len(),
        create: Some(create::<P>),
        destroy: Some(destroy::<P>),
        register_commands: Some(register_commands::<P>),
        command: Some(command::<P>),
        menu_click: Some(menu_click::<P>),
        event: Some(event::<P>),
        permission: Some(permission::<P>),
    }
}

unsafe extern "C-unwind" fn create<P: Plugin>() -> *mut c_void {
    Box::into_raw(Box::new(P::default())).cast()
}

unsafe extern "C-unwind" fn destroy<P: Plugin>(state: *mut c_void) {
    if !state.is_null() {
        // SAFETY: this pointer was produced by `create::<P>` and is destroyed once by the host.
        unsafe { drop(Box::from_raw(state.cast::<P>())) };
    }
}

unsafe extern "C-unwind" fn register_commands<P: Plugin>(
    state: *mut c_void,
    host: *const HostApiV3,
) {
    let Some(host) = (unsafe { host.as_ref() }) else {
        return;
    };
    with_state::<P, _>(state, (), |plugin| {
        plugin.register_commands(&PluginApi::new(host, P::ID))
    });
}

unsafe extern "C-unwind" fn command<P: Plugin>(
    state: *mut c_void,
    host: *const HostApiV3,
    high: u64,
    low: u64,
    line: abi::ByteSlice,
    line_len: usize,
) -> u8 {
    let Some(host) = (unsafe { host.as_ref() }) else {
        return 0;
    };
    let Some(line) = (unsafe { read_utf8(line, line_len) }) else {
        return 0;
    };
    with_state::<P, _>(state, 0, |plugin| {
        u8::from(plugin.command(
            &PluginApi::new(host, P::ID),
            PlayerId::from_parts(high, low),
            line,
        ))
    })
}

unsafe extern "C-unwind" fn menu_click<P: Plugin>(
    state: *mut c_void,
    host: *const HostApiV3,
    high: u64,
    low: u64,
    window_id: i32,
    slot: i32,
    button: i32,
    action: i32,
) {
    let Some(host) = (unsafe { host.as_ref() }) else {
        return;
    };
    with_state::<P, _>(state, (), |plugin| {
        plugin.menu_click(
            &PluginApi::new(host, P::ID),
            PlayerId::from_parts(high, low),
            window_id,
            slot,
            button,
            action,
        );
    });
}

unsafe extern "C-unwind" fn event<P: Plugin>(
    state: *mut c_void,
    host: *const HostApiV3,
    kind: u32,
    payload: abi::ByteSlice,
    payload_len: usize,
) -> u8 {
    let Some(host) = (unsafe { host.as_ref() }) else {
        return 0;
    };
    let Some(payload) = (unsafe { read_utf8(payload, payload_len) }) else {
        return 0;
    };
    let Ok(payload) = serde_json::from_str::<Value>(payload) else {
        return 0;
    };
    if kind == event::SERVER_TASK
        && payload.pointer("/data/plugin_id").and_then(Value::as_str) != Some(P::ID)
    {
        return 0;
    }
    with_state::<P, _>(state, 0, |plugin| {
        u8::from(dispatch_plugin_event(
            plugin,
            &PluginApi::new(host, P::ID),
            kind,
            &payload,
        ))
    })
}

fn dispatch_plugin_event<P: Plugin>(
    plugin: &mut P,
    api: &PluginApi<'_>,
    kind: u32,
    payload: &Value,
) -> bool {
    if P::TYPED_NOTIFICATION_EVENTS
        && let Ok(Some(notification)) = decode_server_notification_event(kind, payload)
    {
        return plugin.notification_event(api, &notification);
    }
    plugin.event(api, kind, payload)
}

unsafe extern "C-unwind" fn permission<P: Plugin>(
    state: *mut c_void,
    high: u64,
    low: u64,
    node: abi::ByteSlice,
    node_len: usize,
    default_level: u8,
) -> i8 {
    let Some(node) = (unsafe { read_utf8(node, node_len) }) else {
        return 0;
    };
    with_state::<P, _>(state, 0, |plugin| {
        match plugin.permission(PlayerId::from_parts(high, low), node, default_level) {
            PermissionDecision::Deny => -1,
            PermissionDecision::Defer => 0,
            PermissionDecision::Grant => 1,
        }
    })
}

fn with_state<P: Plugin, R>(state: *mut c_void, fallback: R, f: impl FnOnce(&mut P) -> R) -> R {
    if state.is_null() {
        return fallback;
    }
    // SAFETY: the loader passes the opaque pointer returned by `create::<P>` to
    // every callback for this descriptor; callback execution is serialized by the host.
    let plugin = unsafe { &mut *state.cast::<P>() };
    f(plugin)
}

unsafe fn read_utf8<'a>(ptr: abi::ByteSlice, len: usize) -> Option<&'a str> {
    if len > MAX_JSON_BYTES {
        return None;
    }
    if len == 0 {
        return Some("");
    }
    if ptr.is_null() {
        return None;
    }
    // SAFETY: ABI callback inputs are borrowed for the duration of the callback.
    str::from_utf8(unsafe { slice::from_raw_parts(ptr, len) }).ok()
}

/// Export the ABI v3 entry point for the selected plugin type.
///
/// Invoke once at crate root in a `cdylib` crate:
///
/// ```rust
/// use horizon_server_plugin_sdk::{declare_plugin, Plugin};
///
/// #[derive(Default)]
/// struct ExamplePlugin;
///
/// impl Plugin for ExamplePlugin {
///     const NAME: &'static str = "Example";
///     const ID: &'static str = "example";
/// }
/// declare_plugin!(ExamplePlugin);
/// ```
#[macro_export]
macro_rules! declare_plugin {
    ($plugin:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C-unwind" fn horizon_server_plugin_v3() -> *const $crate::PluginV3 {
            static DESCRIPTOR: $crate::PluginV3 = $crate::descriptor::<$plugin>();
            &DESCRIPTOR
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    thread_local! {
        static LAST_REQUEST: RefCell<Option<Value>> = const { RefCell::new(None) };
        static RESPONSE_BODY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static ECONOMY_STATUS: Cell<i32> = const { Cell::new(result::OK) };
    }

    type CallJson = unsafe extern "C-unwind" fn(
        *mut c_void,
        u64,
        u64,
        abi::ByteSlice,
        usize,
        *mut u8,
        usize,
        *mut usize,
    ) -> i32;

    unsafe extern "C-unwind" fn respond_ok(
        _: *mut c_void,
        high: u64,
        low: u64,
        request: abi::ByteSlice,
        request_len: usize,
        response: *mut u8,
        response_capacity: usize,
        response_len: *mut usize,
    ) -> i32 {
        let bytes = unsafe { slice::from_raw_parts(request, request_len) };
        LAST_REQUEST.with(|last| *last.borrow_mut() = serde_json::from_slice(bytes).ok());
        assert_eq!(
            PlayerId::from_parts(high, low).as_u128(),
            0x1234_5678_9abc_def0_0123_4567_89ab_cdef
        );
        let body = br#"{"ok":true}"#;
        unsafe { *response_len = body.len() };
        if response_capacity < body.len() {
            return result::BUFFER_TOO_SMALL;
        }
        unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), response, body.len()) };
        result::OK
    }

    unsafe extern "C-unwind" fn respond_invalid_request(
        _: *mut c_void,
        _: u64,
        _: u64,
        _: abi::ByteSlice,
        _: usize,
        _: *mut u8,
        _: usize,
        _: *mut usize,
    ) -> i32 {
        result::INVALID_REQUEST
    }

    unsafe extern "C-unwind" fn respond_bad_json(
        _: *mut c_void,
        _: u64,
        _: u64,
        _: abi::ByteSlice,
        _: usize,
        response: *mut u8,
        _: usize,
        response_len: *mut usize,
    ) -> i32 {
        let body = b"not-json";
        unsafe { *response_len = body.len() };
        unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), response, body.len()) };
        result::OK
    }

    unsafe extern "C-unwind" fn respond_too_large(
        _: *mut c_void,
        _: u64,
        _: u64,
        _: abi::ByteSlice,
        _: usize,
        _: *mut u8,
        _: usize,
        response_len: *mut usize,
    ) -> i32 {
        unsafe { *response_len = MAX_JSON_BYTES + 1 };
        result::BUFFER_TOO_SMALL
    }

    unsafe extern "C-unwind" fn respond_custom(
        _: *mut c_void,
        _: u64,
        _: u64,
        request: abi::ByteSlice,
        request_len: usize,
        response: *mut u8,
        response_capacity: usize,
        response_len: *mut usize,
    ) -> i32 {
        let bytes = unsafe { slice::from_raw_parts(request, request_len) };
        LAST_REQUEST.with(|last| *last.borrow_mut() = serde_json::from_slice(bytes).ok());
        let body = RESPONSE_BODY.with(|body| body.borrow().clone());
        unsafe { *response_len = body.len() };
        if response_capacity < body.len() {
            return result::BUFFER_TOO_SMALL;
        }
        unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), response, body.len()) };
        result::OK
    }

    fn make_host(call_json: Option<CallJson>) -> HostApiV3 {
        HostApiV3 {
            abi_version: ABI_VERSION_V3,
            context: std::ptr::null_mut(),
            common: abi::HostApiV2 {
                abi_version: abi::ABI_VERSION,
                context: std::ptr::null_mut(),
                register_command: None,
                send_message: None,
                open_menu: None,
                get_balance: None,
                transact: None,
            },
            call_json,
        }
    }

    unsafe extern "C-unwind" fn mock_get_balance(
        _: *mut c_void,
        high: u64,
        low: u64,
        currency: abi::ByteSlice,
        currency_len: usize,
        balance: *mut i64,
    ) -> i32 {
        assert_eq!(PlayerId::from_parts(high, low).as_u128(), 0x1234);
        assert_eq!(
            unsafe { slice::from_raw_parts(currency, currency_len) },
            b"test:coin"
        );
        unsafe { *balance = 515 };
        ECONOMY_STATUS.with(Cell::get)
    }

    unsafe extern "C-unwind" fn mock_transact(
        _: *mut c_void,
        high: u64,
        low: u64,
        currency: abi::ByteSlice,
        currency_len: usize,
        balance_delta: i64,
        items: *const abi::ItemDeltaV2,
        item_count: usize,
    ) -> i32 {
        assert_eq!(PlayerId::from_parts(high, low).as_u128(), 0x1234);
        assert!(currency.is_null());
        assert_eq!(currency_len, 0);
        assert_eq!(balance_delta, 0);
        assert_eq!(item_count, 1);
        let item = unsafe { &*items };
        assert_eq!(
            unsafe { slice::from_raw_parts(item.item, item.item_len) },
            b"minecraft:diamond"
        );
        assert_eq!(item.delta, -2);
        ECONOMY_STATUS.with(Cell::get)
    }

    unsafe extern "C-unwind" fn mock_status(
        _: *mut c_void,
        _: u64,
        _: u64,
        _: abi::ByteSlice,
        _: usize,
        _: i64,
        _: *const abi::ItemDeltaV2,
        _: usize,
    ) -> i32 {
        ECONOMY_STATUS.with(Cell::get)
    }

    unsafe extern "C-unwind" fn mock_transaction_batch(
        _: *mut c_void,
        _: u64,
        _: u64,
        currency: abi::ByteSlice,
        currency_len: usize,
        balance_delta: i64,
        items: *const abi::ItemDeltaV2,
        item_count: usize,
    ) -> i32 {
        assert_eq!(
            unsafe { slice::from_raw_parts(currency, currency_len) },
            b"test:coin"
        );
        assert_eq!(balance_delta, -10);
        assert_eq!(item_count, 16);
        let items = unsafe { slice::from_raw_parts(items, item_count) };
        assert!(items.iter().all(|item| item.delta == 1));
        assert!(items.iter().all(|item| unsafe {
            slice::from_raw_parts(item.item, item.item_len) == b"minecraft:stone"
        }));
        result::OK
    }

    #[test]
    fn economy_balance_reads_and_maps_host_failures() {
        let mut host = make_host(None);
        host.common.get_balance = Some(mock_get_balance);
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x1234);
        ECONOMY_STATUS.with(|status| status.set(result::OK));
        assert_eq!(api.get_balance(player, "test:coin"), Ok(515));
        ECONOMY_STATUS.with(|status| status.set(result::STORAGE_ERROR));
        assert_eq!(
            api.get_balance(player, "test:coin"),
            Err(EconomyError::StorageError)
        );
        ECONOMY_STATUS.with(|status| status.set(0));
        assert_eq!(
            api.get_balance(player, "test:coin"),
            Err(EconomyError::InvalidCurrency)
        );
        ECONOMY_STATUS.with(|status| status.set(73));
        assert_eq!(
            api.get_balance(player, "test:coin"),
            Err(EconomyError::UnexpectedStatus(73))
        );
        let host = make_host(None);
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.get_balance(player, "test:coin"),
            Err(EconomyError::FunctionUnavailable)
        );
        assert_eq!(
            api.get_balance(player, "UPPER"),
            Err(EconomyError::InvalidCurrency)
        );
    }

    #[test]
    fn economy_transaction_supports_item_only_and_maps_failures() {
        let mut host = make_host(None);
        host.common.transact = Some(mock_transact);
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x1234);
        let items = [ItemDeltaSpec::new("minecraft:diamond", -2)];
        ECONOMY_STATUS.with(|status| status.set(result::OK));
        assert_eq!(api.transact(player, "", 0, &items), Ok(()));
        ECONOMY_STATUS.with(|status| status.set(result::INSUFFICIENT_BALANCE));
        assert_eq!(
            api.transact(player, "", 0, &items),
            Err(EconomyError::InsufficientBalance)
        );
        ECONOMY_STATUS.with(|status| status.set(result::ITEMS_REJECTED));
        assert_eq!(
            api.transact(player, "", 0, &items),
            Err(EconomyError::ItemsRejected)
        );
        assert_eq!(
            api.transact(player, "", 1, &items),
            Err(EconomyError::InvalidRequest)
        );
        assert_eq!(
            api.transact(player, "test:coin", 0, &[ItemDeltaSpec::new("bad id", 1)]),
            Err(EconomyError::InvalidRequest)
        );
        assert_eq!(
            api.transact(
                player,
                "test:coin",
                0,
                &[ItemDeltaSpec::new("minecraft:stone", 0)]
            ),
            Err(EconomyError::InvalidRequest)
        );
        let too_many = vec![ItemDeltaSpec::new("minecraft:stone", 1); MAX_TRANSACTION_ITEMS + 1];
        assert_eq!(
            api.transact(player, "test:coin", 0, &too_many),
            Err(EconomyError::InvalidRequest)
        );

        let mut host = make_host(None);
        host.common.transact = Some(mock_transaction_batch);
        let api = PluginApi::new(&host, "sdk_test");
        let batch = vec![ItemDeltaSpec::new("minecraft:stone", 1); MAX_TRANSACTION_ITEMS];
        assert_eq!(api.transact(player, "test:coin", -10, &batch), Ok(()));

        let mut host = make_host(None);
        host.common.transact = Some(mock_status);
        let api = PluginApi::new(&host, "sdk_test");
        ECONOMY_STATUS.with(|status| status.set(result::STORAGE_ERROR));
        assert_eq!(
            api.transact(player, "test:coin", 0, &[]),
            Err(EconomyError::StorageError)
        );
        ECONOMY_STATUS.with(|status| status.set(result::INVALID_REQUEST));
        assert_eq!(
            api.transact(player, "test:coin", 0, &[]),
            Err(EconomyError::InvalidRequest)
        );
        ECONOMY_STATUS.with(|status| status.set(73));
        assert_eq!(
            api.transact(player, "test:coin", 0, &[]),
            Err(EconomyError::UnexpectedStatus(73))
        );
        let host = make_host(None);
        assert_eq!(
            PluginApi::new(&host, "sdk_test").transact(player, "test:coin", 0, &[]),
            Err(EconomyError::FunctionUnavailable)
        );
    }

    #[test]
    fn player_id_round_trips_high_and_low_words() {
        let id = PlayerId::from_u128(0x1234_5678_9abc_def0_0123_4567_89ab_cdef);
        assert_eq!(id.parts(), (0x1234_5678_9abc_def0, 0x0123_4567_89ab_cdef));
        assert_eq!(PlayerId::from_parts(id.parts().0, id.parts().1), id);
    }

    #[test]
    fn call_json_builds_envelope_and_decodes_response() {
        let host = make_host(Some(respond_ok));
        let api = PluginApi::new(&host, "sdk_test");
        let response = api
            .call(
                PlayerId::from_u128(0x1234_5678_9abc_def0_0123_4567_89ab_cdef),
                operation::SERVER_LOG,
                &serde_json::json!({"level":"INFO","message":"hello"}),
            )
            .unwrap();
        assert_eq!(response, serde_json::json!({"ok": true}));
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::SERVER_LOG
            );
            assert_eq!(request.as_ref().unwrap()["data"]["message"], "hello");
        });
    }

    #[test]
    fn call_with_response_buffer_reuses_capacity_across_calls() {
        let host = make_host(Some(respond_ok));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x1234_5678_9abc_def0_0123_4567_89ab_cdef);
        let mut response = Vec::new();

        let first = api
            .call_with_response_buffer(
                player,
                operation::SERVER_LOG,
                &serde_json::json!({"level":"INFO","message":"first"}),
                &mut response,
            )
            .unwrap();
        assert_eq!(first, serde_json::json!({"ok": true}));
        assert!(response.capacity() >= MAX_JSON_BYTES);
        let address = response.as_ptr();
        let capacity = response.capacity();

        let second = api
            .call_with_response_buffer(
                player,
                operation::SERVER_LOG,
                &serde_json::json!({"level":"INFO","message":"second"}),
                &mut response,
            )
            .unwrap();
        assert_eq!(second, serde_json::json!({"ok": true}));
        assert_eq!(response.as_ptr(), address);
        assert_eq!(response.capacity(), capacity);
    }

    #[test]
    fn call_with_buffers_reuses_request_and_response_storage() {
        let host = make_host(Some(respond_ok));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x1234_5678_9abc_def0_0123_4567_89ab_cdef);
        let data = serde_json::json!({"level":"INFO","message":"reused"});
        let mut request = Vec::new();
        let mut response = Vec::new();

        let mut request_storage = None;
        let mut response_storage = None;
        for iteration in 0..2 {
            assert_eq!(
                api.call_with_buffers(
                    player,
                    operation::SERVER_LOG,
                    &data,
                    &mut request,
                    &mut response,
                )
                .unwrap(),
                serde_json::json!({"ok": true})
            );
            let current = (
                request.as_ptr(),
                request.capacity(),
                response.as_ptr(),
                response.capacity(),
            );
            if iteration == 0 {
                request_storage = Some((current.0, current.1));
                response_storage = Some((current.2, current.3));
            } else {
                assert_eq!(request_storage, Some((current.0, current.1)));
                assert_eq!(response_storage, Some((current.2, current.3)));
            }
        }
        assert!(response.capacity() >= MAX_JSON_BYTES);
        let serialized: Value = serde_json::from_slice(&request).unwrap();
        assert_eq!(serialized["operation"], operation::SERVER_LOG);
        assert_eq!(serialized["data"]["message"], "reused");
        LAST_REQUEST.with(|last| {
            assert_eq!(last.borrow().as_ref().unwrap()["data"]["message"], "reused");
        });
    }

    #[test]
    fn command_completion_input_recognizes_only_bounded_host_prefix() {
        let request = format!("{}warp home", abi::COMMAND_COMPLETION_PREFIX);
        assert_eq!(command_completion_input(&request), Some("warp home"));
        assert_eq!(command_completion_input("warp home"), None);

        let oversized = format!(
            "{}{}",
            abi::COMMAND_COMPLETION_PREFIX,
            "x".repeat(MAX_COMMAND_COMPLETION_BYTES + 1)
        );
        assert_eq!(command_completion_input(&oversized), None);
    }

    #[test]
    fn typed_notification_opt_in_routes_known_events_and_preserves_raw_fallback() {
        #[derive(Default)]
        struct TypedPlugin {
            typed_called: bool,
            raw_called: bool,
        }
        impl Plugin for TypedPlugin {
            const NAME: &'static str = "Typed test";
            const ID: &'static str = "typed_test";
            const TYPED_NOTIFICATION_EVENTS: bool = true;

            fn event(&mut self, _api: &PluginApi<'_>, _kind: u32, _payload: &Value) -> bool {
                self.raw_called = true;
                false
            }

            fn notification_event(
                &mut self,
                _api: &PluginApi<'_>,
                event: &ServerNotificationEvent,
            ) -> bool {
                self.typed_called = matches!(event, ServerNotificationEvent::PlayerJoined { .. });
                self.typed_called
            }
        }

        let host = make_host(None);
        let api = PluginApi::new(&host, TypedPlugin::ID);
        let mut plugin = TypedPlugin::default();
        let joined = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "dimension": "minecraft:overworld", "data": null
        });
        assert!(dispatch_plugin_event(
            &mut plugin,
            &api,
            event::PLAYER_JOIN,
            &joined
        ));
        assert!(plugin.typed_called);
        assert!(!plugin.raw_called);

        plugin.typed_called = false;
        assert!(!dispatch_plugin_event(
            &mut plugin,
            &api,
            event::PLAYER_MOVE,
            &Value::Null
        ));
        assert!(plugin.raw_called);
        assert!(!plugin.typed_called);
    }

    #[test]
    fn command_spec_serializes_and_reports_operation_rejection() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        api.register_command(
            CommandSpec::new("ola")
                .default_level(2)
                .permission("ola.usar"),
        )
        .unwrap();
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::COMMAND_REGISTER
            );
            assert_eq!(request.as_ref().unwrap()["data"]["usage"], "ola");
            assert_eq!(request.as_ref().unwrap()["data"]["default_level"], 2);
            assert_eq!(request.as_ref().unwrap()["data"]["permission"], "ola.usar");
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":false,"error":"commands_can_only_be_registered_during_startup"}"#.to_vec()
        });
        let error = api.register_command(CommandSpec::new("ola")).unwrap_err();
        assert!(matches!(
            error,
            OperationError::Rejected(reason)
                if reason == "commands_can_only_be_registered_during_startup"
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        api.register_command(CommandSpec::new("ajuda")).unwrap();
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            assert_eq!(request.as_ref().unwrap()["data"]["default_level"], 0);
            assert!(
                request.as_ref().unwrap()["data"]
                    .get("permission")
                    .is_none()
            );
        });

        api.register_command(CommandSpec::new("limitado").default_level(u8::MAX))
            .unwrap();
        LAST_REQUEST.with(|last| {
            assert_eq!(last.borrow().as_ref().unwrap()["data"]["default_level"], 4);
        });

        let no_call_host = make_host(None);
        let no_call_api = PluginApi::new(&no_call_host, "sdk_test");
        let overlong_permission = "x".repeat(129);
        for permission in ["", overlong_permission.as_str()] {
            assert!(matches!(
                no_call_api
                    .register_command(CommandSpec::new("invalid").permission(permission))
                    .unwrap_err(),
                OperationError::InvalidArgument(_)
            ));
        }
    }

    #[test]
    fn command_suggestion_helper_serializes_and_validates_host_context() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x1234_5678_9abc_def0_0123_4567_89ab_cdef);

        api.add_command_suggestion(player, "home").unwrap();
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::COMMAND_SUGGESTION_ADD);
            assert_eq!(request["data"]["value"], "home");
        });

        assert!(matches!(
            api.add_command_suggestion(player, "").unwrap_err(),
            OperationError::InvalidArgument(_)
        ));
        assert!(matches!(
            api.add_command_suggestion(player, &"x".repeat(257))
                .unwrap_err(),
            OperationError::InvalidArgument(_)
        ));
        assert!(matches!(
            api.add_command_suggestion(player, "bad\nvalue")
                .unwrap_err(),
            OperationError::InvalidArgument(_)
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"suggestions_unavailable"}"#.to_vec()
        });
        assert!(matches!(
            api.add_command_suggestion(player, "home").unwrap_err(),
            OperationError::Rejected(reason) if reason == "suggestions_unavailable"
        ));
    }

    #[test]
    fn execute_player_command_preserves_permissions_and_command_feedback() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"result":1,"output":[{"text":"Done"}]}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x2a);
        let result = api.execute_player_command(player, "/say hello").unwrap();
        assert!(result.ran);
        assert_eq!(result.result, 1);
        assert_eq!(result.output, [serde_json::json!({"text":"Done"})]);
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_COMMAND_EXECUTE);
            assert_eq!(request["data"]["uuid"], "0000000000000000000000000000002a");
            assert_eq!(request["data"]["command"], "/say hello");
        });
        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":false,"result":0,"output":[]}"#.to_vec());
        assert!(!api.execute_player_command(player, "say hello").unwrap().ran);
        assert!(matches!(
            api.execute_player_command(player, "  "),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.execute_player_command(player, &"x".repeat(2049)),
            Err(OperationError::InvalidArgument(_))
        ));
    }

    #[test]
    fn scheduler_helpers_scope_task_creation_and_cancellation_by_plugin_id() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"task_id":"42"}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let task = api
            .schedule_task(20, Some(5), &serde_json::json!({"kind":"heartbeat"}))
            .unwrap();
        assert_eq!(task.as_u64(), 42);
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let data = &request.as_ref().unwrap()["data"];
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::SCHEDULER_SCHEDULE
            );
            assert_eq!(data["plugin"], "sdk_test");
            assert_eq!(data["delay_ticks"], 20);
            assert_eq!(data["period_ticks"], 5);
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"cancelled":true}"#.to_vec());
        assert!(api.cancel_task(task).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let data = &request.as_ref().unwrap()["data"];
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::SCHEDULER_CANCEL
            );
            assert_eq!(data["plugin"], "sdk_test");
            assert_eq!(data["task_id"], "42");
        });

        assert!(matches!(
            api.schedule_task(120_001, None, &()),
            Err(OperationError::InvalidArgument(_))
        ));
        let invalid_id_api = PluginApi::new(&host, "../outside");
        assert!(matches!(
            invalid_id_api.schedule_task(1, None, &()),
            Err(OperationError::InvalidArgument(_))
        ));
    }

    #[test]
    fn send_message_builds_structured_chat_and_reports_rejection() {
        let host = make_host(Some(respond_ok));
        let api = PluginApi::new(&host, "sdk_test");
        api.send_message(
            PlayerId::from_u128(0x1234_5678_9abc_def0_0123_4567_89ab_cdef),
            "Olá, servidor!",
        )
        .unwrap();
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::CHAT_SEND);
            assert_eq!(request["data"]["component"]["text"], "Olá, servidor!");
        });

        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"player_not_online"}"#.to_vec()
        });
        assert!(matches!(
            api.send_message(PlayerId::NONE, "Oi"),
            Err(OperationError::Rejected(reason)) if reason == "player_not_online"
        ));
    }

    #[test]
    fn teleport_player_sends_typed_destination_and_preserves_host_result() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(7);
        assert!(
            api.teleport_player(
                player,
                "minecraft:the_nether",
                [12.5, 80.0, -4.25],
                90.0,
                35.0,
            )
            .unwrap()
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_TELEPORT);
            assert_eq!(request["data"]["dimension"], "minecraft:the_nether");
            assert_eq!(
                request["data"]["position"],
                serde_json::json!([12.5, 80.0, -4.25])
            );
            assert_eq!(request["data"]["yaw"], 90.0);
            assert_eq!(request["data"]["pitch"], 35.0);
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":false}"#.to_vec());
        assert!(
            !api.teleport_player(player, "missing:dimension", [0.0, 64.0, 0.0], 0.0, 0.0)
                .unwrap()
        );
        assert!(matches!(
            api.teleport_player(
                player,
                "minecraft:overworld",
                [f64::INFINITY, 0.0, 0.0],
                0.0,
                0.0
            ),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.teleport_player(
                player,
                "minecraft:overworld",
                [0.0, 0.0, 0.0],
                f32::NAN,
                0.0
            ),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":"yes"}"#.to_vec());
        assert!(matches!(
            api.teleport_player(player, "minecraft:overworld", [0.0, 0.0, 0.0], 0.0, 0.0),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn set_game_mode_uses_validated_enum_ids_and_returns_changed_status() {
        assert_eq!(GameMode::Survival.as_id(), 0);
        assert_eq!(GameMode::Creative.as_id(), 1);
        assert_eq!(GameMode::Adventure.as_id(), 2);
        assert_eq!(GameMode::Spectator.as_id(), 3);

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert!(
            api.set_game_mode(PlayerId::from_u128(9), GameMode::Spectator)
                .unwrap()
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_GAME_MODE_SET);
            assert_eq!(request["data"], serde_json::json!({"mode": 3}));
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":false}"#.to_vec());
        assert!(
            !api.set_game_mode(PlayerId::from_u128(9), GameMode::Survival)
                .unwrap()
        );
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{}"#.to_vec());
        assert!(matches!(
            api.set_game_mode(PlayerId::from_u128(9), GameMode::Survival),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn player_display_helper_exposes_clear_semantics_and_partial_result() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"chat":true,"tab":false,"nametag":true}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let spec = PlayerDisplaySpec {
            player: PlayerId::from_u128(0xabc),
            chat: Some(TextComponent::text("§aBuilder")),
            tab: None,
            nametag: Some(NameTagStyle {
                prefix: Some(TextComponent::json(serde_json::json!({"text":"[VIP]"}))),
                suffix: None,
            }),
        };
        let result = api.set_player_display(&spec).unwrap();
        assert_eq!(
            result,
            PlayerDisplayResult {
                applied: false,
                chat: true,
                tab: false,
                nametag: true,
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_DISPLAY_SET);
            assert_eq!(request["data"]["uuid"], "00000000000000000000000000000abc");
            assert_eq!(request["data"]["chat"], "§aBuilder");
            assert!(request["data"]["tab"].is_null());
            assert_eq!(request["data"]["nametag_prefix"]["text"], "[VIP]");
            assert!(request["data"]["nametag_suffix"].is_null());
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"chat":true,"tab":true,"nametag":true}"#.to_vec()
        });
        let clear_names = PlayerDisplaySpec {
            player: spec.player,
            chat: None,
            tab: None,
            nametag: None,
        };
        assert!(api.set_player_display(&clear_names).unwrap().applied);
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let data = &request.as_ref().unwrap()["data"];
            assert!(data["chat"].is_null());
            assert!(data["tab"].is_null());
            assert!(data.get("nametag_prefix").is_none());
            assert!(data.get("nametag_suffix").is_none());
        });
    }

    #[test]
    fn sidebar_helpers_serialize_components_enforce_limit_and_clear() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x55);
        let title = TextComponent::text("§aStats");
        let lines = [
            SidebarLine::new("Kills").with_right("3"),
            SidebarLine::new("unused").with_components(
                TextComponent::json(serde_json::json!({"text":"Deaths","color":"red"})),
                None,
            ),
        ];
        assert!(api.set_sidebar(player, &title, &lines).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::SCOREBOARD_SIDEBAR_SET);
            assert_eq!(request["data"]["title"], "§aStats");
            assert_eq!(request["data"]["lines"][0]["left"], "Kills");
            assert_eq!(request["data"]["lines"][0]["right"], "3");
            assert_eq!(request["data"]["lines"][1]["left"]["color"], "red");
            assert!(request["data"]["lines"][1]["right"].is_null());
        });

        let too_many = vec![SidebarLine::new("line"); 16];
        assert!(matches!(
            api.set_sidebar(player, &title, &too_many),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":false}"#.to_vec());
        assert!(!api.set_sidebar(player, &title, &[]).unwrap());
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        assert!(api.clear_sidebar(player).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::SCOREBOARD_SIDEBAR_CLEAR);
            assert_eq!(request["data"], serde_json::json!({}));
        });
    }

    #[test]
    fn open_menu_serializes_items_and_enforces_slot_and_component_limits() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"window_id":9}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(5);
        let menu = MenuSpec::new(TextComponent::json(serde_json::json!({"text":"Loja"})))
            .rows(2)
            .item(
                MenuItemSpec::new(0, "minecraft:diamond")
                    .count(4)
                    .name(TextComponent::text("§bComprar"))
                    .lore(vec![TextComponent::text("Clique para abrir")])
                    .component(
                        "minecraft:custom_model_data",
                        serde_json::json!({"floats":[1.0]}),
                    ),
            );
        assert_eq!(api.open_menu(player, &menu).unwrap(), Some(9));
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::MENU_OPEN);
            assert_eq!(request["data"]["rows"], 2);
            assert_eq!(request["data"]["title"]["text"], "Loja");
            assert_eq!(request["data"]["items"][0]["item"], "minecraft:diamond");
            assert_eq!(request["data"]["items"][0]["count"], 4);
            assert_eq!(request["data"]["items"][0]["name"], "§bComprar");
            assert_eq!(request["data"]["items"][0]["lore"][0], "Clique para abrir");
            assert!(
                request["data"]["items"][0]["components"]["minecraft:custom_model_data"]
                    .is_object()
            );
        });

        assert!(matches!(
            api.open_menu(player, &MenuSpec::new(TextComponent::text("bad")).rows(0)),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.open_menu(
                player,
                &MenuSpec::new(TextComponent::text("duplicate"))
                    .item(MenuItemSpec::new(0, "stone"))
                    .item(MenuItemSpec::new(0, "dirt")),
            ),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.open_menu(
                player,
                &MenuSpec::new(TextComponent::text("bad component"))
                    .item(MenuItemSpec::new(0, "stone").component("invalid-key", Value::Null)),
            ),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":false,"window_id":null}"#.to_vec());
        assert_eq!(api.open_menu(player, &menu).unwrap(), None);
    }

    #[test]
    fn text_display_helpers_upsert_delete_and_validate_host_contract() {
        let uuid = 0x1234u128;
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"uuid":"00000000000000000000000000001234","entity_id":77}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let spec = TextDisplaySpec::new(
            uuid,
            "minecraft:overworld",
            [1.5, 64.0, -2.0],
            TextComponent::json(serde_json::json!({"text":"Hello"})),
        )
        .line_width(320)
        .background(0x8011_2233)
        .view_range(4.0)
        .shadow(true)
        .see_through(true)
        .alignment(TextDisplayAlignment::Left)
        .billboard(TextDisplayBillboard::Vertical);
        assert_eq!(
            api.upsert_text_display(&spec).unwrap(),
            TextDisplayResult {
                uuid,
                entity_id: 77
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_DISPLAY_UPSERT);
            assert_eq!(request["data"]["uuid"], "00000000000000000000000000001234");
            assert_eq!(request["data"]["text"]["text"], "Hello");
            assert_eq!(request["data"]["line_width"], 320);
            assert_eq!(request["data"]["background"], 0x8011_2233u32);
            assert_eq!(request["data"]["alignment"], "left");
            assert_eq!(request["data"]["billboard"], "vertical");
        });

        assert!(matches!(
            api.upsert_text_display(&TextDisplaySpec::new(
                uuid,
                "minecraft:overworld",
                [f64::NAN, 0.0, 0.0],
                TextComponent::text("bad"),
            )),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.upsert_text_display(&spec.clone().view_range(16.1)),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"uuid":"00000000000000000000000000001234","entity_id":77}"#.to_vec()
        });
        assert!(api.delete_text_display(uuid).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_DISPLAY_DELETE);
            assert_eq!(request["data"]["uuid"], "00000000000000000000000000001234");
        });
        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":true,"uuid":"1","entity_id":77}"#.to_vec());
        assert!(matches!(
            api.upsert_text_display(&spec),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn text_display_create_uses_runtime_entity_id_without_pretending_uuid_upsert() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"entity_id":54}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let spec = TextDisplayCreateSpec::new(
            "minecraft:overworld",
            [1.0, 65.0, -3.0],
            TextComponent::text("§aHello"),
        )
        .line_width(320)
        .background(0x8011_2233)
        .view_range(4.0)
        .shadow(true)
        .see_through(true)
        .alignment(TextDisplayAlignment::Right)
        .billboard(TextDisplayBillboard::Vertical);
        assert_eq!(
            api.create_text_display(&spec).unwrap(),
            EntityRef::new("minecraft:overworld", 54)
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_TEXT_DISPLAY_CREATE);
            assert_eq!(request["data"]["dimension"], "minecraft:overworld");
            assert_eq!(
                request["data"]["position"],
                serde_json::json!([1.0, 65.0, -3.0])
            );
            assert_eq!(request["data"]["text"], "§aHello");
            assert_eq!(request["data"]["line_width"], 320);
            assert_eq!(request["data"]["background"], 0x8011_2233u32);
            assert_eq!(request["data"]["alignment"], "right");
            assert_eq!(request["data"]["billboard"], "vertical");
            assert!(request["data"].get("uuid").is_none());
        });

        assert!(matches!(
            api.create_text_display(&TextDisplayCreateSpec::new(
                "minecraft:overworld",
                [f64::NAN, 0.0, 0.0],
                TextComponent::text("bad"),
            )),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.create_text_display(&spec.clone().view_range(16.1)),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.create_text_display(&TextDisplayCreateSpec::new(
                "minecraft:overworld",
                [0.0; 3],
                TextComponent::json(serde_json::json!("not an object")),
            )),
            Err(OperationError::InvalidArgument(_))
        ));
        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":true,"entity_id":4294967296}"#.to_vec());
        assert!(matches!(
            api.create_text_display(&spec),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn entity_reference_move_and_remove_are_dimension_scoped() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let entity = EntityRef::new("minecraft:overworld", 42);
        assert!(
            api.move_entity(&entity, [1.0, 2.0, 3.0], 450.0, 100.0)
                .unwrap()
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_MOVE);
            assert_eq!(request["data"]["dimension"], "minecraft:overworld");
            assert_eq!(request["data"]["entity_id"], 42);
            assert_eq!(request["data"]["yaw"], 450.0);
            assert_eq!(request["data"]["pitch"], 100.0);
        });
        assert!(matches!(
            api.move_entity(&entity, [f64::INFINITY, 0.0, 0.0], 0.0, 0.0),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(api.remove_entity(&entity).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_REMOVE);
            assert_eq!(request["data"]["entity_id"], 42);
        });
    }

    #[test]
    fn entity_spawn_builds_interaction_request_and_removes_by_uuid() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"uuid":"0000000000000000000000000000002a","entity_id":91}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let spec = EntitySpawnSpec::new(
            42,
            "minecraft:overworld",
            "minecraft:interaction",
            [2.0, 70.0, 3.0],
        )
        .no_ai(true)
        .invulnerable(true)
        .persistent(true)
        .name_visible(true)
        .custom_name("Door trigger")
        .interaction_width(1.5)
        .interaction_height(2.0)
        .interaction_response(false);
        let result = api.spawn_entity(&spec).unwrap();
        assert_eq!(result.uuid, 42);
        assert_eq!(result.entity, EntityRef::new("minecraft:overworld", 91));
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_SPAWN);
            assert_eq!(request["data"]["entity_type"], "minecraft:interaction");
            assert_eq!(request["data"]["uuid"], "0000000000000000000000000000002a");
            assert_eq!(request["data"]["interaction_width"], 1.5);
            assert_eq!(request["data"]["interaction_response"], false);
            assert_eq!(request["data"]["custom_name"], "Door trigger");
        });
        assert!(matches!(
            api.spawn_entity(&EntitySpawnSpec::new(
                1,
                "minecraft:overworld",
                "Interaction",
                [0.0, 0.0, 0.0],
            )),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.spawn_entity(
                &EntitySpawnSpec::new(
                    1,
                    "minecraft:overworld",
                    "minecraft:interaction",
                    [0.0, 0.0, 0.0],
                )
                .interaction_width(65.0)
            ),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"entity_type":"minecraft:interaction"}"#.to_vec()
        });
        assert!(api.remove_entity_by_uuid(42).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_REMOVE_UUID);
            assert_eq!(request["data"]["uuid"], "0000000000000000000000000000002a");
        });
    }

    #[test]
    fn item_display_upsert_serializes_transform_and_rejects_invalid_model_or_rotation() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"uuid":"0000000000000000000000000000002a","entity_id":92}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let spec = ItemDisplaySpec::new(
            42,
            "minecraft:overworld",
            [1.0, 65.0, 2.0],
            "aura:npc/cube_0",
        )
        .translation([0.1, 0.2, 0.3])
        .rotation([0.0, 0.0, 0.70710677, 0.70710677])
        .scale([1.5, 1.5, 1.5]);
        assert_eq!(
            api.upsert_item_display(&spec).unwrap(),
            TextDisplayResult {
                uuid: 42,
                entity_id: 92
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::ENTITY_ITEM_DISPLAY_UPSERT);
            assert_eq!(request["data"]["item_model"], "aura:npc/cube_0");
            assert_eq!(request["data"]["translation"][1], serde_json::json!(0.2f32));
            assert_eq!(
                request["data"]["rotation"][2],
                serde_json::json!(0.70710677f32)
            );
            assert_eq!(request["data"]["scale"][0], 1.5);
        });
        assert!(matches!(
            api.upsert_item_display(&ItemDisplaySpec::new(
                42,
                "minecraft:overworld",
                [0.0, 0.0, 0.0],
                "aura:../secret",
            )),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.upsert_item_display(&spec.clone().rotation([0.0; 4])),
            Err(OperationError::InvalidArgument(_))
        ));
    }

    #[test]
    fn inventory_view_validates_layout_and_decodes_open_result() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"window":42}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let viewer = PlayerId::from_u128(1);
        let target = PlayerId::from_u128(2);
        let spec = InventoryViewSpec::new(target, InventoryViewKind::Ender, "Baú do Ender")
            .rows(1)
            .layout(vec![Some(0), None, Some(1)]);
        assert_eq!(api.open_inventory_view(viewer, &spec).unwrap(), Some(42));
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::INVENTORY_VIEW_OPEN);
            assert_eq!(
                request["data"]["target"],
                "00000000000000000000000000000002"
            );
            assert_eq!(request["data"]["kind"], "ender");
            assert_eq!(request["data"]["title"], "Baú do Ender");
            assert_eq!(request["data"]["rows"], 1);
            assert_eq!(request["data"]["layout"], serde_json::json!([0, null, 1]));
        });

        assert!(matches!(
            api.open_inventory_view(
                viewer,
                &InventoryViewSpec::new(viewer, InventoryViewKind::Player, "same player"),
            ),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.open_inventory_view(
                viewer,
                &InventoryViewSpec::new(
                    target,
                    InventoryViewKind::Player,
                    "too many default slots"
                )
                .rows(1),
            ),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.open_inventory_view(
                viewer,
                &InventoryViewSpec::new(target, InventoryViewKind::Ender, "duplicate")
                    .layout(vec![Some(1), Some(1)]),
            ),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.open_inventory_view(
                viewer,
                &InventoryViewSpec::new(target, InventoryViewKind::Ender, "out of range")
                    .layout(vec![Some(27)]),
            ),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":false,"window":null}"#.to_vec());
        assert_eq!(api.open_inventory_view(viewer, &spec).unwrap(), None);
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"window":null}"#.to_vec());
        assert!(matches!(
            api.open_inventory_view(viewer, &spec),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn resource_pack_helpers_send_remove_and_enforce_native_limits() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"sent":1}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x77);
        let pack = ResourcePackSpec::new(0x1234, "https://example.test/pack.zip")
            .sha1("ab12")
            .prompt(TextComponent::text("Install pack?"))
            .required(true);
        assert!(api.send_resource_pack(player, &pack).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::RESOURCE_PACK_SEND);
            assert_eq!(request["data"]["id"], "00000000000000000000000000001234");
            assert_eq!(request["data"]["url"], "https://example.test/pack.zip");
            assert_eq!(request["data"]["sha1"], "ab12");
            assert_eq!(request["data"]["prompt"], "Install pack?");
            assert_eq!(request["data"]["required"], true);
        });

        assert!(matches!(
            api.send_resource_pack(player, &ResourcePackSpec::new(1, "")),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.send_resource_pack(
                player,
                &ResourcePackSpec::new(1, "https://example.test/pack").sha1("x".repeat(41)),
            ),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.send_resource_pack(player, &ResourcePackSpec::new(1, "x".repeat(32_768)),),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"sent":1,"error":"pack_send_failed"}"#.to_vec()
        });
        let batch = [
            pack.clone(),
            ResourcePackSpec::new(0x5678, "https://example.test/second"),
        ];
        let outcome = api.send_resource_packs(player, &batch).unwrap();
        assert_eq!(outcome.sent, 1);
        assert!(!outcome.complete);
        assert_eq!(outcome.error.as_deref(), Some("pack_send_failed"));
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::RESOURCE_PACK_SEND_BATCH);
            assert_eq!(request["data"]["packs"].as_array().unwrap().len(), 2);
            assert_eq!(
                request["data"]["packs"][0]["id"],
                "00000000000000000000000000001234"
            );
        });
        assert!(matches!(
            api.send_resource_packs(player, &[]),
            Err(OperationError::InvalidArgument(_))
        ));
        let too_many = vec![ResourcePackSpec::new(1, "https://example.test/p"); 9];
        assert!(matches!(
            api.send_resource_packs(player, &too_many),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        assert!(api.remove_resource_pack(player, Some(0x1234)).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::RESOURCE_PACK_REMOVE);
            assert_eq!(request["data"]["id"], "00000000000000000000000000001234");
        });
        assert!(api.remove_resource_pack(player, None).unwrap());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::RESOURCE_PACK_REMOVE);
            assert_eq!(request["data"], serde_json::json!({}));
        });
    }

    #[test]
    fn online_player_helper_deserializes_snapshot_and_rejects_missing_player() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"uuid":"123456789abcdef00123456789abcdef","name":"Alex","dimension":"minecraft:overworld","position":[1.5,64.0,-2.5],"yaw":90.0,"pitch":0.0,"game_mode":1,"permission_level":2,"health":18.0}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = api.get_online_player(PlayerId::from_u128(7)).unwrap();
        assert_eq!(
            player.id.as_u128(),
            0x1234_5678_9abc_def0_0123_4567_89ab_cdef
        );
        assert_eq!(player.name, "Alex");
        assert_eq!(player.dimension, "minecraft:overworld");
        assert_eq!(player.position, [1.5, 64.0, -2.5]);
        assert_eq!(player.game_mode, 1);
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::PLAYER_GET
            );
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"player_not_online"}"#.to_vec()
        });
        assert!(matches!(
            api.get_online_player(PlayerId::from_u128(7)),
            Err(OperationError::Rejected(reason)) if reason == "player_not_online"
        ));
    }

    #[test]
    fn online_players_deserializes_all_snapshots_in_host_order() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"players":[{"uuid":"123456789abcdef00123456789abcdef","name":"Alex","dimension":"minecraft:overworld","position":[1.5,64.0,-2.5],"yaw":90.0,"pitch":0.0,"game_mode":1,"permission_level":2,"health":18.0},{"uuid":"00000000000000000000000000000002","name":"Sam","dimension":"minecraft:the_nether","position":[0.0,32.0,4.0],"yaw":-45.0,"pitch":10.0,"game_mode":0,"permission_level":0,"health":20.0}]}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let players = api.online_players().unwrap();

        assert_eq!(players.len(), 2);
        assert_eq!(
            players[0].id.as_u128(),
            0x1234_5678_9abc_def0_0123_4567_89ab_cdef
        );
        assert_eq!(players[0].name, "Alex");
        assert_eq!(players[0].position, [1.5, 64.0, -2.5]);
        assert_eq!(players[0].health, 18.0);
        assert_eq!(players[1].name, "Sam");
        assert_eq!(players[1].dimension, "minecraft:the_nether");

        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_ONLINE_LIST);
            assert_eq!(request["data"], serde_json::json!({}));
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"players":[]}"#.to_vec());
        assert!(api.online_players().unwrap().is_empty());

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"list_unavailable"}"#.to_vec()
        });
        assert!(matches!(
            api.online_players(),
            Err(OperationError::Rejected(reason)) if reason == "list_unavailable"
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"players":[{"name":"incomplete"}]}"#.to_vec()
        });
        assert!(matches!(
            api.online_players(),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn online_player_lookup_by_name_sends_query_and_requires_one_snapshot() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"players":[{"uuid":"00000000000000000000000000000003","name":"CaseSensitive","dimension":"minecraft:overworld","position":[0.0,64.0,0.0],"yaw":0.0,"pitch":0.0,"game_mode":0,"permission_level":0,"health":20.0}]}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = api.find_online_player_by_name("casesensitive").unwrap();
        assert_eq!(player.name, "CaseSensitive");
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_LOOKUP);
            assert_eq!(request["data"], serde_json::json!({"name":"casesensitive"}));
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"players":[]}"#.to_vec());
        assert!(matches!(
            api.find_online_player_by_name("missing"),
            Err(OperationError::InvalidResponse)
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"player_not_online"}"#.to_vec()
        });
        assert!(matches!(
            api.find_online_player_by_name("missing"),
            Err(OperationError::Rejected(reason)) if reason == "player_not_online"
        ));
    }

    #[test]
    fn online_player_lookup_by_id_sends_hex_uuid_and_decodes_one_snapshot() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"players":[{"uuid":"123456789abcdef00123456789abcdef","name":"Alex","dimension":"minecraft:overworld","position":[0.0,64.0,0.0],"yaw":0.0,"pitch":0.0,"game_mode":0,"permission_level":0,"health":20.0}]}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = api
            .find_online_player_by_id(PlayerId::from_u128(
                0x1234_5678_9abc_def0_0123_4567_89ab_cdef,
            ))
            .unwrap();
        assert_eq!(player.name, "Alex");
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_LOOKUP);
            assert_eq!(request["data"]["uuid"], "123456789abcdef00123456789abcdef");
        });
    }

    #[test]
    fn online_player_stat_reads_i32_and_checks_host_limits_and_errors() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":17}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.online_player_stat(PlayerId::from_u128(7), "custom", "player_kills")
                .unwrap(),
            17
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_STAT_GET);
            assert_eq!(request["data"]["type"], "custom");
            assert_eq!(request["data"]["key"], "player_kills");
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":0}"#.to_vec());
        assert_eq!(
            api.online_player_stat(PlayerId::from_u128(7), "custom", "never_seen")
                .unwrap(),
            0
        );
        assert!(matches!(
            api.online_player_stat(PlayerId::from_u128(7), &"x".repeat(129), "key"),
            Err(OperationError::InvalidArgument(_))
        ));
        assert_eq!(
            api.online_player_stat(PlayerId::from_u128(7), &"x".repeat(128), &"y".repeat(128))
                .unwrap(),
            0
        );
        assert!(matches!(
            api.online_player_stat(PlayerId::from_u128(7), "type", &"x".repeat(129)),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"player_not_online"}"#.to_vec()
        });
        assert!(matches!(
            api.online_player_stat(PlayerId::from_u128(7), "custom", "player_kills"),
            Err(OperationError::Rejected(reason)) if reason == "player_not_online"
        ));
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":1.5}"#.to_vec());
        assert!(matches!(
            api.online_player_stat(PlayerId::from_u128(7), "custom", "player_kills"),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn online_player_stats_decodes_vanilla_file_shape_and_host_errors() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"stats":{"stats":{"minecraft:custom":{"minecraft:jump":11,"player_kills":3}},"DataVersion":5000}}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(0x1234);
        let stats = api.online_player_stats(player).unwrap();
        assert_eq!(stats.data_version, 5000);
        assert_eq!(stats.stats["minecraft:custom"]["minecraft:jump"], 11);
        assert_eq!(stats.stats["minecraft:custom"]["player_kills"], 3);
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::PLAYER_STATS_GET);
            assert_eq!(request["data"], serde_json::json!({}));
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"player_not_online"}"#.to_vec()
        });
        assert!(matches!(
            api.online_player_stats(player),
            Err(OperationError::Rejected(reason)) if reason == "player_not_online"
        ));
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"stats":{"stats":{"minecraft:custom":{"minecraft:jump":"11"}},"DataVersion":5000}}"#.to_vec()
        });
        assert!(matches!(
            api.online_player_stats(player),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn server_maintenance_status_deserializes_and_reports_host_rejections() {
        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":true,"active":true,"online":3}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.server_maintenance_status().unwrap(),
            ServerMaintenanceStatus {
                active: true,
                online: 3,
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::SERVER_MAINTENANCE_STATUS);
            assert_eq!(request["data"], serde_json::json!({}));
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"server_unavailable"}"#.to_vec()
        });
        assert!(matches!(
            api.server_maintenance_status(),
            Err(OperationError::Rejected(reason)) if reason == "server_unavailable"
        ));
    }

    #[test]
    fn server_maintenance_begin_and_end_preserve_disruptive_host_semantics() {
        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":true,"online":2,"kicked":2}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.begin_server_maintenance(Some("Atualização")).unwrap(),
            MaintenanceBeginResult {
                online_after_kick_requests: 2,
                kicked: 2,
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::SERVER_MAINTENANCE_BEGIN);
            assert_eq!(request["data"]["message"], "Atualização");
        });
        assert!(matches!(
            api.begin_server_maintenance(Some("")),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"players_online","online":1}"#.to_vec()
        });
        assert_eq!(
            api.end_server_maintenance().unwrap(),
            MaintenanceEndResult::PlayersOnline(1)
        );
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"active":false}"#.to_vec());
        assert_eq!(
            api.end_server_maintenance().unwrap(),
            MaintenanceEndResult::Ended
        );
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"server_unavailable"}"#.to_vec()
        });
        assert!(matches!(
            api.end_server_maintenance(),
            Err(OperationError::Rejected(reason)) if reason == "server_unavailable"
        ));
    }

    #[test]
    fn world_dimensions_lists_saved_and_loaded_ids() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"dimensions":["minecraft:overworld","example:arena"],"loaded_dimensions":["example:arena","minecraft:overworld"]}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let dimensions = api.world_dimensions().unwrap();
        assert_eq!(
            dimensions.dimensions,
            ["minecraft:overworld", "example:arena"]
        );
        assert_eq!(
            dimensions.loaded_dimensions,
            ["example:arena", "minecraft:overworld"]
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::WORLD_DIMENSIONS_LIST
            );
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"dimension_catalog_unavailable"}"#.to_vec()
        });
        assert!(matches!(
            api.world_dimensions(),
            Err(OperationError::Rejected(reason)) if reason == "dimension_catalog_unavailable"
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"dimensions":[]}"#.to_vec());
        assert!(matches!(
            api.world_dimensions(),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn player_data_reset_requires_preview_and_commits_one_shot_plan() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"token":"reset-token","expires_in_seconds":60,"categories":["advancements","stats"],"affected_paths":["players/stats"],"backup_required":true,"requires_force":false}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert!(matches!(
            api.prepare_player_data_reset(&[]),
            Err(OperationError::InvalidArgument(_))
        ));
        let plan = api
            .prepare_player_data_reset(&[
                PlayerDataCategory::Stats,
                PlayerDataCategory::Advancements,
                PlayerDataCategory::Stats,
            ])
            .unwrap();
        assert_eq!(plan.expires_in_seconds(), 60);
        assert_eq!(
            plan.categories(),
            [PlayerDataCategory::Advancements, PlayerDataCategory::Stats]
        );
        assert_eq!(plan.affected_paths(), ["players/stats"]);
        assert!(plan.backup_required());
        assert!(!plan.requires_force());
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(
                request["operation"],
                operation::WORLD_PLAYER_DATA_RESET_PREPARE
            );
            assert_eq!(
                request["data"]["categories"],
                serde_json::json!(["advancements", "stats"])
            );
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"categories":["advancements","stats"],"backup":".horizon/backups/reset-token","removed_paths":["players/stats"]}"#.to_vec()
        });
        let result = api.commit_player_data_reset(plan, true).unwrap();
        assert_eq!(
            result,
            PlayerDataResetResult {
                categories: vec![PlayerDataCategory::Advancements, PlayerDataCategory::Stats],
                backup: Some(".horizon/backups/reset-token".into()),
                removed_paths: vec!["players/stats".into()],
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(
                request["operation"],
                operation::WORLD_PLAYER_DATA_RESET_COMMIT
            );
            assert_eq!(
                request["data"],
                serde_json::json!({"token":"reset-token","backup":true})
            );
        });
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"token":"partial-token","expires_in_seconds":60,"categories":["stats"],"affected_paths":["players/stats"],"backup_required":true,"requires_force":false}"#.to_vec()
        });
        let plan = api
            .prepare_player_data_reset(&[PlayerDataCategory::Stats])
            .unwrap();
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"partial_reset_failed","backup":".horizon/backups/partial","detail":"unlink failed"}"#.to_vec()
        });
        assert!(matches!(
            api.commit_player_data_reset(plan, true),
            Err(OperationError::Rejected(reason))
                if reason.contains("partial_reset_failed")
                    && reason.contains(".horizon/backups/partial")
                    && reason.contains("unlink failed")
        ));
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["data"]["backup"], true);
        });
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"token":"online-token","expires_in_seconds":60,"categories":["playerdata"],"affected_paths":["players/data"],"backup_required":false,"requires_force":true}"#.to_vec()
        });
        let plan = api
            .prepare_player_data_reset(&[PlayerDataCategory::PlayerData])
            .unwrap();
        assert!(plan.requires_force());
        assert!(!plan.backup_required());
    }

    #[test]
    fn create_custom_dimension_uses_typed_preset_and_checks_host_result() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"dimension":"example:arena","preset":"flat","loaded":false}"#
                    .to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.create_custom_dimension("example:arena", CustomDimensionPreset::Flat)
                .unwrap(),
            CustomDimensionCreateResult {
                dimension: "example:arena".into(),
                preset: CustomDimensionPreset::Flat,
                loaded: false,
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_DIMENSION_CREATE);
            assert_eq!(
                request["data"],
                serde_json::json!({"id":"example:arena","preset":"flat"})
            );
        });
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"dimension_exists"}"#.to_vec()
        });
        assert!(matches!(
            api.create_custom_dimension("example:arena", CustomDimensionPreset::Flat),
            Err(OperationError::Rejected(reason)) if reason == "dimension_exists"
        ));
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"dimension":"example:arena","preset":"flat"}"#.to_vec()
        });
        assert!(matches!(
            api.create_custom_dimension("example:arena", CustomDimensionPreset::Flat),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn world_dimension_load_unload_preserves_owner_ticket_contract() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"dimension":"example:arena","loaded":true,"pinned":true}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.load_dimension("example:arena", Some("plugin:arena_task"))
                .unwrap(),
            DimensionLoadResult {
                dimension: "example:arena".into(),
                loaded: true,
                pinned: true,
            }
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_DIMENSION_LOAD);
            assert_eq!(request["data"]["dimension"], "example:arena");
            assert_eq!(request["data"]["owner"], "plugin:arena_task");
        });
        assert!(matches!(
            api.load_dimension("example:arena", Some("invalid owner")),
            Err(OperationError::InvalidArgument(_))
        ));
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"loaded":true,"pinned":false}"#.to_vec()
        });
        assert!(matches!(
            api.load_dimension("example:arena", None),
            Err(OperationError::InvalidResponse)
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":false}"#.to_vec());
        assert!(
            !api.unload_dimension("example:arena", Some("plugin:arena_task"))
                .unwrap()
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_DIMENSION_UNLOAD);
            assert_eq!(request["data"]["owner"], "plugin:arena_task");
        });
    }

    #[test]
    fn find_safe_location_is_bounded_and_only_returns_loaded_chunk_results() {
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"location":{"dimension":"minecraft:overworld","position":[10.5,64.0,-20.5],"yaw":90.0,"pitch":5.0},"search_scope":"loaded_chunks"}"#.to_vec()
        });
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        let player = PlayerId::from_u128(9);
        let location = api
            .find_safe_location(player, 64, Some(24), Some([10, -20]))
            .unwrap()
            .unwrap();
        assert_eq!(location.dimension, "minecraft:overworld");
        assert_eq!(location.position, [10.5, 64.0, -20.5]);
        assert_eq!(location.yaw, 90.0);
        assert_eq!(location.pitch, 5.0);
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_LOCATION_FIND_SAFE);
            assert_eq!(
                request["data"],
                serde_json::json!({"radius":64,"attempts":24,"center":[10,-20]})
            );
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":false,"error":"no_safe_loaded_location","search_scope":"loaded_chunks"}"#
                    .to_vec()
        });
        assert_eq!(api.find_safe_location(player, 8, None, None).unwrap(), None);
        LAST_REQUEST.with(|last| {
            assert_eq!(
                last.borrow().as_ref().unwrap()["data"],
                serde_json::json!({"radius":8})
            );
        });

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"no_safe_loaded_location"}"#.to_vec()
        });
        assert!(matches!(
            api.find_safe_location(player, 8, None, None),
            Err(OperationError::InvalidResponse)
        ));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"world_read_failed"}"#.to_vec()
        });
        assert!(matches!(
            api.find_safe_location(player, 32, None, None),
            Err(OperationError::Rejected(reason)) if reason == "world_read_failed"
        ));
        assert!(matches!(
            api.find_safe_location(player, 7, None, None),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.find_safe_location(player, 32, Some(257), None),
            Err(OperationError::InvalidArgument(_))
        ));
    }

    #[test]
    fn world_gamerule_deserializes_bool_and_int_values() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.world_gamerule("minecraft:overworld", "doDaylightCycle")
                .unwrap(),
            GameRuleValue::Bool(true)
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_GAMERULE_GET);
            assert_eq!(
                request["data"],
                serde_json::json!({
                    "dimension":"minecraft:overworld",
                    "rule":"doDaylightCycle"
                })
            );
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":12}"#.to_vec());
        assert_eq!(
            api.world_gamerule("example:arena", "randomTickSpeed")
                .unwrap(),
            GameRuleValue::Int(12)
        );

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":false,"error":"unknown_or_unloaded_rule"}"#.to_vec()
        });
        assert!(matches!(
            api.world_gamerule("example:missing", "doDaylightCycle"),
            Err(OperationError::Rejected(reason)) if reason == "unknown_or_unloaded_rule"
        ));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":1.5}"#.to_vec());
        assert!(matches!(
            api.world_gamerule("minecraft:overworld", "randomTickSpeed"),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn custom_dimension_gamerule_reads_effective_rule_without_mutation() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":false}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert_eq!(
            api.custom_dimension_gamerule("example:arena", "doDaylightCycle")
                .unwrap(),
            GameRuleValue::Bool(false)
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_CUSTOM_GAMERULE_GET);
            assert_eq!(
                request["data"],
                serde_json::json!({
                    "dimension":"example:arena",
                    "rule":"doDaylightCycle"
                })
            );
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":8}"#.to_vec());
        assert_eq!(
            api.custom_dimension_gamerule("example:arena", "randomTickSpeed")
                .unwrap(),
            GameRuleValue::Int(8)
        );
        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":false,"error":"unknown_custom_dimension_or_rule"}"#.to_vec()
        });
        assert!(matches!(
            api.custom_dimension_gamerule("example:missing", "doDaylightCycle"),
            Err(OperationError::Rejected(reason)) if reason == "unknown_custom_dimension_or_rule"
        ));
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"value":"false"}"#.to_vec());
        assert!(matches!(
            api.custom_dimension_gamerule("example:arena", "doDaylightCycle"),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn gamerule_writers_keep_loaded_and_custom_dimension_contracts_separate() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        assert!(
            api.set_world_gamerule(
                "minecraft:overworld",
                "doDaylightCycle",
                GameRuleValue::Bool(false),
            )
            .unwrap()
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_GAMERULE_SET);
            assert_eq!(request["data"]["value"], false);
        });

        assert!(
            api.set_custom_dimension_gamerule(
                "example:arena",
                "randomTickSpeed",
                GameRuleValue::Int(12),
            )
            .unwrap()
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::WORLD_CUSTOM_GAMERULE_SET
            );
            assert_eq!(request.as_ref().unwrap()["data"]["value"], 12);
        });

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":false}"#.to_vec());
        assert!(
            !api.clear_custom_dimension_gamerule("example:arena", "randomTickSpeed")
                .unwrap()
        );
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::WORLD_CUSTOM_GAMERULE_CLEAR);
            assert_eq!(request["data"]["dimension"], "example:arena");
            assert_eq!(request["data"]["rule"], "randomTickSpeed");
        });
    }

    #[test]
    fn write_log_maps_levels_and_enforces_host_message_bounds() {
        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");
        api.write_log(ServerLogLevel::Warn, "plugin started")
            .unwrap();
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let request = request.as_ref().unwrap();
            assert_eq!(request["operation"], operation::SERVER_LOG);
            assert_eq!(
                request["data"],
                serde_json::json!({"level":"WARN","message":"plugin started"})
            );
        });
        api.write_log(ServerLogLevel::Info, &"x".repeat(2_048))
            .unwrap();
        assert!(matches!(
            api.write_log(ServerLogLevel::Error, ""),
            Err(OperationError::InvalidArgument(_))
        ));
        assert!(matches!(
            api.write_log(ServerLogLevel::Error, &"x".repeat(2_049)),
            Err(OperationError::InvalidArgument(_))
        ));

        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":false,"error":"invalid_level"}"#.to_vec());
        assert!(matches!(
            api.write_log(ServerLogLevel::Info, "message"),
            Err(OperationError::Rejected(reason)) if reason == "invalid_level"
        ));
    }

    #[test]
    fn storage_helpers_serialize_values_and_preserve_explicit_json_null() {
        let host = make_host(Some(respond_custom));
        let api = PluginApi::new(&host, "sdk_test");

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true}"#.to_vec());
        api.storage_set("coins", &serde_json::json!({"balance":12}))
            .unwrap();
        LAST_REQUEST.with(|last| {
            let request = last.borrow();
            let data = &request.as_ref().unwrap()["data"];
            assert_eq!(
                request.as_ref().unwrap()["operation"],
                operation::PLUGIN_STORAGE_SET
            );
            assert_eq!(data["plugin"], "sdk_test");
            assert_eq!(data["value"]["balance"], 12);
        });

        RESPONSE_BODY
            .with(|body| *body.borrow_mut() = br#"{"ok":true,"found":true,"value":null}"#.to_vec());
        let value: Option<serde_json::Value> = api.storage_get("nullable").unwrap();
        assert_eq!(value, Some(serde_json::Value::Null));

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() = br#"{"ok":true,"found":false,"value":null}"#.to_vec()
        });
        let missing: Option<serde_json::Value> = api.storage_get("missing").unwrap();
        assert_eq!(missing, None);

        RESPONSE_BODY.with(|body| {
            *body.borrow_mut() =
                br#"{"ok":true,"keys":["coins","coins2"],"next":"coins2"}"#.to_vec()
        });
        let page = api.storage_list(Some("coins"), 2, None).unwrap();
        assert_eq!(page.keys, ["coins", "coins2"]);
        assert_eq!(page.next.as_deref(), Some("coins2"));

        RESPONSE_BODY.with(|body| *body.borrow_mut() = br#"{"ok":true,"removed":true}"#.to_vec());
        assert!(api.storage_delete("coins").unwrap());
        assert!(matches!(
            api.storage_list(None, 0, None),
            Err(OperationError::InvalidArgument(_))
        ));
    }

    #[test]
    fn call_json_reports_missing_function_and_host_status() {
        let host = make_host(None);
        assert!(matches!(
            PluginApi::new(&host, "sdk_test").call(PlayerId::NONE, "x", &()),
            Err(CallError::FunctionUnavailable)
        ));
        let mut response = Vec::new();
        assert!(matches!(
            PluginApi::new(&host, "sdk_test").call_with_response_buffer(
                PlayerId::NONE,
                "x",
                &(),
                &mut response,
            ),
            Err(CallError::FunctionUnavailable)
        ));
        assert_eq!(response.capacity(), 0);

        let host = make_host(Some(respond_invalid_request));
        assert!(matches!(
            PluginApi::new(&host, "sdk_test").call(PlayerId::NONE, "x", &()),
            Err(CallError::HostStatus(result::INVALID_REQUEST))
        ));
    }

    #[test]
    fn call_json_rejects_oversized_request_before_calling_host() {
        let host = make_host(Some(respond_ok));
        let data = "x".repeat(MAX_JSON_BYTES);
        let error = PluginApi::new(&host, "sdk_test")
            .call(PlayerId::NONE, "x", &data)
            .unwrap_err();
        assert!(matches!(error, CallError::RequestTooLarge { .. }));
    }

    #[test]
    fn call_json_rejects_malformed_and_oversized_responses() {
        let host = make_host(Some(respond_bad_json));
        assert!(matches!(
            PluginApi::new(&host, "sdk_test").call(PlayerId::NONE, "x", &()),
            Err(CallError::InvalidResponse(_))
        ));

        let host = make_host(Some(respond_too_large));
        let error = PluginApi::new(&host, "sdk_test")
            .call(PlayerId::NONE, "x", &())
            .unwrap_err();
        match error {
            CallError::ResponseTooLarge { required, maximum } => {
                assert_eq!(required, MAX_JSON_BYTES + 1);
                assert_eq!(maximum, MAX_JSON_BYTES);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[derive(Default)]
    struct TestPlugin;

    impl Plugin for TestPlugin {
        const NAME: &'static str = "SDK test plugin";
        const ID: &'static str = "sdk_test";

        fn event(&mut self, _api: &PluginApi<'_>, kind: u32, _payload: &Value) -> bool {
            kind == event::SERVER_TASK
        }
    }

    crate::declare_plugin!(TestPlugin);

    #[test]
    fn exported_descriptor_has_all_required_v3_callbacks() {
        let descriptor = horizon_server_plugin_v3();
        let descriptor = unsafe { &*descriptor };
        assert_eq!(descriptor.abi_version, ABI_VERSION_V3);
        assert_eq!(descriptor.name_len, TestPlugin::NAME.len());
        assert!(descriptor.create.is_some());
        assert!(descriptor.destroy.is_some());
        assert!(descriptor.register_commands.is_some());
        assert!(descriptor.command.is_some());
        assert!(descriptor.menu_click.is_some());
        assert!(descriptor.event.is_some());
        assert!(descriptor.permission.is_some());
    }

    #[test]
    fn server_task_event_is_delivered_only_to_matching_plugin_id() {
        let host = make_host(None);
        let descriptor = descriptor::<TestPlugin>();
        let callback = descriptor.event.unwrap();
        let state = unsafe { create::<TestPlugin>() };

        for (owner, expected) in [("another_plugin", 0), ("sdk_test", 1)] {
            let payload = serde_json::json!({
                "data": {"plugin_id": owner, "task_id": "7", "data": null}
            })
            .to_string();
            let result = unsafe {
                callback(
                    state,
                    &host,
                    event::SERVER_TASK,
                    payload.as_ptr(),
                    payload.len(),
                )
            };
            assert_eq!(result, expected);
        }
        unsafe { destroy::<TestPlugin>(state) };
    }

    #[test]
    fn decode_server_task_event_checks_owner_and_decodes_payload() {
        let host = make_host(None);
        let api = PluginApi::new(&host, TestPlugin::ID);
        let payload = serde_json::json!({
            "data": {
                "plugin_id": "sdk_test",
                "task_id": "42",
                "data": {"job": "cleanup", "attempt": 2}
            }
        });
        let task = api.decode_server_task_event(&payload).unwrap().unwrap();
        assert_eq!(task.task_id.as_u64(), 42);
        assert_eq!(task.data["job"], "cleanup");
        assert_eq!(task.data["attempt"], 2);

        let numeric_id = serde_json::json!({
            "data": {"plugin_id": "sdk_test", "task_id": 9, "data": null}
        });
        assert_eq!(
            api.decode_server_task_event(&numeric_id)
                .unwrap()
                .unwrap()
                .task_id
                .as_u64(),
            9
        );

        let other_owner = serde_json::json!({
            "data": {"plugin_id": "other", "task_id": "5", "data": null}
        });
        assert_eq!(api.decode_server_task_event(&other_owner).unwrap(), None);

        let malformed = serde_json::json!({
            "data": {"plugin_id": "sdk_test", "task_id": "0", "data": null}
        });
        assert!(matches!(
            api.decode_server_task_event(&malformed),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn notification_event_decoder_matches_emitted_host_payloads() {
        let joined = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "dimension": "minecraft:overworld", "data": null
        });
        assert_eq!(
            decode_server_notification_event(event::PLAYER_JOIN, &joined).unwrap(),
            Some(ServerNotificationEvent::PlayerJoined {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                dimension: "minecraft:overworld".into()
            })
        );
        let quit = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"last_location": {
                "dimension": "minecraft:overworld", "position": [1.0, 64.0, -2.0],
                "yaw": 90.0, "pitch": 5.0
            }}
        });
        assert_eq!(
            decode_server_notification_event(event::PLAYER_QUIT, &quit).unwrap(),
            Some(ServerNotificationEvent::PlayerQuit {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                last_location: Some(PlayerEventLocation {
                    dimension: "minecraft:overworld".into(),
                    position: [1.0, 64.0, -2.0],
                    yaw: 90.0,
                    pitch: 5.0
                })
            })
        );
        let quit_without_snapshot = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"last_location": null}
        });
        assert_eq!(
            decode_server_notification_event(event::PLAYER_QUIT, &quit_without_snapshot).unwrap(),
            Some(ServerNotificationEvent::PlayerQuit {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                last_location: None
            })
        );
        let malformed_quit = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"last_location": {"dimension": "minecraft:overworld", "position": [1.0, 2.0]}}
        });
        assert!(matches!(
            decode_server_notification_event(event::PLAYER_QUIT, &malformed_quit),
            Err(OperationError::InvalidResponse)
        ));

        let death = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {
                "damage_type": "minecraft:player_attack", "attacker_entity_id": 91,
                "location": {
                    "dimension": "minecraft:overworld", "position": [1.0, 64.0, -2.0],
                    "yaw": 90.0, "pitch": 5.0
                }
            }
        });
        assert_eq!(
            decode_server_notification_event(event::PLAYER_DEATH, &death).unwrap(),
            Some(ServerNotificationEvent::PlayerDeathAttempt {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                damage_type: "minecraft:player_attack".into(),
                attacker_entity_id: 91,
                location: PlayerEventLocation {
                    dimension: "minecraft:overworld".into(),
                    position: [1.0, 64.0, -2.0],
                    yaw: 90.0,
                    pitch: 5.0,
                },
            })
        );
        assert!(
            decode_server_notification_event(event::PLAYER_DEATH, &death)
                .unwrap()
                .unwrap()
                .is_cancellable()
        );
        let malformed_death = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"damage_type": "minecraft:player_attack", "attacker_entity_id": -1}
        });
        assert!(matches!(
            decode_server_notification_event(event::PLAYER_DEATH, &malformed_death),
            Err(OperationError::InvalidResponse)
        ));

        let block_event = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "dimension": "minecraft:overworld", "position": [-1, 64, 3],
            "data_id": 987, "data": null
        });
        assert_eq!(
            decode_server_notification_event(event::BLOCK_BREAK, &block_event).unwrap(),
            Some(ServerNotificationEvent::BlockBreakAttempt {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                dimension: "minecraft:overworld".into(),
                position: [-1, 64, 3],
                block_state_id: 987
            })
        );
        assert_eq!(
            decode_server_notification_event(event::BLOCK_PLACE, &block_event).unwrap(),
            Some(ServerNotificationEvent::BlockPlaced {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                dimension: "minecraft:overworld".into(),
                position: [-1, 64, 3],
                block_state_id: 987
            })
        );
        assert!(
            decode_server_notification_event(event::BLOCK_BREAK, &block_event)
                .unwrap()
                .unwrap()
                .is_cancellable()
        );
        assert!(
            !decode_server_notification_event(event::BLOCK_PLACE, &block_event)
                .unwrap()
                .unwrap()
                .is_cancellable()
        );

        let menu_closed = serde_json::json!({
            "player": "0000000000000000000000000000002a",
            "name": "", "dimension": "minecraft:overworld", "window_id": 17,
            "data": null
        });
        assert_eq!(
            decode_server_notification_event(event::MENU_CLOSED, &menu_closed).unwrap(),
            Some(ServerNotificationEvent::MenuClosed {
                player: PlayerId::from_u128(42),
                dimension: "minecraft:overworld".into(),
                window_id: 17,
            })
        );

        let chat = serde_json::json!({
            "player": "0000000000000000000000000000002a",
            "name": "Ada", "data": {"message": "hello", "phase": "after_broadcast"}
        });
        assert_eq!(
            decode_server_notification_event(event::CHAT, &chat).unwrap(),
            Some(ServerNotificationEvent::ChatAfterBroadcast {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                message: "hello".into()
            })
        );

        let changed = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"from": "minecraft:overworld", "to": "borizon:moon"}
        });
        assert_eq!(
            decode_server_notification_event(event::PLAYER_CHANGED_DIMENSION, &changed).unwrap(),
            Some(ServerNotificationEvent::DimensionChanged {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                from: "minecraft:overworld".into(),
                to: "borizon:moon".into()
            })
        );

        let teleport = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"dimension": "minecraft:overworld", "from": [0.0, 1.0, 2.0], "to": [3.5, 4.0, 5.0]}
        });
        assert_eq!(
            decode_server_notification_event(event::PLAYER_TELEPORT, &teleport).unwrap(),
            Some(ServerNotificationEvent::Teleported {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                dimension: "minecraft:overworld".into(),
                from: [0.0, 1.0, 2.0],
                to: [3.5, 4.0, 5.0]
            })
        );

        let pack_status = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"pack_id": "00112233445566778899aabbccddeeff", "status": 3}
        });
        assert_eq!(
            decode_server_notification_event(event::RESOURCE_PACK_STATUS, &pack_status).unwrap(),
            Some(ServerNotificationEvent::ResourcePackStatus {
                player: PlayerId::from_u128(42),
                name: "Ada".into(),
                pack_id: "00112233445566778899aabbccddeeff".into(),
                status: 3
            })
        );
        let future_pack_status = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"pack_id": "00112233445566778899aabbccddeeff", "status": 99}
        });
        assert!(matches!(
            decode_server_notification_event(event::RESOURCE_PACK_STATUS, &future_pack_status),
            Ok(Some(ServerNotificationEvent::ResourcePackStatus {
                status: 99,
                ..
            }))
        ));

        assert_eq!(
            decode_server_notification_event(event::PLAYER_MOVE, &Value::Null).unwrap(),
            None
        );
        let invalid = serde_json::json!({
            "player": "not-a-uuid", "name": "Ada",
            "data": {"message": "hello", "phase": "before_broadcast"}
        });
        assert!(matches!(
            decode_server_notification_event(event::CHAT, &invalid),
            Err(OperationError::InvalidResponse)
        ));
        let short_uuid = serde_json::json!({
            "player": "2a", "name": "Ada",
            "data": {"message": "hello", "phase": "after_broadcast"}
        });
        assert!(matches!(
            decode_server_notification_event(event::CHAT, &short_uuid),
            Err(OperationError::InvalidResponse)
        ));
        let missing_field = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"message": "hello"}
        });
        assert!(matches!(
            decode_server_notification_event(event::CHAT, &missing_field),
            Err(OperationError::InvalidResponse)
        ));
        let invalid_window = serde_json::json!({
            "player": "0000000000000000000000000000002a",
            "name": "", "dimension": "minecraft:overworld", "window_id": 2147483648_i64,
            "data": null
        });
        assert!(matches!(
            decode_server_notification_event(event::MENU_CLOSED, &invalid_window),
            Err(OperationError::InvalidResponse)
        ));
        let bad_block = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "dimension": "minecraft:overworld", "position": [1, 2],
            "data_id": -1, "data": null
        });
        assert!(matches!(
            decode_server_notification_event(event::BLOCK_BREAK, &bad_block),
            Err(OperationError::InvalidResponse)
        ));
        let bad_teleport = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"dimension": "minecraft:overworld", "from": [0.0, 1.0], "to": [3.0, 4.0, 5.0]}
        });
        assert!(matches!(
            decode_server_notification_event(event::PLAYER_TELEPORT, &bad_teleport),
            Err(OperationError::InvalidResponse)
        ));
        let bad_pack_status = serde_json::json!({
            "player": "0000000000000000000000000000002a", "name": "Ada",
            "data": {"pack_id": "not-a-uuid", "status": 3}
        });
        assert!(matches!(
            decode_server_notification_event(event::RESOURCE_PACK_STATUS, &bad_pack_status),
            Err(OperationError::InvalidResponse)
        ));
    }

    #[test]
    fn resource_pack_status_codes_round_trip_known_values_and_preserve_unknowns() {
        let statuses = [
            ResourcePackStatusCode::SuccessfullyLoaded,
            ResourcePackStatusCode::Declined,
            ResourcePackStatusCode::FailedDownload,
            ResourcePackStatusCode::Accepted,
            ResourcePackStatusCode::Downloaded,
            ResourcePackStatusCode::InvalidUrl,
            ResourcePackStatusCode::FailedReload,
            ResourcePackStatusCode::Discarded,
        ];
        for (raw, status) in statuses.into_iter().enumerate() {
            assert_eq!(ResourcePackStatusCode::from_raw(raw as i32), Some(status));
            assert_eq!(status.as_raw(), raw as i32);
        }
        assert_eq!(ResourcePackStatusCode::from_raw(-1), None);
        assert_eq!(ResourcePackStatusCode::from_raw(8), None);
    }
}

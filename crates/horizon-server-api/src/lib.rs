//! Shared server command and player data used by the dedicated server and its extension bridges.
//! The legacy client trait API re-exports these types for source compatibility.
use horizon_client_core::text::Text;

/// One player's sidebar sent by the server runtime.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerSidebar {
    pub title: Text,
    pub lines: Vec<(Text, Option<Text>)>,
}
/// A player on the server, as held by the server.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ServerPlayer {
    pub uuid: u128,
    pub name: String,
    /// The operator level, 0 to 4 (`ops.json`; single player's owner with
    /// cheats on is 4).
    pub permission: u8,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub dimension: String,
    pub position: [f64; 3],
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Health points, 20 at full; 0 once dead.
    pub health: f32,
    /// Where they look, in degrees (`yRot`, `xRot`), as their last move told
    /// the server.
    pub yaw: f32,
    pub pitch: f32,
    /// The network entity id the player has this session, the one hooks that only know entity ids
    /// (`living_incoming_damage`, `living_knockback`) give for the player; 0 when not known.
    pub entity_id: i32,
    /// The IP address the player connects from (behind a Velocity proxy: the address the proxy forwards);
    /// empty when not known.
    pub address: String,
    /// The connection's round-trip time in milliseconds, as the keep-alive measured it and the
    /// player list shows it (`ServerCommonPacketListenerImpl.latency`). The fields below are
    /// 0, false or None where the server does not hold them.
    pub latency: i32,
    /// The protocol version the client connected with (the server's own: others are refused).
    pub protocol: i32,
    /// The experience level and the progress to the next one, 0 to 1.
    pub xp_level: i32,
    pub xp_progress: f32,
    /// On the ground, as the player's last accepted move said.
    pub on_ground: bool,
    /// The player's abilities as the client is told them (`Abilities.flying`, `mayfly`,
    /// `invulnerable`, `walkingSpeed`, `flyingSpeed`): speeds 0.1 and 0.05 by default.
    pub flying: bool,
    pub may_fly: bool,
    pub invulnerable: bool,
    pub walk_speed: f32,
    pub fly_speed: f32,
    /// Holding sneak (`Player.isShiftKeyDown`).
    pub sneaking: bool,
    /// The eyes' height above the feet in the current pose (`Entity.getEyeHeight`): 1.62
    /// standing, 1.27 crouching, 0.4 swimming or gliding, 0.2 asleep.
    pub eye_height: f32,
    /// The selected hotbar slot, 0 to 8.
    pub selected_slot: u8,
    /// The name a server extension shows for the player, if one is set.
    pub nickname: Option<String>,
    /// The permission level that can see this player.
    pub hidden: Option<u8>,
    /// A fake player used by a server-side bot with no client.
    pub fake: bool,
    /// Whether the player is using an item (`LivingEntity.isUsingItem`), and blocking with a
    /// shield (using one).
    pub using_item: bool,
    pub blocking: bool,
    /// Sprinting (`Player.isSprinting`) and gliding with an elytra (`isFallFlying`).
    /// Server-side movement state.
    pub sprinting: bool,
    pub gliding: bool,
}

/// A command registered by a server extension, as one usage: words, then arguments
/// `<name:kind>`, optional ones last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`,
/// `item`, `block_pos`; and the kinds whose completions the player's client
/// asks the server for: `word?`, `text?` and
/// `player_name` (a word, any name accepted, that the server completes with
/// the online players' names). Usages of one command share their words; the
/// server merges them into one tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerCommand {
    pub usage: String,
    /// The operator level a player needs to see and run it (0: everyone).
    pub level: u8,
    /// The permission node that decides access instead of the operator level,
    /// when set ([`ServerCommand::permission`]).
    pub permission: Option<String>,
    /// Whether the usage replaces vanilla's command of its name
    /// ([`ServerCommand::replace_vanilla`]).
    pub replace_vanilla: bool,
}

impl ServerCommand {
    /// A usage every player may run.
    pub fn new(usage: &str) -> Self {
        Self {
            usage: usage.to_owned(),
            level: 0,
            permission: None,
            replace_vanilla: false,
        }
    }
    /// The same, for operators of `level` and above.
    pub fn op(usage: &str, level: u8) -> Self {
        Self {
            usage: usage.to_owned(),
            level: level.min(4),
            permission: None,
            replace_vanilla: false,
        }
    }
    /// The same, for players granted the permission node `node`: the server
    /// checks it before showing or running the usage. Each
    /// usage has its own node: a command's root is shown when any of its
    /// usages is allowed, and a line runs as the usage its words match.
    pub fn permission(mut self, node: &str) -> Self {
        self.permission = Some(node.to_owned());
        self
    }
    /// The same, replacing vanilla's command of this name (Bukkit's plugin
    /// command over a vanilla one, as Tab's `/bossbar`): vanilla's tree of
    /// the name is left out of the players' trees, and the extension receives
    /// matching commands. Without it, a usage of a vanilla name is left out.
    pub fn replace_vanilla(mut self, on: bool) -> Self {
        self.replace_vanilla = on;
        self
    }
}

/// What a command run did: whether it
/// ran without failing, its result and the lines it answered with, in order.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ServerCommandResult {
    pub ran: bool,
    /// Vanilla's `CommandResultCallback` value: the sum of the successful
    /// results (one for a plain command, one per fork of `execute as @a run`),
    /// 0 when it failed (`execute if entity` of nobody). An extension command
    /// reports 1 when it ran.
    pub result: i32,
    pub output: Vec<Text>,
}

/// A command of the server: vanilla's or one a server extension registered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerCommandInfo {
    /// Its own name, an alias resolved (`teleport` for `tp`).
    pub name: String,
    /// The other names that run it (`tp` for `teleport`).
    pub aliases: Vec<String>,
    /// Its usages: vanilla's one line, as `/help` gives it, or each usage a
    /// extension registered.
    pub usages: Vec<String>,
    /// The operator level a player needs to run it (0: everyone).
    pub level: u8,
    /// Whether vanilla has it; false for an extension command.
    pub vanilla: bool,
}

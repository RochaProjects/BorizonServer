//! Sidebar configurável do AuraEssentials, atualizada em uma tarefa periódica do servidor.

use crate::plugin_api::call_json;
use horizon_server_plugin_api::{HostApiV3, event, operation};
use serde::Deserialize;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU8, Ordering},
};

const PLUGIN_ID: &str = "AuraEssentials";
const CONFIG_FILE: &str = "scoreboard.json";
const TASK_KIND: &str = "scoreboard_refresh";
static LAST_MODE: AtomicU8 = AtomicU8::new(0);
const DEFAULT_CONFIG: &str = r#"{
  "enabled": true,
  "refresh_ticks": 40,
  "title": "%rank_glyph%",
  "currency": { "money": "aura:coins", "credits": "aura:credits" },
  "glyphs": {
    "dono": "\uE800", "admin": "\uE801", "membro": "\uE802", "vip": "\uE803",
    "diamante": "\uE804", "ferro": "\uE805", "ouro": "\uE806", "pedra": "\uE807",
    "novato": "\uE808", "fim": "\uE809", "nether": "\uE80A", "espaco": "\uE80B",
    "fogo": "\uE80C", "agua": "\uE80D", "ar": "\uE80E", "twitch": "\uE80F",
    "youtube": "\uE810", "atirador": "\uE811", "comando": "\uE812", "construtor": "\uE813",
    "lendas": "\uE814", "discord": "\uE815", "ajudante": "\uE816", "reino": "\uE817",
    "morto": "\uE818", "hacker": "\uE819", "designer": "\uE81A", "abate": "\uE81B",
    "pixel": "\uE81C", "dev": "\uE81D"
  },
  "lines": [
    "&c🗡 &fᴋɪʟʟs: &c%kills_short%",
    "&c☠ &fᴍᴏʀᴛᴇѕ: &c%deaths_short%",
    "&a&l$ &fѕᴀʟᴅᴏ: &a%saldo_short%",
    "&r★ ᴄʀᴇᴅɪᴛᴏ: &6%credito_short%",
    "&f♦ &fᴏɴʟɪɴᴇ: &a%online_short%"
  ]
}"#;

#[derive(Deserialize)]
struct Config {
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default = "refresh_ticks")]
    refresh_ticks: u64,
    #[serde(default = "default_title")]
    title: String,
    #[serde(default)]
    currency: Currency,
    #[serde(default)]
    glyphs: std::collections::HashMap<String, String>,
    #[serde(default)]
    lines: Vec<String>,
}

#[derive(Deserialize)]
struct Currency {
    #[serde(default = "money_currency")]
    money: String,
    #[serde(default = "credits_currency")]
    credits: String,
}

impl Default for Currency {
    fn default() -> Self {
        Self {
            money: money_currency(),
            credits: credits_currency(),
        }
    }
}

fn yes() -> bool {
    true
}
fn refresh_ticks() -> u64 {
    40
}
fn default_title() -> String {
    "%rank_glyph%".into()
}
fn money_currency() -> String {
    "aura:coins".into()
}
fn credits_currency() -> String {
    "aura:credits".into()
}

fn config_path() -> Option<PathBuf> {
    Some(
        std::env::current_dir()
            .ok()?
            .join("plugins")
            .join(PLUGIN_ID)
            .join(CONFIG_FILE),
    )
}

fn ensure_config() {
    let Some(path) = config_path() else { return };
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        if let Err(error) =
            fs::create_dir_all(parent).and_then(|()| fs::write(&path, DEFAULT_CONFIG))
        {
            eprintln!("[AuraEssentials] Não foi possível criar {CONFIG_FILE}: {error}");
        }
    }
}

fn read_config() -> Option<Config> {
    ensure_config();
    let bytes = fs::read(config_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn start(host: &HostApiV3) {
    ensure_config();
    let period = read_config().map_or(40, |config| config.refresh_ticks.clamp(20, 1200));
    let _ = call_json(
        host,
        0,
        serde_json::json!({
            "operation": operation::SCHEDULER_SCHEDULE,
            "data": {
                "plugin": PLUGIN_ID,
                "delay_ticks": period,
                "period_ticks": period,
                "data": {"kind": TASK_KIND}
            }
        }),
    );
}

pub(super) fn event(host: &HostApiV3, kind: u32, payload: &serde_json::Value) {
    if kind != event::SERVER_TASK {
        return;
    }
    let task = &payload["data"]["data"];
    if payload["data"]["plugin_id"] != PLUGIN_ID || task["kind"] != TASK_KIND {
        return;
    }
    let Some(config) = read_config() else { return };
    let Some(online) = call_json(
        host,
        0,
        serde_json::json!({
            "operation": operation::PLAYER_ONLINE_LIST, "data": {}
        }),
    )
    .filter(|value| value["ok"] == true)
    .and_then(|value| value["players"].as_array().cloned()) else {
        return;
    };
    let mode = u8::from(config.enabled) + 1;
    let previous_mode = LAST_MODE.swap(mode, Ordering::Relaxed);
    if !config.enabled && previous_mode == mode {
        return;
    }
    for player in &online {
        let Some(uuid) = player["uuid"]
            .as_str()
            .and_then(|value| u128::from_str_radix(value, 16).ok())
        else {
            continue;
        };
        if !config.enabled {
            let _ = call_json(
                host,
                uuid,
                serde_json::json!({
                    "operation": operation::SCOREBOARD_SIDEBAR_CLEAR, "data": {}
                }),
            );
            continue;
        }
        render_player(host, uuid, &player, &config, online.len());
    }
}

fn render_player(
    host: &HostApiV3,
    player: u128,
    snapshot: &serde_json::Value,
    config: &Config,
    online: usize,
) {
    let kills = stat(host, player, "player_kills");
    let deaths = stat(host, player, "deaths");
    let playtime = stat(host, player, "play_time") / 20;
    let money = balance(host, player, &config.currency.money);
    let credits = balance(host, player, &config.currency.credits);
    let glyph = |key: &str, fallback: &str| {
        config
            .glyphs
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.to_owned())
    };
    let is_admin = snapshot["permission_level"].as_u64().unwrap_or(0) >= 2;
    let rank = if is_admin {
        glyph("admin", "\u{e801}")
    } else {
        glyph("membro", "\u{e802}")
    };
    let values = std::collections::HashMap::from([
        ("player", snapshot["name"].as_str().unwrap_or("").to_owned()),
        (
            "world",
            snapshot["dimension"].as_str().unwrap_or("").to_owned(),
        ),
        ("x", coordinate(snapshot, 0)),
        ("y", coordinate(snapshot, 1)),
        ("z", coordinate(snapshot, 2)),
        ("kills", kills.to_string()),
        ("kills_short", short(kills)),
        ("kill", kills.to_string()),
        ("kill_short", short(kills)),
        ("deaths", deaths.to_string()),
        ("deaths_short", short(deaths)),
        ("death", deaths.to_string()),
        ("death_short", short(deaths)),
        ("mortes", deaths.to_string()),
        ("mortes_short", short(deaths)),
        ("online", online.to_string()),
        ("online_short", short(online as i32)),
        ("playtime", format_playtime(playtime)),
        ("playtime_short", format_playtime(playtime)),
        ("saldo", money.clone()),
        ("money", money.clone()),
        ("saldo_short", money),
        ("credito", credits.clone()),
        ("credit", credits.clone()),
        ("credito_short", credits.clone()),
        ("credit_short", credits),
        ("rank_glyph", rank.clone()),
        ("cargo_glyph", rank.clone()),
        ("vip_glyph", rank.clone()),
        ("cargo", rank.clone()),
        ("vip", rank.clone()),
        (
            "rank_label",
            if is_admin {
                "Admin".into()
            } else {
                "Membro".into()
            },
        ),
        (
            "cargo_label",
            if is_admin {
                "Admin".into()
            } else {
                "Membro".into()
            },
        ),
        (
            "vip_label",
            if is_admin {
                "Admin".into()
            } else {
                "Membro".into()
            },
        ),
        ("money_glyph", glyph("ouro", "\u{e806}")),
        ("clan_glyph", glyph("reino", "\u{e817}")),
        ("clan", "".into()),
        ("cla", "".into()),
        ("clan_short", "".into()),
        ("clan_tag", "".into()),
        ("clan_name", "".into()),
    ]);
    let replace = |input: &str| {
        let mut output = replace_placeholders(input, &values);
        for (name, value) in &config.glyphs {
            output = output.replace(&format!("%glyph_{name}%"), value);
        }
        output
    };
    let title = replace(&config.title);
    let lines: Vec<_> = config
        .lines
        .iter()
        .take(15)
        .map(|line| {
            serde_json::json!({
                "left": legacy_colors(&replace(line)),
            })
        })
        .collect();
    let _ = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::SCOREBOARD_SIDEBAR_SET,
            "data": {"title": legacy_colors(&title), "lines": lines}
        }),
    );
}

fn stat(host: &HostApiV3, player: u128, key: &str) -> i32 {
    call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::PLAYER_STAT_GET,
            "data": {"type": "custom", "key": key}
        }),
    )
    .and_then(|value| value["value"].as_i64())
    .unwrap_or(0)
    .clamp(0, i64::from(i32::MAX)) as i32
}

fn balance(host: &HostApiV3, player: u128, currency: &str) -> String {
    let Some(get) = host.common.get_balance else {
        return "0".into();
    };
    let (high, low) = ((player >> 64) as u64, player as u64);
    let mut value = 0_i64;
    // SAFETY: the currency bytes and output stay alive for the synchronous API call.
    let status = unsafe {
        get(
            host.common.context,
            high,
            low,
            currency.as_ptr(),
            currency.len(),
            &mut value,
        )
    };
    if status != horizon_server_plugin_api::result::OK {
        return "0".into();
    }
    let negative = value < 0;
    let whole = value.unsigned_abs() / 100;
    let cents = value.unsigned_abs() % 100;
    let sign = if negative { "-" } else { "" };
    if cents == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole},{cents:02}")
            .trim_end_matches('0')
            .trim_end_matches(',')
            .to_owned()
    }
}

fn coordinate(snapshot: &serde_json::Value, index: usize) -> String {
    snapshot["position"]
        .get(index)
        .and_then(serde_json::Value::as_f64)
        .map(|value| value.floor() as i64)
        .unwrap_or_default()
        .to_string()
}

fn short(value: i32) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 10_000 {
        format!("{}k", value / 1_000)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn format_playtime(seconds: i32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn replace_placeholders(input: &str, values: &std::collections::HashMap<&str, String>) -> String {
    let mut output = input.to_owned();
    for (key, value) in values {
        output = output.replace(&format!("%{key}%"), value);
    }
    output
}

fn legacy_colors(input: &str) -> String {
    input.replace('&', "§")
}

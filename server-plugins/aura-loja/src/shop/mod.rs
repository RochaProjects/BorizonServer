//! Loja em português baseada na lista de itens e no fluxo de telas do AuraEssentials Java.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

const NAME: &[u8] = b"AuraEssentials";
const DEFAULT_ITEMS_JSON: &str = include_str!("../../shop.json");
const DEFAULT_CONFIG_JSON: &str = r#"{
  "title": "Loja",
  "currency": {
    "normal": "aura:coins",
    "vip": "aura:credits"
  }
}"#;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShopItem {
    display_name: String,
    material: String,
    icon: Option<String>,
    item_model: Option<String>,
    components: Option<HashMap<String, serde_json::Value>>,
    lore: Option<Vec<String>>,
    #[serde(default = "one")]
    amount: u32,
    buy: f64,
    sell: f64,
}

#[derive(Clone, Deserialize, Serialize)]
struct ShopConfig {
    #[serde(default = "default_title")]
    title: String,
    #[serde(default)]
    currency: CurrencyConfig,
}

#[derive(Clone, Deserialize, Serialize)]
struct CurrencyConfig {
    #[serde(default = "default_coins")]
    normal: String,
    #[serde(default = "default_credits")]
    vip: String,
}

impl Default for CurrencyConfig {
    fn default() -> Self {
        Self {
            normal: default_coins(),
            vip: default_credits(),
        }
    }
}

fn default_title() -> String {
    "Loja".into()
}
fn default_coins() -> String {
    "aura:coins".into()
}
fn default_credits() -> String {
    "aura:credits".into()
}

fn one() -> u32 {
    1
}

#[derive(Clone)]
struct Category {
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    items: Vec<ShopItem>,
}

pub(crate) struct State {
    loaded: bool,
    config: ShopConfig,
    categories: Vec<Category>,
    pub(crate) views: HashMap<(u128, i32), View>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            loaded: false,
            config: ShopConfig {
                title: default_title(),
                currency: CurrencyConfig::default(),
            },
            categories: Vec::new(),
            views: HashMap::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) enum View {
    Categories(HashMap<i32, usize>),
    Warps {
        page: usize,
        pages: usize,
        slots: HashMap<i32, String>,
    },
    Items {
        category: usize,
        page: usize,
        rows: u8,
        slots: HashMap<i32, usize>,
    },
    Detail {
        category: usize,
        page: usize,
        item: usize,
        quantity: u32,
    },
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

pub(crate) fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

mod callbacks;
mod config;
mod economy;
mod menus;
mod ui;

pub use callbacks::horizon_server_plugin_v3;

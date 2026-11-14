use crate::plugin_api::call_json;
use horizon_server_plugin_api::{HostApiV3, operation};
use std::collections::HashMap;

pub(super) fn bytes(s: &str) -> (*const u8, usize) {
    (s.as_ptr(), s.len())
}

pub(super) struct ItemSpec {
    pub(super) slot: usize,
    pub(super) item: String,
    pub(super) name: String,
    pub(super) color: String,
    pub(super) lore: Vec<String>,
    pub(super) components: HashMap<String, serde_json::Value>,
    pub(super) count: u32,
}

pub(super) fn spec(slot: usize, item: &str, name: &str, color: &str, count: u32) -> ItemSpec {
    ItemSpec {
        slot,
        item: item.into(),
        name: name.into(),
        color: color.into(),
        lore: Vec::new(),
        components: HashMap::new(),
        count,
    }
}

pub(super) fn spec_lore(mut item: ItemSpec, lore: Vec<String>) -> ItemSpec {
    item.lore = lore.into_iter().map(legacy_colors).collect();
    item
}

/// Accept the `&` color notation commonly used in editable shop JSON, alongside vanilla's `§`.
pub(super) fn legacy_colors(text: String) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '&'
            && let Some(code) = chars.clone().next()
            && "0123456789abcdefklmnor".contains(code.to_ascii_lowercase())
        {
            output.push('§');
            output.push(chars.next().expect("peeked character"));
        } else {
            output.push(ch);
        }
    }
    output
}

pub(super) fn show_menu(
    host: &HostApiV3,
    player: u128,
    title: &str,
    rows: u8,
    items: Vec<ItemSpec>,
) -> Option<i32> {
    let menu_items: Vec<_> = items
        .iter()
        .map(|item| {
            serde_json::json!({
                "slot": item.slot,
                "item": item.item,
                "count": item.count,
                "name": {"text": item.name, "color": item.color, "italic": false},
                "lore": item.lore,
                "components": item.components,
            })
        })
        .collect();
    let result = call_json(
        host,
        player,
        serde_json::json!({
            "operation": operation::MENU_OPEN,
            "data": {"title": title, "rows": rows, "items": menu_items},
        }),
    )?;
    result
        .get("window_id")?
        .as_i64()
        .and_then(|id| i32::try_from(id).ok())
}

pub(super) use crate::plugin_api::message;

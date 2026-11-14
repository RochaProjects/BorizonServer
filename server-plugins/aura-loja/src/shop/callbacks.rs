use super::{
    NAME, View,
    config::{ensure_loaded, load_config, load_shop},
    economy::{trade_message, transact_shop_item},
    menus::{open_categories, open_detail, open_items},
    state,
};
use crate::{
    essentials,
    plugin_api::{call_json, message},
};
use horizon_server_plugin_api::{ABI_VERSION_V3, HostApiV3, PluginV3, event, operation};
use std::ffi::c_void;

unsafe extern "C-unwind" fn create() -> *mut c_void {
    let mut plugin_state = state().lock().unwrap_or_else(|error| error.into_inner());
    ensure_loaded(&mut plugin_state);
    Box::into_raw(Box::new(())) as *mut c_void
}
unsafe extern "C-unwind" fn destroy(state: *mut c_void) {
    if !state.is_null() {
        unsafe { drop(Box::from_raw(state.cast::<()>())) };
    }
}

unsafe extern "C-unwind" fn register_commands(_: *mut c_void, host: *const HostApiV3) {
    if host.is_null() {
        return;
    }
    let host = unsafe { &*host };
    essentials::register_commands(host);
    let _ = call_json(
        host,
        0,
        serde_json::json!({
            "operation": operation::COMMAND_REGISTER,
            "data": {"usage": "loja", "default_level": 0},
        }),
    );
    let _ = call_json(
        host,
        0,
        serde_json::json!({
            "operation": operation::COMMAND_REGISTER,
            "data": {
                "usage": "loja recarregar",
                "default_level": 2,
                "permission": "auraessentials.loja.admin",
            },
        }),
    );
}

unsafe extern "C-unwind" fn command(
    _: *mut c_void,
    host: *const HostApiV3,
    high: u64,
    low: u64,
    line: *const u8,
    len: usize,
) -> u8 {
    if host.is_null() || line.is_null() || len > 512 {
        return 0;
    }
    let line = unsafe { std::slice::from_raw_parts(line, len) };
    let Ok(line) = std::str::from_utf8(line) else {
        return 0;
    };
    let line = line.trim().trim_start_matches('/');
    let host = unsafe { &*host };
    let player = (u128::from(high) << 64) | u128::from(low);
    if essentials::command(host, player, line) {
        return 1;
    }
    if !matches!(line, "loja" | "loja recarregar") {
        return 0;
    }
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    if line == "loja recarregar" {
        // Permission is enforced by Horizon's registered operator-level command.
        match (load_shop(), load_config()) {
            (Ok(categories), Ok(config)) => {
                state.categories = categories;
                state.config = config;
                state.loaded = true;
                state.views.clear();
                message(host, player, "Configuração e itens da loja recarregados.");
            }
            (Err(error), _) | (_, Err(error)) => {
                message(
                    host,
                    player,
                    &format!("Não foi possível recarregar a loja: {error}"),
                );
            }
        }
    } else {
        ensure_loaded(&mut state);
        open_categories(&mut state, host, player);
    }
    1
}

unsafe extern "C-unwind" fn menu_click(
    _: *mut c_void,
    host: *const HostApiV3,
    high: u64,
    low: u64,
    window: i32,
    slot: i32,
    _button: i32,
    mode: i32,
) {
    if host.is_null() || !matches!(mode, 0 | 1) {
        return;
    }
    let host = unsafe { &*host };
    let player = (u128::from(high) << 64) | u128::from(low);
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(view) = state.views.remove(&(player, window)) else {
        return;
    };
    match view {
        View::Categories(mapping) => {
            if let Some(category) = mapping.get(&slot).copied() {
                open_items(&mut state, host, player, category, 0);
            } else {
                state
                    .views
                    .insert((player, window), View::Categories(mapping));
            }
        }
        View::Items {
            category,
            page,
            rows,
            slots,
        } => {
            let nav = usize::from(rows) * 9 - 9;
            if slot == (nav + 4) as i32 {
                open_categories(&mut state, host, player);
            } else if slot == (nav + 3) as i32 {
                if page > 0 {
                    open_items(&mut state, host, player, category, page - 1);
                } else {
                    state.views.insert(
                        (player, window),
                        View::Items {
                            category,
                            page,
                            rows,
                            slots,
                        },
                    );
                }
            } else if slot == (nav + 5) as i32 {
                open_items(&mut state, host, player, category, page + 1);
            } else if let Some(item) = slots.get(&slot).copied() {
                if state.categories[category].id == "loja_vip" {
                    message(
                        host,
                        player,
                        "A compra VIP ainda não está disponível nesta loja.",
                    );
                    open_items(&mut state, host, player, category, page);
                } else {
                    open_detail(&mut state, host, player, category, page, item, 1);
                }
            } else {
                state.views.insert(
                    (player, window),
                    View::Items {
                        category,
                        page,
                        rows,
                        slots,
                    },
                );
            }
        }
        View::Detail {
            category,
            page,
            item,
            quantity,
        } => match slot {
            22 => open_items(&mut state, host, player, category, page),
            10 | 11 | 12 | 14 | 15 | 16 => {
                let delta = match slot {
                    10 => -64,
                    11 => -16,
                    12 => -1,
                    14 => 1,
                    15 => 16,
                    _ => 64,
                };
                let next = (quantity as i64 + delta).clamp(1, 9999) as u32;
                open_detail(&mut state, host, player, category, page, item, next);
            }
            20 => {
                if state.categories[category].id == "loja_vip" {
                    message(
                        host,
                        player,
                        "A compra VIP ainda não está disponível nesta loja.",
                    );
                    open_items(&mut state, host, player, category, page);
                    return;
                }
                let product = state.categories[category].items[item].clone();
                let currency = if state.categories[category].id == "loja_vip" {
                    state.config.currency.vip.clone()
                } else {
                    state.config.currency.normal.clone()
                };
                let result = transact_shop_item(host, player, &product, &currency, quantity, true);
                trade_message(host, player, result, true);
                state.views.insert(
                    (player, window),
                    View::Detail {
                        category,
                        page,
                        item,
                        quantity,
                    },
                );
            }
            24 => {
                if state.categories[category].id == "loja_vip" {
                    message(host, player, "A venda de itens VIP está desativada.");
                    open_items(&mut state, host, player, category, page);
                    return;
                }
                let product = state.categories[category].items[item].clone();
                let currency = if state.categories[category].id == "loja_vip" {
                    state.config.currency.vip.clone()
                } else {
                    state.config.currency.normal.clone()
                };
                let result = transact_shop_item(host, player, &product, &currency, quantity, false);
                trade_message(host, player, result, false);
                state.views.insert(
                    (player, window),
                    View::Detail {
                        category,
                        page,
                        item,
                        quantity,
                    },
                );
            }
            _ => {
                state.views.insert(
                    (player, window),
                    View::Detail {
                        category,
                        page,
                        item,
                        quantity,
                    },
                );
            }
        },
    }
}

unsafe extern "C-unwind" fn event(
    _: *mut c_void,
    host: *const HostApiV3,
    kind: u32,
    payload: *const u8,
    payload_len: usize,
) -> u8 {
    if payload.is_null() || payload_len > 65_536 {
        return 0;
    }
    let payload = unsafe { std::slice::from_raw_parts(payload, payload_len) };
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(payload) else {
        return 0;
    };
    let player = payload
        .get("player")
        .and_then(serde_json::Value::as_str)
        .and_then(|uuid| u128::from_str_radix(uuid, 16).ok())
        .unwrap_or(0);
    if let Some(host) = unsafe { host.as_ref() } {
        essentials::event(host, kind, &payload);
    }
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    match kind {
        event::PLAYER_QUIT => {
            state.views.retain(|(uuid, _), _| *uuid != player);
        }
        event::MENU_CLOSED => {
            if let Some(window) = payload
                .get("window_id")
                .and_then(serde_json::Value::as_i64)
                .and_then(|window| i32::try_from(window).ok())
            {
                state.views.remove(&(player, window));
            }
        }
        _ => {}
    }
    0
}

static DESCRIPTOR: PluginV3 = PluginV3 {
    abi_version: ABI_VERSION_V3,
    name: NAME.as_ptr(),
    name_len: NAME.len(),
    create: Some(create),
    destroy: Some(destroy),
    register_commands: Some(register_commands),
    command: Some(command),
    menu_click: Some(menu_click),
    event: Some(event),
    permission: None,
};

#[unsafe(no_mangle)]
pub extern "C-unwind" fn horizon_server_plugin_v3() -> *const PluginV3 {
    &DESCRIPTOR
}

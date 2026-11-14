use super::{
    State, View,
    config::ensure_loaded,
    economy::balance_text,
    ui::{message, show_menu, spec, spec_lore},
};
use horizon_server_plugin_api::HostApiV3;
use std::collections::HashMap;

pub(super) fn open_categories(state: &mut State, host: &HostApiV3, player: u128) {
    ensure_loaded(state);
    let mut mapping = HashMap::new();
    let items = state
        .categories
        .iter()
        .enumerate()
        .map(|(index, category)| {
            let slot = match category.id {
                "blocos" => 10,
                "minerios" => 11,
                "drops_mobs" => 12,
                "comida" => 13,
                "plantacao" => 14,
                "livros_encantados" => 15,
                "madeiras" => 16,
                _ => 22,
            };
            mapping.insert(slot as i32, index);
            spec(
                slot,
                &format!("minecraft:{}", category.icon),
                category.title,
                "white",
                1,
            )
        })
        .collect();
    state.views.retain(|(uuid, _), _| *uuid != player);
    if let Some(window) = show_menu(host, player, &state.config.title, 3, items) {
        state
            .views
            .insert((player, window), View::Categories(mapping));
    } else {
        message(host, player, "Não foi possível abrir a loja.");
    }
}

pub(super) fn open_items(
    state: &mut State,
    host: &HostApiV3,
    player: u128,
    category: usize,
    requested_page: usize,
) {
    let Some(cat) = state.categories.get(category) else {
        return;
    };
    let item_count = cat.items.len();
    let item_rows = item_count.saturating_sub(4).max(1).div_ceil(9).max(1);
    let rows = (item_rows + 1).clamp(3, 6) as u8;
    let size = usize::from(rows) * 9;
    let nav_row = size - 9;
    let slots: Vec<usize> = (0..nav_row)
        .chain([nav_row, nav_row + 1, nav_row + 7, nav_row + 8])
        .collect();
    let page_size = slots.len();
    let pages = item_count.div_ceil(page_size).max(1);
    let page = requested_page.min(pages - 1);
    let start = page * page_size;
    let visible = item_count.saturating_sub(start).min(page_size);
    let mut mapping = HashMap::new();
    let mut items = Vec::new();
    for (i, slot) in slots.iter().take(visible).enumerate() {
        let item_index = start + i;
        let item = &cat.items[item_index];
        mapping.insert(*slot as i32, item_index);
        let item_color = "yellow";
        let lore = if cat.id == "loja_vip" {
            vec![
                "§7Produto VIP".into(),
                "§8----------------".into(),
                format!("§7Comprar: §6{} Créditos", price(item.buy)),
                "§aClique para comprar".into(),
            ]
        } else {
            vec![
                "§7Quantidade: §f1x".into(),
                format!("§7Comprar: §6{} moedas", price(item.buy)),
                format!("§7Vender: §6{} moedas", price(item.sell)),
                "§8----------------".into(),
                "§aClique esquerdo: §7comprar 1x".into(),
                "§bClique direito: §7vender 1x".into(),
            ]
        };
        let icon = item
            .icon
            .as_deref()
            .unwrap_or(&item.material)
            .to_ascii_lowercase();
        let product = spec(
            *slot,
            &format!("minecraft:{icon}"),
            &item.display_name,
            item_color,
            item.amount.clamp(1, 64),
        );
        let mut product = product;
        if let Some(model) = &item.item_model {
            product.components.insert(
                "minecraft:item_model".into(),
                serde_json::Value::String(model.clone()),
            );
        }
        if let Some(components) = &item.components {
            product.components.extend(components.clone());
        }
        let lore = item.lore.clone().unwrap_or(lore);
        items.push(spec_lore(product, lore));
    }
    items.push(spec(
        nav_row + 2,
        "minecraft:gray_stained_glass_pane",
        " ",
        "gray",
        1,
    ));
    items.push(spec(
        nav_row + 3,
        "minecraft:stone_button",
        &format!("Página anterior ({}/{})", page + 1, pages),
        "white",
        1,
    ));
    items.push(spec(nav_row + 4, "minecraft:arrow", "Voltar", "white", 1));
    items.push(spec(
        nav_row + 5,
        "minecraft:oak_button",
        &format!("Próxima página ({}/{})", page + 1, pages),
        "white",
        1,
    ));
    items.push(spec(
        nav_row + 6,
        "minecraft:gray_stained_glass_pane",
        " ",
        "gray",
        1,
    ));
    let title = format!("Loja: {}", cat.title);
    state.views.retain(|(uuid, _), _| *uuid != player);
    if let Some(window) = show_menu(host, player, &title, rows, items) {
        state.views.insert(
            (player, window),
            View::Items {
                category,
                page,
                rows,
                slots: mapping,
            },
        );
    }
}

pub(super) fn open_detail(
    state: &mut State,
    host: &HostApiV3,
    player: u128,
    category: usize,
    page: usize,
    item_index: usize,
    quantity: u32,
) {
    let Some(item) = state
        .categories
        .get(category)
        .and_then(|c| c.items.get(item_index))
    else {
        return;
    };
    let quantity = quantity.clamp(1, 9999);
    let material = item.material.to_ascii_lowercase();
    let currency = if state.categories[category].id == "loja_vip" {
        "Créditos"
    } else {
        "moedas"
    };
    let currency_id = if state.categories[category].id == "loja_vip" {
        state.config.currency.vip.as_str()
    } else {
        state.config.currency.normal.as_str()
    };
    let mut quantity_lore = vec![
        format!(
            "§7Comprar: §6{} {currency}",
            price(item.buy * f64::from(quantity))
        ),
        format!(
            "§7Vender: §6{} {currency}",
            price(item.sell * f64::from(quantity))
        ),
        format!("§7Itens por pacote: §f{}", item.amount),
    ];
    if let Some(balance) = balance_text(host, player, currency_id) {
        quantity_lore.push(format!("§7Saldo: §6{balance} {currency}"));
    }
    quantity_lore.extend([
        "§8----------------".into(),
        "§7Ajuste a quantidade nos botões".into(),
        format!("§aClique para comprar {quantity}x"),
        format!("§cClique para vender {quantity}x"),
    ]);
    let mut product_icon = spec(
        4,
        &format!("minecraft:{material}"),
        &item.display_name,
        "yellow",
        item.amount.clamp(1, 64),
    );
    if let Some(model) = &item.item_model {
        product_icon.components.insert(
            "minecraft:item_model".into(),
            serde_json::Value::String(model.clone()),
        );
    }
    if let Some(components) = &item.components {
        product_icon.components.extend(components.clone());
    }
    let mut items = vec![
        spec(0, "minecraft:gray_stained_glass_pane", " ", "dark_gray", 1),
        product_icon,
        spec(10, "minecraft:red_stained_glass_pane", "−64", "red", 1),
        spec(11, "minecraft:orange_stained_glass_pane", "−16", "gold", 1),
        spec(
            12,
            "minecraft:pink_stained_glass_pane",
            "−1",
            "light_purple",
            1,
        ),
        spec_lore(
            spec(
                13,
                "minecraft:paper",
                &format!("Quantidade: {quantity}"),
                "yellow",
                1,
            ),
            quantity_lore,
        ),
        spec(
            14,
            "minecraft:light_blue_stained_glass_pane",
            "+1",
            "aqua",
            1,
        ),
        spec(15, "minecraft:cyan_stained_glass_pane", "+16", "aqua", 1),
        spec(16, "minecraft:lime_stained_glass_pane", "+64", "green", 1),
        spec_lore(
            spec(20, "minecraft:emerald", "Comprar", "green", 1),
            vec!["§7Clique para confirmar a compra".into()],
        ),
        spec(22, "minecraft:arrow", "Voltar", "white", 1),
        spec_lore(
            spec(24, "minecraft:redstone", "Vender", "red", 1),
            vec!["§7Clique para vender do inventário".into()],
        ),
    ];
    for slot in [1, 2, 3, 5, 6, 7, 8, 9, 17, 18, 19, 21, 23, 25, 26] {
        items.push(spec(
            slot,
            "minecraft:gray_stained_glass_pane",
            " ",
            "dark_gray",
            1,
        ));
    }
    let title = format!("Loja: {}", item.display_name);
    state.views.retain(|(uuid, _), _| *uuid != player);
    if let Some(window) = show_menu(host, player, &title, 3, items) {
        state.views.insert(
            (player, window),
            View::Detail {
                category,
                page,
                item: item_index,
                quantity,
            },
        );
    }
}

pub(super) fn price(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .replace('.', ",")
    }
}

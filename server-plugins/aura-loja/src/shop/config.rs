use super::{Category, DEFAULT_CONFIG_JSON, DEFAULT_ITEMS_JSON, ShopConfig, ShopItem, State};
use std::{collections::HashMap, fs, path::PathBuf};

pub(super) fn plugin_dir() -> Option<PathBuf> {
    let root = std::env::current_dir().ok()?;
    let plugins = root.join("plugins");
    let target = plugins.join("AuraEssentials");
    let legacy = plugins.join("AuraEssentialsHorizon");
    if legacy.is_dir() {
        let _ = fs::create_dir_all(&target);
        if let Ok(entries) = fs::read_dir(&legacy) {
            for entry in entries.flatten() {
                let from = entry.path();
                let to = target.join(entry.file_name());
                if !to.exists() {
                    let _ = fs::rename(from, to);
                }
            }
        }
    }
    Some(target)
}

pub(super) fn ensure_default_files() -> Result<PathBuf, String> {
    let dir =
        plugin_dir().ok_or_else(|| "não foi possível localizar a pasta do servidor".to_owned())?;
    fs::create_dir_all(&dir)
        .map_err(|error| format!("não foi possível criar {}: {error}", dir.display()))?;
    let shop = dir.join("shop.json");
    if !shop.exists() {
        fs::write(
            &shop,
            format!(
                "{}\n",
                serde_json::to_string_pretty(
                    &serde_json::from_str::<serde_json::Value>(DEFAULT_ITEMS_JSON)
                        .map_err(|error| error.to_string())?
                )
                .map_err(|error| error.to_string())?
            ),
        )
        .map_err(|error| format!("não foi possível criar shop.json: {error}"))?;
    }
    let config = dir.join("config.json");
    if !config.exists() {
        fs::write(&config, DEFAULT_CONFIG_JSON)
            .map_err(|error| format!("não foi possível criar config.json: {error}"))?;
    }
    Ok(dir)
}

pub(super) fn load_shop() -> Result<Vec<Category>, String> {
    let dir = ensure_default_files()?;
    let bytes = fs::read(dir.join("shop.json"))
        .map_err(|error| format!("não foi possível ler shop.json: {error}"))?;
    let data: HashMap<String, Vec<ShopItem>> =
        serde_json::from_slice(&bytes).map_err(|error| format!("shop.json inválido: {error}"))?;
    let categories = category_meta()
        .into_iter()
        .map(|(id, title, icon)| Category {
            id,
            title,
            icon,
            items: data.get(id).cloned().unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    if categories.iter().all(|category| category.items.is_empty()) {
        return Err("shop.json não tem itens em nenhuma categoria".into());
    }
    Ok(categories)
}

pub(super) fn load_config() -> Result<ShopConfig, String> {
    let dir = ensure_default_files()?;
    let bytes = fs::read(dir.join("config.json"))
        .map_err(|error| format!("não foi possível ler config.json: {error}"))?;
    let config: ShopConfig =
        serde_json::from_slice(&bytes).map_err(|error| format!("config.json inválido: {error}"))?;
    for currency in [&config.currency.normal, &config.currency.vip] {
        if currency.is_empty()
            || currency.len() > 128
            || !currency.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_:.-".contains(&byte)
            })
        {
            return Err("config.json contém um identificador de moeda inválido".into());
        }
    }
    Ok(config)
}

pub(super) fn ensure_loaded(state: &mut State) {
    if state.loaded {
        return;
    }
    let loaded =
        load_shop().and_then(|categories| load_config().map(|config| (categories, config)));
    match loaded {
        Ok((categories, config)) => {
            state.categories = categories;
            state.config = config;
        }
        Err(error) => {
            eprintln!("[AuraEssentials] {error}; usando a lista interna inicial");
            state.categories = categories_from_json(DEFAULT_ITEMS_JSON).unwrap_or_default();
        }
    }
    state.loaded = true;
}

pub(super) fn categories_from_json(json: &str) -> Result<Vec<Category>, String> {
    let data: HashMap<String, Vec<ShopItem>> =
        serde_json::from_str(json).map_err(|error| error.to_string())?;
    Ok(category_meta()
        .into_iter()
        .map(|(id, title, icon)| Category {
            id,
            title,
            icon,
            items: data.get(id).cloned().unwrap_or_default(),
        })
        .collect())
}

pub(super) fn category_meta() -> [(&'static str, &'static str, &'static str); 8] {
    [
        ("blocos", "Blocos", "bricks"),
        ("minerios", "Minérios", "diamond"),
        ("drops_mobs", "Drops de Mobs", "blaze_rod"),
        ("comida", "Comida", "cooked_beef"),
        ("plantacao", "Plantação", "wheat"),
        ("livros_encantados", "Livros Encantados", "enchanted_book"),
        ("madeiras", "Madeiras", "oak_log"),
        ("loja_vip", "Loja VIP", "nether_star"),
    ]
}

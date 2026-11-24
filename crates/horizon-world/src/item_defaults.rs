//! Items' default components, as everything in the game reads them: the
//! resource root's `items.json`, including vanilla data-pack stack-size codec
//! bounds. The item table, loot and other readers share this snapshot.

use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
};

/// An immutable default-component snapshot. Kept only while a resource load holds it.
pub type Defaults = BTreeMap<String, Value>;
static SHARED: Mutex<Vec<(PathBuf, Weak<Defaults>)>> = Mutex::new(Vec::new());

/// A resource load already reading and applying hooks to `items.json` for a
/// root. Several startup tables ask for defaults in parallel; without this
/// gate they can all miss `SHARED` and rerun the native hook for every item.
struct InFlight {
    result: Mutex<Option<Result<Arc<Defaults>, (io::ErrorKind, String)>>>,
    ready: Condvar,
}
static IN_FLIGHT: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<InFlight>>>> =
    OnceLock::new();

fn in_flight() -> &'static Mutex<std::collections::HashMap<PathBuf, Arc<InFlight>>> {
    IN_FLIGHT.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Share parsed defaults with the other tables built during this load. Callers
/// that need a fresh read regardless of other loads can still use [`read`].
pub fn shared(root: &Path) -> io::Result<Arc<Defaults>> {
    let held = || {
        SHARED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(path, _)| path == root)
            .and_then(|(_, value)| value.upgrade())
    };
    if let Some(value) = held() {
        return Ok(value);
    }

    let (flight, leader) = {
        let mut flights = in_flight().lock().unwrap_or_else(|e| e.into_inner());
        match flights.entry(root.to_owned()) {
            std::collections::hash_map::Entry::Occupied(entry) => (entry.get().clone(), false),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let flight = Arc::new(InFlight {
                    result: Mutex::new(None),
                    ready: Condvar::new(),
                });
                entry.insert(flight.clone());
                (flight, true)
            }
        }
    };
    if !leader {
        let mut result = flight.result.lock().unwrap_or_else(|e| e.into_inner());
        while result.is_none() {
            result = flight.ready.wait(result).unwrap_or_else(|e| e.into_inner());
        }
        return result
            .clone()
            .expect("item defaults flight result")
            .map_err(|(kind, message)| io::Error::new(kind, message));
    }

    let result = read(root)
        .map(Arc::new)
        .map_err(|e| (e.kind(), e.to_string()));
    if let Ok(value) = &result {
        let mut cache = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(other) = cache
            .iter()
            .find(|(path, _)| path == root)
            .and_then(|(_, v)| v.upgrade())
        {
            drop(cache);
            let result = Ok(other);
            let mut published = flight.result.lock().unwrap_or_else(|e| e.into_inner());
            *published = Some(result.clone());
            flight.ready.notify_all();
            in_flight()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(root);
            return result.map_err(|(kind, message)| io::Error::new(kind, message));
        }
        cache.retain(|(_, value)| value.strong_count() > 0);
        cache.push((root.to_owned(), Arc::downgrade(value)));
    }
    {
        let mut published = flight.result.lock().unwrap_or_else(|e| e.into_inner());
        *published = Some(result.clone());
        flight.ready.notify_all();
    }
    in_flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(root);
    result.map_err(|(kind, message)| io::Error::new(kind, message))
}

/// Vanilla's `ExtraCodecs.intRange(1, 99)` on a data pack's own value.
const VANILLA_MAX_STACK_SIZE: i64 = 99;
const STACK_SIZE: &str = "minecraft:max_stack_size";

/// `value` as a stack size in `1..=max`, or None when it is not a number.
fn stack_size(value: &Value, max: i64) -> Option<i64> {
    value.as_f64().map(|n| (n as i64).clamp(1, max))
}

/// Every item's default components at `root`, by item id. Stack sizes are
/// normalized to vanilla's `ExtraCodecs.intRange(1, 99)`.
pub fn read(root: &Path) -> io::Result<BTreeMap<String, Value>> {
    let mut items: BTreeMap<String, Value> =
        serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
    for components in items.values_mut() {
        if let Value::Object(components) = components {
            // A data pack's own value goes through vanilla's codec.
            if let Some(size) = components
                .get(STACK_SIZE)
                .and_then(|size| stack_size(size, VANILLA_MAX_STACK_SIZE))
            {
                components.insert(STACK_SIZE.into(), size.into());
            }
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn a_load_shares_one_snapshot_and_the_next_load_reads_changes() {
        let root =
            std::env::temp_dir().join(format!("horizon-default-snapshot-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let write = |size| {
            fs::write(
                root.join("items.json"),
                format!(r#"{{"test:item":{{"minecraft:max_stack_size":{size}}}}}"#),
            )
            .unwrap()
        };
        write(16);
        let first = super::shared(&root).unwrap();
        write(64);
        let second = super::shared(&root).unwrap();
        assert_eq!(second["test:item"]["minecraft:max_stack_size"], 16);
        // Explicit fresh reads remain available even while a load owns its snapshot.
        assert_eq!(
            super::read(&root).unwrap()["test:item"]["minecraft:max_stack_size"],
            64
        );
        drop((first, second));
        let next = super::shared(&root).unwrap();
        assert_eq!(next["test:item"]["minecraft:max_stack_size"], 64);
        drop(next);
        fs::write(root.join("items.json"), "broken").unwrap();
        assert!(super::shared(&root).is_err());
        write(2);
        assert_eq!(
            super::shared(&root).unwrap()["test:item"]["minecraft:max_stack_size"],
            2
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// A data pack's stack size is kept to vanilla's 1..=99, so stack
    /// arithmetic never sees a 0, and a mod may raise it above 99 (up to the
    /// count the wire and the save carry).
    #[test]
    fn data_pack_sizes_stay_in_vanilla_range() {
        let root = std::env::temp_dir().join(format!("horizon-stack-sizes-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let items = r#"{"a:none": {"minecraft:max_stack_size": 0},
            "a:less": {"minecraft:max_stack_size": -5},
            "a:many": {"minecraft:max_stack_size": 500},
            "a:some": {"minecraft:max_stack_size": 16}, "a:unset": {}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let items = super::read(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let size = |id: &str| items[id].get("minecraft:max_stack_size").cloned();
        assert_eq!(
            ["a:none", "a:less", "a:many", "a:some", "a:unset"].map(size),
            [
                Some(1.into()),
                Some(1.into()),
                Some(99.into()),
                Some(16.into()),
                None
            ]
        );
    }
}

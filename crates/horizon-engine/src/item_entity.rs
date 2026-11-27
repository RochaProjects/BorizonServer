//! Server-owned item entities replicated to PLAY clients.

use crate::inventory::PlayerInventory;
use horizon_protocol::item::Stack;
use std::{collections::HashMap, sync::Mutex, time::SystemTime};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DroppedItem {
    pub(crate) id: i32,
    pub(crate) uuid: u128,
    pub(crate) stack: Stack,
    pub(crate) position: [f64; 3],
    pub(crate) velocity: [f64; 3],
    pub(crate) age: u32,
    pub(crate) pickup_delay: u32,
    pub(crate) on_ground: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ItemPickup {
    pub(crate) id: i32,
    pub(crate) amount: i32,
}

#[derive(Default)]
struct Store {
    next_id: i32,
    items: HashMap<i32, DroppedItem>,
}

/// Server-owned item-entity lifecycle shared by PLAY sessions.
#[derive(Default)]
pub(crate) struct ItemEntityStore {
    store: Mutex<Store>,
}

impl ItemEntityStore {
    pub(crate) fn spawn(
        &self,
        stack: Stack,
        position: [f64; 3],
        velocity: [f64; 3],
    ) -> Result<i32, String> {
        let mut store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let id = store.next_id.max(1_000_000);
        store.next_id = id
            .checked_add(1)
            .ok_or_else(|| "PLAY item entity ID space exhausted".to_owned())?;
        let time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let uuid = time.rotate_left(29) ^ (id as u128).wrapping_mul(0x9e3779b97f4a7c15);
        store.items.insert(
            id,
            DroppedItem {
                id,
                uuid,
                stack,
                position,
                velocity,
                age: 0,
                pickup_delay: 10,
                on_ground: false,
            },
        );
        Ok(id)
    }

    pub(crate) fn all(&self) -> Vec<DroppedItem> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .items
            .values()
            .cloned()
            .collect()
    }

    /// Advance loaded item entities once on the shared server tick. Returning
    /// `None` from `move_item` means the entity's collision chunks are not
    /// resident; vanilla also freezes entities outside entity-ticking chunks.
    pub(crate) fn tick(
        &self,
        mut move_item: impl FnMut(&DroppedItem, [f64; 3]) -> Result<Option<([f64; 3], bool)>, String>,
    ) -> Result<(), String> {
        let mut store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let mut ids: Vec<_> = store.items.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(item) = store.items.get_mut(&id) else {
                continue;
            };
            let mut velocity = item.velocity;
            velocity[1] -= 0.04;
            let wanted = std::array::from_fn(|axis| item.position[axis] + velocity[axis]);
            let Some((position, on_ground)) = move_item(item, wanted)? else {
                continue;
            };
            item.age = item.age.saturating_add(1);
            item.pickup_delay = item.pickup_delay.saturating_sub(1);
            item.position = position;
            item.on_ground = on_ground;
            for axis in [0, 2] {
                if (position[axis] - wanted[axis]).abs() > 1.0e-7 {
                    velocity[axis] = 0.0;
                }
                velocity[axis] *= if on_ground { 0.588 } else { 0.98 };
            }
            if on_ground && wanted[1] != position[1] {
                velocity[1] = 0.0;
            } else {
                velocity[1] *= 0.98;
            }
            item.velocity = velocity;
            if item.age >= 6000 {
                store.items.remove(&id);
            }
        }
        Ok(())
    }

    /// Atomically transfer nearby eligible stacks into the player's inventory.
    /// The store lock arbitrates competing player pickups of the same entity.
    pub(crate) fn collect_near(
        &self,
        player_position: [f64; 3],
        inventory: &mut PlayerInventory,
        mut max_stack: impl FnMut(&Stack) -> i32,
    ) -> Vec<ItemPickup> {
        let mut store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let mut ids: Vec<_> = store.items.keys().copied().collect();
        ids.sort_unstable();
        let player_min = [
            player_position[0] - 0.3,
            player_position[1],
            player_position[2] - 0.3,
        ];
        let player_max = [
            player_position[0] + 0.3,
            player_position[1] + 1.8,
            player_position[2] + 0.3,
        ];
        let mut picked = Vec::new();
        for id in ids {
            let Some(item) = store.items.get_mut(&id) else {
                continue;
            };
            if item.pickup_delay > 0 {
                continue;
            }
            let item_min = [
                item.position[0] - 0.125,
                item.position[1],
                item.position[2] - 0.125,
            ];
            let item_max = [
                item.position[0] + 0.125,
                item.position[1] + 0.25,
                item.position[2] + 0.125,
            ];
            let distance_squared = (0..3)
                .map(|axis| {
                    let separation = (player_min[axis] - item_max[axis])
                        .max(item_min[axis] - player_max[axis])
                        .max(0.0);
                    separation * separation
                })
                .sum::<f64>();
            if distance_squared > 1.0 {
                continue;
            }
            let moved = inventory.insert_stack(&item.stack, max_stack(&item.stack));
            if moved <= 0 {
                continue;
            }
            item.stack.count -= moved;
            picked.push(ItemPickup { id, amount: moved });
        }
        store.items.retain(|_, item| item.stack.count > 0);
        picked
    }

    #[cfg(test)]
    pub(crate) fn remove(&self, id: i32) -> bool {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .items
            .remove(&id)
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_entity_owner_allocates_distinct_replicable_ids() {
        let store = ItemEntityStore::default();
        let stack = Stack::new(1, 1);
        let first = store
            .spawn(stack.clone(), [1.5, 64.0, -2.5], [0.0, 0.2, 0.0])
            .unwrap();
        let second = store
            .spawn(stack, [2.5, 64.0, -2.5], [0.0, 0.2, 0.0])
            .unwrap();
        assert_ne!(first, second);
        let items = store.all();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.id >= 1_000_000));
        assert!(store.remove(first));
        assert!(!store.remove(first));
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn item_tick_counts_pickup_delay_and_collection_updates_inventory_atomically() {
        let store = ItemEntityStore::default();
        let id = store
            .spawn(Stack::new(4, 3), [0.5, 64.0, 0.5], [0.0, 0.0, 0.0])
            .unwrap();
        for _ in 0..10 {
            store
                .tick(|item, _| Ok(Some((item.position, true))))
                .unwrap();
        }
        assert_eq!(store.all()[0].pickup_delay, 0);
        let mut inventory = PlayerInventory::default();
        let pickups = store.collect_near([0.5, 65.0, 0.5], &mut inventory, |_| 64);
        assert_eq!(pickups, vec![ItemPickup { id, amount: 3 }]);
        assert!(store.all().is_empty());
        assert_eq!(inventory.content_packet().items[9], Some(Stack::new(4, 3)));
    }
}

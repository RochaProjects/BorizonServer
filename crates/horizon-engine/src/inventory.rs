//! Server-owned player inventory menu state for PLAY.
//!
//! Slot order follows vanilla `InventoryMenu`: result, 2x2 crafting grid,
//! armor, main inventory, hotbar, and offhand. This owner intentionally lives
//! in the dedicated engine; it has no renderer or client-game dependency.

use horizon_protocol::{item::Stack, packets::play};

pub(crate) const PLAYER_MENU_SLOTS: usize = 46;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlayerInventory {
    slots: Vec<Option<Stack>>,
    carried: Option<Stack>,
    state_id: i32,
}

impl Default for PlayerInventory {
    fn default() -> Self {
        Self {
            slots: vec![None; PLAYER_MENU_SLOTS],
            carried: None,
            state_id: 0,
        }
    }
}

impl PlayerInventory {
    /// Builds the authoritative `InventoryMenu` snapshot sent when entering
    /// PLAY and when rejecting a client prediction that the server cannot
    /// apply yet. The monotonically changing state id follows the menu's
    /// 15-bit revision counter.
    pub(crate) fn content_packet(&mut self) -> play::ContainerSetContent {
        self.state_id = self.state_id.wrapping_add(1) & 0x7fff;
        play::ContainerSetContent {
            container: 0,
            state: self.state_id,
            items: self.slots.clone(),
            carried: self.carried.clone(),
        }
    }

    pub(crate) fn hotbar(&self, selected: i16) -> Option<&Stack> {
        let slot = usize::try_from(selected).ok()?;
        (slot < 9).then_some(())?;
        self.slots[36 + slot].as_ref()
    }

    pub(crate) fn hand(&self, hand: i32, selected: i16) -> Option<&Stack> {
        match hand {
            0 => self.hotbar(selected),
            1 => self.slots[45].as_ref(),
            _ => None,
        }
    }

    pub(crate) fn set_creative_slot(&mut self, slot: i16, item: Option<Stack>) -> bool {
        let Ok(slot) = usize::try_from(slot) else {
            return false;
        };
        let Some(target) = self.slots.get_mut(slot) else {
            return false;
        };
        *target = item.filter(|stack| stack.count > 0);
        true
    }

    pub(crate) fn consume_hand(&mut self, hand: i32, selected: i16) -> bool {
        let slot = match hand {
            0 => match usize::try_from(selected) {
                Ok(slot) if slot < 9 => slot + 36,
                _ => return false,
            },
            1 => 45,
            _ => return false,
        };
        let Some(stack) = self.slots[slot].as_mut() else {
            return false;
        };
        stack.count -= 1;
        if stack.count <= 0 {
            self.slots[slot] = None;
        }
        true
    }

    /// Insert as much of a server-authored stack as the vanilla player
    /// inventory can hold. Returns the amount accepted; the caller retains
    /// any remainder in its item entity.
    pub(crate) fn insert_stack(&mut self, stack: &Stack, max_stack: i32) -> i32 {
        let limit = max_stack.max(1);
        let mut remaining = stack.count.max(0);
        for slot in &mut self.slots[9..PLAYER_MENU_SLOTS] {
            let Some(existing) = slot else { continue };
            if existing.item != stack.item
                || existing.added != stack.added
                || existing.removed != stack.removed
            {
                continue;
            }
            let room = (limit - existing.count).max(0);
            let moved = room.min(remaining);
            existing.count += moved;
            remaining -= moved;
            if remaining == 0 {
                return stack.count.max(0);
            }
        }
        for slot in &mut self.slots[9..PLAYER_MENU_SLOTS] {
            if slot.is_some() {
                continue;
            }
            let moved = limit.min(remaining);
            let mut inserted = stack.clone();
            inserted.count = moved;
            *slot = Some(inserted);
            remaining -= moved;
            if remaining == 0 {
                break;
            }
        }
        stack.count.max(0) - remaining
    }

    /// Apply the ordinary left/right pickup input for player storage slots.
    /// The client-provided changed-slot list is never trusted; the owner
    /// produces a fresh full snapshot after the operation.
    pub(crate) fn click_storage(
        &mut self,
        slot: i16,
        button: i8,
        input: i32,
        mut max_stack: impl FnMut(i32) -> i32,
    ) -> bool {
        if input != 0
            || !(0..=1).contains(&button)
            || !(9..PLAYER_MENU_SLOTS as i16).contains(&slot)
        {
            return false;
        }
        let slot = usize::try_from(slot).expect("validated storage slot");
        match (button, self.carried.take(), self.slots[slot].take()) {
            (0, None, Some(stack)) => self.carried = Some(stack),
            (0, Some(carried), None) => self.slots[slot] = Some(carried),
            (0, Some(mut carried), Some(mut existing))
                if carried.item == existing.item
                    && carried.added == existing.added
                    && carried.removed == existing.removed =>
            {
                let limit = max_stack(existing.item).max(1);
                let moved = (limit - existing.count).max(0).min(carried.count);
                existing.count += moved;
                carried.count -= moved;
                self.slots[slot] = Some(existing);
                self.carried = (carried.count > 0).then_some(carried);
            }
            (0, Some(carried), Some(existing)) => {
                self.carried = Some(existing);
                self.slots[slot] = Some(carried);
            }
            (1, None, Some(mut stack)) => {
                let take = (stack.count + 1) / 2;
                stack.count -= take;
                self.carried = Some(Stack {
                    count: take,
                    ..stack.clone()
                });
                self.slots[slot] = (stack.count > 0).then_some(stack);
            }
            (1, Some(mut carried), None) => {
                let mut placed = carried.clone();
                placed.count = 1;
                self.slots[slot] = Some(placed);
                carried.count -= 1;
                self.carried = (carried.count > 0).then_some(carried);
            }
            (1, Some(mut carried), Some(mut existing))
                if carried.item == existing.item
                    && carried.added == existing.added
                    && carried.removed == existing.removed =>
            {
                if existing.count < max_stack(existing.item).max(1) {
                    existing.count += 1;
                    carried.count -= 1;
                }
                self.slots[slot] = Some(existing);
                self.carried = (carried.count > 0).then_some(carried);
            }
            (1, Some(carried), Some(existing)) => {
                self.carried = Some(existing);
                self.slots[slot] = Some(carried);
            }
            (0 | 1, None, None) => {}
            _ => unreachable!("button was validated"),
        }
        true
    }

    pub(crate) fn state_id(&self) -> i32 {
        self.state_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_menu_snapshot_has_vanilla_slot_count_and_revision() {
        let mut inventory = PlayerInventory::default();
        let first = inventory.content_packet();
        assert_eq!(first.container, 0);
        assert_eq!(first.state, 1);
        assert_eq!(first.items.len(), PLAYER_MENU_SLOTS);
        assert!(first.items.iter().all(Option::is_none));
        assert!(first.carried.is_none());

        let second = inventory.content_packet();
        assert_eq!(second.state, 2);
    }

    #[test]
    fn inventory_uses_vanilla_hotbar_menu_slots() {
        let mut inventory = PlayerInventory::default();
        let stack = Stack::new(1, 3);
        inventory.slots[36] = Some(stack.clone());
        assert_eq!(inventory.hotbar(0), Some(&stack));
        assert_eq!(inventory.hotbar(1), None);
    }

    #[test]
    fn pickup_insert_merges_matching_stacks_then_uses_empty_slots() {
        let mut inventory = PlayerInventory::default();
        inventory.slots[9] = Some(Stack::new(4, 60));
        let incoming = Stack::new(4, 8);
        assert_eq!(inventory.insert_stack(&incoming, 64), 8);
        assert_eq!(inventory.slots[9].as_ref().unwrap().count, 64);
        assert_eq!(inventory.slots[10].as_ref().unwrap().count, 4);

        let mut full = PlayerInventory::default();
        for slot in &mut full.slots[9..PLAYER_MENU_SLOTS] {
            *slot = Some(Stack::new(9, 64));
        }
        full.slots[9] = Some(Stack::new(4, 64));
        assert_eq!(full.insert_stack(&incoming, 64), 0);
    }

    #[test]
    fn storage_clicks_move_and_split_stacks_using_server_state() {
        let mut inventory = PlayerInventory::default();
        inventory.slots[9] = Some(Stack::new(4, 5));
        assert!(inventory.click_storage(9, 1, 0, |_| 64));
        assert_eq!(inventory.slots[9].as_ref().unwrap().count, 2);
        assert_eq!(inventory.carried.as_ref().unwrap().count, 3);
        assert!(inventory.click_storage(36, 0, 0, |_| 64));
        assert_eq!(inventory.slots[36].as_ref().unwrap().count, 3);
        assert!(inventory.carried.is_none());
        assert!(!inventory.click_storage(0, 0, 0, |_| 64));
    }

    #[test]
    fn empty_storage_slot_click_is_a_noop_instead_of_panicking() {
        let mut inventory = PlayerInventory::default();
        assert!(inventory.click_storage(9, 0, 0, |_| 64));
        assert!(inventory.click_storage(10, 1, 0, |_| 64));
        assert!(inventory.slots[9].is_none());
        assert!(inventory.slots[10].is_none());
        assert!(inventory.carried.is_none());
    }
}

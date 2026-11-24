//! Vanilla scoreboard display-slot identifiers, independent of scoreboard state or text rendering.

pub const SLOT_LIST: &str = "list";
pub const SLOT_SIDEBAR: &str = "sidebar";
pub const SLOT_BELOW_NAME: &str = "below_name";

/// The 19 vanilla display-slot names in registry/id order.
pub const DISPLAY_SLOTS: [&str; 19] = [
    "list",
    "sidebar",
    "below_name",
    "sidebar.team.black",
    "sidebar.team.dark_blue",
    "sidebar.team.dark_green",
    "sidebar.team.dark_aqua",
    "sidebar.team.dark_red",
    "sidebar.team.dark_purple",
    "sidebar.team.gold",
    "sidebar.team.gray",
    "sidebar.team.dark_gray",
    "sidebar.team.blue",
    "sidebar.team.green",
    "sidebar.team.aqua",
    "sidebar.team.red",
    "sidebar.team.light_purple",
    "sidebar.team.yellow",
    "sidebar.team.white",
];

/// Returns whether `name` is one of the vanilla scoreboard display slots.
pub fn display_slot_valid(name: &str) -> bool {
    DISPLAY_SLOTS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_has_vanilla_order_and_no_duplicates() {
        assert_eq!(DISPLAY_SLOTS.len(), 19);
        assert_eq!(
            &DISPLAY_SLOTS[..3],
            &[SLOT_LIST, SLOT_SIDEBAR, SLOT_BELOW_NAME]
        );
        assert_eq!(DISPLAY_SLOTS[3], "sidebar.team.black");
        assert_eq!(DISPLAY_SLOTS[18], "sidebar.team.white");
        assert_eq!(
            DISPLAY_SLOTS.iter().copied().collect::<BTreeSet<_>>().len(),
            DISPLAY_SLOTS.len()
        );
    }

    #[test]
    fn lookup_accepts_only_vanilla_slots() {
        assert!(display_slot_valid("list"));
        assert!(display_slot_valid("sidebar.team.red"));
        assert!(!display_slot_valid("sidebar.team.unknown"));
        assert!(!display_slot_valid("example:sidebar"));
    }
}

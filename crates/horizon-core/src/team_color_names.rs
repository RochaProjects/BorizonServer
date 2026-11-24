//! Vanilla scoreboard team color identifiers, independent of text rendering.

/// The 16 color names accepted by vanilla team-color arguments, in registry order.
pub const TEAM_COLOR_NAMES: [&str; 16] = [
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
];

/// Returns whether `name` is one of the vanilla team color identifiers.
pub fn is_team_color_name(name: &str) -> bool {
    TEAM_COLOR_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn names_match_vanilla_color_order_without_duplicates() {
        let expected = [
            "black",
            "dark_blue",
            "dark_green",
            "dark_aqua",
            "dark_red",
            "dark_purple",
            "gold",
            "gray",
            "dark_gray",
            "blue",
            "green",
            "aqua",
            "red",
            "light_purple",
            "yellow",
            "white",
        ];
        assert_eq!(TEAM_COLOR_NAMES, expected);
        assert_eq!(
            TEAM_COLOR_NAMES
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            TEAM_COLOR_NAMES.len()
        );
    }

    #[test]
    fn lookup_accepts_only_vanilla_team_color_names() {
        assert!(is_team_color_name("dark_purple"));
        assert!(!is_team_color_name("Dark_Purple"));
        assert!(!is_team_color_name("example:purple"));
    }
}

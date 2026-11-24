//! Vanilla `/bossbar` argument names; display formatting and saved state live elsewhere.

/// Accepted vanilla boss-bar colors, in command suggestion order.
pub const COLORS: [&str; 7] = ["pink", "blue", "red", "green", "yellow", "purple", "white"];

/// Accepted vanilla boss-bar overlays, in command suggestion order.
pub const STYLES: [&str; 5] = [
    "progress",
    "notched_6",
    "notched_10",
    "notched_12",
    "notched_20",
];

#[cfg(test)]
mod tests {
    use super::{COLORS, STYLES};

    #[test]
    fn names_match_vanilla_command_order() {
        assert_eq!(
            COLORS,
            ["pink", "blue", "red", "green", "yellow", "purple", "white"]
        );
        assert_eq!(
            STYLES,
            [
                "progress",
                "notched_6",
                "notched_10",
                "notched_12",
                "notched_20"
            ]
        );
    }

    #[test]
    fn names_are_unique() {
        let mut colors = COLORS.to_vec();
        colors.sort_unstable();
        colors.dedup();
        assert_eq!(colors.len(), COLORS.len());

        let mut styles = STYLES.to_vec();
        styles.sort_unstable();
        styles.dedup();
        assert_eq!(styles.len(), STYLES.len());
    }
}

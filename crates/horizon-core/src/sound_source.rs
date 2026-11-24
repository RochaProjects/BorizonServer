//! Vanilla sound sources (`SoundSource`), used by command argument validation.

/// Sound source names in vanilla order.
pub const SOUND_SOURCES: [&str; 11] = [
    "master", "music", "record", "weather", "block", "hostile", "neutral", "player", "ambient",
    "voice", "ui",
];

#[cfg(test)]
mod tests {
    use super::SOUND_SOURCES;

    #[test]
    fn names_match_vanilla_order() {
        assert_eq!(
            SOUND_SOURCES,
            [
                "master", "music", "record", "weather", "block", "hostile", "neutral", "player",
                "ambient", "voice", "ui",
            ]
        );
    }

    #[test]
    fn names_are_unique_and_unqualified() {
        let mut sorted = SOUND_SOURCES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), SOUND_SOURCES.len());
        assert!(SOUND_SOURCES.iter().all(|name| !name.contains(':')));
    }
}

//! Vanilla player game modes, independent of command parsing and presentation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    pub const NAMES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];

    pub const ALL: [GameMode; 4] = [
        GameMode::Survival,
        GameMode::Creative,
        GameMode::Adventure,
        GameMode::Spectator,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_match_vanilla_names_and_order() {
        assert_eq!(GameMode::ALL.map(GameMode::name), GameMode::NAMES);
        for mode in GameMode::ALL {
            assert_eq!(GameMode::from_name(mode.name()), Some(mode));
        }
    }

    #[test]
    fn lookup_is_exact_and_case_sensitive() {
        assert_eq!(GameMode::from_name("survival"), Some(GameMode::Survival));
        assert_eq!(GameMode::from_name("Survival"), None);
        assert_eq!(GameMode::from_name("minecraft:survival"), None);
        assert_eq!(GameMode::from_name("unknown"), None);
    }
}

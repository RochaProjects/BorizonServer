//! Shared mesh policy consumed by terrain meshing regardless of who owns the
//! resulting mesh. Kept outside the graphics-settings module so server-side
//! chunk processing can use the policy without importing display settings.

/// What the section mesher does with leaf faces inside a canopy. Terrain
/// geometry only: entities, particles and block entities are untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeafCullingMode {
    /// Every leaf face, as vanilla's fancy leaves draw them.
    #[default]
    Off,
    /// Drops a face between two leaves when the leaf it faces has a leaf or
    /// a full opaque cube behind it too: that face only shows through two
    /// cutout layers. The faces behind the outer layer stay, so a crown
    /// looks the same from outside. The faced leaf must be in the section
    /// (as `Hollow`), so the block behind it is at most a border cell of the
    /// next section, whose edits remesh this one.
    Hidden,
    /// Leaves surrounded on all six sides by leaves or full opaque cubes draw
    /// in the opaque layer, so their cutouts no longer show the layers behind.
    Solid,
    /// Like `Solid`, but a leaf with one open side draws opaque too: the
    /// exposed shell thins by a layer and the canopy looks chunkier.
    SolidAggressive,
    /// Drops the faces two leaves share. Faces toward air, cavities, partial
    /// models, unknown neighbours and section boundaries stay.
    Hollow,
}

impl LeafCullingMode {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Hidden,
        Self::Solid,
        Self::SolidAggressive,
        Self::Hollow,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Hidden => "hidden",
            Self::Solid => "solid",
            Self::SolidAggressive => "solid_aggressive",
            Self::Hollow => "hollow",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Hidden => "Hidden Faces",
            Self::Solid => "Solid",
            Self::SolidAggressive => "Solid Aggressive",
            Self::Hollow => "Hollow",
        }
    }

    /// Whether this mode draws interior leaf faces in the solid layer.
    pub fn is_interior(self) -> bool {
        matches!(self, Self::Solid | Self::SolidAggressive)
    }

    pub fn parse(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name() == s)
            .unwrap_or_default()
    }
}

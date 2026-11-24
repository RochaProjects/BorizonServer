use horizon_sim_data::mesh_policy::LeafCullingMode as Mode;

#[test]
fn names_parse_and_labels_are_stable() {
    let expected = [
        (Mode::Off, "off", "Off"),
        (Mode::Hidden, "hidden", "Hidden Faces"),
        (Mode::Solid, "solid", "Solid"),
        (
            Mode::SolidAggressive,
            "solid_aggressive",
            "Solid Aggressive",
        ),
        (Mode::Hollow, "hollow", "Hollow"),
    ];
    assert_eq!(Mode::ALL.len(), expected.len());
    for (mode, name, label) in expected {
        assert_eq!(mode.name(), name);
        assert_eq!(mode.label(), label);
        assert_eq!(Mode::parse(name), mode);
    }
    assert_eq!(Mode::parse("unknown"), Mode::Off);
}

#[test]
fn only_solid_modes_draw_interior_faces_opaque() {
    assert!(!Mode::Off.is_interior());
    assert!(!Mode::Hidden.is_interior());
    assert!(Mode::Solid.is_interior());
    assert!(Mode::SolidAggressive.is_interior());
    assert!(!Mode::Hollow.is_interior());
}

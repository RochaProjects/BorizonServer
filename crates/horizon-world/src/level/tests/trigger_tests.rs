//! Vanilla plate/wire entity triggers and the sculk charge cursor on a fresh chunk.
use super::*;

#[test]
fn vanilla_entity_triggers_power_plates_and_wires() {
    let pos = BlockPos::new(0, 1, 0);
    for (name, props) in [
        ("stone_pressure_plate", vec![("powered", "false")]),
        ("oak_pressure_plate", vec![("powered", "false")]),
        ("heavy_weighted_pressure_plate", vec![("power", "0")]),
        ("tripwire", vec![("powered", "false")]),
    ] {
        let set = |w: &World<'_>| {
            let state = w.get(pos);
            let b = w.b();
            if name.contains("weighted") {
                b.table.int(state, b.props.power) > 0
            } else {
                b.table.bool(state, b.props.powered)
            }
        };
        for (occupants, expected) in [(vec![1], true), (vec![], false)] {
            // A fresh level each time: a tripped wire keeps a scheduled tick.
            let mut w = world!();
            let off = w.id(name, &props);
            w.put(pos, off);
            w.level.occupants = occupants.clone();
            w.b().entity_inside(&mut w.level, &mut w.ticks, pos);
            assert_eq!(set(&w), expected, "{name}: {occupants:?}");
        }
    }
}

#[test]
fn fresh_chunk_sculk_charge_does_not_wait_for_ticking_readiness() {
    for (ticketed, ready) in [(true, false), (false, true), (true, true), (false, false)] {
        let mut w = world!();
        w.level.tick_ready = ready;
        w.level.block_events_ready = ticketed;
        let origin = BlockPos::new(0, 1, 0);
        let mut spreader = sculk::SculkSpreader::default();
        let mut cursor = sculk::ChargeCursor::new(origin, 5);
        cursor.update_delay = 2;
        spreader.add_cursor(cursor);
        w.f.behaviours.sculk_update_cursors(
            &mut w.level,
            &mut w.ticks,
            &mut spreader,
            origin,
            false,
        );
        assert_eq!(
            spreader.cursors[0].update_delay,
            if ticketed { 1 } else { 2 },
            "ticketed={ticketed}, ticking={ready}"
        );
    }
}

use horizon_sim_data::{
    chunk::MIN_Y,
    lod_data::{Column, Span, Surface},
};
use horizon_world::registry::BlockStateId;

fn span(min: i16, max: i16, state: u32) -> Span {
    Span {
        min,
        max,
        state: BlockStateId(state),
        biome: 0,
        light: 240,
    }
}

#[test]
fn reduced_columns_keep_rare_supporting_trunk_under_shared_crown() {
    let mut columns: [Column; 256] = std::array::from_fn(|_| Column {
        surface: None,
        spans: vec![span(0, 4, 1)].into(),
    });
    columns[0].spans.push(span(4, 12, 2));
    for z in 0..4 {
        for x in 0..4 {
            columns[z * 16 + x].spans.push(span(12, 14, 3));
        }
    }

    assert_eq!(
        Column::reduced(&columns, 0, 0, 4).spans,
        [span(0, 4, 1), span(4, 12, 2), span(12, 14, 3)]
    );
    assert_eq!(Column::reduced(&columns, 0, 0, 1), columns[0]);
}

#[test]
fn reduced_material_votes_ignore_light_and_keep_real_exposure() {
    let mut columns: [Column; 256] = std::array::from_fn(|_| Column::default());
    for i in 0..16 {
        columns[(i / 4) * 16 + i % 4].spans.push(Span {
            min: 60,
            max: 64,
            state: BlockStateId(if i < 10 { 2 } else { 1 }),
            biome: 0,
            light: if i < 10 { (i as u16 + 6) << 4 } else { 0 },
        });
    }

    let reduced = Column::reduced(&columns, 0, 0, 4);
    assert_eq!(reduced.spans.len(), 1);
    assert_eq!(reduced.spans[0].state, BlockStateId(2));
    assert_eq!(reduced.spans[0].light, 240);

    for column in &mut columns {
        for run in &mut column.spans {
            run.light = 0;
        }
    }
    assert_eq!(Column::reduced(&columns, 0, 0, 4).spans[0].light, 0);
    columns[0].spans[0].light = 7 << 12;
    assert_eq!(Column::reduced(&columns, 0, 0, 4).spans[0].light, 7 << 12);
}

#[test]
fn surface_conversion_preserves_solid_and_water_bounds() {
    let surface = Surface {
        height: 40,
        state: BlockStateId(1),
        water: Some((63, BlockStateId(2))),
        biome: 7,
    };
    let column = Column::from_surface(Some(surface));
    assert_eq!(column.surface, Some(surface));
    assert_eq!(
        column.spans.as_slice(),
        &[
            Span {
                min: MIN_Y as i16,
                max: 40,
                state: BlockStateId(1),
                biome: 7,
                light: 240,
            },
            Span {
                min: 40,
                max: 63,
                state: BlockStateId(2),
                biome: 7,
                light: 240,
            },
        ]
    );
    assert!(Column::from_surface(None).spans.is_empty());
}

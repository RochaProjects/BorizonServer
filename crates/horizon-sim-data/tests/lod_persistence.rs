use horizon_sim_data::lod_data::{Column, Span, Surface, bake::LodBake, cache::DiskCache};
use horizon_world::registry::BlockStateId;
use std::{fs, sync::Arc};

fn column(seed: i32) -> Column {
    Column {
        surface: Some(Surface {
            height: 70 + seed,
            state: BlockStateId(9),
            water: (seed % 2 == 0).then_some((80, BlockStateId(86))),
            biome: 3,
        }),
        spans: vec![Span {
            min: -64,
            max: (70 + seed) as i16,
            state: BlockStateId(1),
            biome: 3,
            light: 0,
        }]
        .into(),
    }
}

#[test]
fn disk_cache_round_trips_and_uses_its_registry_key() {
    let dir = std::env::temp_dir().join(format!(
        "horizon-lod-cache-integration-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    let columns: Arc<[Column; 256]> = Arc::new(std::array::from_fn(|i| column(i as i32)));
    {
        let cache = DiskCache::new(dir.clone(), 7);
        assert!(cache.get(3, -40).is_none());
        cache.put(3, -40, &columns);
        cache.put(3, -40, &std::array::from_fn(|_| column(999)));
        assert_eq!(*cache.get(3, -40).unwrap(), *columns);
    }
    assert_eq!(
        *DiskCache::new(dir.clone(), 7).get(3, -40).unwrap(),
        *columns
    );
    assert!(DiskCache::new(dir.clone(), 8).get(3, -40).is_none());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lod_bake_round_trips_and_separates_approximate_samples() {
    let world = std::env::temp_dir().join(format!(
        "horizon-lod-bake-integration-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&world);
    let dimension = world.join("dimensions").join("minecraft").join("overworld");
    fs::create_dir_all(&dimension).unwrap();
    let full: Vec<_> = (0..256).map(column).collect();
    let approximate: Vec<_> = (1..257).map(column).collect();
    {
        let bake = LodBake::open(
            &dimension,
            horizon_sim_data::chunk::LevelShape::OVERWORLD,
            100,
            10,
        )
        .unwrap();
        assert!(bake.grid(false, [-64, 32], 4, 16).is_none());
        bake.put_grid(false, [-64, 32], 4, 16, &full);
        bake.put_grid(true, [-64, 32], 4, 16, &approximate);
        assert_eq!(
            bake.grid(false, [-64, 32], 4, 16).as_deref(),
            Some(&full[..])
        );
        assert_eq!(
            bake.grid(true, [-64, 32], 4, 16).as_deref(),
            Some(&approximate[..])
        );
    }
    let bake = LodBake::open(
        &dimension,
        horizon_sim_data::chunk::LevelShape::OVERWORLD,
        100,
        10,
    )
    .unwrap();
    assert_eq!(
        bake.grid(false, [-64, 32], 4, 16).as_deref(),
        Some(&full[..])
    );
    drop(bake);
    fs::remove_dir_all(world).unwrap();
}

#[test]
fn server_folder_preserves_client_cache_path_format() {
    use horizon_sim_data::server_address::server_folder;

    assert_eq!(
        server_folder(" Play.Example.net:25565 "),
        "play.example.net"
    );
    assert_eq!(
        server_folder("play.example.net:25566"),
        "play.example.net_25566"
    );
    assert_eq!(server_folder("[::1]:25570"), "___1__25570");
    assert_eq!(server_folder("../.."), "_");
}

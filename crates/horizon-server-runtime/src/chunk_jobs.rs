//! Server-owned execution of the non-visual chunk-streaming jobs.
//! The renderer still owns queueing and mesh work; these functions are the
//! shared gameplay operations used by that queue and by the dedicated runtime.
use horizon_sim_data::{
    chunk::{Chunk, Light},
    light::{LightTables, light_chunk},
    streaming_api::{ChunkGenerator, Proto},
};
use std::sync::Arc;

/// Immutable 3x3 chunk snapshot used by generation and lighting workers.
pub type ChunkRegion = [[Option<Arc<Chunk>>; 3]; 3];

/// Data decoration writes into already-final chunks in its 3x3 region.
#[derive(Default)]
pub struct FinalWrites {
    pub blocks: Vec<([i32; 3], u32)>,
    pub block_entities: Vec<(
        (i32, i32, i32),
        horizon_sim_data::chunk::block_entity::BlockEntity,
    )>,
}

/// Whether generating a column loaded an existing final chunk or made a proto.
pub enum GeneratedColumn {
    Loaded(Chunk),
    Proto(Proto, bool),
}

/// Work owned by the server's chunk simulation, independent of meshing.
pub enum ServerJob {
    Generate(i32, i32),
    Decorate(i32, i32, Box<[Proto; 9]>, Vec<(usize, Chunk)>),
    Finish(Proto),
    Light(i32, i32, Box<ChunkRegion>, u64, bool),
    Save(Vec<((i32, i32), Proto)>),
    LightView(i32, i32, Proto),
}

/// Result of executing one server-owned chunk job.
pub enum ServerJobDone {
    Generated(Chunk),
    Proto(i32, i32, Proto, bool),
    Decorated(i32, i32, Box<[Proto; 9]>, Vec<usize>, FinalWrites),
    Lit(i32, i32, Vec<Light>, u64),
    Saved(Vec<((i32, i32), Proto)>),
    LightView(i32, i32, Proto, Option<Chunk>),
}

/// Execute one gameplay/save/light job using the existing generator and light
/// algorithms. Queueing and all mesh jobs remain owned by the caller.
pub fn execute_server_job<G: ChunkGenerator>(
    generator: &G,
    worker: &mut G::Worker,
    light: &LightTables,
    job: ServerJob,
) -> ServerJobDone {
    match job {
        ServerJob::Generate(x, z) => match generate_column(generator, worker, x, z) {
            GeneratedColumn::Loaded(chunk) => ServerJobDone::Generated(chunk),
            GeneratedColumn::Proto(proto, decorated) => {
                ServerJobDone::Proto(x, z, proto, decorated)
            }
        },
        ServerJob::Decorate(x, z, region, finals) => {
            let (region, final_indices, writes) =
                decorate_region(generator, worker, region, finals);
            ServerJobDone::Decorated(x, z, region, final_indices, writes)
        }
        ServerJob::Finish(proto) => ServerJobDone::Generated(finish_column(generator, proto)),
        ServerJob::Light(x, z, region, sequence, sky_light) => {
            ServerJobDone::Lit(x, z, light_region(&region, light, sky_light), sequence)
        }
        ServerJob::Save(protos) => ServerJobDone::Saved(save_protos(generator, protos)),
        ServerJob::LightView(x, z, proto) => {
            let (x, z, proto, view) = make_light_view(generator, x, z, proto);
            ServerJobDone::LightView(x, z, proto, view)
        }
    }
}

/// Generate or load one column using the existing vanilla-backed generator.
pub fn generate_column<G: ChunkGenerator>(
    generator: &G,
    worker: &mut G::Worker,
    x: i32,
    z: i32,
) -> GeneratedColumn {
    match generator.load(x, z) {
        Some(chunk) => {
            generator.prepare_loaded(worker, &chunk);
            GeneratedColumn::Loaded(chunk)
        }
        None => {
            let proto = generator.proto(worker, x, z);
            let decorated = generator.is_decorated(&proto);
            GeneratedColumn::Proto(proto, decorated)
        }
    }
}

/// Decorate one region and preserve writes spilling into final saved chunks.
pub fn decorate_region<G: ChunkGenerator>(
    generator: &G,
    worker: &mut G::Worker,
    mut region: Box<[Proto; 9]>,
    finals: Vec<(usize, Chunk)>,
) -> (Box<[Proto; 9]>, Vec<usize>, FinalWrites) {
    for (index, snapshot) in &finals {
        if let Some(proto) = generator.proto_beside_final(worker, snapshot) {
            region[*index] = proto;
        }
    }
    generator.decorate(worker, &mut region);
    let mut writes = FinalWrites::default();
    for (index, snapshot) in &finals {
        let blocks = generator.final_writes(snapshot, &region[*index]);
        writes
            .block_entities
            .extend(generator.final_block_entities(&region[*index], &blocks));
        writes.blocks.extend(blocks);
        region[*index] = Box::new(());
    }
    let final_indices = finals.into_iter().map(|(index, _)| index).collect();
    (region, final_indices, writes)
}

/// Turn a decorated proto into the same final chunk the streamer has always used.
pub fn finish_column<G: ChunkGenerator>(generator: &G, proto: Proto) -> Chunk {
    let mut chunk = generator.finish(proto);
    chunk.generated = true;
    chunk
}

/// Save checked-out proto chunks off-thread and return ownership to the caller.
pub fn save_protos<G: ChunkGenerator>(
    generator: &G,
    protos: Vec<((i32, i32), Proto)>,
) -> Vec<((i32, i32), Proto)> {
    let refs: Vec<&Proto> = protos.iter().map(|(_, proto)| proto).collect();
    generator.save_protos(&refs);
    protos
}

/// Compute the light view while preserving ownership of the proto chunk.
pub fn make_light_view<G: ChunkGenerator>(
    generator: &G,
    x: i32,
    z: i32,
    proto: Proto,
) -> (i32, i32, Proto, Option<Chunk>) {
    let view = generator.light_view(&proto);
    (x, z, proto, view)
}

/// Light a captured neighbourhood with the canonical simulation algorithm.
pub fn light_region(region: &ChunkRegion, tables: &LightTables, sky_light: bool) -> Vec<Light> {
    let borrowed = std::array::from_fn(|z| std::array::from_fn(|x| region[z][x].as_deref()));
    light_chunk(&borrowed, tables, sky_light)
}

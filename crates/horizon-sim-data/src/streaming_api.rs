//! Server-safe contracts for chunk generation and far-terrain sampling.
use crate::{chunk::Chunk, section_mask::SectionMask};
use std::sync::Arc;

/// How long an idle generator worker keeps its reusable scratch buffers.
pub const IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(5);

/// A chunk under generation, opaque to the streamer.
pub type Proto = Box<dyn std::any::Any + Send>;
/// Chunk generation in vanilla's stages: a neighbour-independent proto chunk
/// (biomes, noise, surface, carvers), decoration of a 3x3 neighbourhood
/// (FEATURES, which may write into neighbours), then conversion of a chunk
/// whose eight neighbours are decorated into a final column. Implementations
/// must be thread-safe; each worker of the shared pool
/// (`horizon_threads::jobs`) keeps one `Worker` per streamer.
pub trait ChunkGenerator: Send + Sync + 'static {
    type Worker: Send + 'static;
    fn worker(&self) -> Self::Worker;
    /// The persistent wrapper supplies starts from their owning chunk, in every dimension.
    fn set_saved_starts(
        &self,
        _source: Arc<horizon_world::worldgen::generator::SavedStructureStarts>,
    ) {
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn LodSampler>
    where
        Self: Sized,
    {
        let worker = self.worker();
        Box::new(GeneratorSampler {
            generator: self,
            worker,
        })
    }
    /// An undecorated far-terrain column, without allocating gameplay chunks.
    fn lod_surface(
        &self,
        _worker: &mut Self::Worker,
        _x: i32,
        _z: i32,
    ) -> Option<crate::lod_data::Surface> {
        None
    }
    /// A previously saved final chunk, bypassing generation.
    fn load(&self, _x: i32, _z: i32) -> Option<Chunk> {
        None
    }
    /// What a loaded chunk needs from the generator before the game uses it,
    /// on the worker that loaded it.
    fn prepare_loaded(&self, _worker: &mut Self::Worker, _chunk: &Chunk) {}
    /// The worker has had nothing to do for [`IDLE_RELEASE`]: give back what
    /// it keeps between jobs only to go faster.
    fn idle(&self, _worker: &mut Self::Worker) {}
    /// Every worker has had nothing to do for [`IDLE_RELEASE`]: give back the
    /// caches the workers share, which only spare work for chunks generated
    /// next to each other in a run. Results do not change.
    fn all_idle(&self) {}
    fn proto(&self, worker: &mut Self::Worker, x: i32, z: i32) -> Proto;
    fn decorates(&self) -> bool {
        false
    }
    /// Decorate `region[4]`; the region is row-major (dz, dx) around it.
    fn decorate(&self, _worker: &mut Self::Worker, _region: &mut [Proto; 9]) {}
    /// Whether a chunk next to final ones (loaded from the save, as the
    /// chunks of a world an older version made) is decorated, its features
    /// spilling into them (`proto_beside_final`, `final_writes`), as vanilla's
    /// are; otherwise it is left undecorated.
    fn decorates_beside_final(&self) -> bool {
        false
    }
    /// The proto chunk of a final chunk, for a decoration region to hold: its
    /// blocks are `snapshot`'s.
    fn proto_beside_final(&self, _worker: &mut Self::Worker, _snapshot: &Chunk) -> Option<Proto> {
        None
    }
    /// What decoration wrote into the proto chunk of a final chunk: the blocks
    /// that differ from `snapshot`, as `([x, y, z], state)`.
    fn final_writes(&self, _snapshot: &Chunk, _proto: &Proto) -> Vec<([i32; 3], u32)> {
        Vec::new()
    }
    /// The block entities decoration made at the blocks `writes` put into a
    /// final chunk (a chest of a structure piece, a spawner), with their
    /// contents.
    fn final_block_entities(
        &self,
        _proto: &Proto,
        _writes: &[([i32; 3], u32)],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        Vec::new()
    }
    fn finish(&self, proto: Proto) -> Chunk;
    /// Whether `proto` has been decorated (a saved one can come back decorated).
    fn is_decorated(&self, _proto: &Proto) -> bool {
        false
    }
    /// What to save of a proto chunk dropped before it became final.
    /// A decorated proto chunk's blocks as a column, to light a finished
    /// neighbour against (vanilla lights with neighbours at FEATURES).
    fn light_view(&self, _proto: &Proto) -> Option<Chunk> {
        None
    }
    /// A decorated proto chunk's column sections (`Chunk::sections`
    /// indexes) holding a block other than air: what its neighbours' light
    /// layers count once it is at INITIALIZE_LIGHT.
    fn non_air_sections(&self, _proto: &Proto) -> Option<SectionMask> {
        None
    }
    /// What the generator's shared caches hold: (name, entries, bytes, if
    /// known), for `horizon-ctl memory`.
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        Vec::new()
    }
    /// A proto chunk's heap bytes by part, and its sections by bits per
    /// block, for `horizon-ctl memory` (`ProtoChunk::heap_bytes`).
    fn proto_heap_bytes(&self, _proto: &Proto) -> ([usize; 6], [usize; 16]) {
        ([0; 6], [0; 16])
    }
    fn saved_proto(&self, _proto: &Proto) -> Option<crate::chunk::SavedProto> {
        None
    }
    /// Put a saved proto chunk's state back onto `proto`, generated again.
    fn restore_proto(&self, _proto: &mut Proto, _saved: crate::chunk::SavedProto) {}
    /// Save proto chunks decoration wrote to, so that they come back as they
    /// were: generating them again would lose those writes.
    fn save_protos(&self, _protos: &[&Proto]) {}
    /// Save a column that finished after the player moved away: its
    /// neighbours' decoration is done, so it can't be generated again.
    fn save_finished(&self, _chunk: Chunk) {}
}
pub type LodDetail = ((i32, i32), Arc<[crate::lod_data::Column; 256]>);

/// Independent, thread-owned far-terrain sampling state.
pub trait LodSampler: Send {
    /// Independent sampling state; native detail generation stays with the owner.
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        None
    }
    fn request_detail(&mut self, _chunks: &[(i32, i32)], _center: [i32; 2]) {}
    fn drain_details(&mut self) -> Vec<LodDetail> {
        Vec::new()
    }
    /// A loaded chunk's summary, made on the far-terrain worker.
    fn observed(&mut self, _key: (i32, i32), _columns: &Arc<[crate::lod_data::Column; 256]>) {}
    /// Reduced far detail (`graphics::FarDetail::Reduced`): `grid` may then
    /// approximate wide cells.
    fn set_approximate(&mut self, _approximate: bool) {}
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod_data::Surface>;
    fn column(&mut self, x: i32, z: i32) -> crate::lod_data::Column {
        crate::lod_data::Column::from_surface(self.sample(x, z))
    }
    fn reduced_column(&mut self, x: i32, z: i32, _width: i32) -> crate::lod_data::Column {
        self.column(x, z)
    }
    /// Whether a column since the last call was left empty because its data
    /// is still on its way (a BlueMap tile downloading, nothing coarser to
    /// stand in): that sample isn't drawn, what it would replace is
    /// (`Lod::finish_any_sample`), and it is taken again later.
    fn waited(&mut self) -> bool {
        false
    }
    /// `side` x `side` columns from (`x`, `z`), `stride` blocks apart; none
    /// when sampling failed and should be tried again later.
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod_data::Column> {
        let mut columns = Vec::with_capacity(side * side);
        for dz in 0..side {
            for dx in 0..side {
                columns.push(self.reduced_column(
                    x + dx as i32 * stride,
                    z + dz as i32 * stride,
                    stride,
                ));
            }
        }
        columns
    }
}
pub trait LodSource: Send + Sync {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler>;
    /// Whether its samplers' `observed` chunks become its own (a server's
    /// kept terrain): tiles sampled without one are sampled again.
    fn keeps_observed(&self) -> bool {
        false
    }
}
struct GeneratorSampler<G: ChunkGenerator> {
    generator: Arc<G>,
    worker: G::Worker,
}
impl<G: ChunkGenerator> LodSampler for GeneratorSampler<G> {
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod_data::Surface> {
        self.generator.lod_surface(&mut self.worker, x, z)
    }
}
impl<G: ChunkGenerator> LodSource for G {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        ChunkGenerator::lod_sampler(self)
    }
}

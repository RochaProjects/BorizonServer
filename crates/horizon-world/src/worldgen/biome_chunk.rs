//! Quart-resolution biome storage and section-atomic noise-biome filling.
use crate::heightmap::{Heightmap, HeightmapError, HeightmapType};
use horizon_core::{
    biome::BiomeZoom,
    position::{BlockPos, ChunkPos},
    storage::PackedStorage,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

#[derive(Clone, Debug)]
pub struct BiomeChunk<T> {
    position: ChunkPos,
    min_y: i32,
    height: i32,
    sections: Vec<[T; 64]>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum BiomeFillError<E> {
    InvalidBounds,
    Resolve(E),
}
impl<E: std::fmt::Display> std::fmt::Display for BiomeFillError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBounds => f.write_str("biome generation bounds are outside the chunk"),
            Self::Resolve(error) => error.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BiomeFillError<E> {}
impl<T: Clone> BiomeChunk<T> {
    /// The heap bytes of its quart cells.
    pub fn heap_bytes(&self) -> usize {
        self.sections.capacity() * std::mem::size_of::<[T; 64]>()
    }
    /// A chunk's biome values use the same supported vertical bounds as terrain.
    /// Each section stores a dense 4×4×4 array; palette/network codecs are separate.
    pub fn new(
        position: ChunkPos,
        min_y: i32,
        height: i32,
        default: T,
    ) -> Result<Self, HeightmapError> {
        Heightmap::new(HeightmapType::WorldSurfaceWg, min_y, height)?;
        Ok(Self {
            position,
            min_y,
            height,
            sections: (0..height / 16)
                .map(|_| std::array::from_fn(|_| default.clone()))
                .collect(),
        })
    }
    pub fn section(&self, index: usize) -> Option<&[T; 64]> {
        self.sections.get(index)
    }
    /// Vanilla clamps quart Y to the chunk and wraps X/Z to the local 4×4 grid.
    /// The caller is responsible for selecting the chunk containing X/Z.
    pub fn noise_biome(&self, quart: [i32; 3]) -> &T {
        let bottom = self.min_y >> 2;
        let y = quart[1].clamp(bottom, bottom + (self.height >> 2) - 1);
        let section = ((y - bottom) >> 2) as usize;
        &self.sections[section][index(quart[0], y, quart[2])]
    }
    /// The quart Ys `noise_biome` clamps to: the lowest and the highest.
    pub fn quart_range(&self) -> (i32, i32) {
        let bottom = self.min_y >> 2;
        (bottom, bottom + (self.height >> 2) - 1)
    }
    /// Encode each 4×4×4 quart biome section as the vanilla chunk-NBT `biomes`
    /// palette compound. This is one field adapter, not a complete chunk codec.
    pub fn biome_sections_nbt(
        &self,
        mut name: impl FnMut(&T) -> Option<String>,
    ) -> io::Result<Vec<crate::nbt::Tag>>
    where
        T: Eq,
    {
        self.sections
            .iter()
            .enumerate()
            .map(|(section_index, values)| {
                let mut palette = Vec::<(&T, String)>::new();
                let mut cell_ids = Vec::with_capacity(64);
                for value in values {
                    let id = if let Some(id) = palette.iter().position(|(known, _)| *known == value)
                    {
                        id
                    } else {
                        let identifier = name(value).ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidData, "unknown biome identity")
                        })?;
                        palette.push((value, identifier));
                        palette.len() - 1
                    };
                    cell_ids.push(id as u32);
                }
                let bits = if palette.len() <= 1 {
                    0
                } else {
                    (usize::BITS - (palette.len() - 1).leading_zeros()).max(1) as u8
                };
                let mut biome_palette = crate::nbt::Tag::compound([(
                    "palette",
                    crate::nbt::Tag::List(
                        palette
                            .iter()
                            .map(|(_, id)| crate::nbt::Tag::String(id.as_str().into()))
                            .collect(),
                    ),
                )]);
                if bits > 0 {
                    let mut storage = PackedStorage::new(bits, 64).map_err(|error| {
                        io::Error::new(io::ErrorKind::InvalidData, error.to_string())
                    })?;
                    for (index, id) in cell_ids.into_iter().enumerate() {
                        storage.set(index, id).map_err(|error| {
                            io::Error::new(io::ErrorKind::InvalidData, error.to_string())
                        })?;
                    }
                    let crate::nbt::Tag::Compound(fields) = &mut biome_palette else {
                        unreachable!()
                    };
                    fields.insert(
                        "data".into(),
                        crate::nbt::Tag::LongArray(
                            storage.words().iter().map(|&word| word as i64).collect(),
                        ),
                    );
                }
                let section_y = self.min_y / 16 + section_index as i32;
                let y = i8::try_from(section_y).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "biome section Y out of NBT range",
                    )
                })?;
                Ok(crate::nbt::Tag::compound([
                    ("Y", crate::nbt::Tag::Byte(y)),
                    ("biomes", biome_palette),
                ]))
            })
            .collect()
    }
    /// Fill the sections intersecting inclusive generation bounds. Sections are
    /// visited bottom to top; each resolver sees X/Y/Z loop order. A failed
    /// section retains its previous values while earlier sections stay committed.
    /// This does not decorate the resolver with blending or below-zero retrogen.
    pub fn fill<E>(
        &mut self,
        min_y: i32,
        max_y: i32,
        mut resolve: impl FnMut([i32; 3]) -> Result<T, E>,
    ) -> Result<(), BiomeFillError<E>> {
        self.fill_using_chunk(min_y, max_y, |quart, _| resolve(quart))
    }
    /// Decorate candidate sampling with the pinned below-zero biome policy.
    /// `upgrading` is captured once for this fill, as at native resolver creation.
    /// Keys describe holder identity; unkeyed/direct values must return `None`.
    pub fn fill_with_retrogen<E>(
        &mut self,
        min_y: i32,
        max_y: i32,
        upgrading: bool,
        resolve: impl FnMut([i32; 3]) -> Result<T, E>,
        biome_key: impl Fn(&T) -> Option<&str>,
    ) -> Result<(), BiomeFillError<E>> {
        let mut resolver = crate::retrogen::RetrogenBiomeResolver::new(upgrading, resolve);
        self.fill_using_chunk(min_y, max_y, |quart, chunk| {
            resolver.sample(quart, &biome_key, |old| Ok(chunk.noise_biome(old).clone()))
        })
    }
    fn fill_using_chunk<E>(
        &mut self,
        min_y: i32,
        max_y: i32,
        mut resolve: impl FnMut([i32; 3], &Self) -> Result<T, E>,
    ) -> Result<(), BiomeFillError<E>> {
        if min_y < self.min_y || max_y >= self.min_y + self.height || min_y > max_y {
            return Err(BiomeFillError::InvalidBounds);
        }
        let x = self.position.x.wrapping_shl(4) >> 2;
        let z = self.position.z.wrapping_shl(4) >> 2;
        for section_y in (min_y >> 4)..=(max_y >> 4) {
            let section = (section_y - (self.min_y >> 4)) as usize;
            let mut values = self.sections[section].clone();
            for dx in 0..4 {
                for dy in 0..4 {
                    for dz in 0..4 {
                        let quart = [
                            x.wrapping_add(dx),
                            section_y.wrapping_shl(2).wrapping_add(dy),
                            z.wrapping_add(dz),
                        ];
                        values[index(dx, dy, dz)] =
                            resolve(quart, self).map_err(BiomeFillError::Resolve)?;
                    }
                }
            }
            self.sections[section] = values;
        }
        Ok(())
    }
}
fn index(x: i32, y: i32, z: i32) -> usize {
    (((y & 3) << 4) | ((z & 3) << 2) | (x & 3)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::Tag;

    #[test]
    fn biome_section_nbt_uses_stable_palette_and_quart_cell_order() {
        let position = ChunkPos::new(-3, 5);
        let mut chunk = BiomeChunk::new(position, -64, 384, 0u8).unwrap();
        chunk
            .fill(-64, -49, |[x, _, _]| Ok::<_, ()>((x & 1) as u8))
            .unwrap();

        let sections = chunk
            .biome_sections_nbt(|id| {
                Some(
                    if *id == 0 {
                        "minecraft:plains"
                    } else {
                        "minecraft:forest"
                    }
                    .into(),
                )
            })
            .unwrap();
        assert_eq!(sections.len(), 24);
        assert_eq!(sections[0].get("Y"), Some(&Tag::Byte(-4)));
        let Some(Tag::Compound(biomes)) = sections[0].get("biomes") else {
            panic!("missing biome palette compound");
        };
        assert_eq!(
            biomes.get(&"palette".into()),
            Some(&Tag::List(vec![
                Tag::String("minecraft:plains".into()),
                Tag::String("minecraft:forest".into()),
            ]))
        );
        let Some(Tag::LongArray(words)) = biomes.get(&"data".into()) else {
            panic!("mixed biome palette must carry packed data");
        };
        let storage =
            PackedStorage::from_words(1, 64, words.iter().map(|&word| word as u64).collect())
                .unwrap();
        let quart_min = -16;
        for dy in 0..4 {
            for dz in 0..4 {
                for dx in 0..4 {
                    let cell = index(dx, dy, dz);
                    let name = if storage.get(cell).unwrap() == 0 {
                        "minecraft:plains"
                    } else {
                        "minecraft:forest"
                    };
                    assert_eq!(
                        name,
                        if ((-12 + dx) & 1) == 0 {
                            "minecraft:plains"
                        } else {
                            "minecraft:forest"
                        },
                        "quart at y {} (range begins {quart_min})",
                        quart_min + dy
                    );
                }
            }
        }
        assert!(sections[1..].iter().all(|section| {
            matches!(
                section.get("biomes"),
                Some(Tag::Compound(map)) if map.get(&"palette".into()) == Some(&Tag::List(vec![Tag::String("minecraft:plains".into())])) && !map.contains_key(&"data".into())
            )
        }));
    }
}

/// Resident biome sections shared by decoration selection and block biome zoom.
/// The seed is already obfuscated, as for `BiomeZoom` and the client protocol.
pub struct BiomeRegion<T> {
    chunks: BTreeMap<(i32, i32), BiomeChunk<T>>,
    zoom: BiomeZoom,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiomeRegionError {
    InvalidChunks,
    MissingChunk(ChunkPos),
}
impl std::fmt::Display for BiomeRegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for BiomeRegionError {}
impl<T: Clone> BiomeRegion<T> {
    pub fn new(chunks: Vec<BiomeChunk<T>>, obfuscated_seed: i64) -> Result<Self, BiomeRegionError> {
        if chunks.is_empty() || chunks.len() > 4096 {
            return Err(BiomeRegionError::InvalidChunks);
        }
        let min = chunks[0].min_y;
        let height = chunks[0].height;
        let mut stored = BTreeMap::new();
        for chunk in chunks {
            if chunk.min_y != min || chunk.height != height {
                return Err(BiomeRegionError::InvalidChunks);
            }
            if stored
                .insert((chunk.position.x, chunk.position.z), chunk)
                .is_some()
            {
                return Err(BiomeRegionError::InvalidChunks);
            }
        }
        Ok(Self {
            chunks: stored,
            zoom: BiomeZoom::new(obfuscated_seed),
        })
    }
    pub fn chunk(&self, p: ChunkPos) -> Result<&BiomeChunk<T>, BiomeRegionError> {
        self.chunks
            .get(&(p.x, p.z))
            .ok_or(BiomeRegionError::MissingChunk(p))
    }
    pub fn chunk_mut(&mut self, p: ChunkPos) -> Result<&mut BiomeChunk<T>, BiomeRegionError> {
        self.chunks
            .get_mut(&(p.x, p.z))
            .ok_or(BiomeRegionError::MissingChunk(p))
    }
    pub fn noise_biome(&self, quart: [i32; 3]) -> Result<&T, BiomeRegionError> {
        Ok(self
            .chunk(ChunkPos::new(quart[0] >> 2, quart[2] >> 2))?
            .noise_biome(quart))
    }
    pub fn biome(&self, block: BlockPos) -> Result<&T, BiomeRegionError> {
        self.noise_biome(self.zoom.quart_at_block(block))
    }
    pub fn quart_at_block(&self, block: BlockPos) -> [i32; 3] {
        self.zoom.quart_at_block(block)
    }
    pub fn zoom(&self) -> BiomeZoom {
        self.zoom
    }
    /// Distinct values currently stored anywhere in the chunk. The result is
    /// sorted by biome identity; callers must not use this order as registry order.
    pub fn present_in_chunk(&self, p: ChunkPos) -> Result<Vec<T>, BiomeRegionError>
    where
        T: Ord,
    {
        Ok(self
            .chunk(p)?
            .sections
            .iter()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
}

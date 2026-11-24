//! Compact far-terrain data shared by the dedicated server and client renderer.
use crate::chunk::{Chunk, MIN_Y};
use horizon_world::registry::BlockStateId;

pub mod bake;
pub mod cache;

/// Bits in the cached block-state facts consumed by far-terrain summarization.
#[doc(hidden)]
pub const KEPT: u8 = 1;
#[doc(hidden)]
pub const FLUID: u8 = 2;
#[doc(hidden)]
pub const AIR: u8 = 4;
#[doc(hidden)]
pub const WATERY: u8 = 8;

/// Top solid face and optional fluid face, in absolute block coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: u16,
}
/// A half-open material run; gaps between runs remain air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub min: i16,
    pub max: i16,
    pub state: BlockStateId,
    pub biome: u16,
    pub light: u16,
}
/// A column's material runs. Almost every far column has one to four, so
/// they live inline: the sampler builds millions of columns and a heap
/// allocation each dominated its memory and allocator time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spans(smallvec::SmallVec<[Span; 4]>);
impl Spans {
    pub const fn new() -> Self {
        Self(smallvec::SmallVec::new_const())
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self(smallvec::SmallVec::with_capacity(capacity))
    }
    pub fn push(&mut self, span: Span) {
        self.0.push(span);
    }
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }
    pub fn as_slice(&self) -> &[Span] {
        &self.0
    }
}
impl std::ops::Deref for Spans {
    type Target = [Span];
    fn deref(&self) -> &[Span] {
        &self.0
    }
}
impl std::ops::DerefMut for Spans {
    fn deref_mut(&mut self) -> &mut [Span] {
        &mut self.0
    }
}
impl From<Vec<Span>> for Spans {
    fn from(spans: Vec<Span>) -> Self {
        Self(spans.into())
    }
}
impl FromIterator<Span> for Spans {
    fn from_iter<I: IntoIterator<Item = Span>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}
impl IntoIterator for Spans {
    type Item = Span;
    type IntoIter = smallvec::IntoIter<[Span; 4]>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a Spans {
    type Item = &'a Span;
    type IntoIter = std::slice::Iter<'a, Span>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl<'a> IntoIterator for &'a mut Spans {
    type Item = &'a mut Span;
    type IntoIter = std::slice::IterMut<'a, Span>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}
impl<const N: usize> PartialEq<[Span; N]> for Spans {
    fn eq(&self, other: &[Span; N]) -> bool {
        self.as_slice() == other
    }
}
impl PartialEq<Vec<Span>> for Spans {
    fn eq(&self, other: &Vec<Span>) -> bool {
        self.as_slice() == other.as_slice()
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub surface: Option<Surface>,
    pub spans: Spans,
}
impl Column {
    /// Reduce an entire horizontal footprint, rather than inflating its corner
    /// column. A height is drawn where any column of the footprint holds a
    /// block there, in the material most of them hold: anything joined to
    /// the ground stays joined at every width. Drawing only heights most of
    /// the footprint held dropped trunks (one column of 16 or 256) under the
    /// crowns they hold up, and the crowns hung in the air as slabs.
    pub fn reduced(columns: &[Column; 256], x: usize, z: usize, width: usize) -> Self {
        if width == 1 {
            return columns[z * 16 + x].clone();
        }
        let mut events = Vec::new();
        for dz in z..z + width {
            for dx in x..x + width {
                for span in &columns[dz * 16 + dx].spans {
                    let material = (span.state, span.biome, span.light);
                    events.push((span.min, material, 1));
                    events.push((span.max, material, -1));
                }
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let mut result = Self::default();
        let mut votes = Vec::<((BlockStateId, u16), i32)>::new();
        let mut sky = [0i32; 16];
        let mut block = [0i32; 16];
        let mut occupied = 0;
        let mut bottom = MIN_Y as i16;
        for (y, material, change) in events {
            if bottom < y {
                if occupied > 0
                    && let Some(&((state, biome), _)) =
                        votes.iter().max_by_key(|&(material, count)| {
                            (*count, std::cmp::Reverse((material.0.0, material.1)))
                        })
                {
                    // Lighting must not split votes for the same material.
                    // Reduction can expose buried faces; retain the brightest
                    // actual light in this occupied footprint, per channel.
                    let light = (sky.iter().rposition(|&n| n > 0).unwrap_or(0) << 4)
                        | (block.iter().rposition(|&n| n > 0).unwrap_or(0) << 12);
                    result.push(Span {
                        min: bottom,
                        max: y,
                        state,
                        biome,
                        light: light as u16,
                    });
                }
                bottom = y;
            }
            occupied += change;
            sky[usize::from((material.2 >> 4) & 15)] += change;
            block[usize::from((material.2 >> 12) & 15)] += change;
            let material = (material.0, material.1);
            if let Some(vote) = votes.iter_mut().find(|vote| vote.0 == material) {
                vote.1 += change;
            } else {
                votes.push((material, change));
            }
        }
        result
    }
    pub fn from_surface(surface: Option<Surface>) -> Self {
        let mut column = Self {
            surface,
            spans: Spans::new(),
        };
        if let Some(s) = surface {
            if s.height > MIN_Y {
                column.spans.push(Span {
                    min: MIN_Y as i16,
                    max: s.height as i16,
                    state: s.state,
                    biome: s.biome,
                    light: 240,
                });
            }
            if let Some((height, state)) = s.water {
                column.spans.push(Span {
                    min: s.height as i16,
                    max: height as i16,
                    state,
                    biome: s.biome,
                    light: 240,
                });
            }
        }
        column
    }
    pub fn push(&mut self, span: Span) {
        if let Some(last) = self.spans.last_mut()
            && last.max == span.min
            && last.state == span.state
            && last.biome == span.biome
            && last.light == span.light
        {
            last.max = span.max;
        } else {
            self.spans.push(span);
        }
    }
    /// `kinds`: `state_kinds`, per block state; `water`: `water_source(kinds)`,
    /// what blocks holding water (kelp, seagrass, waterlogged ones) are drawn as.
    pub fn from_chunk(
        chunk: &Chunk,
        kinds: &[u8],
        water: Option<BlockStateId>,
        x: i32,
        z: i32,
    ) -> Self {
        let kind = |state: BlockStateId| kinds.get(state.0 as usize).copied().unwrap_or(0);
        let face_light = |y: i32| {
            let mut sky = 0;
            let mut block = 0;
            for (dx, dy, dz) in [
                (0, 0, 0),
                (-1, 0, 0),
                (1, 0, 0),
                (0, -1, 0),
                (0, 1, 0),
                (0, 0, -1),
                (0, 0, 1),
            ] {
                if !(0..16).contains(&(x + dx)) || !(0..16).contains(&(z + dz)) {
                    continue;
                }
                let value = chunk.light(x + dx, y + dy, z + dz);
                sky = sky.max(value & 0xf0);
                block = block.max(value & 15);
            }
            sky | block
        };
        let mut column = Self::default();
        for (sy, section) in chunk.sections.iter().enumerate() {
            let min = chunk.min_y() + sy as i32 * 16;
            let uniform = section.blocks.bits() == 0
                && section.biomes.bits() == 0
                && matches!(section.light, crate::chunk::Light::Uniform(_));
            let uniform = uniform
                && face_light(min) == chunk.light(x, min, z)
                && face_light(min + 15) == chunk.light(x, min, z);
            let step = if uniform { 16 } else { 1 };
            for offset in (0..16).step_by(step) {
                let y = min + offset;
                let mut state = BlockStateId(chunk.get(x, y, z));
                let mut kind = kind(state);
                if kind & KEPT == 0 {
                    continue;
                }
                if kind & WATERY != 0
                    && let Some(water) = water
                {
                    state = water;
                    kind = kinds[water.0 as usize];
                }
                let light = face_light(y);
                column.push(Span {
                    min: y as i16,
                    max: (y + step as i32) as i16,
                    state,
                    biome: chunk.biome(x, y, z),
                    light: u16::from(light & 0xf0) | (u16::from((light & 15).max(kind >> 4)) << 12),
                });
            }
        }
        let mut water = None;
        for span in column.spans.iter().rev() {
            if kind(span.state) & FLUID != 0 {
                water.get_or_insert((i32::from(span.max), span.state));
            } else {
                column.surface = Some(Surface {
                    height: i32::from(span.max),
                    state: span.state,
                    water,
                    biome: span.biome,
                });
                break;
            }
        }
        if column.surface.is_none()
            && let Some((height, state)) = water
        {
            column.surface = Some(Surface {
                height: chunk.min_y(),
                state: BlockStateId(
                    kinds.iter().position(|&kind| kind & AIR != 0).unwrap_or(0) as u32
                ),
                water: Some((height, state)),
                biome: chunk.biome(x, height - 1, z),
            });
        }
        column
    }
}

//! Server-owned light-shape facts and pairwise face occlusion.
//!
//! The mask table is exported from vanilla 26.3's `BlockState.getFaceOcclusionShape`.
//! The Java exporter checks every unique face-shape pair against
//! `Shapes.faceShapeOccludes` before the table can be used here.

use horizon_core::geometry::Direction;
use horizon_protocol::{buf::set_bit, chunk::LightData};
use horizon_world::{
    level::Behaviours,
    registry::{BlockRegistry, BlockStateId},
    worldgen::generator::ProtoChunk,
};
use serde_json::Value;
use std::{collections::VecDeque, fs, path::Path};

const REGION_SIDE: usize = 48;
const WORLD_MIN_Y: i32 = -64;
const WORLD_HEIGHT: usize = 384;
const WORLD_MAX_Y: i32 = WORLD_MIN_Y + WORLD_HEIGHT as i32;
const GRID_VOLUME: usize = REGION_SIDE * REGION_SIDE * WORLD_HEIGHT;

#[derive(Clone, Debug)]
pub(crate) struct LightOcclusion {
    masks_by_axis: [Vec<[u64; 4]>; 3],
    state_faces: Vec<[u32; 6]>,
}

impl LightOcclusion {
    pub(crate) fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("light-occlusion.json");
        let bytes = fs::read(&path).map_err(|error| {
            format!("read server light occlusion ({}): {error}", path.display())
        })?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse server light occlusion: {error}"))?;
        Self::from_value(value)
    }

    fn from_value(value: Value) -> Result<Self, String> {
        if value.get("format").and_then(Value::as_u64) != Some(1) {
            return Err("unsupported light-occlusion.json format".into());
        }
        let axes = value
            .get("faces_by_axis")
            .and_then(Value::as_array)
            .filter(|axes| axes.len() == 3)
            .ok_or("light-occlusion.json must contain three face axes")?;
        let mut masks_by_axis: [Vec<[u64; 4]>; 3] = std::array::from_fn(|_| Vec::new());
        for (axis, masks) in axes.iter().enumerate() {
            let masks = masks.as_array().ok_or("light face masks must be arrays")?;
            if masks.is_empty() {
                return Err(format!("light face axis {axis} has no masks"));
            }
            for mask in masks {
                let text = mask
                    .as_str()
                    .filter(|text| text.len() == 64 && text.is_ascii())
                    .ok_or("a light face mask must be 64 ASCII hex digits")?;
                let mut words = [0_u64; 4];
                for (index, word) in words.iter_mut().enumerate() {
                    *word = u64::from_str_radix(&text[index * 16..index * 16 + 16], 16)
                        .map_err(|_| "a light face mask contains a non-hex digit")?;
                }
                masks_by_axis[axis].push(words);
            }
        }

        let states = value
            .get("states")
            .and_then(Value::as_array)
            .ok_or("light-occlusion.json has no state table")?;
        let mut state_faces = Vec::with_capacity(states.len());
        for (state_id, faces) in states.iter().enumerate() {
            let faces = faces
                .as_array()
                .filter(|faces| faces.len() == 6)
                .ok_or_else(|| format!("light state {state_id} must have six face ids"))?;
            let mut row = [0_u32; 6];
            for (face, id) in faces.iter().enumerate() {
                let id = id
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or_else(|| format!("light state {state_id} has an invalid face id"))?;
                let axis = direction_axis(face);
                if id as usize >= masks_by_axis[axis].len() {
                    return Err(format!(
                        "light state {state_id} face {face} references absent axis-{axis} mask {id}"
                    ));
                }
                row[face] = id;
            }
            state_faces.push(row);
        }
        if state_faces.is_empty() {
            return Err("light-occlusion.json has no block states".into());
        }
        Ok(Self {
            masks_by_axis,
            state_faces,
        })
    }

    /// Whether the two adjacent vanilla block faces completely occlude light.
    pub(crate) fn occludes(&self, first: u32, second: u32, direction: Direction) -> bool {
        let Some(first_faces) = self.state_faces.get(first as usize) else {
            return false;
        };
        let Some(second_faces) = self.state_faces.get(second as usize) else {
            return false;
        };
        let face = direction as usize;
        let opposite = direction.opposite() as usize;
        let masks = &self.masks_by_axis[direction_axis(face)];
        let a = masks[first_faces[face] as usize];
        let b = masks[second_faces[opposite] as usize];
        a.into_iter().zip(b).all(|(a, b)| a | b == u64::MAX)
    }

    pub(crate) fn state_count(&self) -> usize {
        self.state_faces.len()
    }

    /// Calculate Overworld block and sky light for the center chunk of a 3×3
    /// generated region. A one-chunk border is sufficient because light can
    /// attenuate at most 15 levels before it has no effect at the center.
    pub(crate) fn light_overworld_region(
        &self,
        chunks: &[ProtoChunk; 9],
        registry: &BlockRegistry,
        behaviours: &Behaviours,
    ) -> Result<[LightData; 9], String> {
        if self.state_count() != registry.len() {
            return Err("light table and block registry state counts differ".into());
        }
        let mut states = vec![0_u32; GRID_VOLUME];
        for chunk_z in 0..3 {
            for chunk_x in 0..3 {
                let chunk = &chunks[chunk_z * 3 + chunk_x];
                for y in WORLD_MIN_Y..WORLD_MAX_Y {
                    for z in 0..16 {
                        for x in 0..16 {
                            let gx = chunk_x * 16 + x;
                            let gz = chunk_z * 16 + z;
                            states[grid_index(gx, (y - WORLD_MIN_Y) as usize, gz)] =
                                chunk.block(x as i32, y, z as i32).0;
                        }
                    }
                }
            }
        }

        let mut dampening = Vec::with_capacity(registry.len());
        let mut opacity = Vec::with_capacity(registry.len());
        let mut emission = Vec::with_capacity(registry.len());
        for index in 0..registry.len() {
            let facts = behaviours.facts(BlockStateId(index as u32));
            dampening.push(facts.dampening);
            opacity.push(facts.dampening.max(1));
            emission.push(facts.emission.min(15));
        }
        if let Some(state) = states
            .iter()
            .find(|&&state| state as usize >= registry.len())
        {
            return Err(format!(
                "generated chunk references absent block state {state}"
            ));
        }

        let mut block = vec![0_u8; GRID_VOLUME];
        let mut block_queue = VecDeque::new();
        for (index, &state) in states.iter().enumerate() {
            let value = emission[state as usize];
            if value != 0 {
                block[index] = value;
                block_queue.push_back(index);
            }
        }
        propagate(&states, &opacity, &mut block, &mut block_queue, self);

        let air = registry
            .default_state("minecraft:air")
            .ok_or("block registry is missing minecraft:air")?
            .0;
        let mut sky = vec![0_u8; GRID_VOLUME];
        let mut sky_queue = VecDeque::new();
        for z in 0..REGION_SIDE {
            for x in 0..REGION_SIDE {
                let mut above = air;
                let mut incoming = 15_u8;
                for y in (0..WORLD_HEIGHT).rev() {
                    let index = grid_index(x, y, z);
                    let state = states[index];
                    if self.occludes(above, state, Direction::Down) {
                        break;
                    }
                    // A vertical sky ray loses the state's actual opacity; air-like
                    // states have opacity zero and therefore preserve level 15.
                    incoming = incoming.saturating_sub(dampening[state as usize].min(15));
                    if incoming == 0 {
                        break;
                    }
                    sky[index] = incoming;
                    sky_queue.push_back(index);
                    above = state;
                }
            }
        }
        propagate(&states, &opacity, &mut sky, &mut sky_queue, self);

        Ok(std::array::from_fn(|index| {
            encode_region_light(&sky, &block, index % 3, index / 3)
        }))
    }
}

#[cfg(test)]
fn direct_sky_step(incoming: u8, opacity: u8) -> u8 {
    incoming.saturating_sub(opacity.min(15))
}

fn direction_axis(face: usize) -> usize {
    match face {
        0 | 1 => 1,
        2 | 3 => 2,
        4 | 5 => 0,
        _ => unreachable!("six Minecraft block faces"),
    }
}

fn grid_index(x: usize, y: usize, z: usize) -> usize {
    (y * REGION_SIDE + z) * REGION_SIDE + x
}

fn propagate(
    states: &[u32],
    opacity: &[u8],
    light: &mut [u8],
    queue: &mut VecDeque<usize>,
    occlusion: &LightOcclusion,
) {
    const NEIGHBORS: [(i32, i32, i32, Direction); 6] = [
        (0, -1, 0, Direction::Down),
        (0, 1, 0, Direction::Up),
        (0, 0, -1, Direction::North),
        (0, 0, 1, Direction::South),
        (-1, 0, 0, Direction::West),
        (1, 0, 0, Direction::East),
    ];
    while let Some(index) = queue.pop_front() {
        let current = light[index];
        if current <= 1 {
            continue;
        }
        let x = index % REGION_SIDE;
        let z = (index / REGION_SIDE) % REGION_SIDE;
        let y = index / (REGION_SIDE * REGION_SIDE);
        let source = states[index];
        for (dx, dy, dz, direction) in NEIGHBORS {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            let nz = z as i32 + dz;
            if !(0..REGION_SIDE as i32).contains(&nx)
                || !(0..WORLD_HEIGHT as i32).contains(&ny)
                || !(0..REGION_SIDE as i32).contains(&nz)
            {
                continue;
            }
            let target = grid_index(nx as usize, ny as usize, nz as usize);
            let state = states[target];
            if occlusion.occludes(source, state, direction) {
                continue;
            }
            let value = current.saturating_sub(opacity[state as usize]);
            if value > light[target] {
                light[target] = value;
                queue.push_back(target);
            }
        }
    }
}

fn encode_region_light(sky: &[u8], block: &[u8], chunk_x: usize, chunk_z: usize) -> LightData {
    let mut data = LightData {
        sky_mask: Vec::new(),
        block_mask: Vec::new(),
        empty_sky_mask: Vec::new(),
        empty_block_mask: Vec::new(),
        sky: Vec::new(),
        block: Vec::new(),
    };
    // Light section 0 is below the Overworld; 25 is above its 24 block sections.
    for light_section in 0..26 {
        let (sky_layer, block_layer) = match light_section {
            0 => (vec![0_u8; 2048], vec![0_u8; 2048]),
            25 => (vec![0xff_u8; 2048], vec![0_u8; 2048]),
            section => {
                let block_section = section - 1;
                (
                    pack_region_layer(sky, block_section, chunk_x, chunk_z),
                    pack_region_layer(block, block_section, chunk_x, chunk_z),
                )
            }
        };
        let sky_has_values = sky_layer.iter().any(|&byte| byte != 0);
        let block_has_values = block_layer.iter().any(|&byte| byte != 0);
        if sky_has_values {
            set_bit(&mut data.sky_mask, light_section);
            data.sky.push(sky_layer);
        } else {
            set_bit(&mut data.empty_sky_mask, light_section);
        }
        if block_has_values {
            set_bit(&mut data.block_mask, light_section);
            data.block.push(block_layer);
        } else {
            set_bit(&mut data.empty_block_mask, light_section);
        }
    }
    data
}

fn pack_region_layer(light: &[u8], section: usize, chunk_x: usize, chunk_z: usize) -> Vec<u8> {
    let mut layer = vec![0_u8; 2048];
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                let gx = chunk_x * 16 + x;
                let gy = section * 16 + y;
                let gz = chunk_z * 16 + z;
                let value = light[grid_index(gx, gy, gz)].min(15);
                let index = (y << 8) | (z << 4) | x;
                if index & 1 == 0 {
                    layer[index >> 1] |= value;
                } else {
                    layer[index >> 1] |= value << 4;
                }
            }
        }
    }
    layer
}

#[cfg(test)]
mod tests {
    use super::{LightOcclusion, direct_sky_step};
    use horizon_core::geometry::Direction;

    fn table() -> LightOcclusion {
        let empty = "0000000000000000000000000000000000000000000000000000000000000000";
        let full = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let left = "5555555555555555555555555555555555555555555555555555555555555555";
        let right = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let json = format!(
            r#"{{"format":1,"faces_by_axis":[["{empty}","{full}","{left}","{right}"],["{empty}","{full}","{left}","{right}"],["{empty}","{full}","{left}","{right}"]],"states":[[0,0,0,0,0,0],[1,1,1,1,1,1],[2,2,2,2,2,2],[3,3,3,3,3,3]]}}"#
        );
        LightOcclusion::from_value(serde_json::from_str(&json).unwrap()).unwrap()
    }

    #[test]
    fn combines_adjacent_vanilla_face_masks() {
        let table = table();
        assert!(table.occludes(1, 0, Direction::East));
        assert!(table.occludes(2, 3, Direction::East));
        assert!(!table.occludes(2, 2, Direction::East));
        assert_eq!(table.state_count(), 4);
    }

    #[test]
    fn direct_sky_preserves_zero_opacity_and_applies_block_opacity() {
        assert_eq!(direct_sky_step(15, 0), 15);
        assert_eq!(direct_sky_step(15, 1), 14);
        assert_eq!(direct_sky_step(1, 1), 0);
        assert_eq!(direct_sky_step(15, 15), 0);
    }
}

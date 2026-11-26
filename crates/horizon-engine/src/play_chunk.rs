//! Conversion of generated server chunks into the vanilla 26.3 PLAY payload.

use horizon_protocol::chunk::{self, Paletted, Section, Strategy};
use horizon_world::{
    level::{Behaviours, data::state_flags},
    nbt::{JavaString, Tag},
    registry::BlockRegistry,
    worldgen::generator::ProtoChunk,
};

const MIN_SECTION_Y: i32 = -4;
const SECTION_COUNT: usize = 24;
const BLOCK_BITS: u8 = 16;
const BIOME_BITS: u8 = 7;

/// Encode block, biome and heightmap fields for a generated Overworld chunk.
/// The caller builds the matching light payload separately. Block-entity
/// packets and a resident `Full` chunk lifecycle are not implemented here.
pub(crate) fn encode_generated_chunk(
    proto: &ProtoChunk,
    registry: &BlockRegistry,
    behaviours: &Behaviours,
) -> Result<chunk::ChunkData, String> {
    let sections_tag = proto
        .sections_nbt()
        .map_err(|error| format!("encode generated section NBT: {error}"))?;
    let Tag::List(saved_sections) = sections_tag else {
        return Err("generated sections are not an NBT list".into());
    };
    let mut sections_by_y = std::collections::BTreeMap::new();
    for saved in saved_sections {
        let Some(Tag::Byte(y)) = saved.get("Y") else {
            return Err("generated section has no byte Y".into());
        };
        let y = i32::from(*y);
        if sections_by_y.insert(y, saved).is_some() {
            return Err(format!("duplicate generated section Y={y}"));
        }
    }

    let mut wire_sections = Vec::with_capacity(SECTION_COUNT);
    for section_y in MIN_SECTION_Y..MIN_SECTION_Y + SECTION_COUNT as i32 {
        let Some(saved) = sections_by_y.get(&section_y) else {
            return Err(format!("generated chunk is missing section Y={section_y}"));
        };
        let block_tag = saved
            .get("block_states")
            .ok_or_else(|| format!("section Y={section_y} has no block_states"))?;
        let block_palette =
            horizon_world::chunk::PalettedContainer::from_nbt(block_tag, chunk::BLOCKS, 4)
                .map_err(|error| format!("read block palette at section Y={section_y}: {error}"))?;

        let mut states = Vec::with_capacity(chunk::BLOCKS);
        let mut non_empty = 0_i16;
        let mut fluid_count = 0_i16;
        for index in 0..chunk::BLOCKS {
            let tag = block_palette
                .get(index)
                .map_err(|error| format!("read block {index} at section Y={section_y}: {error}"))?;
            let state = registry
                .saved_state_id(tag)
                .ok_or_else(|| format!("unrecognized block state in section Y={section_y}"))?;
            let facts = behaviours.facts(state);
            if facts.flags & state_flags::AIR == 0 {
                non_empty += 1;
            }
            if facts.fluid != horizon_world::fluid::FluidState::EMPTY {
                fluid_count += 1;
            }
            states.push(state.0 as i32);
        }

        let biome_tag = saved
            .get("biomes")
            .ok_or_else(|| format!("section Y={section_y} has no biomes"))?;
        let biome_palette =
            horizon_world::chunk::PalettedContainer::from_nbt(biome_tag, chunk::BIOMES, 1)
                .map_err(|error| format!("read biome palette at section Y={section_y}: {error}"))?;
        let mut biomes = Vec::with_capacity(chunk::BIOMES);
        for index in 0..chunk::BIOMES {
            let tag = biome_palette
                .get(index)
                .map_err(|error| format!("read biome {index} at section Y={section_y}: {error}"))?;
            let Tag::String(name) = tag else {
                return Err(format!(
                    "non-string biome palette entry at section Y={section_y}"
                ));
            };
            let name = name
                .to_string()
                .map_err(|error| format!("decode biome id at section Y={section_y}: {error}"))?;
            let id = proto
                .data()
                .biome_id(&name)
                .ok_or_else(|| format!("biome `{name}` is absent from the loaded registry"))?;
            biomes.push(id.0 as i32);
        }

        wire_sections.push(Section {
            non_empty,
            fluids: fluid_count,
            states: Paletted::from_values(&states, Strategy::Blocks, BLOCK_BITS),
            biomes: Paletted::from_values(&biomes, Strategy::Biomes, BIOME_BITS),
        });
    }

    let heightmap_tag = proto
        .terrain
        .heightmaps_nbt()
        .map_err(|error| format!("encode generated heightmaps: {error}"))?;
    let Tag::Compound(heightmap_compound) = heightmap_tag else {
        return Err("generated heightmaps are not an NBT compound".into());
    };
    let mut heightmaps = Vec::with_capacity(3);
    for (name, id) in [
        ("WORLD_SURFACE_WG", 1),
        ("WORLD_SURFACE", 1),
        ("MOTION_BLOCKING", 4),
        ("MOTION_BLOCKING_NO_LEAVES", 5),
    ] {
        let name = JavaString::from(name);
        let Some(Tag::LongArray(values)) = heightmap_compound.get(&name) else {
            continue;
        };
        let values = values.clone();
        if let Some(existing) = heightmaps
            .iter_mut()
            .find(|(existing_id, _)| *existing_id == id)
        {
            *existing = (id, values);
        } else {
            heightmaps.push((id, values));
        }
    }
    if !heightmaps.iter().any(|(id, _)| *id == 1)
        || !heightmaps.iter().any(|(id, _)| *id == 4)
        || !heightmaps.iter().any(|(id, _)| *id == 5)
    {
        return Err("generated chunk is missing a client heightmap".into());
    }

    Ok(chunk::ChunkData {
        heightmaps,
        buffer: chunk::ChunkData::from_sections(&wire_sections),
        // The terrain stage can produce structure block entities, but their
        // registry-backed network type IDs are not wired up yet.
        block_entities: Vec::new(),
    })
}

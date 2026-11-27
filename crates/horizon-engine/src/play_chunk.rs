//! Conversion of generated server chunks into the vanilla 26.3 PLAY payload.

use horizon_core::position::BlockPos;
use horizon_protocol::{
    buf::Reader,
    chunk::{self, Paletted, Section, Strategy},
    packets::play,
};
use horizon_world::{
    chunk::ChunkData as SavedChunkData,
    level::{Behaviours, data::state_flags},
    nbt::{JavaString, Tag},
    registry::{BlockRegistry, BlockStateId},
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

/// Keep a cached client chunk's block palette coherent with one resident
/// block mutation. The packet's light and heightmaps are retained as-is; the
/// caller must separately schedule their vanilla recomputation.
pub(crate) fn patch_cached_block_state(
    packet: &mut Vec<u8>,
    pos: BlockPos,
    state: BlockStateId,
    behaviours: &Behaviours,
) -> Result<(), String> {
    let mut reader = Reader::new(packet);
    let packet_id = reader
        .varint()
        .map_err(|error| format!("read cached chunk packet ID: {error}"))?;
    let decoded = play::Clientbound::decode(packet_id, reader.rest())
        .map_err(|error| format!("decode cached chunk packet: {error}"))?;
    let play::Clientbound::LevelChunkWithLight(mut chunk_packet) = decoded else {
        return Err("cached packet is not LevelChunkWithLight".into());
    };
    let expected_chunk = pos.chunk();
    if (chunk_packet.x, chunk_packet.z) != (expected_chunk.x, expected_chunk.z) {
        return Err("cached LevelChunkWithLight coordinates do not match block".into());
    }
    let mut sections = chunk_packet
        .chunk
        .sections(SECTION_COUNT, BLOCK_BITS, BIOME_BITS)
        .map_err(|error| format!("decode cached chunk sections: {error}"))?;
    let section_y = pos.y.div_euclid(16);
    let section_index = usize::try_from(section_y - MIN_SECTION_Y)
        .ok()
        .filter(|&index| index < SECTION_COUNT)
        .ok_or_else(|| format!("block y={} is outside the cached Overworld", pos.y))?;
    let local_y = pos.y.rem_euclid(16) as usize;
    let index =
        (local_y << 8) | ((pos.z.rem_euclid(16) as usize) << 4) | pos.x.rem_euclid(16) as usize;
    let section = &mut sections[section_index];
    let old_id = section.states.get(index);
    let new_id = i32::try_from(state.0)
        .map_err(|_| format!("block state ID {} does not fit the wire format", state.0))?;
    if old_id == new_id {
        return Ok(());
    }

    let old_facts = behaviours.facts(BlockStateId(old_id as u32));
    let new_facts = behaviours.facts(state);
    match (
        old_facts.flags & state_flags::AIR != 0,
        new_facts.flags & state_flags::AIR != 0,
    ) {
        (true, false) => section.non_empty = section.non_empty.saturating_add(1),
        (false, true) => section.non_empty = section.non_empty.saturating_sub(1),
        _ => {}
    }
    match (
        old_facts.fluid != horizon_world::fluid::FluidState::EMPTY,
        new_facts.fluid != horizon_world::fluid::FluidState::EMPTY,
    ) {
        (true, false) => section.fluids = section.fluids.saturating_sub(1),
        (false, true) => section.fluids = section.fluids.saturating_add(1),
        _ => {}
    }
    let mut values = (0..chunk::BLOCKS)
        .map(|index| section.states.get(index))
        .collect::<Vec<_>>();
    values[index] = new_id;
    section.states = Paletted::from_values(&values, Strategy::Blocks, BLOCK_BITS);
    chunk_packet.chunk.buffer = chunk::ChunkData::from_sections(&sections);
    *packet = play::Clientbound::LevelChunkWithLight(chunk_packet).encode();
    Ok(())
}

/// Overlay the authoritative saved block palettes onto a freshly encoded packet.
/// Worldgen still supplies biome, heightmap and light payloads; their saved-state
/// restoration/recomputation remains a separate lifecycle step.
pub(crate) fn overlay_saved_block_states(
    packet: &mut Vec<u8>,
    saved: &SavedChunkData,
    registry: &BlockRegistry,
    behaviours: &Behaviours,
) -> Result<(), String> {
    let mut reader = Reader::new(packet);
    let packet_id = reader
        .varint()
        .map_err(|error| format!("read generated chunk packet ID: {error}"))?;
    let decoded = play::Clientbound::decode(packet_id, reader.rest())
        .map_err(|error| format!("decode generated chunk packet: {error}"))?;
    let play::Clientbound::LevelChunkWithLight(mut chunk_packet) = decoded else {
        return Err("generated packet is not LevelChunkWithLight".into());
    };
    if (chunk_packet.x, chunk_packet.z) != (saved.position().x, saved.position().z) {
        return Err("saved chunk coordinates do not match generated packet".into());
    }

    let saved_sections = saved
        .section_block_states()
        .map_err(|error| format!("read saved chunk palettes: {error}"))?;
    let saved_sections = saved_sections
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    let air = registry
        .default_state("minecraft:air")
        .ok_or("block registry is missing minecraft:air")?;
    let mut sections = chunk_packet
        .chunk
        .sections(SECTION_COUNT, BLOCK_BITS, BIOME_BITS)
        .map_err(|error| format!("decode generated chunk sections: {error}"))?;

    for (section_index, section) in sections.iter_mut().enumerate() {
        let section_y = MIN_SECTION_Y + section_index as i32;
        let palette = match saved_sections.get(&section_y).copied().flatten() {
            Some(tag) => Some(
                horizon_world::chunk::PalettedContainer::from_nbt(tag, chunk::BLOCKS, 4).map_err(
                    |error| format!("decode saved block palette at section Y={section_y}: {error}"),
                )?,
            ),
            None => None,
        };
        let mut values = Vec::with_capacity(chunk::BLOCKS);
        let mut non_empty = 0_i16;
        let mut fluids = 0_i16;
        for index in 0..chunk::BLOCKS {
            let state = if let Some(palette) = &palette {
                registry
                    .saved_state_id(palette.get(index).map_err(|error| {
                        format!("read saved block at section Y={section_y}: {error}")
                    })?)
                    .ok_or_else(|| {
                        format!("saved palette contains unknown block at Y={section_y}")
                    })?
            } else {
                air
            };
            let facts = behaviours.facts(state);
            if facts.flags & state_flags::AIR == 0 {
                non_empty += 1;
            }
            if facts.fluid != horizon_world::fluid::FluidState::EMPTY {
                fluids += 1;
            }
            values.push(i32::try_from(state.0).map_err(|_| {
                format!("saved block state {} does not fit the wire format", state.0)
            })?);
        }
        section.states = Paletted::from_values(&values, Strategy::Blocks, BLOCK_BITS);
        section.non_empty = non_empty;
        section.fluids = fluids;
    }
    chunk_packet.chunk.buffer = chunk::ChunkData::from_sections(&sections);
    *packet = play::Clientbound::LevelChunkWithLight(chunk_packet).encode();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_chunk_packet_reflects_block_state_mutation() {
        let Some(resources_path) = std::env::var_os("BORIZON_SERVER_RESOURCES") else {
            eprintln!("skipped: set BORIZON_SERVER_RESOURCES for cached chunk packet test");
            return;
        };
        let resources = crate::resources::load(std::path::Path::new(&resources_path))
            .expect("load official server resources");
        let stone = resources
            .registry
            .default_state("minecraft:stone")
            .expect("vanilla stone state");
        let air = resources
            .registry
            .default_state("minecraft:air")
            .expect("vanilla air state");
        let sections = (0..SECTION_COUNT)
            .map(|_| Section {
                non_empty: 4096,
                fluids: 0,
                states: Paletted::from_values(
                    &vec![stone.0 as i32; chunk::BLOCKS],
                    Strategy::Blocks,
                    BLOCK_BITS,
                ),
                biomes: Paletted::from_values(&[0; chunk::BIOMES], Strategy::Biomes, BIOME_BITS),
            })
            .collect::<Vec<_>>();
        let chunk_data = chunk::ChunkData {
            heightmaps: Vec::new(),
            buffer: chunk::ChunkData::from_sections(&sections),
            block_entities: Vec::new(),
        };
        let mut packet = play::Clientbound::LevelChunkWithLight(play::LevelChunkWithLight {
            x: 0,
            z: 0,
            chunk: chunk_data,
            light: chunk::LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        })
        .encode();

        patch_cached_block_state(
            &mut packet,
            BlockPos::new(0, 0, 0),
            air,
            &resources.behaviours,
        )
        .expect("patch the cached wire chunk");

        let mut reader = Reader::new(&packet);
        let id = reader.varint().unwrap();
        let play::Clientbound::LevelChunkWithLight(updated) =
            play::Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("cached chunk packet changed type");
        };
        let updated_sections = updated
            .chunk
            .sections(SECTION_COUNT, BLOCK_BITS, BIOME_BITS)
            .unwrap();
        let target_section = &updated_sections[4];
        assert_eq!(target_section.states.get(0), air.0 as i32);
        assert_eq!(target_section.non_empty, 4095);
        assert_eq!(updated_sections[5].non_empty, 4096);
    }
}

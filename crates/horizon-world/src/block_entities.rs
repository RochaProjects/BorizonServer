//! Proto-chunk block-entity storage. Active payloads are caller-owned runtime
//! types; this module does not create entities or execute their behavior.
use crate::{compression::invalid, nbt::Tag};
use horizon_core::position::{BlockPos, ChunkPos};
use std::{
    collections::{HashMap, HashSet},
    io,
};

/// Vanilla block-entity type accepted by each corresponding block. Kept in
/// `horizon-world` so dedicated worldgen/save code does not depend on the
/// mixed client renderer's chunk encoder.
pub fn block_entity_type(block_name: &str) -> Option<&'static str> {
    let name = block_name.strip_prefix("minecraft:")?;
    Some(match name {
        "chest" => "minecraft:chest",
        "trapped_chest" => "minecraft:trapped_chest",
        "ender_chest" => "minecraft:ender_chest",
        "barrel" => "minecraft:barrel",
        "furnace" => "minecraft:furnace",
        "blast_furnace" => "minecraft:blast_furnace",
        "smoker" => "minecraft:smoker",
        "hopper" => "minecraft:hopper",
        "dispenser" => "minecraft:dispenser",
        "dropper" => "minecraft:dropper",
        "brewing_stand" => "minecraft:brewing_stand",
        "beacon" => "minecraft:beacon",
        "bell" => "minecraft:bell",
        "campfire" | "soul_campfire" => "minecraft:campfire",
        "enchanting_table" => "minecraft:enchanting_table",
        "jukebox" => "minecraft:jukebox",
        "lectern" => "minecraft:lectern",
        "spawner" => "minecraft:mob_spawner",
        "comparator" => "minecraft:comparator",
        "daylight_detector" => "minecraft:daylight_detector",
        "conduit" => "minecraft:conduit",
        "decorated_pot" => "minecraft:decorated_pot",
        "moving_piston" => "minecraft:piston",
        "structure_block" => "minecraft:structure_block",
        "jigsaw" => "minecraft:jigsaw",
        "command_block" | "chain_command_block" | "repeating_command_block" => {
            "minecraft:command_block"
        }
        "end_portal" => "minecraft:end_portal",
        "end_gateway" => "minecraft:end_gateway",
        "sculk_sensor" => "minecraft:sculk_sensor",
        "calibrated_sculk_sensor" => "minecraft:calibrated_sculk_sensor",
        "sculk_catalyst" => "minecraft:sculk_catalyst",
        "sculk_shrieker" => "minecraft:sculk_shrieker",
        "crafter" => "minecraft:crafter",
        "potent_sulfur" => "minecraft:potent_sulfur",
        "chiseled_bookshelf" => "minecraft:chiseled_bookshelf",
        "trial_spawner" => "minecraft:trial_spawner",
        "vault" => "minecraft:vault",
        "beehive" | "bee_nest" => "minecraft:beehive",
        "suspicious_sand" | "suspicious_gravel" => "minecraft:brushable_block",
        "creaking_heart" => "minecraft:creaking_heart",
        "test_block" => "minecraft:test_block",
        "test_instance_block" => "minecraft:test_instance_block",
        value if value.ends_with("shulker_box") => "minecraft:shulker_box",
        value if value.ends_with("_wall_hanging_sign") || value.ends_with("_hanging_sign") => {
            "minecraft:hanging_sign"
        }
        value if value.ends_with("_sign") => "minecraft:sign",
        value if value.ends_with("_banner") => "minecraft:banner",
        value if value.ends_with("_head") || value.ends_with("_skull") => "minecraft:skull",
        value if value.ends_with("_shelf") => "minecraft:shelf",
        value if value.ends_with("copper_golem_statue") => "minecraft:copper_golem_statue",
        _ => return None,
    })
}

/// Resolve native block-entity coordinates, relocating mismatched X/Z into the
/// owning chunk while retaining local offsets and the full Y coordinate. The
/// supplied tag is not rewritten. NumericTag float/double reads use Mth.floor.
pub fn position_from_tag(chunk: ChunkPos, tag: &Tag) -> io::Result<BlockPos> {
    if !matches!(tag, Tag::Compound(_)) {
        return Err(invalid("block entity is not a compound"));
    }
    let read = |name| match tag.get(name) {
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Long(v)) => *v as i32,
        Some(Tag::Float(bits)) => f64::from(f32::from_bits(*bits)).floor() as i32,
        Some(Tag::Double(bits)) => f64::from_bits(*bits).floor() as i32,
        _ => 0,
    };
    let (mut x, y, mut z) = (read("x"), read("y"), read("z"));
    if x >> 4 != chunk.x || z >> 4 != chunk.z {
        x = chunk.x.wrapping_shl(4).wrapping_add(x & 15);
        z = chunk.z.wrapping_shl(4).wrapping_add(z & 15);
    }
    Ok(BlockPos::new(x, y, z))
}

pub struct ProtoBlockEntities<E> {
    position: ChunkPos,
    pending: HashMap<BlockPos, Tag>,
    active: HashMap<BlockPos, E>,
}
impl<E> ProtoBlockEntities<E> {
    pub fn new(position: ChunkPos) -> Self {
        Self {
            position,
            pending: HashMap::new(),
            active: HashMap::new(),
        }
    }
    /// Ignore a pending record when an active entity already owns its resolved
    /// position. Inserting an active entity removes any earlier pending record.
    pub fn set_pending(&mut self, tag: Tag) -> io::Result<()> {
        let position = position_from_tag(self.position, &tag)?;
        if !self.active.contains_key(&position) {
            self.pending.insert(position, tag);
        }
        Ok(())
    }
    /// Position must be the payload's immutable world position, as supplied by
    /// the runtime's block-entity constructor. No chunk/build-height gate applies.
    pub fn set_active(&mut self, position: BlockPos, entity: E) -> Option<E> {
        self.pending.remove(&position);
        self.active.insert(position, entity)
    }
    pub fn active(&self, position: BlockPos) -> Option<&E> {
        self.active.get(&position)
    }
    pub fn pending(&self, position: BlockPos) -> Option<&Tag> {
        self.pending.get(&position)
    }
    /// Mutating coordinate fields does not re-key an existing pending entry.
    pub fn pending_mut(&mut self, position: BlockPos) -> Option<&mut Tag> {
        self.pending.get_mut(&position)
    }
    pub fn pending_entries(&self) -> &HashMap<BlockPos, Tag> {
        &self.pending
    }
    pub fn positions(&self) -> HashSet<BlockPos> {
        self.pending
            .keys()
            .chain(self.active.keys())
            .copied()
            .collect()
    }
    pub fn remove(&mut self, position: BlockPos) -> Option<E> {
        self.pending.remove(&position);
        self.active.remove(&position)
    }
    /// Serialize an active entity when present, otherwise clone its pending tag.
    /// The caller supplies runtime serialization; this does not instantiate NBT.
    pub fn for_saving<T>(
        &self,
        position: BlockPos,
        serialize: impl FnOnce(&E) -> Result<Tag, T>,
    ) -> Result<Option<Tag>, T> {
        if let Some(entity) = self.active.get(&position) {
            serialize(entity).map(Some)
        } else {
            Ok(self.pending.get(&position).cloned())
        }
    }
}

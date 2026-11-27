//! Server-side PLAY movement over the prepared vanilla collision registry.
//!
//! This adapter owns only the callbacks available in the current bootstrap. It
//! refuses movement when the scan touches an unloaded chunk; entity collisions,
//! border state, and active block-entity snapshots remain separate runtime work.
use crate::resources::ServerResources;
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision::{DynamicCollisionWorld, ShulkerCollisionState},
    block_collision_scan::{BlockCollisionWorld, CollisionBlock},
    collision_context::{CollisionContext, CollisionEntity, CollisionItem, EntityContextKind},
    dynamic_support::DynamicSupportWorld,
    entity_obstruction::EntityIdentity,
    fluid::FluidState,
    level::{LevelAccess, ResidentLevel},
    liquid_collision::LiquidCollisionWorld,
    powder_snow_collision::{PowderSnowEntity, PowderSnowWorld},
    world_movement::{EntityMovementWorld, MovementRequest, resolve_world_movement},
};

const COLLISION_CANDIDATE_BUDGET: usize = 16_384;

pub(crate) fn resolve_player_move(
    level: &ResidentLevel<'_>,
    resources: &ServerResources,
    entity_id: i32,
    from: [f64; 3],
    requested: [f64; 3],
    fall_distance: f64,
    on_ground: bool,
) -> Result<([f64; 3], bool), String> {
    let identity = EntityIdentity(entity_id as u32 as u64);
    let movement = Vec3::new(
        requested[0] - from[0],
        requested[1] - from[1],
        requested[2] - from[2],
    );
    let context_entity = CollisionEntity {
        identity,
        kind: EntityContextKind::Player,
        y: from[1],
        descending: false,
        main_hand: Some(CollisionItem::EMPTY),
    };
    let context = CollisionContext::for_entity(context_entity, false);
    let mut world = ResidentMovementWorld {
        level,
        resources,
        fall_distance,
        unloaded_chunk_seen: false,
    };
    let resolved = resolve_world_movement(
        &mut world,
        MovementRequest {
            entity: identity,
            context,
            bounds: player_box(from),
            movement,
            max_step: 0.6,
            on_ground,
            candidate_budget: COLLISION_CANDIDATE_BUDGET,
        },
    )
    .map_err(|error| format!("resolve PLAY movement collision: {error}"))?;
    if world.unloaded_chunk_seen {
        return Err("movement collision scan reached an unloaded chunk".into());
    }
    let position = [
        from[0] + resolved.x,
        from[1] + resolved.y,
        from[2] + resolved.z,
    ];
    let grounded = movement.y < 0.0 && resolved.y != movement.y;
    Ok((position, grounded))
}

fn player_box(position: [f64; 3]) -> Aabb {
    Aabb::new(
        Vec3::new(position[0] - 0.3, position[1], position[2] - 0.3),
        Vec3::new(position[0] + 0.3, position[1] + 1.8, position[2] + 0.3),
    )
}

struct ResidentMovementWorld<'a, 'r> {
    level: &'a ResidentLevel<'r>,
    resources: &'a ServerResources,
    fall_distance: f64,
    unloaded_chunk_seen: bool,
}
impl DynamicSupportWorld for ResidentMovementWorld<'_, '_> {
    type Error = String;

    fn shulker_is_closed(&mut self, _position: [i32; 3]) -> Result<Option<bool>, Self::Error> {
        // A missing block-entity snapshot follows vanilla's closed-shulker fallback.
        Ok(None)
    }

    fn moving_piston_shape(
        &mut self,
        _position: [i32; 3],
    ) -> Result<Option<VoxelShape>, Self::Error> {
        // No active moving-piston block entities are owned by this bootstrap yet.
        Ok(None)
    }
}
impl DynamicCollisionWorld for ResidentMovementWorld<'_, '_> {
    fn shulker_collision_state(
        &mut self,
        _position: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, Self::Error> {
        // Missing block-entity data uses the registry's vanilla full-cube fallback.
        Ok(None)
    }
}
impl LiquidCollisionWorld for ResidentMovementWorld<'_, '_> {
    type Error = String;

    fn entity_liquid_shape(&mut self, _entity: EntityIdentity) -> Result<VoxelShape, Self::Error> {
        VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(0.6, 1.8, 0.6)))
            .map_err(|error| error.to_string())
    }

    fn fluid(&mut self, position: BlockPos) -> Result<FluidState, Self::Error> {
        let state = self.level.block_state(position);
        Ok(self.resources.behaviours.facts(state).fluid)
    }

    fn entity_can_stand(
        &mut self,
        _entity: EntityIdentity,
        _fluid: FluidState,
    ) -> Result<bool, Self::Error> {
        // The current player snapshot has no stand-on-fluid effect or equipment.
        Ok(false)
    }
}
impl PowderSnowWorld for ResidentMovementWorld<'_, '_> {
    type Error = String;

    fn powder_snow_entity(
        &mut self,
        _entity: EntityIdentity,
    ) -> Result<PowderSnowEntity, Self::Error> {
        Ok(PowderSnowEntity {
            fall_distance: self.fall_distance,
            falling_block: false,
        })
    }

    fn powder_snow_walkable_tag(&mut self, _entity: EntityIdentity) -> Result<bool, Self::Error> {
        Ok(false)
    }

    fn feet_are_leather_boots(&mut self, _entity: EntityIdentity) -> Result<bool, Self::Error> {
        Ok(false)
    }
}
impl BlockCollisionWorld for ResidentMovementWorld<'_, '_> {
    type Error = String;
    type Chunk = ChunkPos;

    fn collision_chunk(&mut self, position: ChunkPos) -> Result<Option<Self::Chunk>, Self::Error> {
        let probe = BlockPos::new(
            position.x.saturating_mul(16),
            self.level.min_y(),
            position.z.saturating_mul(16),
        );
        if self.level.is_loaded(probe) {
            Ok(Some(position))
        } else {
            self.unloaded_chunk_seen = true;
            Ok(None)
        }
    }

    fn collision_block(
        &mut self,
        _chunk: Self::Chunk,
        position: BlockPos,
    ) -> Result<CollisionBlock, Self::Error> {
        self.resources
            .collisions
            .collision_block(self.level.block_state(position))
            .map_err(|error| error.to_string())
    }

    fn is_suffocating(
        &mut self,
        _chunk: Self::Chunk,
        _block: CollisionBlock,
        _position: BlockPos,
    ) -> Result<bool, Self::Error> {
        Err("PLAY movement unexpectedly requested a suffocation-only collision scan".into())
    }

    fn context_shape(
        &mut self,
        block: CollisionBlock,
        position: BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, Self::Error> {
        self.resources
            .collisions
            .collision_shape_with_context(self, block.state, position, context)
            .map_err(|error| error.to_string())
    }

    fn missing_chunks_stay_missing(&self) -> bool {
        true
    }
}
impl EntityMovementWorld for ResidentMovementWorld<'_, '_> {
    type Error = String;

    fn entity_collisions(
        &mut self,
        _entity: EntityIdentity,
        _bounds: Aabb,
    ) -> Result<Vec<VoxelShape>, Self::Error> {
        // PLAY has no authoritative entity-position index yet.
        Ok(Vec::new())
    }

    fn movement_border(
        &mut self,
        _entity: EntityIdentity,
        _bounds: Aabb,
    ) -> Result<Option<VoxelShape>, Self::Error> {
        // A saved/ticking WorldBorder is not yet attached to this prepared world.
        Ok(None)
    }
}

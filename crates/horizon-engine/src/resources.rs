//! Server-side loading of the prepared vanilla data used by world simulation.

use crate::registry_sync::RegistryBundle;
use horizon_world::{
    level::{Behaviours, data::load_registry},
    registry::BlockRegistry,
    worldgen::generator::GeneratorData,
};
use std::{path::Path, sync::Arc};

/// Immutable vanilla data shared by dimensions and chunk-generation workers.
///
/// The worldgen API currently requires a process-lifetime registry reference;
/// the dedicated engine loads this once during startup and retains it until exit.
pub(crate) struct ServerResources {
    pub(crate) registry: &'static BlockRegistry,
    pub(crate) behaviours: Behaviours,
    pub(crate) worldgen: Arc<GeneratorData>,
    pub(crate) synchronized_registries: RegistryBundle,
    pub(crate) light_occlusion: crate::light::LightOcclusion,
}

/// Load only gameplay resources. Client model bindings, textures, fonts, audio,
/// shaders and other presentation assets are deliberately not read here.
pub(crate) fn load(root: &Path) -> Result<ServerResources, String> {
    if !root.is_dir() {
        return Err(format!(
            "server resource directory does not exist: {}",
            root.display()
        ));
    }

    let registry = Box::leak(Box::new(
        load_registry(root).map_err(|error| format!("load block registry: {error}"))?,
    ));
    let light_occlusion = crate::light::LightOcclusion::load(root)?;
    if light_occlusion.state_count() != registry.len() {
        return Err(format!(
            "light-occlusion.json has {} states but facts.json has {}",
            light_occlusion.state_count(),
            registry.len()
        ));
    }
    let behaviours = Behaviours::load(registry, root)
        .map_err(|error| format!("load vanilla block behaviours: {error}"))?;
    let worldgen = GeneratorData::load(&root.join("worldgen"), registry)
        .map_err(|error| format!("load vanilla worldgen resources: {error}"))?;
    let synchronized_registries = RegistryBundle::load(root)?;

    Ok(ServerResources {
        registry,
        behaviours,
        worldgen,
        synchronized_registries,
        light_occlusion,
    })
}

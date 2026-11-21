//! Server-safe simulation data types extracted from the client render crate.
//! This crate has no GPU, window, font or shader dependencies — only
//! simulation, networking, world data and utility code that the dedicated
//! server and the client both need.
#![forbid(unsafe_code)]

pub use horizon_client_core::Result;
pub use horizon_client_core::text;

// reexports from other crates (mirrors horizon-client-render's lib.rs)
pub use horizon_client_core::lang;
pub use horizon_client_core::scoreboard;
pub use horizon_items::blocks;
pub use horizon_items::enchant;
pub use horizon_items::items;
pub use horizon_world::json_folder;

pub mod assets;
pub mod auth;
pub mod ban_notice;
pub mod block_edit;
pub mod block_entities;
pub mod block_entity_anim;
pub mod block_entity_data;
pub mod brewing;
pub mod chunk;
pub mod chunk_map;
pub mod collisions;
pub mod command_block;
pub mod containers;
pub mod crash;
pub mod custom_sky;
pub mod effects;
pub mod fast_hash;
pub mod hud_layout;
pub mod java_text;
pub mod jigsaw_block;
pub mod light;
pub mod logging;
pub mod pack_hooks;
pub mod pack_palette;
pub mod poi_sections;
pub mod providers;
pub mod random_source;
pub mod sculk;
pub mod section_mask;
pub mod signs;
pub mod skins;
pub mod spans;
pub mod structure_block;
pub mod tab_list;
pub mod test_block;
pub mod test_instance_block;
pub mod vault;
pub mod world;

//! Gameplay and startup, above the reusable client library.
// No `#![recursion_limit]` or other inner attribute horizon-mixins doesn't know here: the
// launcher's horizon-mixins then rejects every native mod targeting this crate (#8169).
#![forbid(unsafe_code)]
// The dedicated-server build compiles shared source that the client calls but the server
// never reaches (screens, split-screen, fixtures, controllers, narration). Until the
// server/client ownership split extracts those behind a client boundary, suppress dead-code
// warnings for the server build only so CI's `clippy -D warnings` stays green.
#![cfg_attr(feature = "dedicated-server", allow(dead_code, unused_imports))]
extern crate self as horizon_client;
pub use horizon_client_lib::*;
pub mod entry;
pub mod game;
#[cfg(feature = "client-graphics")]
mod horizon_look;
pub mod mod_api;
pub use horizon_server_api as server_api;

#[cfg(test)]
mod brewing_speed_load_tests;
#[cfg(test)]
mod living_death_tests;

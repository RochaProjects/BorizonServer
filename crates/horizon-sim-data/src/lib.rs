//! Renderer-independent rules and data used by world simulation.
//!
//! This crate is an extraction boundary: simulation can depend on it without
//! pulling client rendering, windowing, or input systems into the dependency graph.

pub mod raid;
pub mod ticket_levels;
pub mod world_spawn;

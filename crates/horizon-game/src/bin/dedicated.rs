//! Dedicated server executable root. Client builds keep their own launchers;
//! this binary intentionally selects only the server feature set.
#![forbid(unsafe_code)]

#[global_allocator]
static ALLOCATOR: horizon_game::Allocator = horizon_game::ALLOCATOR;

fn main() {
    horizon_game::main();
}

# Server-side build scope

The dedicated engine build uses `horizon-client` with the `dedicated-server` and
`server-plugins` features. It has two different extension systems in the source:

- `server-plugins/` is the fork's runtime `.so` plugin API and loader. This remains
  enabled for the dedicated engine.
- `mod_api` and the native hook modules are Horizon's older in-process Rust trait
  mod system. That system is not the server plugin API.

## Current server behavior

With `dedicated-server`, startup no longer installs the old trait-mod callbacks,
native renderers, or client sound callback. A non-empty trait-mod list passed to
`entry::main_with` is ignored with a warning. Logging, crash reporting, vanilla
world and gameplay code, and the `.so` plugin loader still start normally.
The dedicated executable also omits the legacy mod API marker that launchers use
to identify a mod-capable client.

This removes the old trait-mod runtime from the dedicated server's startup path;
it does **not** yet remove all old mod code from compilation. The shared game
source still refers to `mod_api` types, and `horizon-mods` also supplies content
and manifest types used by shared resource-loading code. Deleting those modules
or their dependencies now would break compilation or change resource loading.
The shared server command/player records now live in `server_api`, and the
plugin bridge consumes those generic records and the core gamerule type directly.
`mod_api` re-exports the old record names for the client trait API, preserving
that source compatibility without making the `.so` host depend on it.
The dedicated feature set also omits the former client mod registry/index:
`wasmparser`, `minisign-verify`, and the index's SHA-256 validation path are
only enabled by `legacy-mod-index` for client builds. The common `horizon-mods`
crate remains for content manifests and resource/data-pack loading; it is not
the runtime `.so` plugin API. The shared game still contains dormant Mods UI
state, backed by a no-op worker when the registry is absent.
The old `cargo-horizon-mod` workflow and old mod/nightly/parity release chain were
removed from this fork; they do not build or publish the server plugin system.
`horizon-bots` is kept as a standalone utility outside the workspace, and no
workflow builds or releases it.

## Build-time impact

The old-hook startup change alone is not expected to make a large difference to
compile time. A first build-graph cut now makes GPU backend implementations
optional: `client-graphics` activates Vulkan, Metal, DX12, GLES, and the HDR
Vulkan support; the dedicated command omits that feature. `wgpu`'s common API
types remain available to the shared source, but its platform backends and the
HDR crate's Vulkan/`ash` path are absent from the dedicated dependency graph.

This is a contained reduction, not a full server/client split. The dedicated
binary still compiles `horizon-client`'s shared game crate and renderer source,
and `horizon-client/build.rs` still generates the Rust trait-mod API because
shared modules reference its declarations. The CI workflow now checks only the
dedicated engine, server tools, and public plugin SDK/loader; it does not build
the client, bots, nightly binaries, or parity harnesses. The next large compile
costs are the renderer/window stack and shared client-facing game modules.
Removing those safely requires extracting server gameplay and the `.so` host
into a server-specific crate, then gating client rendering and legacy mod API
declarations at that boundary; simply dropping their dependencies today would
break shared source imports.

## Client-only build candidates

The current dedicated target is not a server-only Cargo graph. It builds
`horizon-client` with `--no-default-features --features server-plugins,dedicated-server`,
but `horizon-client-lib` still points at `horizon-client/src/client_lib.rs` and
unconditionally re-exports the renderer and input crates. `game.rs` also compiles
most screen and rendering modules for the dedicated target. Removing dependencies
from a manifest without first gating those imports will make the target fail.

| Area | Current status | Safe treatment |
| --- | --- | --- |
| Audio, controllers, Horizon Extras/Friends/cosmetics | Already excluded by `--no-default-features`; the relevant default features are `audio`, `controllers`, and `horizon-extras`. | Keep excluded from the dedicated build. |
| `wgpu`, `winit`, `naga`, Vulkan/Metal/DirectX/GLES and window-system crates | GPU backend implementations are now behind `client-graphics`; dedicated builds retain shared `wgpu` types and still compile renderer source, `winit`, and shader code. | Next priority is removing the renderer/window crates from the dedicated graph after extracting server-used resource/gameplay types. |
| Menus, screens, HUD, mouse/keyboard input and gamepad UI | Most are declared unconditionally in `game.rs` and re-exported by `client_lib.rs`. | Gate the UI/input module tree as a unit after the server runtime no longer stores or calls those types. |
| Shader packs, OptiFine visual formats, Polytone and shader UI | Visual processing lives in `horizon-client-render`; some shared resource loading still reads the same resource root. | Exclude rendering and shader compilation in the server feature while retaining vanilla data-pack/worldgen loading. |
| Narrator and client theme | The theme package and icon/style initialization are now excluded from the dedicated target. Narration still compiles through shared chat and screen code. | Gate narrator calls after the screen/chat presentation layer is split. |
| Realms, mod browser, desktop/window services | Registry/index and its validators are now absent from the dedicated graph; the shared game keeps a no-op browser worker. Other client-facing modules remain in the shared source tree. | Gate with the client feature; keep server networking, `server.properties`, player auth, and protocol code. |
| Server simulation, protocol, world storage/generation, commands, anticheat, chat filter and plugin host | Used by the dedicated runtime or `.so` bridge. | Keep in the server target. |

The first four candidate groups are not independent manifest toggles today.
The structural change with a real payoff is a server-facing crate/feature
boundary that compiles the simulation and plugin host without linking the client
renderer. The current source evidence does not justify quoting a byte or minute
saving; measure that only after the boundary removes `wgpu`/`naga`/window-system
crates from the dedicated dependency tree.

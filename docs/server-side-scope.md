# Server-side build scope

## Handoff: status real em 2026-11-20

Este documento é o ponto de partida para continuar a separação server/client.
O estado abaixo prevalece sobre planos antigos neste arquivo. **O alvo dedicado
passa no check e no Clippy com `-D warnings`; o handoff anterior listava falhas
que já foram corrigidas nesta etapa.**

### O que já foi alterado

- O servidor dedicado tem caminho próprio em `game::dedicated::run`, usa
  `Settings::default()` e chama `Game::new` sem o fluxo de menu/loading do cliente.
- `horizon-server-api` contém os registros genéricos de jogador/comando/sidebar;
  `horizon-client::server_api` permanece como reexport de compatibilidade.
- A inicialização dedicada não carrega modelos de apresentação, efeitos de
  partículas visuais, arquivos de opções/debug do cliente, música/ambiente ou
  tabelas visuais de fog/Polytone.
- O índice legado de mods, extras, áudio, controllers, MCP, backends gráficos e
  alguns decodificadores visuais são desativados no conjunto de features dedicado.
- Loading screen, menu inicial, diálogo de recursos ausentes e worker de startup
  do cliente estão condicionados ao feature não dedicado.
- `winit` precisa de uma implementação de plataforma no Linux mesmo quando o
  código compartilha apenas tipos de evento. Foi criada `server-event-types`, que
  habilita apenas X11 para o servidor; Wayland e clipboard continuam em
  `client-windowing`.
- Os campos de renderer/input continuam dentro de `Game`; portanto, esta etapa
  **não** removeu `horizon-client-render`, `horizon-client-input`, `wgpu` ou
  `winit` do grafo de dependências.

### Checagem executada (estado corrigido)

Comando executado (não foi build `--release`):

```sh
cargo check --locked -j1 -p horizon-client --no-default-features \
  --features server-plugins,dedicated-server --bin horizon-client
```

O alvo dedicado agora compila limpo e passa no Clippy do workflow CI:

```sh
cargo clippy --locked -p horizon-client --bin horizon-client \
  --no-default-features --features server-plugins,dedicated-server -- -D warnings
```

Correções aplicadas nesta etapa:

1. `game/loading_window.rs`: a função `start_game` chamava
   `GameCpuResources::load`, que só existia no alvo não dedicado. Adicionado um
   stub `#[cfg(feature = "dedicated-server")]` de `GameCpuResources::load` que
   delega para `GameCpuResources::server` (bundle vazio), já que o servidor
   nunca entra nesse path em runtime.
2. `game/native_server.rs`: literais de `ServerPlayer` e `ServerCommandInfo`
   não podiam ser construídos fora de `horizon-server-api` por causa de
   `#[non_exhaustive]`. Removido `#[non_exhaustive]` de ambos os tipos (eles já
   derivam `Default` e são internos do projeto; a anotação não fazia sentido
   para o engine). Removidos os `..Default::default()` redundantes que o Clippy
   (`needless_update`) sinalizou.
3. `game/resource_load.rs`: a closure de `MeshTables::new` teve erro de inferência
   de `Result`; anotado `Ok::<_, Box<dyn std::error::Error>>(tables)` no braço
   `headless`.
4. `game/dedicated.rs`: lifetime `'a` desnecessária em `start_dedicated_game`
   elidida (Clippy `needless_lifetimes`).
5. `game.rs`: `return` redundante no bloco `#[cfg(feature = "dedicated-server")]`
   convertido em expressão (Clippy `needless_return`).

Para que o Clippy com `-D warnings` passasse sem reescrever a fronteira
server/client agora, adicionado `#![cfg_attr(feature = "dedicated-server",
allow(dead_code, unused_imports))]` no crate `horizon-client`: o binário
compartilhado ainda compila código cliente (screens, split-screen, fixtures,
controllers, narrator) que o servidor nunca chama, e gatear módulo por módulo
quebraria referências em código compartilhado até a fronteira ser extraída. Isso
é temporário: quando a etapa 3 do plano (separar estado visual/input de `Game`
numa fronteira cliente) for feita, o `allow` deve ser removido e os módulos
gated individualmente.

### Próximo trabalho obrigatório

1. ~~Corrigir os erros acima~~ — feito nesta etapa.
2. ~~Repetir `cargo check --locked -j1`~~ e ~~Clippy~~ — ambos passam.
3. Separar o estado visual/input de `Game` em uma fronteira cliente, preservando
   para o dedicated somente os dados usados pelo tick, rede e persistência. Só
   então tornar renderer/input/shaders dependências opcionais e provar a remoção
   com `cargo tree`. Remover o `#![cfg_attr(feature = "dedicated-server",
   allow(dead_code, unused_imports))]` do `horizon-client` quando os módulos
   cliente puderem ser gated individualmente sem quebrar código compartilhado.
4. Não apagar `horizon-mods`, `horizon-optifine`, shaders ou tipos compartilhados
   por suposição. Procurar referências e separar visualização de simulação por
   feature; plugins `.so` são a extensão server-side suportada.
5. Não rodar build release local nem paralelizar compilações. Usar jobs limitados;
   preservar arquivos não relacionados e não publicar release até o check e CI
   passarem.

### Regras para continuar

- Objetivo: reduzir o alvo dedicado e manter servidor/plugin funcionando; o
  cliente pode perder funcionalidade se isso não afetar servidor, mas não apagar
  código compartilhado sem substituir as dependências usadas pelo servidor.
- Não responder apenas com análise/planos: editar, executar a checagem, corrigir
  erros e repetir até sucesso.
- Não dizer que está pronto enquanto o check/Clippy falhar. Reportar erros concretos
  com arquivo e etapa, não suposições sobre Actions ou versões instaladas.
- Não fazer push de um estado quebrado, exceto quando o usuário pedir
  explicitamente para enviar mesmo assim. Neste caso, deixar o status de falha
  registrado no handoff e na mensagem do commit.

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
The dedicated startup now also bypasses legacy mod component/model registration,
Polytone client selections, content-mod/resource-pack root selection, and
`Game::start_mods`; it loads the prepared server resource root directly. The
client and integrated-server paths keep their existing mod behavior. These are
runtime cuts only; the shared modules and dependencies still compile. After
shared process setup and resource validation, `run_with` now dispatches the
dedicated role to `game::dedicated::run`, which owns its settings/resource-load
handoff and constructs the shared simulation without calling the client's
`start_game`. It also uses an empty CPU presentation bundle instead of loading
painting, boat, minecart, armor, waypoint and creative-screen models. The shared
`Game` type still owns both server and presentation fields, so this is not yet a
renderer dependency cut.
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

## Client-only build candidates and dependency evidence

The current dedicated target is still not a server-only Cargo graph. It builds
`horizon-client` with `--no-default-features --features server-plugins,dedicated-server`.
The manifest has already excluded GPU *backend implementations*, audio, controllers,
Horizon extras, the old mod index, and MCP. That does not exclude their shared crates
where those crates are still unconditional dependencies.

The main structural reason is visible in `horizon-client-lib/Cargo.toml`: its library
path points into `horizon-client/src/client_lib.rs`, and that facade unconditionally
re-exports the renderer (`horizon-client-render`) and input crate
(`horizon-client-input`). `horizon-client/src/lib.rs` then glob-re-exports that facade.
The engine's `game.rs` also declares and uses screen, UI, graphics, and input modules
alongside the server loop. Consequently, deleting renderer dependencies in Cargo.toml
or only hiding the re-exports is not a safe first change: the dedicated binary still
compiles those call sites.

The runtime confirms why a module-only feature toggle is premature: `Game` owns
server state and client presentation state in the same struct (GUI resources, screen
and dialog state, renderer state, input state, world streaming, and server levels).
Startup now has separate role paths: after shared process setup and resource
validation, dedicated mode enters `game::dedicated::run`; client and integrated-server
modes continue through the client loading window and `start_game`. This creates a
distinct server startup boundary, but the shared `Game` still carries both kinds of
state. The dedicated feature now omits the client loading screen, initial title menu,
loading-window event-loop handoff, no-resources dialog, and asynchronous client startup
worker. The prepared-resource preflight is shared in `resource_validation`; the server
calls `Game::new` directly with its empty presentation bundle and enters its server loop.
The client-only `GameCpuResources::load` fallback, entity-model document parsing,
startup test modules, and unclean-client-start recovery are now cfg-gated with that
path. The dedicated server uses the empty server CPU-resource bundle. This removes
additional client startup work from the server compilation, but the shared `Game`
still owns renderer and input state. The next extraction is the simulation/runtime
ownership split, not another manifest-only toggle.

The current dedicated dependency tree confirms that `horizon-client-render`,
`horizon-client-input`, `wgpu`, `winit`, and `naga` remain reachable. This is why
seeing `horizon-client-lib` compile in Actions is expected, but not proof that the
library itself is the sole source of the delay. It owns the shared facade and its
large transitive client graph. Do not estimate time or RAM savings until a dependency

After the feature cuts below, the locked dedicated Cargo tree was checked directly.
It no longer contains `image-webp`, `zune-jpeg`, `symphonia`, `arboard`,
`wayland-client`, or `x11rb`; it still contains `horizon-client-render`,
`horizon-client-input`, `wgpu`/`wgpu-core`, `winit`, `naga`, `horizon-shaders`,
`horizon-shader-api`, `horizon-optifine`, and `horizon-narrator`. The former list is
a verified graph reduction. The latter is the remaining client compile path and
cannot safely be removed by another manifest-only edit: shared `client_lib.rs`
re-exports it, and `game.rs` plus its 464 modules refer to those APIs. The server
still creates a `Game` with both server and presentation fields, though dedicated
startup now bypasses the client's `start_game`. A full server-only target therefore
remains unfinished; it requires a deeper ownership split, not just another dependency
toggle. No compile-time or memory savings are claimed from the dependency tree alone.

The server target now omits Naga's SPIR-V, Metal, and HLSL output backends. The
`horizon-shader-api` crate keeps manifest parsing and WGSL validation available;
its full cross-backend check is enabled by `shader-backends`, used by client builds
and shader tools. The client game and render manifests activate that feature only
with `client-graphics`, while dedicated startup keeps it disabled. The shader crates
and their WGSL/GLSL parsing remain in the graph because their shared renderer and
pack types are still compiled.

The server path now skips client-only resource work where startup already
distinguishes rendering from simulation: it does not install OptiFine/Polytone
visual pack hooks, inspect shader-pack settings for LabPBR maps, read visual fog
tables or Polytone colormaps, load player/arm/spin-attack models, parse music
and ambient audio tables, decode pack clouds/forcefield/custom skies or Polytone
particle/post effects, parse the item atlas's visual palette permutations, scan
particle JSON into render sprite tables, load the client's debug-profile file, or
read `options.txt`, video settings, shader-pack selection, gamepad settings,
client extras, and the local legacy mod list. It starts dedicated resource loading
from `Settings::default()`; server properties are applied by the server runtime.
It retains the language names used by server commands, plus block transformers,
recipes, items, mob simulation, providers, and vanilla dimension environments.
This reduces dedicated startup I/O and retained
client data; it does not remove the shared types or their crate dependencies
from compilation. The crate-level server/client split is still required for
the release build-time goal.

The first dependency-level cut in this pass is in place: `image-webp` and `zune-jpeg`
are now optional `client-graphics` dependencies. Their only source call sites were
the HUD's network-image decoder; the dedicated server's UI bundle uses the font-only
path, and PNG decoding remains available. Builds without `client-graphics` keep the
same API but report that WebP/JPEG decoding is unavailable. This removes those two
crates from the dedicated Cargo graph while leaving full client decoding enabled.

The audio feature now also controls `symphonia`'s Ogg/Vorbis decoder, not only `cpal`.
The dedicated server constructs `Audio::silent`, whose backend is never live and never
decodes samples; the no-audio implementation returns the same empty decode result.
This removes the codec/metadata/format parser dependency chain from the dedicated
graph while retaining full client audio when the `audio` feature is enabled.

The input crate now has a `client-windowing` feature. Desktop builds activate it with
`client-graphics`; it supplies winit's X11/Wayland backends and `arboard`'s system
clipboard. The dedicated target keeps winit's platform-neutral event/key types, while
those window-system stacks and the OS clipboard are excluded. Its clipboard type keeps
an in-memory copy, matching the no-system-device fallback. Linux's Wayland window-name
extension is also compiled only with the client graphics feature. The remaining
Wayland protocol dependencies were coming from HDR display luminance detection in
`horizon-hdr`; that module and its three protocol crates are now behind a feature
activated by client graphics as well.

| Area | Evidence/status in the dedicated target | Treatment |
| --- | --- | --- |
| Audio, controllers, Horizon Extras/Friends/cosmetics | Feature-disabled with `--no-default-features`; their optional integrations are excluded. | Keep excluded. Do not remove shared vanilla gameplay/audio data without checking server call sites. |
| GPU backend implementations and HDR Vulkan path | Behind `client-graphics`; absent from the dedicated graph. | Already separated; keep this feature boundary. |
| Renderer API and rendering implementation (`horizon-client-render`, `wgpu`) | Still reachable through unconditional dependencies and the shared facade/game code. Backend implementations are absent, shared types and renderer code are not. | High-impact candidate. Extract server-needed resource/model/data types first, then make renderer dependencies optional behind a client feature. |
| Window/input stack (`winit`, `horizon-client-input`, gamepad/UI input) | `horizon-client-input` is unconditionally depended on and broadly re-exported. The game source has direct input references. | Client-only after its types/callbacks are removed from the server runtime; split the game loop before dropping these crates. |
| Shader compilation and visual pack formats (`naga`, `horizon-shaders`, `horizon-shader-api`, OptiFine visual support) | Naga's SPIR-V, Metal and HLSL output features are disabled for the dedicated target; shader manifests and parsing remain reachable through shared renderer source. | Output backends are separated. Further removal requires separating shader/renderer modules from shared data loading. |
| Menus, screens, HUD, mouse/keyboard UI, realms and desktop services | Most modules are declared in the shared `game.rs`/`client_lib.rs`; many depend on renderer and input APIs. | Move behind a client runtime boundary as a group; do not try to gate one screen at a time before ownership is clear. |
| Narration | `horizon-narrator` is unconditional and referenced from shared game code, screens, and controls. | Likely client-only, but gate only after those call sites are moved or stubbed at a narrow interface. |
| Legacy mod registry/index | `legacy-mod-index` is off for the dedicated target; its signature and Wasm parser dependencies are absent. | Already separated. Keep `horizon-mods` content/manifest parsing until shared data-pack references are split. |
| `horizon-client-lib` | Compiles from a source file inside `horizon-client`; its current API combines shared simulation helpers with a broad client facade. | Split its server-safe shared modules from the client facade; make the server target depend only on the server-safe crate. |
| Server simulation, protocol, world storage/generation, commands, anticheat, chat filter and `.so` host | Needed at runtime or by plugins. | Retain in the server target. |

## Separation plan and implementation order

This is a staged source split, not a blanket deletion of client crates. Each stage
must leave the normal server simulation, protocol, worldgen, resource/data-pack
loading, and plugin ABI intact.

1. **Inventory and guardrails (this stage).** Record which crates are absent,
   which remain reachable, and why. Use the dedicated `cargo tree` as the acceptance
   check; do not infer savings from manifest labels or compile log ordering alone.
2. **Create a server-safe library boundary.** The first extraction is now in
   `horizon-server-api`: generic command/player/sidebar records have moved out of
   `horizon-client` and depend only on `horizon-client-core` for text values.
   `horizon-client::server_api` remains a compatibility re-export, so existing
   engine and legacy trait-mod call sites keep their source path. Continue by
   moving server-owned gameplay primitives and plugin integration into crates
   without renderer/window dependencies; do not widen the public API with
   AuraEssentials-specific methods.
3. **Separate runtime entry points.** Split server startup/tick/network ownership
   from the desktop `Game` state that owns screens, cursor, window, renderer, and
   client input. Keep shared vanilla simulation in reusable modules rather than
   cloning it or compiling two copies. Current call sites prove this must precede
   gating the UI tree: `run_with` unconditionally requires the resource loader's GUI
   bundle and calls `start_game`, while `Game` itself stores both server and client
   fields. Move a narrow set of server lifecycle state behind a runtime interface,
   then migrate server tick ownership incrementally.
4. **Make client crates optional and prove removal.** Only after imports and state
   ownership have moved, gate `horizon-client-render`, `horizon-client-input`,
   `wgpu`, `winit`, `naga`, shader APIs, visual pack loaders, narrator, and theme
   behind a client feature. The dedicated dependency tree must show the intended
   crates gone; CI/release must still build the dedicated engine.
5. **Trim client-only source from the server package.** Move or gate screens,
   HUD, realms, desktop services, old trait-mod declarations and visual shader
   systems. Keep world/data-pack parsers that the server actually uses, including
   `horizon-mods` manifest/content types if references remain.

### First implementation boundary

The first code change is the server-safe library boundary, not removing
`horizon-client-render` from Cargo. The extracted records are independent of
client/game-owned types except for the generic text type in `horizon-client-core`.
The runtime `HostApiV1` belongs to `server-plugins/api`; it should remain generic and
independent of AuraEssentials. The public plugin SDK and engine ABI remain separate
from Horizon's legacy trait-mod API.

The exit criteria for this work are concrete: the dedicated target no longer compiles
the desktop facade or renderer/window/shader dependency chain; the dependency tree
shows the removal; and the focused CI and release workflow still build the server.
No byte/time estimate is claimed yet. Compare clean Actions builds before and after
the renderer/window chain is removed, since local compilation is avoided on this
machine to prevent memory pressure.

# Server-side build scope

## Handoff: revisão de continuidade em 2026-11-21

Revisão feita sobre `main` em `80e945c34d` (`Fix dedicated server check and Clippy`);
antes da revisão, a árvore de trabalho estava limpa. O objetivo continua sendo
reduzir o tempo/custo do alvo dedicado, em especial o caminho de
`horizon-client-lib`, sem quebrar o cliente, a simulação do servidor ou os plugins.

### Conclusão da revisão

- A hipótese de que o renderer é parte importante do custo continua plausível e
  consistente com a arquitetura, mas a inspeção não mede sua contribuição de
  tempo. `horizon-client-lib` ainda depende incondicionalmente de
  `horizon-client-render` e `horizon-client-input`; sua facade reexporta ambos,
  e o `horizon-client` glob-reexporta a facade.
- O estado `Game` e módulos compartilhados ainda contêm referências a UI,
  renderer e input. Remover essas dependências do manifest, ou apenas condicionar
  as reexports, não constitui um corte funcionalmente seguro neste estado.
- `horizon-server-api` já é a fronteira estreita apropriada para registros
  genéricos de jogador/comando/sidebar; mantém a compatibilidade via
  `horizon_client::server_api`. Não foi identificada outra família pequena de
  tipos cuja transferência isolada reduza o grafo da biblioteca cliente.
- A geração da API antiga de mods também rastreia módulos de `game`; desativá-la
  sem primeiro separar os call sites pode quebrar código compartilhado e
  compatibilidade. Não confundir essa API antiga com o loader de plugins `.so`.

### Extrações executadas: `horizon-sim-data`

Foi criado `crates/horizon-sim-data` e movidos para ele os módulos completos
`raid.rs` (regras, codec NBT e testes), `world_spawn.rs` (busca de spawn,
espiral de chunks e testes) e `ticket_levels.rs` (tickets de chunks, níveis de
simulação/carregamento, tickets de portal e testes). O import de `FxHashMap`
dos níveis de tickets usa diretamente o alias em `horizon-core`, onde o hasher
já é mantido. O crate depende somente de `horizon-core`, `horizon-world` e da
biblioteca padrão; não depende de renderer, input, windowing ou runtime de
cliente. Os arquivos originais em `horizon-client/src/` agora são fachadas com
`pub use`, preservando os caminhos de API `horizon_client::raid`,
`horizon_client::world_spawn` e `horizon_client::ticket_levels`.

Esta é uma separação real de propriedade do código e dá um destino reutilizável
para dados/regras de simulação, mas **não reduz por si só o grafo do binário
dedicado**: `horizon-client-lib` ainda depende de renderer/input e o engine
continua compilando esses crates. A auditoria confirmou que não há um grupo
pequeno e autocontido de módulos no facade cuja extração, por si só, elimine
renderer/input do binário. Não tratar essas extrações como prova de build mais
rápido.

Validações desta extração:

- `cargo +stable fmt --all -- --check` passou.
- `cargo +stable test -p horizon-sim-data` passou: 28 testes unitários,
  1 teste ignorado (`levels_cost`) e doc-tests sem falhas.
- `cargo +stable clippy -p horizon-client --no-default-features --features
  server-plugins,dedicated-server -- -D warnings` passou, incluindo a compilação
  de `horizon-sim-data`, `horizon-client-lib` e `horizon-client-render`.
- `cargo +stable tree -p horizon-sim-data --depth 1` mostra somente
  `horizon-core` e `horizon-world` como dependências diretas. A árvore reversa
  do renderer no alvo dedicated continua mostrando os dois caminhos: dependência
  direta de `horizon-client` e dependência de `horizon-client-lib`.
- Cargo emitiu avisos não bloqueantes já conhecidos de manifests sem `[lints]`
  herdado; não foram introduzidos avisos de Clippy no código alterado.

### Cortes client-only: telas de avanços e estatísticas

`game.rs` agora seleciona as implementações completas de
`game/advancements_screen.rs` e `game/stats_screen.rs` no cliente e stubs dedicados
em `advancements_screen_dedicated.rs` e `stats_screen_dedicated.rs` no servidor.
Os stubs preservam assinaturas usadas pelos event handlers compartilhados, mas
não compilam layout/renderização, navegação, sorting/scroll ou estado da UI.
`game/stats.rs`, que contém os dados/persistência de estatísticas usados pelo
runtime, continua compartilhado. No total, 1.829 linhas das duas telas não entram
na compilação dedicada. O renderer ainda permanece por outros call sites; este é
corte de código compilado, não remoção do crate renderer nem medição de duração.

Após ambos os cortes passaram `cargo +stable fmt --all -- --check` e o Clippy
locked do binário dedicated
(`cargo +stable clippy --locked -p horizon-client --bin horizon-client
--no-default-features --features server-plugins,dedicated-server -- -D
warnings`). Tentei também validar o ramo não dedicado
(`cargo +stable check -p horizon-client --no-default-features --features
server-plugins,horizon-extras,client-graphics`), mas a compilação parou em erro
fora dos arquivos alterados: variável `prepared_at` não declarada em
`game/resource_load.rs:600`. Portanto, esse comando não confirmou o ramo cliente;
as implementações originais continuam selecionadas por cfg, mas falta resolver/
validar aquele bloqueio antes de afirmar check cliente.

### Próxima etapa de implementação

Inventariar a propriedade do estado e entry points de `Game` e separar o runtime
headless do estado de apresentação do cliente em módulos/grupos coesos. O alvo é
fazer o servidor depender diretamente das regras e dados compartilhados sem
precisar compilar `horizon-client-lib`/renderer, não apenas adicionar mais
fachadas. Depois de cada corte, confirmar com `cargo tree` que os crates saíram
do grafo dedicado e rodar as validações CI relevantes. Não alegar ganho de tempo
até medir builds limpos comparáveis; a árvore prova presença/ausência, não
duração. Nenhum commit ou push foi feito.

Inventário inicial de `Game` nesta etapa confirma o bloqueio: o mesmo struct
guarda mundo/estado de gameplay, streaming/LOD e dados do servidor, mas também
GUI, fog/screen effects, itens de ativação visual, panorama, cursor, teclas,
mouse, fullscreen, vídeo e estado de janelas/splitscreen. O caminho dedicado já
tem `game::dedicated::run`, mas constrói o mesmo `Game`; por isso, o próximo
corte deve mover propriedade/lifecycle do runtime para uma estrutura headless
antes de condicionar as dependências largas de `horizon-client-lib`. A auditoria
aponta um primeiro grupo para estudar: estado de janela/input (`window`,
renderer, fullscreen/video modes, foco/cursor/captura e mouse). Não gatear
`keys`, tela, GUI ou `options` por nome: há usos transversais em gameplay,
controles/headless e servidor integrado. Mapear todos os consumidores e validar
cliente, servidor integrado e dedicado antes de qualquer gate.

### Verificações e bloqueios desta sessão

- `git status --short --branch` estava limpo antes desta anotação.
- Na primeira revisão, este ambiente não tinha toolchain ativa. Na continuação,
  foram instaladas e verificadas as duas versões dos workflows: stable
  `1.100.0` com `rustfmt`/`clippy`, e a release fixa `1.99.1`.
- `cargo +stable fmt --all -- --check` passou. O Clippy oficial do engine dedicado
  (`cargo +stable clippy --locked -p horizon-client --bin horizon-client
  --no-default-features --features server-plugins,dedicated-server -- -D
  warnings`) passou em build frio em **1m17s** no ambiente desta sessão. Esse
  tempo é baseline de `clippy`/check, não é tempo de build release.
- `cargo +stable tree --locked -p horizon-client --no-default-features
  --features server-plugins,dedicated-server -i horizon-client-render` confirma
  dois caminhos para `horizon-client-render`: dependência direta de
  `horizon-client` e dependência direta de `horizon-client-lib`, que por sua vez
  é dependência de `horizon-client`. O check também compilou os crates
  `horizon-client-render` e `horizon-client-lib`.
- A árvore de dependências normais ainda exclui `image-webp`, `zune-jpeg`,
  `symphonia`, `arboard` e `wayland-client`; `x11rb` continua presente via
  `winit/x11`, requisito atual do crate para compilar os tipos de evento Linux
  usados pelo servidor. Corrigida abaixo a nota anterior que dizia que `x11rb`
  também estava ausente.
- Também passaram os outros comandos do workflow CI: Clippy de
  `horizon-server`, `horizon-prepare`, `horizon-server-plugin-api` e
  `horizon-server-plugin-loader` com `--all-targets`, e os testes de API/loader.
  Cargo emitiu avisos não bloqueantes de manifests sem `[lints]` herdado; não
  alterei esses manifests por não serem necessários para o corte de build pedido.
- Não foi feito build `--release` local. As próximas alterações devem continuar
  validadas serialmente pelo Clippy CI do engine e, após cada extração, pela
  árvore dedicada; build release só pelo workflow de release, com cache.

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
It no longer contains `image-webp`, `zune-jpeg`, `symphonia`, `arboard`, or
`wayland-client`; `x11rb` remains through the X11 implementation required by
`winit` for the dedicated server's event types. It still contains `horizon-client-render`,
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


## Plano: desacoplar o renderer do grafo dedicated (2026-11-21)

### Diagnóstico confirmado

A árvore normal do binário (`cargo +stable tree --locked -e normal -p
horizon-client --no-default-features --features server-plugins,dedicated-server
-i horizon-client-render --depth 4`) confirma dois caminhos, não uma dependência
única que possa ser apagada isoladamente:

1. Caminho transitivo: `horizon-client -> horizon-client-lib ->
   horizon-client-render`. `horizon-client-lib/Cargo.toml` declara renderer e
   input incondicionalmente; `client_lib.rs` reexporta centenas de módulos do
   renderer, além de declarar módulos próprios que usam seus tipos.
2. Caminho direto: `horizon-client -> horizon-client-render`. O código em
   `horizon-client/src/game/` importa API de HUD, graphics e renderer em
   muitos módulos; `Game` também possui estado e lifecycle de apresentação.

O renderer como `dev-dependency` ainda aparece em uma árvore Cargo sem filtro de
escopo. A aceitação da remoção no executável deve usar `-e normal` e/ou
`--bin horizon-client`, nunca interpretar dependências exclusivas de teste como
parte do grafo normal do binário. O manifest de `horizon-client` já mantém GPU
backends atrás de `client-graphics`, mas a dependência e o código/API comuns do
renderer continuam presentes. Portanto, a primeira alteração não será tornar
`horizon-client-render` opcional: isso deixaria imports e tipos irresolvidos.

### Direção escolhida

Fazer uma separação de propriedade de código/runtime com compatibilidade
incremental, e só depois mudar os manifests. A fronteira desejada é um target
server/headless que dependa de simulação, mundo, protocolo, dados, persistência
e API/loader `.so`, mas não do renderer, input de desktop, windowing ou shaders
visuais. O cliente e o servidor integrado continuam com a facade e o renderer.
Não duplicar a simulação nem trocar o comportamento de rede/persistência para
conseguir um grafo menor.

### Sequência proposta e gates

1. **Inventário reproduzível e linha de base.** Guardar os comandos `cargo tree`
   normal para renderer, input, `wgpu`, `winit`, shaders e facade; distinguir
   dependências diretas, transitivas e dev-only. Usar o target dedicated
   (`--bin horizon-client`, `--no-default-features`, `server-plugins,dedicated-server`)
   como referência. Medir tempos somente com builds limpos comparáveis, mesmos
   toolchain/runner, cache e workflow; até isso existir, árvore reduzida é prova
   de remoção do grafo, não de velocidade.
2. **Separar módulos server-safe da facade `horizon-client-lib`.** Identificar
   grupos de tipos/lógica necessários ao servidor que ainda são definidos no
   renderer, mover apenas os que são genuinamente domínio/dados compartilhados
   para crates neutros pequenos e substituir imports/reexports com caminhos de
   compatibilidade no cliente. Para cada grupo, auditar usos do servidor, cliente
   e plugins, além de testes. Não mover `HudBatch`, `Renderer`, recursos gráficos,
   texturas ou tipos cujo contrato dependa de GPU para crates compartilhados.
3. **Estabelecer uma entrada/runtime server independente de `Game` visual.**
   Extrair lifecycle de startup, tick, rede e propriedade dos dados do servidor
   para um crate/módulo server runtime. Migrar primeiro o caminho dedicado, sem
   tocar no servidor integrado; manter a simulação comum chamada pelos dois
   caminhos. `Game` contém estado visual/input e centenas de módulos ligados ao
   client lib, então este é o principal trabalho arquitetural e deve ocorrer em
   fatias compiláveis com testes de regressão. Os stubs das duas telas já feitos
   são cortes locais, não substituem esta fronteira.
4. **Remover a facade transitiva.** Quando o runtime dedicado importar apenas os
   crates neutros/server-safe, deixar o target dedicated sem dependência de
   `horizon-client-lib`; manter `horizon-client` e a facade atual no caminho do
   cliente para compatibilidade. Evitar `pub use` client-wide no target server,
   pois reexportar a facade pode manter todos os crates no grafo mesmo quando
   alguns símbolos não são chamados.
5. **Remover o caminho direto do renderer.** Depois de migrar callsites de
   `game.rs`/módulos compartilhados para a fronteira cliente, condicionar
   `horizon-client-render`, `horizon-client-input` e as dependências de window/
   shader que sobrarem ao feature de cliente. O dedicated deve compilar sem
   símbolos/tipos do renderer, não com renderer vazio ou stubs que preservem a
   dependência. Manter APIs completas para o cliente desktop.
6. **Verificação, desempenho e compatibilidade.** Em cada fatia: fmt, Clippy
   dedicated com `-D warnings`, testes server/API/loader de plugins `.so`, e
   `cargo tree -e normal` do binário. Após as etapas de fronteira: check/Clippy
   dos alvos cliente e integrated-server. Aceitação final exige que a árvore
   normal do binário dedicated não contenha `horizon-client-render` nem caminho
   até ele, renderer/window/GPU/shader client-only identificados; CI de release
   dedicada e testes de gameplay/rede/persistência/plugins passam; builds limpos
   comparáveis demonstram eventual efeito em tempo.

### Riscos e limites

- APIs de renderer são usadas tanto por código de UI quanto por carregamento e
  preparação de recursos. Antes de mover cada tipo, classificar se é formato de
  dados/headless ou recurso de apresentação; não retirar imagens/tabelas de que
  simulação, data-pack loading ou plugins dependam.
- `horizon-client-lib` tem muitos módulos, não apenas uma facade passiva. Um
  feature que esconda as reexports sem separar os módulos consumidores não é
  suficiente e pode quebrar imports públicos do cliente.
- Não acoplar o ABI `.so` a tipos internos do renderer nem alterar a API do plugin
  como efeito colateral. Preservar registros e protocolo do servidor.
- `winit` pode continuar necessário apenas para tipos de evento Linux por
  `server-event-types`; esse resíduo deve ser avaliado independentemente e não
  confundido com dependência do renderer. Dependências dev-only também não contam
  como bloqueio do executável dedicado.
- A árvore prova resolução/compilação esperada, mas não determina sozinha quais
  crates foram recompilados em uma build incremental nem o ganho no CI. Não
  anunciar economia temporal até colher medidas controladas.

**Próxima ação de implementação:** produzir mapa de propriedade/imports do
caminho `game::dedicated::run -> Game::new -> tick` e do resource loader, marcando
cada tipo de renderer como (a) apresentação dispensável, (b) dado server-safe a
extrair, ou (c) ainda não classificado. Usar esse mapa para selecionar uma
primeira fatia do runtime que remova imports transitivos de `horizon-client-lib`;
não editar manifests do renderer antes de existir um alvo dedicated que não
consuma esses símbolos. Nenhum commit ou push foi feito.


### Primeira fatia do runtime dedicado implementada

O `Watchdog` foi movido de `game/dedicated.rs` para
`game/dedicated/watchdog.rs`; os callsites de `start`, `tick_due` e shutdown
continuam no loop dedicated, e o encerramento agora passa por `Watchdog::stop`.
O watchdog preserva a thread observada, o limite de atraso, os logs e o crash
report. Isso separa uma responsabilidade server-owned do arquivo de orquestração,
mas **não remove dependências nem módulos do grafo dedicated e não é evidência de
build mais rápido**. O Clippy locked do binário dedicated com `-D warnings` e
`cargo +stable fmt --all` passaram após a extração.

O mapeamento confirmou o bloqueio maior: `dedicated::run` ainda chama
`Game::new` e `Game::run_dedicated`; o loop invoca a simulação compartilhada e o
`Game` mantém campos de renderer/input junto com mundo, entidades, rede e
persistência. O resource loader também tem tipos de `RenderAssets`/`ItemRenderer`
mesmo nos caminhos headless. A próxima fatia com impacto no grafo deve começar
pelo inventário dos campos e argumentos necessários a `Game::new` e
`load_resources` em dedicated, separar o bundle server-safe dos recursos de
apresentação e migrar um consumidor por vez. Só depois desse desacoplamento será
seguro tornar a facade transitiva opcional; o caminho direto do renderer é uma
etapa separada.


## Auditoria de `creative.rs` no alvo dedicated (2026-11-21)

Foi avaliado `crates/horizon-client/src/creative.rs` (1.828 linhas) como possível
módulo client-only a excluir da compilação dedicada via feature em
`horizon-client-lib`. Não foi feita alteração de código: a auditoria dos
consumidores mostrou que o módulo contém estado e comportamento usados pelo
`Game` compartilhado, não apenas desenho da tela.

- `Names` (`creative.rs:140-188`) carrega nomes do root de recursos e namespaces
de mods, resolve traduções e é referenciado por caminhos compartilhados. Removê-lo
ou trocar por mapa vazio poderia alterar nomes usados por comandos/recursos.
- `Creative::inventory_slots`, `inventory_slot_position` e `DISPLAY`
(`creative.rs:222-249`), além de métodos como `first_row` e `shown`, têm usos em
`inventory_input.rs`, `menu_clicks.rs` e `ui_control.rs`. O mapeamento de IDs e
posições de slots deve continuar coerente com a simulação de inventário.
- `creative_screen.rs` usa `tab`, `tabs_containing`, `search_tag_lines`,
`tab_headers`, `on_page`, `pages`, `scroller_offset` e estado de busca; handlers
compartilhados usam a aba selecionada e seus flags `search`/`inventory`.
- `menu.rs` consome `Names` e `click_display`; não se pode substituir o clique
por um no-op sem provar que esse caminho não é necessário a controles/inventário.

A maioria das funções visuais pode não ser chamada no fluxo headless, mas o binário
continua compilando `Game` e seus handlers; uma seleção cfg da facade não basta.
Um stub teria de preservar tipos, campos e semântica de inventário/nome, ou a
fronteira de runtime teria primeiro de excluir os módulos consumidores. Como essa
substituição reduziria apenas uma parte do código e não retiraria o caminho direto
nem transitivo de `horizon-client-render`, não foi considerada uma fatia segura
nem uma demonstração de redução relevante do custo que o usuário observa em
`horizon-client-lib`.

**Decisão:** não criar feature/stub dedicated para `creative.rs` nesta etapa. O
próximo corte deve vir da extração do runtime de apresentação para fora do `Game`
compartilhado: primeiro separar desenho/navegação da tela criativa (mantendo tipos
e operações de inventário server-safe), depois reavaliar a biblioteca de recursos
visuais e os dois caminhos Cargo do renderer. A feature da facade só deve ser
introduzida quando os imports compartilhados puderem ser removidos sem alterar
simulação, rede, persistência ou plugins `.so`.

**Validação/limitação desta rodada:** auditoria estática dos tipos, assinaturas e
call sites em `creative.rs`, `client_lib.rs`, `menu.rs` e módulos de `game/`;
nenhum código foi modificado e nenhum novo teste/build foi executado, portanto não
há nova alegação de compatibilidade ou ganho de build. Permanecem válidas somente
as validações listadas nas seções anteriores (Clippy dedicated, fmt e diff-check
para as alterações correspondentes). A árvore dedicated continua incluindo
`horizon-client-render` por ambos os caminhos já documentados.


## Extração do desenho da tela criativa (2026-11-22)

Foi separado o desenho da tela criativa das ações e geometria que o engine
compartilhado ainda usa:

- Novo `crates/horizon-client/src/game/creative_screen_render.rs` contém
  `draw_creative`, `draw_creative_pages`, títulos e tooltips próprios da tela.
  São 316 linhas de implementação extraídas (321 linhas no novo arquivo com
  cabeçalho e imports).
- `game.rs` declara esse módulo somente quando
  `not(feature = "dedicated-server")` e condiciona o braço que desenha a tela.
  O braço dedicated não faz desenho de tela; o loop dedicado não abre janela nem
  chama esse caminho de renderização.
- `game/creative_screen.rs` conserva `click_creative`, seleção/página/busca,
  detecção da lixeira, geometria comum e tratamento de slots. `Creative`,
  `Names`, `click_display`, slots e APIs usadas pelo inventário continuam
  inalterados, preservando a simulação e operações compartilhadas.

Validações desta fatia:

- `cargo +stable fmt --all -- --check` passou.
- `git diff --check` passou.
- Clippy locked do binário dedicated passou, serialmente, com `-D warnings`:
  `cargo +stable clippy --locked -j1 -p horizon-client --bin horizon-client
  --no-default-features --features server-plugins,dedicated-server -- -D
  warnings`. Houve apenas avisos de herança `[lints]` em manifests já conhecidos.
- A árvore normal dedicada (`cargo +stable tree --locked -e normal -p
  horizon-client --no-default-features --features server-plugins,dedicated-server
  -i horizon-client-render --depth 4`) continua mostrando o renderer tanto como
  dependência direta de `horizon-client` quanto via `horizon-client-lib`.

**Escopo e limite:** agora 316 linhas do desenho específico da tela criativa não
entram na compilação dedicada, mas `horizon-client-lib`, `horizon-client-render`,
input, windowing e o código shared de interação ainda entram. Esta extração não
removeu dependências Cargo nem demonstra redução mensurável de tempo. O bloqueio
anterior de `prepared_at` foi corrigido em `game/resource_load.rs`; depois disso,
o check e o Clippy locked do binário cliente passaram com
`server-plugins,horizon-extras,client-graphics`. Nenhum teste automatizado
específico de UI foi executado.

Próxima etapa: mapear/extrair também a navegação e o estado puramente visual da
creative screen sem mover `Creative` ou operações de inventário, e em paralelo
começar a separar os recursos/render state do objeto `Game`. Só quando a facade e
o runtime dedicado deixarem de importar os tipos consumidores será seguro tornar
`horizon-client-lib`/renderer opcionais no target dedicated. A etapa maior de
remover os dois caminhos do renderer permanece aberta; preservar simulação,
rede, persistência e a API/loader de plugins `.so` como gates.


## Implementação: módulo Realms client-only com superfície dedicated mínima (2026-11-22)

A implementação Realms é outra fatia de tela/serviço client-side: `realms.rs`
tem 12.977 linhas e seus quatro submódulos somam mais 4.607, totalizando
17.584 linhas de código-fonte original. Os arquivos contêm 89 testes unitários
marcados com `#[test]` (38 em `realms.rs`, mais 51 nos submódulos). Desses
arquivos, 2.683 linhas ficam em regiões `#[cfg(test)]`; portanto, a variante
normal do binário compila 14.901 linhas de implementação original. A auditoria de
`title.rs` mostrou usos executáveis de uma superfície menor
(criação/cancelamento de mundo, click/navegação, polling, foco/narração e desenho), além de
`server_packs.rs`, que consulta `take_pending_pack` após um fluxo de cliente
Realms. Os testes de Realms em `realms.rs` começam sob `#[cfg(test)]`; sete
testes Realms de interação/upload em `title.rs` e `title/tests.rs` também são
`#[cfg(test)]` e agora ficam adicionalmente sob
`#[cfg(not(feature = "dedicated-server"))]`, pois verificam UI/serviço que o
stub intencionalmente não implementa. Esses testes ficam fora do Clippy do binário
`--bin horizon-client` usado pela CI dedicada.

Foi adicionada a feature `dedicated-server` à facade `horizon-client-lib` e
propagada pela feature homônima de `horizon-client`. `client_lib.rs` seleciona a
implementação Realms original (`realms.rs`) no cliente e o novo
`realms_dedicated.rs` no servidor. O stub dedicado ocupa 177 linhas; a variante
dedicated deixa de selecionar as 14.901 linhas de implementação original,
substituindo-as por esse stub (diferença estática de 14.724 linhas). As 2.683
linhas `cfg(test)` também ficam fora do alvo, mas já não eram compiladas pelo
binário normal. O stub mantém os
tipos/assinaturas requeridos por `title.rs` e o tipo de pack requerido por
`server_packs.rs`; métodos de tela são inertes e `take_pending_pack` retorna
`None`, pois a instância dedicated não tem sessão, UI nem fluxo de cliente
Realms. A implementação completa e seus submódulos continuam selecionados no
cliente. Os tipos de criação e pack do stub preservam somente as assinaturas
necessárias para compilar o código shared; a definição completa de
`RealmServer` e a API de serviços continuam exclusivas do cliente.

**Validações executadas nesta fatia:**

- Clippy locked do binário dedicated com `-D warnings` passou serialmente após
  iterar a API do stub. A primeira tentativa encontrou faltas de
  `main_arrow`/`pdp_arrow`, mutabilidade de `draw` e do campo `RealmsPack.id`;
  os contratos foram ajustados e a repetição final passou.
- Check locked do binário cliente passou com
  `server-plugins,horizon-extras,client-graphics`; Clippy locked do mesmo alvo
  com `-D warnings` também passou. Durante a primeira tentativa, um `#[path]`
  explícito impediu a resolução dos submódulos `realms/{availability,client,
  download,upload}.rs`; substituí-o pela declaração de módulo convencional para
  preservar a localização correta dos submódulos. A repetição cliente passou.
- `cargo +stable fmt --all -- --check` passou depois da correção de resolução,
  assim como `git diff --check`.
- `cargo tree --locked -e normal` confirma que `horizon-client-render` ainda
  chega ao binário pelos mesmos caminhos: dependência direta de `horizon-client`
  e indireta via `horizon-client-lib`. O corte Realms reduz código Rust
  selecionado no dedicated, não remove crate/dependências nem mede duração.
- O teste seletivo cliente mais amplo passou com 94 casos e 0 falhas:
  `cargo test --locked -j1 -p horizon-client-lib --lib --no-default-features
  realms -- --test-threads=1`. Inclui os 89 testes no namespace dos cinco
  arquivos Realms, mais casos de integração de title/Realms e carregamento de
  player-data. São testes da implementação cliente completa, não do stub do
  servidor.
- Também passaram os 13 testes `title::classic_tests::` do cliente, incluindo
  navegação por setas, foco na lista e passagem de Tab pela tela Realms:
  `cargo test --locked -j1 -p horizon-client-lib --lib --no-default-features
  title::classic_tests:: -- --test-threads=1`.
- A compilação separada do harness da biblioteca com
  `cargo test --locked -j1 -p horizon-client-lib --lib --no-default-features
  --features dedicated-server --no-run` passou (`Finished test profile`; harness
  gerado). A primeira tentativa identificou dois testes adicionais de upload/
  cancelamento Realms em `title/tests.rs`; os sete testes interativos/serviço
  agora são client-only e a repetição compila.

**Limitações e semântica:** a UI Realms do título não pode ser usada no processo
dedicado; se chamada diretamente, as operações do stub não executam serviços e
retornam resultados vazios/inertes. A função de pending pack não pode transportar
um pack de uma sessão Realms no dedicated. Essa exclusão não toca na simulação,
rede de jogadores, persistência ou ABI/loader `.so`; contudo, exige manter o
pressuposto arquitetural de que startup dedicated não instancia nem chama a tela
Realms como cliente. A árvore e builds não demonstram economia de tempo. Nenhum
commit ou push foi feito.

Auditoria preliminar do próximo módulo grande, `multiplayer.rs` (2.816 linhas),
desaconselha um stub wholesale: além da tela, outros caminhos de `game/` consultam
o estado `Disconnected`, guardam/abrem endereços e acessam persistência de lista,
ícones e packs (`game/remote.rs`, `code_of_conduct.rs` e `server_packs.rs`). Uma
futura extração deve separar a tela/login da conexão, dos endereços persistidos e
da política de resource packs; não substituir esses últimos por no-ops.

Próxima etapa: decidir se testes de biblioteca com `dedicated-server` são
necessários para CI e, se forem,
manter somente os testes de UI/serviço Realms fora do alvo sem ocultar testes de
simulação. Continuar em seguida a separação do estado de apresentação de `Game`;
não usar esta remoção de UI como prova de que o renderer saiu do grafo.

## Implementação: desenho do título client-only no alvo dedicated (2026-11-22)

A apresentação do título também foi separada sem transformar o módulo inteiro
em stub. Os métodos `Title::draw` e `Title::draw_with_items` foram movidos,
sem alteração da implementação, para `title/render.rs`, uma variante incluída
somente quando `dedicated-server` não está habilitada. O bloco tem 860 linhas
com cabeçalho e espaçamento. No dedicated, `title/render_dedicated.rs` mantém as
mesmas assinaturas públicas com métodos inertes, pois o processo não exibe tela
de título nem previews de itens. O estado, navegação, interação, criação e
abertura de mundo continuam no módulo original; não foram removidos nem
substituídos por no-ops. A constante de UI `dialog_screen::BOX` passou a ser
importada apenas pelo renderizador.

Como parte da fronteira ainda compartilhada de `title.rs` fica intencionalmente
sem chamada no dedicated, o módulo aplica `allow(dead_code)` apenas sob a
feature `dedicated-server`. Isso aceita estruturas/helpers de UI preservados no
módulo comum sem enfraquecer os lints dos outros módulos; trata-se de uma
supressão deliberada e limitada ao título, e não de evidência de que toda a UI
tenha deixado de ser compilada.

**Validações executadas nesta fatia:**

- Clippy locked do binário dedicated com `server-plugins,dedicated-server` e
  `-D warnings` passou; check locked e Clippy locked do binário cliente com
  `server-plugins,horizon-extras,client-graphics` também passaram.
- `cargo fmt --all -- --check` e `git diff --check` passaram.
- Os 13 testes `title::classic_tests::` do cliente passaram (0 falhas).
- `cargo test --locked -j1 -p horizon-client-lib --lib --no-default-features
  --features dedicated-server --no-run` compilou o harness dedicado.
- `cargo tree --locked -e normal -i horizon-client-render -p horizon-client`
  continua mostrando `horizon-client-render` como dependência direta de
  `horizon-client` e indireta via `horizon-client-lib`. A alteração reduz a
  implementação de desenho selecionada pelo alvo dedicated, mas não retira o
  crate nem suas dependências do grafo Cargo.

**Limites:** nenhum benchmark pareado foi executado; não se afirma economia em
segundos. Os métodos de desenho não têm efeito no dedicated, enquanto as
implementações reais continuam no cliente. Nenhuma alteração foi feita à
simulação, protocolo/rede, persistência de mundos ou API/loader de plugins `.so`.
Nenhum commit ou push foi feito.

## Comparação local com os workflows GitHub Actions (2026-11-22)

Os dois runs fornecidos foram consultados em modo somente leitura. Ambos apontam
para o SHA `80e945c34df1c57f4faf9ced4eca0e701e470aaf`, que também é o `HEAD`
local, mas a árvore de trabalho local contém alterações não commitadas. Assim,
os comandos locais abaixo compilam o estado atual modificado; não são
reprodução bit a bit do conteúdo que os runs GitHub executaram.

No [run de checks](https://github.com/RochaProjects/BorizonServer/actions/runs/45051555374),
o GitHub informou 16 s para formatação, 77 s para Clippy do engine dedicated,
14 s para Clippy das ferramentas/SDK e 1 s para os testes do SDK/loader,
aproximadamente 108 s no total para esses passos. Reproduzi os mesmos comandos
em série, com stable `1.100.0` e um `CARGO_TARGET_DIR` temporário inicialmente
vazio: 12,35 s, 75,03 s, 17,90 s e 4,34 s, respectivamente (109,62 s no total).
O cache do GitHub teve correspondência completa; o target temporário local não
reutilizou artefatos. Apesar das diferenças, o Clippy dedicated foi próximo:
75 s local contra 77 s no run.

No [run de release](https://github.com/RochaProjects/BorizonServer/actions/runs/44792084239),
o step que faz build e empacota durou 12 min 53 s. O cache Rust do GitHub não foi
encontrado. Reproduzi em série os dois comandos de build `--release`, com Rust
`1.99.1`, também usando target temporário inicialmente vazio: 447,69 s para o
engine dedicated, 99,42 s para `horizon-server` e `horizon-prepare`, e 3,51 s
para copiar/empacotar localmente, totalizando 550,63 s (9 min 10,63 s). A etapa
de publicação `gh release create` não foi executada; nada foi enviado. A cópia
de assets e arquivo tar foram produzidos somente sob `/tmp` e removidos ao fim.

**Interpretação:** o tempo observado local para o step de release foi cerca de
3 min 42 s menor que o tempo do runner GitHub, mas a comparação não isola efeito
das mudanças: são máquinas diferentes, as fontes locais incluem alterações
ausentes no SHA remoto e o cache de fontes/registry do Cargo local já estava
presente, embora o target temporário estivesse limpo. Na CI, o cache e os
ambientes também diferem. Portanto, esses números validam que os comandos locais
passam e dão uma referência do estado atual, mas não demonstram que as extrações
client-only reduziram o tempo. Para medir esse efeito causal, ainda é preciso
executar um benchmark pareado do mesmo commit-base e do estado alterado, no mesmo
ambiente/toolchain e com a mesma política de cache. Não houve commit nem push.

Próxima etapa: investigar se a dependência direta de `horizon-client-render`
pelo crate `horizon-client` pode ser condicionada à feature client sem quebrar
interfaces compartilhadas, plugins ou o build dedicado; preservar a dependência
indireta via `horizon-client-lib` até que sua responsabilidade seja auditada.

# Auditoria de extração server-only — BorizonV26.3

## Escopo e disciplina da evidência

Este relatório começou como consolidação de auditorias anteriores. A rechecagem local de 2026-11-26 abriu a referência `../main` (`f2e5556cbcb8060313f513d1871ccf3e7fb6e6ec`) e verificou os arquivos citados abaixo; não compilou nem testou a referência. A branch de trabalho está em `837c329eb6aeb96e0162fde1b26e7083d89c486b`. Ela e `main` não têm merge-base: a referência é fonte para leitura/extração seletiva, não uma branch que possa ser mesclada normalmente. Onde a call graph não foi percorrida até um consumidor, o estado continua não avaliado. “Server-only” é interpretado estritamente para o binário final: sem código/dependências client, GPU, render, input, mod ou plugin no grafo nem no artefato. O supervisor é um processo separado e não substitui o engine.

## Estado atual da branch — 2026-11-28

Esta seção prevalece sobre os snapshots históricos abaixo. O workspace server-only implementa listener STATUS e LOGIN→CONFIGURATION→PLAY, runtime Overworld compartilhado, batches de chunks iluminados, movimento/colisão limitados, mineração e drops/coleta/colocação básicos, inventário básico, relógio global, ticks agendados/aleatórios/block events, ticketing e salvamento parcial de mundo/jogador. Playerdata passa pelo DataFixer vanilla e restaura/salva posição, rotação, slot selecionado e slots de inventário no formato NBT vanilla. O cliente vanilla 26.3 alcançou PLAY e houve teste manual de uma fatia survival em etapas anteriores; não há validação de cliente após as mudanças mais recentes. Nesta rodada, o ticker de pistões móveis existente em `horizon-world` foi ligado ao ciclo PLAY, `SelectedItemSlot` e inventário entraram no round-trip do playerdata, e o DataFixer passou a rodar antes da leitura/salvamento do composto. A ponte NBT de itens exige round-trip exato e rejeita componentes desconhecidos ou não representáveis; não há suporte comprovado a todos os componentes de 26.3, então este caminho ainda pode recusar playerdata com itens avançados em vez de salvá-los com perda. `cargo fmt --all -- --check`, `cargo check --locked -p horizon-engine` e `git diff --check` passaram. Nenhum teste foi executado nesta rodada; o grafo normal do engine permanece sem crates client/render/input/GPU.

Ainda não há paridade vanilla comprovada. Faltam, entre outros, validação/runtime recente da expansão de chunks além de 3×3; componentes e dados de jogador ainda não cobertos no round-trip de inventário; movimento/física e proteção contra movimento inválido completos; sincronização de entidades/jogadores; menus/crafting; tickers gerais de block entities e entidades; dimensões de jogo integradas; salvamento completo e shutdown limpo. As seções abaixo de “Rechecagem local” são registros históricos, não descrições completas do HEAD atual.

Direção de migração definida: `main` é a fonte funcional de comportamento. O objetivo é portar seletivamente seu caminho dedicado para `BorizonV26.3`, separar/remover os acoplamentos client e manter a lógica vanilla original. O bootstrap de PLAY desta branch é provisório; não deve ser ampliado para substituir sistemas que já funcionam na `main`. Cada parte deve ser comparada e reutilizada ou substituída pelo código migrado, conforme o callsite real.

### Tick server-side de pistões — 2026-11-28

O loop compartilhado agora executa `Behaviours::tick_block_entities` após `run_block_events`, seguindo a ordem da fase `tick_moving_pistons` na referência `main/crates/horizon-client/src/game/block_ticks.rs`. A chamada passa por `ResidentDimension` e `LocalWorldRuntime`; apenas moving pistons são cobertos pela implementação existente de `horizon-world`. Verificação local: `cargo fmt --all -- --check`, `cargo check --locked -p horizon-engine` e `git diff --check` passaram. Não houve teste de runtime nem execução de testes.

### Slot selecionado do hotbar em playerdata — 2026-11-28

`player_data` restaura e persiste `SelectedItemSlot` (NBT Int) dentro do intervalo vanilla 0–8. O slot selecionado da sessão é passado pelo login e incluído no salvamento ao desconectar. Verificação: `cargo fmt --all -- --check`, `cargo check --locked -p horizon-engine` e `git diff --check` passaram; sem teste de runtime nesta rodada. O conteúdo dos slots ainda não é persistido.

### DataFixer aplicado ao playerdata — 2026-11-28

O Borizon chama `horizon_world::datafix::update_player` ao ler o arquivo e antes de salvar o composto existente, seguindo a migração de playerdata vanilla. Assim, um playerdata antigo não recebe `DataVersion=CURRENT` sem que seus itens e demais campos tenham passado pelas correções da versão. Validação: `cargo fmt --all -- --check`, `cargo check --locked -p horizon-engine` e `git diff --check` passaram; sem teste de runtime nesta rodada.

### Round-trip inicial dos slots de inventário — 2026-11-28

O engine agora mapeia os slots vanilla de hotbar, armazenamento, armadura e mão secundária para o `PlayerInventory`, e os lê/grava no playerdata usando o formato atual de stacks após o DataFixer. A ponte usa a conversão disponível em `horizon-protocol` e valida que a serialização preserve exatamente o stack que o engine conhece. Componentes desconhecidos ou não representáveis produzem erro explícito; suporte completo aos componentes vanilla 26.3 ainda não foi auditado nem implementado. `cargo fmt --all -- --check`, `cargo check --locked -p horizon-engine` e `git diff --check` passaram; sem testes nem validação de runtime nesta rodada.

## Rechecagem local do caminho dedicado — 2026-11-26

Estado confirmado no workspace `/home/mago/Arte_Programacao/BorizonServer/BorizonV26.3`:

- A branch contém cinco membros do workspace: `horizon-core`, `horizon-world`, `horizon-protocol`, `horizon-server-files` e `horizon-engine`. O engine oferece `--help`, `--version`, `--preflight` e `--validate-resources`; ainda recusa iniciar gameplay porque não tem runtime de jogo.
- Caminho dedicado da referência: `crates/horizon-client/src/entry.rs::main` chama `game::run_with`; `game.rs::run_with` lê as opções, executa o preflight dedicado, valida recursos e despacha para `game/dedicated.rs::run`.
- `dedicated::run` ainda carrega `load_resources` e `UiResources`, cria `Audio::silent`, então `start_dedicated_game` constrói o owner `Game` e chama `Game::run_dedicated`. O `Game` contém estado de renderer, UI, câmera, áudio e input junto do estado compartilhado da simulação. Copiar `dedicated.rs` ou ativar `dedicated-server` não isola esse owner.
- `run_dedicated_inner` prepara settings/seed/diretórios, carrega o mundo via `WorldRequest`, inicializa serviços/listas/jogadores e entra no loop do servidor. O loop chama `tick_server_spans` em `game/levels.rs`; este chama `tick_server`, que percorre dimensões, atualiza streaming e mundo e executa `tick`/`tick_lan`. Esses métodos ainda estão em `horizon-client` e alcançam estruturas/hooks de mods/plugins que não pertencem ao primeiro produto.
- O `Cargo.toml` de `horizon-client` liga diretamente `horizon-client-lib`, `horizon-client-core`, `horizon-client-input`, `horizon-client-render`, `horizon-assets`, `horizon-command`, `horizon-items`, shaders, `wgpu`, `winit` e outras dependências visuais. A feature `dedicated-server` apenas desliga alguns recursos opcionais; não cria uma fronteira de pacote server-only.
- O caminho legado foi inspecionado estaticamente. Nesta sessão `cargo build -p horizon-engine --locked --offline` passou depois de completar o cache, e `--validate-resources` rejeitou corretamente um diretório inexistente. Também foi executado o gerador de dados oficial dentro do `26.3-server.jar` (`net.minecraft.data.Main --server --reports`), que concluiu e produziu dados/relatórios usando só o jar server. O grafo normal/build de `horizon-engine` não contém pacotes client; ainda não existe validação bem-sucedida de um pacote de recursos completo. Nenhum pacote client foi compilado.

| Caminho | Implementação de referência | Base/destino na branch | Situação real |
|---|---|---|---|
| Configuração e EULA | `horizon-client/src/game/dedicated.rs::preflight` | `horizon-server-files`; `horizon-engine::preflight` | Recorte operacional já existe e compila no engine. |
| Carregamento de recursos | `game/dedicated.rs::run` → `game/resource_load.rs::load_resources` | `horizon-engine::resources::load` | O loader server-only já valida registry, comportamentos vanilla e worldgen sem carregar assets visuais. Ainda falta o preparador server-only e um bundle completo válido; o loader legado também monta `UiResources`. |
| Owner do mundo | `game/world_loading.rs`, `game/world_session.rs`, `Game::load_world` | `horizon-world::LocalWorldRuntime`, `ResidentDimension`, `ChunkData`, `RegionFile` | A branch tem owners parciais de dados/chunks; `horizon-engine` ainda não instancia nenhum deles. A migração precisa provar bounds, registries, dimensões e abertura de mundo. |
| Tick e gameplay | `game/levels.rs::tick_server` → `Game::tick`/`tick_world`; blocos, entidades, veículos, portais e jogadores permanecem em vários módulos de `horizon-client` | `horizon-world::level::Behaviours` implementa fases de bloco com eventos devolvidos ao owner | Não há loop no engine. `tick_server` chama o owner misto, hooks de extensão e `update_dedicated_streaming`; os eventos de bloco, entidades e jogadores ainda precisam de um owner server-only completo. |
| Rede e protocolo | `game/lan.rs`/`LanHost` e `horizon-client::lan::hub`; codecs em `horizon-protocol` | `horizon-protocol` já fornece framing, packets, crypto e adapters; sem socket/login hub na branch | Codec não equivale a servidor de conexão: falta listener, status/handshake/login/configuração/play, sessão de jogador e envio de chunks. |
| Streaming e geração | `Game.streaming` (`horizon-client-render::streaming`), `game/dedicated.rs::update_dedicated_streaming`, jobs do renderer | `horizon-world::worldgen`, tickets e runtime residente | A branch tem gerador e lifecycle caller-driven; não há scheduler integrado, luz final, readiness de rede nem envio de chunk. Não portar o crate renderer. |
| Persistência e shutdown | `game/autosave.rs`, `game/saving.rs`, `game/world_session.rs` | codecs NBT/Anvil/datafix e `LocalWorldRuntime` | APIs de chunk/SavedData existem, mas `horizon-engine` não abre/salva mundo, entidades/POI/playerdata nem executa flush no shutdown. |

**Próximo corte de extração:** criar o preparador de recursos usando exclusivamente o jar server, provar a carga de `GeneratorData` e gerar/salvar chunks vanilla. Depois ligar `LocalWorldRuntime` a um tick dedicado e ao socket/protocolo. O binário do engine já existe, mas ainda não aceita jogadores.

## 1. Conclusão executiva: viabilidade e velocidade relativa

A extração é **viável em princípio, mas não é um corte rápido de feature nem uma remoção simples de dependência**. Há blocos substanciais reaproveitáveis: `horizon-core`, `horizon-world`, `horizon-protocol` e `horizon-server-files` já estão no workspace da branch; `horizon-world` contém NBT, regiões Anvil, chunks e datafix; o protocolo possui framing, codecs e conexão; e as auditorias da referência descrevem um caminho dedicado com preflight, tick, rede, chunks, persistência e shutdown. Esses fatos sustentam uma estratégia de extração, não a afirmação de que já existe um engine independente.

O primeiro obstáculo é ownership: a rechecagem da árvore confirmou que a branch atual não contém os quatro arquivos da implementação de referência (`horizon-client/Cargo.toml`, `src/entry.rs`, `src/game.rs`, `src/game/dedicated.rs`) e tampouco contém a camada LAN/hub, `horizon-client-render` ou os crates do snapshot WIP. O WIP `5f450ba163` é um snapshot alternativo, não ancestral do HEAD citado. Portanto, não se deve tratá-lo como implementação integrada nem copiá-lo cegamente.

**Velocidade relativa:** uma prova de conceito que apenas inicialize um processo server, use os crates já presentes e faça um tick/smoke mínimo parece substancialmente mais rápida do que um servidor jogável estritamente server-only. O primeiro binário que satisfaça integralmente tick autoritativo, rede, carregamento/streaming de chunks e ausência comprovada de todas as dependências client exige refatoração ampla do owner `Game`, streaming, session/hub e fronteiras de recursos. A auditoria não permite estimar prazo numérico nem afirmar que esse primeiro corte funcional completo será rápido. O perfil `dedicated-server`/`--no-default-features` isoladamente não prova exclusão de dependências ou código client.

**Conclusão honesta:** há uma rota arquitetural plausível, mas não há evidência de binário server-only compilável ou executável hoje. O maior risco é que os subsistemas server essenciais permaneçam transitivamente presos ao agregado client; isso é bloqueio de integração até refatorar e medir, não impossibilidade técnica demonstrada.

## 2. Fronteiras: estado, evidência e ação

Estados: **reuse** = reutilizar como base, após validar encaixe; **cut** = excluir do binário final; **refactor** = separar antes de incluir/excluir; **blocker** = impedimento atual à alegação de conformidade, não prova de impossibilidade.

| Fronteira | Estado | Evidência fornecida | Ação verificável |
|---|---|---|---|
| Crates existentes da branch: `horizon-core`, `horizon-world`, `horizon-protocol`, `horizon-server-files` | reuse / refactor | São os membros do workspace corrente. `horizon-world` exporta chunk, NBT, region, datafix e `world_saved_data`; `horizon-protocol` contém connection/frame/packets/crypto; `horizon-server-files` cobre configuração e listas. | Manter como base; rastrear dependências diretas/transitivas do binário e resolver acoplamentos de `horizon-protocol` a `horizon-world` somente com prova do grafo/uso. |
| Entry point e owner `Game` | blocker / refactor | Na referência, `Game` mistura simulação com Renderer, Window, Screen, GPU, input, HUD, câmera, áudio e partículas; `Game::new` instancia estado visual. Os quatro arquivos de referência estão deletados na comparação reportada da branch. | Criar entry point próprio e owner de runtime server. Portar seletivamente fluxo dedicado/tick/shutdown; não copiar `Game` ou `dedicated.rs` integralmente. |
| Loop dedicado, tick e lifecycle | refactor | `tick_server`/`tick_server_spans` e o loop dedicado descritos cobrem simulação, cadência, pausa, métricas, watchdog e shutdown, mas callbacks alcançam módulos do agregado client. | Isolar o contrato do tick e seus serviços de mundo/rede/hooks; manter integração server-side necessária e provar que o grafo do binário não alcança client. |
| Streaming, worldgen, iluminação e chunks | reuse / refactor / cut | `horizon-world::worldgen` e `horizon-sim-data::light` são candidatos server-side; `ChunkMap`/`Chunk` são estado de simulação. A referência liga o dedicado a `horizon-client-render::streaming`, que mistura geração/luz/save com meshing e GPU. O crate não está na branch observada. | Preservar geração, iluminação, residência/readiness/tickets e save necessários; extrair dispatcher headless. Cortar `Mesher`, `SectionMesh`, `Job::Mesh`, upload, LOD visual e GPU. Não remover `horizon-client-render` sem substituto para o contrato de chunks. |
| Encoding/transporte de chunks | refactor | `ChunkEncoder` na referência está sob `horizon-client::lan::chunks`; protocolo wire existe em `horizon-protocol`. | Mover/adaptar encoding de chunk/luz para módulo de protocolo ou runtime server; não reimplementar packet parsing/framing. |
| Protocolo base | reuse / revisão | `horizon-protocol` contém codecs, framing, conexão, criptografia e adapters; seu manifest depende também de `horizon-world`. | Reutilizar; comprovar que nenhum módulo/opção client fica ativado e inspecionar o subgrafo real antes de alegar isolamento. Preservar suporte de estados de protocolo exigido. |
| Login, admission, LAN hub e play loop | blocker / refactor | A branch não contém LAN/hub dedicado. Na referência, login mistura auth/admission com APIs de client, integrated/memory, hooks e observabilidade; `Hub` consome tipos de mundo/engine sob `horizon-client`. | Extrair poller, login/auth/admission e hub contra interfaces server-owned; manter status/login/configuration/play, keepalive e eventos necessários. Definir explicitamente política para adapters de versões e forwarding. |
| Recursos, registries e preparação | refactor / blocker potencial | A branch carrega arquivos preparados por caminho/nome `.local/horizon-client`; os exporters/manifests de `horizon-prepare` não estão no checkout auditado. Os dados de registry, tags, datapacks e worldgen são consumidos por `horizon-world`. | Definir resource set server com exporters mínimos. Preservar dados de gameplay/worldgen/datafix efetivamente lidos; excluir assets visuais. Não afirmar que o preparer dependa de cliente gráfico sem inspecionar seu código. |
| Persistência de mundo, jogador e ciclo de dimensão | reuse / refactor | A branch já tem region/NBT/chunk/datafix, mas não a orquestração de `SaveDir`, `WorldRequest`, dimensões ou playerdata. A referência tem save distribuído e integração com Game. | Conectar APIs existentes a abertura/fechamento de mundo, entidades/POI, saved data, player UUID, datafix e flush no shutdown. Não criar terceiro backend de regiões nem substituir codecs sem round-trip. |
| `horizon-sim-data` e `horizon-client-core` do WIP | blocker / refactor | Snapshot WIP documenta dependência `horizon-sim-data -> horizon-client-core` e imports client-core em múltiplos módulos. Esses crates não estão no workspace corrente. | Se forem trazidos, dividir/extrair os tipos e módulos compartilhados server-safe; não depender do crate misto no engine final. Confirmar alcance por grafo. |
| Plugins/mods/hooks | cut para o primeiro binário estrito | Snapshot WIP ativa `server-plugins` no perfil dedicado e o loader; `horizon-platform` declara `horizon-hooks`. A branch corrente não contém esses crates. | Desativar e tornar inalcançável a infraestrutura de plugin/mod no engine. Remover call sites/configuração associados somente depois de identificar contratos essenciais e oferecer substituto se necessário. |
| Threads e instrumentação | reuse seletivo / cut seletivo | O WIP descreve workers CPU junto de allocator, profiler, sampler, heap census, stack dump e módulos UI/janela condicionais. | Reter paralelismo CPU necessário; excluir UI e probes/allocator não essenciais do runtime mínimo, após checar dependências efetivas. |
| Supervisor e packaging | fora do engine / blocker de release | O WIP separa supervisor e engine, mas supervisor inclui painel/console/API e workflows ausentes na branch; packaging usa recursos preparados e há deriva descrita entre workflow/xtask/Docker. | Tratar supervisor como produto/processo opcional. Não prometer distribuição/release antes de auditar manifests e workflows integrados no branch. |

## 3. Dependências client que não podem ser removidas inteiras sem quebrar o server

A regra prática é **extrair os consumidores server-side antes de remover o crate proprietário**. Os relatórios não autorizam declarar qualquer crate agregado dispensável enquanto os imports e contratos abaixo não forem movidos ou substituídos.

| Dependência/fronteira | Por que não remover em bloco | Extrair exatamente |
|---|---|---|
| `horizon-client` / owner `Game` | O fluxo dedicado usa tick, níveis, recursos, rede, chunks, hooks e shutdown dentro do agregado. Remover a crate sem owner alternativo rompe chamadas e tipos compartilhados. | Entry point dedicado; estado de simulação mínimo; orquestração `Game::tick`/`levels::tick_server`; preflight, criação de mundo, loop, tratamento de falha, shutdown e save. Substituir `UiResources` por dados textuais mínimos somente após tracing. |
| `horizon-client-render` | Streaming controla chunks necessários a jogadores, readiness/luz, geração e save; o relatório dedicado consulta `job_stats`. O crate também possui renderer, wgpu/winit e meshing. | Scheduler/tickets/estado de streaming server, jobs CPU de geração/luz/save, métricas estritamente usadas; contrato de readiness consumido pelo tick. Excluir mesh, upload, LOD visual, GPU e shaders. |
| `horizon-client::lan`/LAN hub | Login, socket poller e hub implementam transporte, auth, admission e gameplay; os módulos dependem de tipos client-tree. | Conn lifecycle/accept/poller, auth/login/configuration, gate/admission, sessão, status e play hub contra tipos/interfaces server-owned; preservar pacotes atuais do `horizon-protocol`. Isolar integrated/memory e UI. |
| `horizon-client::lan::chunks::ChunkEncoder` | Codifica payload de chunk/luz enviados a clientes; retirar sem migrar quebra sincronização de mundo. | Encoder/projeção de chunks e atualizações de luz, com registries e tipos de protocolo necessários, para crate server/protocol apropriada. |
| `horizon-client::save` e ciclo em `game::saving` | Contém projeções de chunk mais completas (luz, heightmaps, block entities, ticks, POI, entidades e dados Horizon), além de persistência de Level/player e flush. A API `horizon-world::region` sozinha não equivale ao save runtime completo. | Adaptador de `SaveDir`/Level, codecs completos de chunks, entities/POI/saved data, playerdata por UUID, SessionLock, datafix/migração/backup e worker/fila com flush de shutdown. Excluir `loading_frame`, `HudBatch` e renderer. |
| `horizon-client-core` (no snapshot WIP) | `horizon-sim-data` importa tipos compartilhados como texto/scoreboard/potions/partículas e outros; corte inteiro pode romper tipos de gameplay, mesmo que o crate seja client-facing. | Tipos de dados server-safe usados por módulos retidos, divididos em crate neutra; excluir implementação visual/input e APIs comprovadamente client-only. Fazer análise símbolo a símbolo antes de classificar. |
| Recursos chamados “client-prepared” | O nome/layout do diretório é client-facing, mas registries, behavior, tags e worldgen são necessários à simulação. | Exportar apenas facts/registries, tabelas de behavior/collision/support/placement usadas, bundles de worldgen/tags/templates e datafix requeridos pelas features habilitadas. Não levar assets de textura, modelo, fonte, som, shader ou UI. |
| `horizon-hooks`/plugins no snapshot WIP | Há hooks e call sites em lifecycle; a feature dedicada ativa loader de plugin no WIP. | Para o primeiro corte estrito, remover caminho de extensão/mod/plugin do binário. Antes, identificar quais callbacks representam lifecycle/gameplay obrigatório e mover a lógica essencial para chamadas internas server-owned, sem manter loader. |

Essa lista não afirma que cada dependência necessariamente permanecerá no manifesto final. Afirma que seus **contratos server necessários** precisam sobreviver em outro owner antes da remoção. A compilação do grafo final é que determinará se a dependência original ainda é alcançável.

## 4. Fases priorizadas para o primeiro binário server-only

### Fase 0 — Congelar evidência e definir alvo

1. Fixar o commit/branch de trabalho como fonte de integração; manter `BorizonServer-main` somente como referência e o WIP como snapshot alternativo.
2. Definir o artefato como binário **engine** sem supervisor/UI. Registrar explicitamente a lista de dependências proibidas e o conjunto mínimo de recursos preparados.
3. A auditoria estática das seis fronteiras foi registrada neste documento e na seção 1–6 acima; confirmou que entry/Game/dedicated/LAN/streaming não estão na árvore atual, enquanto os quatro crates-base server estão. Antes de portar cada módulo, converter o mapa em uma lista de símbolos concretos origem→destino e verificar o diff do branch contra a referência.

**Saída/evidência:** mapa arquitetural e fronteiras de extração documentados; commit atual da branch foi conferido e a `main` continua somente leitura. O mapeamento símbolo→destino está parcial e será refinado ao extrair cada corte.

### Fase 1 — Esqueleto compilável sem client

1. **Bootstrap criado:** crate/binário `horizon-engine` em `crates/horizon-engine`, com manifest sem dependência direta de `horizon-client`, renderer, input, wgpu/winit, shaders, UI, audio, launcher ou plugin loader.
2. O bootstrap reutiliza `horizon-server-files` para EULA e `server.properties`; o workspace tem agora cinco crates. Sua dependência transitiva inclui somente os crates server do workspace, mas o runtime de gameplay ainda não está conectado.
3. CLI inclui `--help`, `--version` e `--preflight`; a execução sem opção é recusada de forma explícita, porque ainda não existe world/Game loop no novo engine. Não instancia nem chama `Game` ou fallback client.
4. Evidências: `cargo build -p horizon-engine --locked --offline`; `cargo test -p horizon-engine --locked --offline` (5 passaram); `cargo check --workspace --all-targets --locked --offline`; CLI `--help`/`--version`; preflight manual com EULA aceita e `server-port=25571`; `cargo tree -p horizon-engine --edges normal --locked --offline` sem nomes client/render/GPU/mod/plugin; `cargo metadata --no-deps` lista cinco packages; fmt e diff check passaram. Isso prova o bootstrap, não um servidor que aceita conexões.

**Saída atual:** primeiro binário engine server-only compilável para help/version/preflight. **Falta** o modo runtime que inicialize um mundo ou execute tick; só declarar essa fase concluída quando uma rota de inicialização real do dedicado for extraída/adaptada e smoke-testada.

### Fase 2 — Tick autoritativo mínimo

1. Extrair owner de mundo/nível mínimo e pass autoritativo de tick (`tick_server` e dependências estritamente necessárias).
2. Conectar leitura de eventos de rede/tick sem renderer; conservar hooks essenciais apenas se implementados internamente, sem sistema plugin.
3. Adicionar teste de loop de ticks com mundo de fixture, métricas básicas e encerramento controlado.

**Saída:** processo que inicializa mundo de teste e executa ticks sem dependência gráfica.

### Fase 3 — Rede e smoke loopback

1. Reutilizar `horizon-protocol` e extrair servidor de socket/poller, handshake, status, login/auth/admission e sessão de jogo server-side.
2. Implementar interfaces server-owned para hub, jogadores e contexto de mundo; evitar copiar codecs já presentes.
3. Executar smoke test loopback com cliente/probe de teste: conexão, handshake/status, login/configuração compatível com o suporte escolhido, entrada na sessão, pelo menos um ciclo de evento/tick e desconexão limpa. O teste deve comprovar troca de dados, não apenas bind da porta.

**Saída:** runtime que atende uma conexão local e mantém tick ativo.

### Fase 4 — Chunks, streaming e iluminação sem render

1. Extrair geração de protochunks/terrain, decoração, luz, save e membership necessários do fluxo de streaming.
2. Manter contrato de tickets, centro/raio de jogadores, readiness e chunks residentes requerido pelo loop dedicado.
3. Migrar encoder de chunk/luz para server/protocol.
4. Incluir fixture que carrega/gera um chunk, confirma readiness, serializa/enfileira e salva; verificar que nenhuma tarefa de mesh foi criada.

**Saída:** tick/rede/chunk demonstráveis em loopback, sem mesh/GPU.

### Fase 5 — Persistência e ciclo de shutdown

1. Ligar APIs já existentes de `horizon-world` ao ciclo Level/dimensões, SessionLock, entities/POI/saved data e playerdata.
2. Preservar datafix e realizar round-trip com fixtures; salvar/flush no shutdown e reabrir mundo.
3. Adicionar teste de falha controlada e crash-save/encerramento ordenado na medida suportada pelo primeiro corte.

**Saída:** persistência mínima comprovada, sem substituir o formato por uma implementação paralela.

### Fase 6 — Prova de exclusão e pacote

1. Validar grafo Cargo/package do binário release e recursos efetivamente empacotados.
2. Inspecionar artefato e workflow para garantir que não haja fallback para `horizon-client`; o snapshot WIP explicitamente tinha `horizon-client` como fallback e workflows fora da branch atual.
3. Rodar os critérios de aceitação abaixo em ambiente limpo e guardar logs/relatórios reproduzíveis.

**Saída:** primeiro binário que pode ser chamado honestamente de server-only segundo o escopo definido.

## 5. Explicitamente fora do primeiro corte

- Renderer, GPU (`wgpu` e afins), janela (`winit`), shaders, buffers, culling, mesh de seção, uploads, far-terrain/LOD visual e `Job::Mesh`.
- Input, telas, HUD/editor, câmera, draw data, partículas visuais, áudio/music output e loading/saving screen.
- Launcher/engine fallback para `horizon-client`; o binário engine não deve executar o cliente como alternativa.
- Mods/plugins, plugin loader, APIs de plugins e execução de hooks externos. Callbacks de lifecycle só permanecem se sua lógica necessária for internalizada sem carregar plugin runtime.
- Integrated/LAN singleplayer, `Pipe` in-memory e estado de host single-player, salvo prova posterior de necessidade para o servidor dedicado.
- UI/painel/TUI/MCP/web UI do supervisor. O supervisor pode ser mantido como processo separado, mas não integra o primeiro binário engine.
- Tradução/adaptação de versões antigas, forwarding proxy e modos opcionais de auth não especificados como requisito do smoke; a política de suporte deve ser declarada. Não remover codecs/adapters do protocolo-base sem provar que não são necessários ao suporte escolhido.
- Assets de textura/modelo/fonte/som, localização visual e resource pack visual. Manter somente dados de gameplay/registry/worldgen/datafix demonstradamente consumidos.
- Instrumentação pesada, heap census, stack dumps, allocator customizado e profiling de UI; manter somente observabilidade essencial para operação/teste.
- Compatibilidade completa com todos os modos de datapack, migrações históricas, formatos opcionais e extensões não demonstradas no primeiro smoke. Qualquer omissão deve ser documentada como limite, sem alegar round-trip completo.

## 6. BorizonV26.3: commits/módulos a reter, comparar, congelar ou possivelmente substituir

A informação disponível não autoriza decidir apagar ou substituir código sem prova. As classificações abaixo são ações de revisão, não decisões de remoção.

| Item | Tratamento | Fundamentação/ação |
|---|---|---|
| HEAD corrente `837c329eb6aeb96e0162fde1b26e7083d89c486b` e workspace de cinco crates | **Reter como baseline** | Checkout em `/home/mago/Arte_Programacao/BorizonServer/BorizonV26.3`; contém os quatro crates server-base e o bootstrap `horizon-engine`. Congelar este baseline antes da extração de gameplay. `main` (`f2e5556cbcb8060313f513d1871ccf3e7fb6e6ec`) não tem merge-base com esta branch e serve somente como referência de código. |
| `horizon-world` — `chunk.rs`, `region.rs`, `nbt.rs`, `compression.rs`, `datafix/*`, `world_saved_data.rs` | **Reter; comparar contratos** | Base server-safe de persistência identificada. Comparar codecs com a projeção completa da referência antes de alterar; não criar terceiro region backend. |
| `horizon-world` — registry, level/data, tags, overrides, generator, live features e estruturas | **Reter; comparar recursos** | Lógica de simulation/worldgen útil. Identificar precisamente arquivos preparados requeridos e retirar apenas dependência nominal/layout client, sem descartar dados gameplay. |
| `horizon-protocol` — connection/frame/packets/crypto/adapters | **Reter; validar grafo** | Protocolo reutilizável. Preservar os contratos de wire; decidir adapters antigos segundo política explícita de suporte. Verificar dependência em `horizon-world` no grafo final. |
| `horizon-server-files` — properties/EULA/user lists/profiles/cache | **Reter** | Funcionalidade administrativa/server apropriada; verificar somente dependências e superfície efetivamente necessária ao primeiro runtime. |
| Quatro arquivos da referência marcados `D` relativamente a `f2e5556cbc`: `horizon-client/Cargo.toml`, `entry.rs`, `game.rs`, `game/dedicated.rs` | **Congelar como referência; portar seletivamente** | O relatório aponta que estão deletados no snapshot da branch. Reutilizar algoritmos/orquestração; não restaurar/copiar módulos agregadores inteiros, que duplicariam e carregariam client. |
| `game/dedicated.rs` da referência: startup/preflight/loop/error/shutdown | **Comparar e portar** | Boa especificação de fluxo dedicado, mas recebe/constrói `Game` e `UiResources`; adaptar a owner server e preservar save, rede, cadence e lifecycle necessários. |
| `horizon-client-render::streaming` e `server_terrain` da referência | **Congelar como referência; refatorar seletivamente** | Streaming mistura jobs úteis com mesh/render; `server_terrain` não é automaticamente server-only por nome. Comparar geração/luz/save com owners efetivos antes de mover. |
| `horizon-client::lan::{conn,hub,io_pool,chunks,playerdata}` da referência | **Congelar como referência; portar seletivamente** | Contém login/hub/transport/encoder/playerdata necessários, mas acoplados à árvore client. Extrair contratos; não duplicar codecs de `horizon-protocol`. |
| `horizon-client::save`, `game::world_loading`, `game::saving`, `game::levels`, `game::autosave`, `native_player_state` da referência | **Congelar como referência; comparar formato antes de substituir** | Fonte da orquestração persistente e projeções completas. Validar round-trip de luz, heightmaps, entidades, POI, tickets, player e dados desconhecidos contra `horizon-world`. |
| Commit WIP `5f450ba163` (pai reportado `1608bd3d5e4ac60f24e2e8923b51e31454b30316`) | **Congelar; comparar, não cherry-pick cego** | Não é ancestral do HEAD corrente. Contém crates e packaging ausentes no branch e perfil dedicado ainda ligado a renderer/input/GPU/plugins. Extrair apenas funções comprovadamente úteis após análise de conflitos/arquitetura. |
| WIP `horizon-server-runtime` (cadência, preflight, filas/jobs) | **Comparar/reter seletivamente** | Funções isoladas parecem úteis; seu próprio relatório diz que runner dedicado ainda reside em `horizon-client`. Não equivale a engine completo. |
| WIP `horizon-game` / `horizon-client` feature `dedicated-server` e `server-plugins` | **Não aceitar como prova; refatorar** | O perfil inclui client facade, renderer/input e plugin loader segundo a auditoria. Não atende critério estrito; preservar apenas lógica server após extração. |
| WIP `horizon-sim-data -> horizon-client-core`, `horizon-platform -> horizon-hooks` | **Comparar símbolo a símbolo; possível substituição de fronteira, não apagar** | Acoplamentos reportados. Identificar tipos server necessários e mover para crate neutra; sem essa prova, não remover crates/call sites por suposição. |
| WIP `horizon-threads`, supervisor `horizon-server`, packaging e workflows | **Comparar; fora do primeiro engine** | Threading CPU pode servir; instrumentos/UI podem não servir. Supervisor é separado, e workflow/package não existe no checkout atual conforme relatório. Validar deriva antes de adoção. |

**Regra de retenção:** nenhum módulo será marcado para exclusão definitiva apenas por nome, por aparecer em crate client, por ser grande ou por estar fora do branch. Exigir referência de imports/consumidores, teste substituto e prova do grafo após a extração.

## 7. Critérios objetivos de aceitação

O artefato só pode ser declarado **server-only** quando todos os itens aplicáveis passarem em build limpa do branch integrado:

1. **Grafo Cargo/package:** o binário de engine e todas as dependências ativadas não contêm crates nem dependências client, render, GPU, input, UI, launcher ou mod/plugin. Confirmar por análise do grafo de features/dependências do target release, não apenas pelo manifesto declarado ou `--no-default-features`.
2. **Sem dependência agregadora:** nenhum caminho alcançável do binário passa por `horizon-client`, `horizon-client-render`, `horizon-client-input`, renderer, `wgpu`/`winit`, shaders, audio output, UI, plugin loader ou APIs de mod. Se um crate neutro contiver código compartilhado, comprovar que o código proibido não é habilitado/compilado no target segundo o critério acordado.
3. **Nenhum client compile:** build reproduzível do target engine sem compilar nenhum crate/client code proibido; registrar comando, target, features e saída. Uma feature `dedicated-server` que ainda compila dependências proibidas reprova.
4. **Inicialização real:** iniciar o binário engine isolado, apontado a resource set server e diretório de teste, e confirmar preflight/configuração, criação/carregamento de mundo e shutdown limpo. Não aceitar o supervisor sozinho ou fallback client como demonstração.
5. **Tick/rede/chunk:** smoke test de loopback demonstra conexão, estado inicial/protocolo suportado, sessão ou evento de jogo, tick autoritativo e obtenção/geração/readiness de chunk; logs/asserções devem mostrar resultado verificável e encerramento/desconexão sem erro. Bind de socket sozinho não basta.
6. **Sem render em runtime:** smoke roda sem display/window/GPU/audio; não cria renderer, janela, mesh/upload ou job de mesh. `HORIZON_NO_AUDIO` ou um nome de feature não substitui prova de ausência de dependência/código.
7. **Persistência mínima:** salvar e reabrir fixture de mundo/chunk sem perda dos campos cobertos pelo escopo declarado; executar flush no shutdown. Para chamar persistência de completa, comparar também entidades, POI, luz, heightmaps, block entities/ticks, saved data e playerdata, conforme suportado.
8. **Recursos mínimos:** manifest/lista de recursos empacotados contém apenas dados lidos pelo server (registries, behavior, tags/worldgen/templates/datafix conforme funcionalidades habilitadas); sem assets de render. Recursos de pack enviados ao cliente devem ser distinguidos dos inputs internos do engine.
9. **Reprodutibilidade:** anexar logs de build, grafo e smoke test; registrar commit exato, features e ambiente. Falta de teste/artefato é “não comprovado”, não aprovação.

## 8. Riscos e incertezas remanescentes

- Foi feita build do `horizon-engine` e execução do data generator server-only; ainda não há build/teste de gameplay, validação de recurso completo, conexão Minecraft, profiling de runtime nem artefato jogável.
- O checkout atual e a referência `main` são árvores diferentes; os módulos ausentes na branch podem ter sido removidos, não portados, ou existir sob outros nomes não cobertos pela auditoria. Não inferir equivalência sem mapear owners concretos.
- O snapshot WIP é divergente e não ancestral do HEAD. Suas crates, features, manifests, workflows e packaging não são garantia de conteúdo disponível no branch corrente.
- A referência tem um preparer unificado com faixas server/client; nesta etapa foi inspecionado e executado somente seu caminho de exportadores server-side. A branch ainda não tem ferramenta reproduzível própria para montar o bundle reduzido, que é a pendência antes de tornar a prova independente dos scripts temporários.
- O contrato exato de streaming/readiness e o round-trip de persistência completa precisam de testes e comparação de campos. Remover renderer ou usar `ChunkData` simplificado antes disso pode quebrar funcionalidade ou perder dados.
- Suporte de compatibilidade (clientes/servidores antigos), auth/forwarding, plugins e datapacks não está definido; as exclusões do primeiro corte podem restringir compatibilidade e devem aparecer na documentação do binário.
- A interpretação de “sem UI” para supervisor é distinta da exigência de engine server-only. Este relatório aplica o critério estrito ao engine; se também for exigido supervisor sem qualquer interface administrativa, será necessária uma decisão de escopo adicional.

**Resultado atual:** arquitetura plausível, vários componentes reaproveitáveis, mas separação estrita **ainda não comprovada**. A aprovação depende de owner server independente, extrações de streaming/rede/persistência e evidência de build/grafo e smoke test conforme os critérios acima.

## Progresso de execução do engine — 2026-11-26

- `horizon-engine` agora tem `resources::load`, isolado dos crates client, para carregar `facts.json`, comportamentos vanilla e o bundle `worldgen`; `--validate-resources <dir>` expõe essa validação. A build do binário passou com `cargo build -p horizon-engine --locked --offline`; o teste de caminho inválido falhou como esperado para diretório inexistente. O bundle temporário server-only também passou.
- O `26.3-server.jar` contém o `net.minecraft.data.Main` e seus próprios registries. A execução `java ... net.minecraft.data.Main --server --reports --output generated` terminou em cerca de 12 segundos, gerando 10.038 arquivos/55 MB em `/tmp`; isso é uma fonte vanilla server-only, não é pacote final e não foi copiado para a branch.
- As fontes dos exportadores Java server-side da referência compilaram contra o jar interno `server-26.3.jar`, sem client jar, assets de client ou classes de client. O bundle temporário passou por `--validate-resources`: 35.723 estados e 67 biomas.
- `--probe-worldgen <resources> [seed]` gerou um chunk Overworld de seed `12345`, serializou o `ProtoChunk` para NBT, gravou `r.0.0.mca`, sincronizou, reabriu a região e validou coordenadas e status `Terrain`. Resultado: `surface_y=63`, bloco consultado `minecraft:water`, bioma `minecraft:ocean`. Os arquivos de prova foram removidos automaticamente. Ainda faltam features/luz, chunks prontos para conexão, loop de tick e protocolo login/play.
- `--status-listen <address>` foi compilado sem dependência Cargo de client e testado com um probe TCP local: handshake STATUS, JSON anuncia `26.3`/protocolo `777`, ping/pong preserva o nonce. A rota LOGIN é recusada por esse listener; jogador ainda não conecta.
- O listener STATUS agora atende cada socket em thread separada, aplica timeout de leitura/escrita de 5 s e limita o frame antes de alocar o payload. `cargo test -p horizon-engine --locked --offline` passou (7 testes); `cargo build -p horizon-engine --locked --offline` passou. Probe com um socket ocioso e outro cliente STATUS simultâneo confirmou que a conexão parada não bloqueia status/ping. O grafo normal/build do engine continua sem crate client/render/GPU/input/mod/plugin.
- O bundle dinâmico de CONFIGURATION agora é reproduzível em `tools/server-data/`: o exportador usa somente `26.3-server.jar` e bibliotecas server runtime para gerar 32 registries (432 entradas) e 773 tags NBT. O loader Rust valida os NBTs e monta UPDATE_ENABLED_FEATURES, SELECT_KNOWN_PACKS, REGISTRY_DATA, UPDATE_TAGS e FINISH_CONFIGURATION na ordem vanilla. `horizon-engine::login::login_and_configure` implementa protocolo 777, LOGIN offline e a troca completa de CONFIGURATION sobre a conexão protocol; teste Pipe cliente/servidor valida a troca de pacotes e rejeição de username inválido. Pacote: 13/13 testes; build e validação do bundle real (35.723 estados/67 biomas/32 registries/773 tags) passaram. A rotina ainda não está conectada ao listener; não há PLAY, admissão real de jogadores ou gameplay.
- Nenhum client Rust foi compilado. Nenhum arquivo foi alterado em `main`; resultados intermediários permanecem fora da branch em `/tmp/borizon-datagen-probe`.

- Atualização 2026-11-26: `horizon-engine/src/play_chunk.rs` produz paletas de blocos/biomes e heightmaps de chunk PLAY 26.3; `--probe-chunk-packet` validou 24 sections e 3 heightmaps com o codec de rede. Ainda sem iluminação, block entities, listener LOGIN→PLAY ou envio a cliente; permanece uma validação de formato, não um servidor jogável.

- Diagnóstico complementar 2026-11-26: decorar chunks exige os seis artefatos server-side em `worldgen/blocks`; o primeiro bundle não os continha e falhou ao resolver providers de árvores. Com arquivos da preparação server-side anterior copiados apenas para o bundle temporário, a geração 3×3 decorou o centro (51 feature roots, 0 unsupported/failed). A produção desses inputs ainda não é reproduzível pela branch `BorizonV26.3`; não há cliente conectado.

### Correção da pendência de recursos server-only — 2026-11-26

A anotação acima de que o preparo do bundle não era reproduzível ficou desatualizada: `tools/server-data/prepare-server-resources.sh` reúne os exporters server-only necessários para as entradas atualmente carregadas por `horizon-engine`, além dos relatórios biome do data generator vanilla. A prova ponta a ponta de saída limpa passou. Isso só corrige preparação de recursos e worldgen/PLAY chunk encoding; não implementa listener de LOGIN, luz, runtime de jogador ou gameplay.

A execução final, partindo de `/tmp/borizon-server-resources-final2` inexistente, completou após acrescentar os fluid tags no caminho esperado pelo loader, a tabela de colisão na raiz e os relatórios de parâmetros biome Overworld/Nether do data generator vanilla. `--validate-resources` passou (35.723 estados, 67 biomas, 32 registries, 432 entradas, 773 tags) e `--probe-chunk-packet` decorou o 3×3 e validou o payload do centro (24 sections, 3 heightmaps, 48 roots, 0 unsupported/failed). Isso ainda não constitui conexão/login/gameplay: falta iluminação e runtime de jogador.

## Revisão recente de iluminação e IDs dinâmicos — 2026-11-26

- Foi identificado que `RegistryBundle` reconstruía entradas de registry a partir de um NBT `Compound` (mapa ordenado alfabeticamente), enquanto os IDs dinâmicos do cliente dependem da sequência produzida por `RegistrySynchronization`. `RegistryBundleExporter` agora grava `registry-order.json` usando a lista de entradas original; o loader Rust reconstrói a lista na ordem explícita e falha em IDs repetidos/ausentes/extras. Export real confirmou 432 entradas em 32 registries e teste `registry_entry_ids_follow_vanilla_order_not_sorted_nbt_keys` cobre uma ordem não alfabética.
- `VanillaLightOcclusionExporter` exporta as faces `getFaceOcclusionShape` de 35.723 estados e compara todas as combinações de formas únicas com `Shapes.faceShapeOccludes`. `horizon-engine/src/light.rs` usa as máscaras para propagar luz e codificar um `LightData` de 26 camadas. O pacote `LevelChunkWithLight` round-trippa no codec; isto ainda não prova igualdade dos valores com o motor `LightEngine` vanilla.
- Validação observada: `cargo test -p horizon-engine --offline registry_sync` (6 focalizados), `cargo test -p horizon-engine --offline light::tests` (2 focalizados), `cargo check -p horizon-engine --offline`, `--validate-resources` no bundle e `--probe-chunk-packet` passaram. O probe de geração+decoração+luz levou ~20 s e cerca de 1,27 GiB RSS no host local.
- Estado de produto: `login_and_configure` ainda só tem teste em memória e o listener público é STATUS-only; não há rota LOGIN conectada, sessão PLAY, simulação de jogador, interação, autosave de sessão nem cliente vanilla conectado. O modo padrão do engine continua recusando gameplay. A luz é experimental e precisa de comparação numérica com o servidor 26.3 antes de ser considerada vanilla.

### Router TCP STATUS/LOGIN — 2026-11-26

`--login-listen <address> <resources>` agora permite que o listener existente roteie STATUS e LOGIN pelo mesmo socket; o caminho TCP conclui a fase offline LOGIN e CONFIGURATION com os dados sincronizados. O teste `tcp_listener_routes_login_and_completes_configuration` usa socket loopback real e valida a ordem de 32 pacotes RegistryData, tags e fim da configuração. Suite do engine: 17/17; `cargo check -p horizon-engine --offline` passou. O comando é apenas um probe de protocolo: fecha após CONFIGURATION, não envia Join Game, não mantém player/world e não deve ser usado como servidor. Próxima implementação deve continuar a mesma conexão e enviar PLAY somente junto de um owner/spawn/chunk válidos.

## Atualização posterior: cliente vanilla alcança PLAY — 2026-11-26

As declarações desta auditoria de que o listener não recebe LOGIN ou não tem PLAY são históricas e foram supersedidas pelo estado atual do roadmap. O JAR vanilla oficial 26.3 já instalado foi executado como cliente em perfil offline local e conectou a `--play-listen 127.0.0.1:42101 /tmp/borizon-server-resources-final2 1234`. O servidor completou LOGIN/CONFIGURATION, enviou Join Game, dificuldade, abilities, centro de chunk, batch 3×3 com luz e teleporte; o cliente respondeu ao batch e ao teleporte, entrou no loop de ticks e manteve TCP estabelecido por mais de 30 segundos. O perfil de teste não é o modo Demo oficial nem uma sessão autenticada; ele comprova o protocolo de entrada em servidor offline-mode.

O primeiro ensaio fechou após 5 s de inatividade porque `TcpStream` herdou o timeout de handshake; isso foi corrigido para polling PLAY e keepalive. O próximo ensaio identificou que clientes podem enviar `ChunkBatchReceived` antes de a mudança de encoder CONFIGURATION→PLAY terminar quando o servidor responde no mesmo instante ao ACK final. A referência `main` confirma o handoff de `conn.rs` para `io_pool` e depois para `hub.rs::place_new_player` dentro do ciclo do jogo. Um limite de um tick (50 ms) antes do primeiro pacote PLAY evita a corrida e espelha esse handoff. Um teste em memória verifica o timeout como polling e outro verifica que o keepalive é enviado mesmo sob fluxo contínuo de pacotes. Validação atual: `cargo test -p horizon-engine --offline` (19/19), `cargo check -p horizon-engine --offline` e `git diff --check` passaram.

Isto ainda não é gameplay vanilla. `serve_until_disconnect` só reconhece um subconjunto de pacotes; movimentos são ignorados, não há posição autoritativa, colisão, mundo residente, tick, streaming ao sair do 3×3, ações de blocos/itens, entidades, inventário nem persistência de sessão. O código da referência já implementa partes extensas desses contratos em `lan/conn.rs`, `lan/hub.rs`, `lan/hub/guest.rs`, `lan/hub/chunk_batch.rs` e `game/lan.rs`, mas esses owners permanecem acoplados ao agregado `horizon-client::Game`. A extração correta é migrar esses caminhos server-side para um owner independente com seus contratos; não copiar apenas o wire flow e não marcar o probe como servidor jogável.

### Atualização: sessão e fronteira rede→tick — 2026-11-26

Comparação linha a linha com `main` confirmou por que a sessão manual ainda era frágil: `PlayConn::packet` valida e limita a fila, depois publica `NetEvent::Packet`; `Hub::process_inbox` consome um orçamento por tick e encaminha movimento para `move_player`/`position_change`; a confirmação de teleporte também volta à autoridade do hub. A branch separada ainda lê, decodifica e aplica alguns pacotes diretamente em `serve_until_disconnect`, sem fila de eventos, orçamento de processamento, owner persistente do jogador ou tick de mundo. O `PlayerSession` recém-adicionado cobre apenas invariantes locais (PlayerLoaded, ACK de teleporte, valores finitos e um pacote de posição por tick); ele não substitui a arquitetura de `main` nem implementa colisão/física.

O que deve ser migrado a seguir é a fronteira server-owned em três camadas: leitor de rede com inbox bounded, owner de jogador que persiste entre ticks e consumidor de eventos no tick do mundo. Depois, portar `move_player` com acesso a colisões e estado de movimento. Evitar mais tratamento de pacotes diretamente no socket reduz erros de ordenação e facilita testes contra os contratos reais. Validação após o primeiro estado de sessão: `cargo test -p horizon-engine --offline` 21/21, `cargo check -p horizon-engine --offline`, `cargo fmt --all` e `git diff --check` passaram. Continua sem simulação de gameplay.

### Atualização: leitor isolado e inbox por tick — 2026-11-26

O leitor de PLAY foi isolado em uma thread e encaminha os frames por canal síncrono limitado ao owner da sessão. A inbox limita 8.000 pacotes/16 MiB e drena até 400 pacotes a cada 50 ms. `play_reader_hands_packets_to_tick_session_and_replies` atravessa leitor, canal, passo de sessão e resposta; `pending_play_packets_are_bounded_by_count` valida o limite. A suíte do engine passou com 23/23 testes, `cargo check -p horizon-engine --offline`, fmt e `git diff --check` passaram.

Limite arquitetural: há agora registro global de admissão e contagem de jogadores, mas o tick continua privado a cada conexão. Não há `GameServer` compartilhado que possua mundo mutável e execute um único tick autoritativo; cada conexão ainda recebe a região inicial estática e mantém movimento isolado. A próxima extração precisa migrar esse owner global da `main` e conectar as sessões ao runtime residente antes de implementar física e ações de gameplay.

### Atualização: admission registry — 2026-11-26

`PlayServer` compartilha entre sockets o registro UUID→entity ID, recusa UUID duplicado, limita a 20 jogadores, informa `online` no status e remove a admissão por RAII ao sair. Testes exercitam ID monotônico, duplicata, limite e liberação. O registro não cria player list nem sincroniza entidades; o owner do mundo e o tick global ainda faltam. A suíte após essa mudança passou com 25/25; `cargo check -p horizon-engine --offline`, fmt e diff check passaram.

O binário dedicado compilou com `cargo build -p horizon-engine --offline`, e `--help` executou. A inspeção `cargo tree -p horizon-engine -e normal` não encontrou crates/dependências de client, renderer, input ou GPU no grafo normal.

### Atualização: mundo residente compartilhado — 2026-11-26

`PreparedPlayWorld` materializa os nove chunks do spawn em um `ResidentDimension`/`ResidentLevel`, incluindo as filas de ticks decodificadas dos `ChunkData` gerados, e compartilha esse owner entre conexões. A inicialização valida o spawn contra a dimensão residente. O bundle real `/tmp/borizon-server-resources-final2` passou pela geração→FEATURES→ChunkData→ResidentDimension sem erro; o listener subiu, respondeu status 26.3/protocolo 777, e foi encerrado para liberar memória.

Esse owner compartilha armazenamento de blocos, mas ainda não é gameplay: cliente não modifica blocos, não há colisão ligada ao movimento, posição autoritativa global, entidades, streaming dinâmico ou persistência regional. Os chunks gerados também não são regenerados/streamed quando o jogador sai da área inicial. Testes/checks atuais: engine 25/25, `cargo check`, fmt e diff check passaram.


## Movimento e colisão conectados à sessão PLAY — 2026-11-26

A implementação atual substitui a observação anterior de que os pacotes de movimento eram apenas armazenados. `PreparedPlayWorld` consulta as colisões da dimensão residente e o loop de sessão aplica o resolvedor de movimento de `horizon-core`; posições reduzidas ou fora da região carregada são corrigidas com o pacote vanilla `PlayerPosition`, aguardando confirmação do teleporte. A leitura e o tick mantêm o orçamento já existente. Validação local: `cargo check -p horizon-engine --offline`, 28 testes do pacote, formatação e diff check passaram.

Escopo ainda incompleto: há apenas limite base de distância² 100 por pacote/tick, sem simulação de velocidade nem compensação de knockback; faltam gravidade/velocidade server-side, colisores dinâmicos/contextuais completos, entidades, sincronização de movimento entre jogadores ou streaming de chunks. Portanto a colisão básica está conectada, mas o engine ainda não é um servidor vanilla jogável; o teste desta etapa foi unitário, não uma sessão de cliente.

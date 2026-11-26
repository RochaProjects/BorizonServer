# Auditoria de extração server-only — BorizonV26.3

## Escopo e disciplina da evidência

Este relatório consolida **exclusivamente** os relatórios estruturados fornecidos. Não reabri repositórios, não executei builds ou testes e não presumo que código visto em `BorizonServer-main` ou no snapshot WIP esteja integrado à branch de trabalho `BorizonV26.3` (`/home/ubuntu/BorizonServer`). Onde a auditoria não confirmou a árvore concreta, o estado é marcado como potencial ou não avaliado. “Server-only” é interpretado estritamente para o binário do engine: sem crates/dependências/código client, GPU, render, input, mod ou plugin alcançáveis no grafo de build normal. O supervisor é um processo opcional separado; sua interface administrativa não deve ser confundida com o engine.

## 1. Conclusão executiva: viabilidade e velocidade relativa

A extração é **viável em princípio, mas não é um corte rápido de feature nem uma remoção simples de dependência**. Há blocos substanciais reaproveitáveis: `horizon-core`, `horizon-world`, `horizon-protocol` e `horizon-server-files` já estão no workspace da branch; `horizon-world` contém NBT, regiões Anvil, chunks e datafix; o protocolo possui framing, codecs e conexão; e as auditorias da referência descrevem um caminho dedicado com preflight, tick, rede, chunks, persistência e shutdown. Esses fatos sustentam uma estratégia de extração, não a afirmação de que já existe um engine independente.

O primeiro obstáculo é ownership: os relatórios indicam que a branch atual não contém os quatro arquivos da implementação de referência (`horizon-client/Cargo.toml`, `src/entry.rs`, `src/game.rs`, `src/game/dedicated.rs`) e tampouco contém a camada LAN/hub, `horizon-client-render` ou os crates do snapshot WIP. O WIP `5f450ba163` é um snapshot alternativo, não ancestral do HEAD citado. Portanto, não se deve tratá-lo como implementação integrada nem copiá-lo cegamente.

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
| HEAD corrente `c8b61389a8e6fc128392154b6bc9a18cd58f350f` e workspace de quatro crates | **Reter como baseline** | É o checkout de trabalho citado; contém `horizon-core`, `horizon-world`, `horizon-protocol`, `horizon-server-files`. Congelar baseline antes de integração e medir alterações contra ele. |
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

- Não houve build, teste, inspeção dinâmica de grafo, profiling de runtime nem validação de artefato; toda conclusão é estática e condicionada aos relatórios.
- O checkout atual e a referência `main` são árvores diferentes; os módulos ausentes na branch podem ter sido removidos, não portados, ou existir sob outros nomes não cobertos pela auditoria. Não inferir equivalência sem mapear owners concretos.
- O snapshot WIP é divergente e não ancestral do HEAD. Suas crates, features, manifests, workflows e packaging não são garantia de conteúdo disponível no branch corrente.
- O preparer/exporters não foram inspecionados; a ligação `.local/horizon-client` demonstra nomenclatura/layout, não dependência técnica inevitável de client runtime.
- O contrato exato de streaming/readiness e o round-trip de persistência completa precisam de testes e comparação de campos. Remover renderer ou usar `ChunkData` simplificado antes disso pode quebrar funcionalidade ou perder dados.
- Suporte de compatibilidade (clientes/servidores antigos), auth/forwarding, plugins e datapacks não está definido; as exclusões do primeiro corte podem restringir compatibilidade e devem aparecer na documentação do binário.
- A interpretação de “sem UI” para supervisor é distinta da exigência de engine server-only. Este relatório aplica o critério estrito ao engine; se também for exigido supervisor sem qualquer interface administrativa, será necessária uma decisão de escopo adicional.

**Resultado atual:** arquitetura plausível, vários componentes reaproveitáveis, mas separação estrita **ainda não comprovada**. A aprovação depende de owner server independente, extrações de streaming/rede/persistência e evidência de build/grafo e smoke test conforme os critérios acima.

# Migração para um repositório dedicado ao servidor

## Objetivo

Transformar esta branch em um projeto exclusivamente server-side para o Horizon
vanilla. A `main` é a fonte de referência do código existente: vamos inspecionar,
selecionar, copiar/mover e adaptar o que o servidor realmente precisa, por etapas.
Não vamos reimplementar do zero nem preservar client, launcher ou compatibilidade
com eles. A árvore final desta branch conterá somente o servidor vanilla e as
ferramentas server necessárias, sem dependências de client.

O primeiro resultado contém somente o engine dedicado vanilla e as ferramentas
indispensáveis para preparar, iniciar e administrar esse servidor. Renderer,
janela, input, menus do client, launcher, APIs de mods e extras de gameplay não
fazem parte desse marco.

Mods, APIs de mods, plugins, SDKs e loaders (incluindo `.so`) estão fora do
escopo desta branch e não serão trazidos nem integrados. Permanecem na `main` e
no histórico. O objetivo aqui é exclusivamente o servidor Horizon vanilla.

“Server-only” descreve o produto e o grafo final. A branch começa como trabalho
server-only; a `main` será consultada como origem do código a selecionar e adaptar,
sem incorporar componentes client à árvore final. Nenhuma etapa compila o client.
O primeiro build do binário de servidor dedicado (engine) ocorrerá quando existir um pacote server-owned que não dependa de `horizon-client`, renderer, input ou outras dependências client. Os testes/checks já executados compilam somente os quatro pacotes do workspace server-owned; nenhum binário client foi compilado.

## Painel de andamento da branch `BorizonV26.3`

Este painel é a fonte de verdade do que já foi feito e do que falta nesta
migração. Atualizá-lo no mesmo commit de cada avanço relevante; não marcar uma
etapa como concluída só porque o código foi movido ou porque um comando começou.
Cada item concluído precisa de evidência verificável (commit/arquivos, comando e
resultado). Tentativas que falharem ficam registradas como tentativa/limite, não
como conclusão. A `main` permanece intacta; esta migração parte dela e só altera
os arquivos exigidos pelas etapas abaixo. Não copiar/mover diretórios inteiros
por conveniência: migrar somente código necessário e já identificado.

No painel, selecionar um único estado por fase (`Feito`, `Em andamento` ou
`Pendente`; registrar `Bloqueada` no log quando houver impedimento). Nunca marcar
uma ação ou critério de aceite como feito sem implementação e evidência verificável.
No início, todas as ações e critérios ficam desmarcados; não inferir conclusão por
existir branch, documentação ou checkout de referência.

| Fase | Feito | Em andamento | Pendente | Feito / evidência | Falta / próximo critério verificável |
|---|---|---|---|---|---|
| Branch e baseline | [ ] | [x] | [ ] | Cópia de referência da `main` em `/home/ubuntu/BorizonServer-main` (`f2e5556cbc`). Capturado `cargo metadata --no-deps` (57 pacotes na origem) e árvore estática do dedicado sem a feature opcional `.so` (947 linhas); ainda contém client/GPU e resíduos antigos de mods (`horizon-mods`, `horizon-mod-api-gen`). | Verificar relação histórica da branch; registrar tempo/RAM dos Actions e smoke de instalação limpa; confirmar o contrato de empacotamento do preparer. |
| Inventário do uso server real | [ ] | [x] | [ ] | Auditoria estática: `horizon-core`, `horizon-world`, `horizon-protocol` e `horizon-server-files` formam a base portada; `horizon-command`, `horizon-items`, `horizon-sim-data` e `horizon-client-core` foram mapeados, mas exigem extração de módulos comuns e remoção de fronteiras client. Capturado `cargo tree --locked --edges normal -p horizon-server` (404 linhas: sem `horizon-client`, GPU, janela ou plugin); `horizon-prepare` (44 linhas, sem deps Cargo de workspace). | Mapear símbolo/import e separar runtime do engine, serviços da CLI, Java/exporters e assets; completar inventário dos módulos mistos. |
| Engine server-owned | [ ] | [ ] | [x] | Ainda não há binário/engine migrado nesta branch. | Extrair a entrada dedicada e runtime de jogo para pacote próprio sem `Game` visual nem `horizon-client`; iniciar mundo e aceitar conexões. |
| Migração dos módulos mistos | [ ] | [x] | [ ] | Portados da `main`: `horizon-core`, `horizon-world`, `horizon-protocol` e `horizon-server-files`, com cortes server-only. De `horizon-sim-data` foram migrados a `horizon-world` os módulos isolados `ticket_levels` (11 testes passaram; 1 benchmark ignorado) e `world_spawn` (4 testes passaram). Nesta continuação, além de callbacks e extensões não-vanilla, foram removidos hooks client/mod/plugin identificados na auditoria do protocolo e mundo; validações finais atuais: `cargo test --workspace --locked` passou com 1.248 testes, 0 falhas e 11 ignorados (52 alvos), 0 warnings; `cargo tree --workspace --locked` (181 linhas) não contém pacotes client/mod/plugin/render/GPU/shader; `cargo fmt --all -- --check` e `git diff --check` passam. Nenhuma build de client. | Ainda extrair runtime dedicado/estado/tick/rede/save de `Game`, terminar a auditoria global e implementar o engine server-owned; não há engine executável nesta branch. |
| Remoção de extras/mods do marco vanilla | [ ] | [ ] | [x] | — | Excluir referências antigas só após confirmar datapacks vanilla, worldgen e recursos; validar que nenhum comportamento vanilla foi perdido. |
| Redução do workspace/CI/release | [ ] | [ ] | [x] | — | Remover pacotes client do grafo e workflows; automatizar ausência de renderer/input/GPU e testar o pacote publicado em diretório limpo. |
| Limpeza do produto server-only | [ ] | [ ] | [x] | — | Limpar arquivos exclusivamente client identificados, atualizar README/docs, validar critérios finais e preparar integração separada. |
| Mods e plugins | [ ] | [ ] | [x] | — Fora do escopo: não copiar, compilar ou integrar API de mods, plugin API, SDK ou loader `.so`. | Manter esses componentes na `main`/histórico; garantir que não apareçam na árvore nem no grafo do servidor vanilla. |

### Registro de trabalho descoberto durante a migração

Antes de executar uma tarefa material que não esteja descrita no plano, registrá-la
aqui com etapa, motivo, arquivos afetados, validação e estado; depois atualizar o
critério da etapa correspondente se a descoberta mudar o escopo. Usar `—` enquanto
não houver itens adicionais.

| Fase | Tarefa descoberta / motivo | Arquivos afetados | Validação e resultado | Feito | Em andamento | Pendente |
|---|---|---|---|---|---|---|
| Inventário / threads | A cópia integral de `horizon-threads` mistura scheduler/allocator server com UI de boot/menu e profiling de heap; testes de amostragem excedem limiares no host ARM64. Não portar pacote completo no escuro. | `crates/horizon-threads/`, `third_party/libmimalloc-sys/` (cópia provisória descartada) | 56 testes passaram; 13 de sampling falharam; UI removida antes do teste. Crate e patch não ficaram na branch. | [ ] | [ ] | [x] |
| Teste server-only / world probe | O agregado `world_probe` ainda referenciava `face_visibility_fixture`, que chamava o módulo client `block_render` já removido do crate server. Excluir apenas esse fixture e suas referências, mantendo probes de simulação. | `crates/horizon-world/examples/probe/main.rs`; removido `crates/horizon-world/examples/probe/face_visibility_fixture.rs` | `cargo fmt --all -- --check` passou; `cargo test --workspace` passou (1.260 testes, 0 falhas, 10 ignorados; ARM64). | [x] | [ ] | [ ] |
| Baseline antigo / resíduos de mods | O grafo dedicado sem a feature `.so` ainda inclui o crate antigo `horizon-mods` e o build-time generator `horizon-mod-api-gen`; release da main ativa `server-plugins` embora o archive não contenha `.so`. Isso não deve ser confundido com árvore vanilla server-only nem trazido ao novo workspace. | `docs/server-only-migration-roadmap.md`; ver `crates/horizon-client/Cargo.toml`, `.github/workflows/server-release.yml` somente no checkout de referência | `cargo tree -i` estático: `horizon-mods` via client/client-lib; `horizon-mod-api-gen` via build-dependency client; plugin API/loader não resolvem sem feature. Nenhuma build foi executada. | [x] | [ ] | [ ] |
| Corte server-safe / chunk tickets | `ticket_levels` implementa níveis vanilla de carregamento/simulação dos chunks (player, forced, portal e dragon). Imports de produção limitam-se a `horizon_core::fast_hash::FxHashMap` e `std`; sem dependência de `horizon-client-core`, mods, plugins ou GPU. Copiado como módulo de `horizon-world`, sem trazer `horizon-sim-data` inteiro. | `crates/horizon-world/src/ticket_levels.rs`, `crates/horizon-world/src/lib.rs` | `cargo fmt --all -- --check`; teste direcionado: 11 passaram, 0 falhas, 1 benchmark ignorado. `cargo test --workspace --locked` e verificação do grafo concluídos sem falha. | [x] | [ ] | [ ] |
| Corte server-safe / spawn | `world_spawn` contém a espiral de spawn inicial e a busca vanilla de spawn/respawn; depende somente do `HeightmapType` de `horizon-world` e tipos std. Copiado da main com o import ajustado para `crate::heightmap`; sem lógica de UI, assets, mods ou plugins. | `crates/horizon-world/src/world_spawn.rs`, `crates/horizon-world/src/lib.rs` | `cargo fmt --all -- --check`; teste direcionado: 4 passaram, 0 falhas; validação completa depois do corte também passou (workspace: 1.275 passaram, 0 falhas, 11 ignorados; árvore sem client/mod/plugin/render/GPU). | [x] | [ ] | [ ] |
| Exclusão de APIs nativas de mods | Removidos callbacks explícitos de loot/item defaults, contexto/place nativo e worldgen, além dos hooks de farmland (`FarmlandTrampleEvent`), block entities nativas customizadas e registro de componentes customizados do protocolo. Também excluída a API de odds para recipe viewer sem consumidores, e reduzido `native_upgrade_block_state` a helper privado somente de teste. Mantida a semântica vanilla: gates de trample, flags de block entity/pistão, parser/transport de componentes conhecidos, pipeline de loot e datafix normal. | `crates/horizon-world/src/{block_landing.rs,level/{mod.rs,update.rs,pistons.rs},fluid.rs,datafix/{mod.rs,block_state_data.rs,tests.rs},loot.rs}`; removidos `src/loot/odds.rs`, `src/loot/odds_tests.rs`; `crates/horizon-protocol/src/{item.rs,registry.rs}` e removido `src/item/custom.rs` | `cargo check --tests --examples` e testes dos dois pacotes passaram. A última `cargo test --workspace --locked` passou: 1.210 testes, 0 falhas, 11 ignorados em 52 alvos, 0 warnings; `cargo fmt --all -- --check`, `git diff --check` e `cargo tree --workspace --locked` (181 linhas, sem client/mod/plugin/render/GPU/shader) passaram. Nenhum client compilado. | [ ] | [x] | [ ] |
| Migração de game rules vanilla | A fonte contém catálogo tipado, defaults, bounds, serialização e `game_rules.dat`; o workspace dedicado só tem um struct de dez campos em `level::GameRules`. Migrar a fonte como modelo server data e projetar os campos usados no nível, centralizando defaults e sem copiar dependências do client. | Referência somente leitura: `BorizonServer-main/crates/horizon-client-core/src/gamerules.rs`; destino: `crates/horizon-world/src/gamerules.rs`, `src/lib.rs`, `src/level/mod.rs` | Catálogo portado/adaptado com validação, regra experimental, NBT/gzip e fallback de arquivo inválido; `LevelAccess::game_rule_catalog()` empresta o estado completo e `game_rules()` projeta só os campos de simulação. Teste direcionado: 7 passaram, 0 falhas; workspace: 1.217 aprovados, 0 falhas, 11 ignorados (52 alvos); `cargo check -p horizon-world --tests --examples --locked` passou. API concluída, mas a branch ainda não tem engine dono do estado: ligar o carregamento/salvamento deste catálogo ao ciclo de mundo segue pendente da extração do runtime. | [x] | [ ] | [ ] |
| Avaliação de corte / `section_mask` | `horizon-sim-data::section_mask` é um módulo pequeno e `std`-only, mas não tem callsites no runtime de `horizon-world`; os usos observados estão em módulos do client (`random_tick_sections`, `portal_sections`, LAN/remote chunks e nether portal). Não copiar utilitários só por serem portáveis sem consumidor server comprovado. | Referência: `BorizonServer-main/crates/horizon-sim-data/src/section_mask.rs`; auditoria estática read-only; nenhum arquivo de código alterado. | Decisão de não portar nesta etapa; reavaliar apenas quando um consumidor server-owned concreto for extraído. | [x] | [ ] | [ ] |
| Auditoria de fixtures do protocolo | Não foram identificados handlers de Forge/Fabric/Quilt/NeoForge ou extensões específicas em produção no `horizon-protocol`; codecs genéricos de Custom Payload e Login Plugin Query são protocolo vanilla e devem permanecer. Para não sugerir integração de plugin/autenticação nos testes, trocar somente exemplos `velocity:player_info`, `authme:prejoin-register/submit` e valor fictício de senha por identificadores/dados neutros. | `crates/horizon-protocol/src/packets/login.rs`, `packets/configuration.rs`; auditoria estática e ajustes de fixtures apenas. | Testes completos após a alteração: `cargo test --workspace --locked` passou (1.217, 0 falhas, 11 ignorados, 52 alvos). Nenhum codec genérico foi removido. | [x] | [ ] | [ ] |
| Avaliação de módulo / scoreboard | O `scoreboard.rs` de `horizon-client-core` tem 1.631 linhas e mistura NBT/dados de pontuação com componentes de texto, estilos, formatação visual e o agregado `Registries`; este destino tem NBT/registry de blocos, mas não a camada comum de `component`/`text` nem o catálogo agregado usados pela referência. Não portar o arquivo inteiro nem introduzir o crate client-core. | Referência somente leitura: `BorizonServer-main/crates/horizon-client-core/src/scoreboard.rs`; inventário estático; nenhum código alterado. | Deixar como pendência de extração em submódulos (dados/codec primeiro; formatação depois de criar camada comum server-owned). | [x] | [ ] | [ ] |
| Avaliação do dispatcher de comandos vanilla | `horizon-command` inteiro é server-relevante, mas árvore/dispatcher/parsing/mensagens formam um conjunto acoplado e ainda pedem fronteiras server-owned para `Text`/Component JSON, registries/identifiers, scoreboard e partículas; gamerules, motor NBT paths, nomes de bossbar e fontes de som já foram extraídos em unidades server-only, ainda sem consumidor de comando nesta branch. `network.rs` e os codecs de `horizon-protocol` já são vanilla; `/pregen` é extensão Horizon e deve ficar fora. Não há recorte funcional pequeno nem callsite no workspace destino que justifique cópia direta. | Referência: `/home/ubuntu/BorizonServer-main/crates/horizon-command`; auditoria estática read-only, sem Cargo/edição. | Próximo bloqueio: desacoplar Text/Component, snapshot de registries versus contexto de execução/crosshair e scoreboard vanilla; então portar parser/árvore e ligar os primitivos já extraídos, omitindo `/pregen`. | [x] | [ ] | [ ] |
| Avaliação / agregado Registries do dispatcher | A referência `horizon-client-core::registries::Registries` mistura estado útil ao servidor (registry ids/tags, listas administrativas, scoreboard e dados de datapacks) com contexto de cliente: `custom_completions` e campos derivados do crosshair (`selected_entities`, `target_block`, `target_point`). Callsites de `horizon-command/src/args.rs:846-848,1703-1705` leem os campos de bloco/ponto/entidades; `lib.rs:711-742` usa listas de jogadores/ops. | Referência somente leitura `BorizonServer-main/crates/horizon-client-core/src/registries.rs`; callsites verificados por `rg` em `horizon-command` e `horizon-client-core`. Uma cópia temporária foi avaliada e descartada antes de qualquer commit; nenhum módulo/dependência foi acrescentado. | Auditoria estática; não executar teste de módulo descartado. Decisão: não portar o agregador integral. Separar snapshot de registries server-owned de contexto/sugestões do cliente e provar consumidores server antes de implementar. | [x] | [ ] | [ ] |
| Auditoria / proveniência dos recursos preparados | `horizon-world` ainda usa `HORIZON_RESOURCES` e, como fallback de produção para tabelas de datafix, `.local/horizon-client/<version>`; diversos fixtures usam a mesma pasta. O `horizon-prepare` da referência não é um recorte server-only pronto: `lib.rs` (78 KB) mantém jar/classpath/lane client e server; `artifacts.rs` (94 KB) descobre downloads dos dois lados; `sounds.rs` (40 KB) extrai assets, fontes, panorama e idiomas do client. O contrato descreve dados vanilla extraídos do jogo, não dependência runtime de client, mas nome/proveniência/geração ainda não são server-neutral. | Auditoria estática dos caminhos `horizon-world/src/datafix/tables.rs:117-132` e fixtures; inventário em `/home/ubuntu/BorizonServer-main/crates/horizon-prepare/{src/lib.rs,src/artifacts.rs,src/sounds.rs}`; o manifesto tem deps externas e a rotina possui lanes server/client. Nenhum path/código foi alterado; nenhum build do client ou preparer executado. | Bloqueado até existir preparer/exporter server-owned e contrato de diretório alternativo. Não copiar a crate integral nem renomear/remover fallback antes de mapear outputs efetivamente consumidos e migrar fixtures. | [ ] | [ ] | [x] |
| Auditoria / autoridade de simulação vanilla | Os nomes residuais `EntityMoveWorld::client_side` e `locally_authoritative` em `horizon-world/src/entity_move.rs:75,102,176,193,205` controlam transições vanilla de flags de colisão, fall damage e emissões; `PlayerFoodWorld::level_is_client` em `player_food.rs:7,14` evita aplicar exhaustion no lado cliente. São dados de domínio/autoridade, sem import ou dependência de código client. | Inspeção de definições e consumidores no branch; comparação estática com a API portável da referência, sem edição. A branch ainda não tem implementador do runtime dedicado para suprir esses valores. | Preservar por ora; ao extrair o runtime, fornecer o contexto server/local-authority correto e adicionar testes server. Não apagar apenas pelo nome `client_side`. | [x] | [ ] | [ ] |
| Corte server-safe / categorias de áudio | `horizon-client-core/src/sound_source.rs` é um catálogo vanilla de 11 nomes (`SoundSource`) sem imports/dependências; a única callsite localizada é `horizon-command/src/tree_world.rs:10`, para argumento/sugestão de `/playsound`. Portado como dado de domínio para `horizon-core`, sem transportar `horizon-client-core`. | `crates/horizon-core/src/sound_source.rs`, `crates/horizon-core/src/lib.rs`; referência somente leitura em `BorizonServer-main`. | Teste direcionado: 2 passaram, 0 falhas. Primeira tentativa falhou em `dedup` aplicado a array dentro do teste (`E0599`); corrigido para `Vec` e rerun passou. Validação pós-corte: `cargo test --workspace --locked` (1.219 aprovados, 0 falhas, 11 ignorados, 52 alvos, 0 warnings); `cargo fmt --all -- --check`, `git diff --check`; grafo `cargo tree --workspace --locked` sem client/mod/plugin/render/GPU/shader. | [x] | [ ] | [ ] |
| Corte server-safe / valores de `/bossbar` | A referência usa `horizon-client-core::bossbar::{COLORS, STYLES}` na árvore de comandos (`horizon-command/src/tree_misc.rs:233-238`). O arquivo completo de bossbar é misto: salva `CustomBossEvents`, usa NBT, `Comp`/`Text`, hover, coloração e progress; só os dois arrays de nomes são necessários para argumentos e são dados vanilla sem formatação. | `crates/horizon-core/src/bossbar.rs`, `crates/horizon-core/src/lib.rs`; referência somente leitura `BorizonServer-main/crates/horizon-client-core/src/bossbar.rs`; nenhum manifesto/dependency alterado. | Teste direcionado: 2 passaram, 0 falhas. `cargo test --workspace --locked`: 1.221 aprovados, 0 falhas, 11 ignorados em 52 alvos e 0 warnings; `cargo fmt --all -- --check`, `git diff --check`; árvore (181 linhas) sem nomes client/mod/plugin/render/GPU/shader. Nenhum client compilado. Não copiar display name, text style, JSON/NBT ou modelo completo. | [x] | [ ] | [ ] |
| Auditoria / fronteira Text–Component–JSON | `horizon-client-core/src/component.rs:10-14` faz parse de SNBT/JSON e converte para `Text`/`Style`; `text.rs` importa `lang` e resolve traduções, enquanto `component.rs` depende de named colors. `horizon-core::text` no destino contém somente parsing numérico JDK; `horizon-protocol::item::Component::Text(Tag)` é valor de item, não modelo de chat component. Na referência, `horizon-command/src/args.rs:1450` chama `horizon_client_core::component::parse_snbt` para argumentos Component/Style; o branch atual ainda não possui `horizon-command`, portanto não há consumidor local imediato. Não existe tipo neutro dedicado que preserve JSON de chat sem resolver apresentação. | Inspeção estática read-only da referência e destino (`component.rs`, `text.rs`, `lang.rs`, `horizon-core/src/text.rs`, `horizon-protocol/src/item.rs`); nenhum código copiado. | Bloqueio arquitetural: definir um modelo/codec server-owned de componente JSON que preserve conteúdo/unknown fields e round-trip; manter tradução, estilos resolvidos, `key_display` e layout em camada separada. Não portar `text.rs`/`component.rs` integrais. | [x] | [ ] | [ ] |
| Corte server-safe / motor de NBT paths | `horizon-client-core::nbt_path` contém operações vanilla de path e seletores `NbtUtils.compareNbt`; as callsites da referência incluem o parser `horizon-command/src/args.rs:1665`, a validação de component em `component.rs:281`, árvores de `/data` (`tree_data.rs:90,104,109,129,135` e `tree_misc.rs:287`) e `/execute` (`tree_execute.rs:123,219`). O núcleo usa apenas `Tag`, `Compound`, `JavaString`, parse/compare SNBT de `horizon-world` e formatação SNBT canônica em string; não depende de `Text`, idioma, UI ou runtime client. | `crates/horizon-world/src/nbt_path.rs`, `crates/horizon-world/src/lib.rs`; copiado da referência somente leitura e ajustados imports locais e formatter para `crate::snbt_text::to_snbt`; nenhum manifesto/lockfile alterado. | `cargo test -p horizon-world --lib nbt_path --locked`: 12 passaram, 0 falhas; a suíte completa `cargo test --workspace --locked`: 1.233 passaram, 0 falhas, 11 ignorados em 52 alvos e 0 warnings. `cargo fmt --all -- --check`, `git diff --check` e `cargo tree --workspace --locked` (181 linhas, sem dependências proibidas) passaram. Não há consumidor de `NbtPath` no workspace server atual, pois `horizon-command` ainda não foi migrado. | [x] | [ ] | [ ] |
| Corte server-safe / canonicalização NBT de block entities | `horizon-client-core::block_entity_nbt` implementa canonicalização vanilla de NBT para 47 tipos da lista base mais dois tipos mais recentes de 26.3 (49 cobertos pelos testes) e merge `overlay`, preservando campos/componentes desconhecidos como `Tag`; a referência usa isso na persistência (`horizon-client/src/save.rs`, `game/persist.rs`, spill/machines). O módulo depende apenas de NBT (`Compound`, `Tag`) e normalização de identifiers; não traduz nem renderiza `SignText`, e não carrega client API. | `crates/horizon-world/src/block_entity_nbt.rs`, `src/lib.rs`; copiado da referência somente leitura, imports NBT/testes ajustados para `crate`, `normalize_id` substituído por `Identifier::parse`; nenhum manifesto/dependência alterado. | Teste focado `cargo test -p horizon-world --lib block_entity_nbt --locked`: 15 passaram; suíte completa `cargo test --workspace --locked`: 1.248 aprovados, 0 falhas, 11 ignorados em 52 alvos, 0 warnings. `cargo fmt --all -- --check`, `git diff --check`, árvore (181 linhas, sem pacotes client/mod/plugin/render/GPU/shader) passaram. Ainda não há callsite server-owned nesta branch que consuma especificamente este canonicalizador de block entities. | [x] | [ ] | [ ] |

## Diário de execução

| Data | Atividade/evidência | Estado | Próximo passo |
|---|---|---|---|
| 2026-11-24 | Clonada a `main` em `/home/ubuntu/BorizonServer-main` (`f2e5556cbc`); levantados metadados (57 pacotes) e baseline estático com `dedicated-server` sem feature `.so` (947 linhas; client/render/GPU e resíduos `horizon-mods`/`horizon-mod-api-gen` presentes). | Em andamento | Comparar histórico de branch, Actions e smoke existente; não compilar o client. |
| 2026-11-24 | Copiado `horizon-core` da main sem alterações; workspace mínimo só com crates server. `cargo fmt --all -- --check` e `cargo test -p horizon-core`: 160 unit, 2 integration passaram; 4 ignorados. `cargo tree -p horizon-core`: só `libm 0.2.16`. | Validado (crate) | Ligar os primitives ao runtime dedicado depois de extrair seus módulos. |
| 2026-11-24 | Copiado `horizon-world`; removido `block_render` (culling de faces de render) e chamadas de `horizon-threads::work` (marcadores de profiling, não gameplay). `cargo check -p horizon-world --lib`; `cargo test -p horizon-world --lib`: 501 passaram, 3 ignorados. | Validado (crate) | Continuar conferindo módulos de apresentação e uso server real durante a extração de `Game`. |
| 2026-11-24 | Copiado `horizon-protocol`; excluídos módulos custom `mod_list`/`mod_net`; mantidos pacotes de handshake/login vanilla (inclui packet vanilla chamado Login Plugin Query, não API de mods). `cargo test -p horizon-protocol --lib`: 456 passaram, 1 ignorado. | Validado (crate) | Integrar transporte na entrada server-owned e revisar suporte opcional a versões antigas. |
| 2026-11-24 | Copiado `horizon-server-files` (properties/EULA/listas/cache/perfis). `cargo test -p horizon-server-files --lib`: 26 passaram. `cargo check --workspace` passou com 4 crates; graph do workspace não contém `horizon-client`, renderer, wgpu ou winit. | Validado (crates/base) | Integrar API de arquivos no host/engine e seguir com runtime. |
| 2026-11-24 | Tentativa provisória de copiar o `horizon-threads` inteiro: removidos boot_paint e system_menu de UI; 56 testes passaram, 13 de sampling/profiling falharam por limiares de amostragem no host cloud ARM64 (3 ignorados). A cópia foi descartada: não é dependência dos crates portados e a fronteira server mínima ainda precisa ser identificada. | Tentativa/limite registrado | Auditar o uso runtime antes de extrair só scheduler/sinais/allocator necessários; não reintroduzir pacote completo sem teste alvo. |
| 2026-11-24 | `cargo test --workspace` inicialmente parou ao compilar `world_probe`: o `face_visibility_fixture` exigia `block_render`, removido por ser culling visual. Removida só a fixture e as duas referências; probes server de mundo/simulação foram preservados. `cargo fmt --all -- --check` e a suíte completa passaram: 1.260 testes, 0 falhas, 10 ignorados; nenhuma build client. | Validado (workspace, host ARM64) | Seguir o inventário e manter o alvo x86_64/Actions como validação pendente. |
| 2026-11-24 | A baseline dedicada sem `server-plugins` conta 947 linhas, mas não é vanilla limpa: `cargo tree -i` confirma `horizon-mods` como dependência normal e `horizon-mod-api-gen` como build-dependency do `horizon-client`; os pacotes `horizon-server-plugin-api`/`loader` não aparecem sem a feature opcional. O workflow de release da main habilita essa feature (server-release.yml:65) e apenas omite `.so` externo. Nada disso foi migrado ou compilado. | Advertência de baseline | Excluir também os resíduos de mods/gerador do engine server-owned; não usar workflow legado como build de destino. |
| 2026-11-24 | Auditoria estática separou a CLI do engine: o grafo normal de `horizon-server` (404 linhas) não contém crate client/GPU/janela/plugin; seu código procura primeiro `horizon-engine` e ainda aceita `horizon-client` como fallback (`crates/horizon-server/src/engine.rs:14-18, 51-64`). `horizon-prepare` (44 linhas; grafo sem workspace deps) exige `facts.json`, `block-tags.json`, `registries.json` e `commands.json`; usa inputs/jars do jogo e compila exporters server e client (`crates/horizon-prepare/src/lib.rs:47-50, 121-159`). O engine atual ainda valida fontes de font/textures (`engine.rs:158-168`), e a geração de worldgen compara os dois lados. | Auditoria concluída (sem build/edição de código) | Ao extrair engine/preparer, remover fallback e saídas client só depois de substituir dependências verificadas; mapear exporters e consumidores de assets antes de podar. |
| 2026-11-24 | Portado `ticket_levels` de `horizon-sim-data` para `horizon-world`: níveis de chunk vanilla a partir de player/view/simulation distance, forced chunks, portal tickets e arena do dragão. Imports de produção: somente `horizon-core::fast_hash` e `std`; atualizado o comentário do benchmark para o pacote novo. `cargo fmt --all -- --check`, teste direcionado (11 passed, 1 ignored) e `cargo test --workspace --locked` passaram; o grafo atual (181 linhas) não contém client/mod/plugin/render/GPU. | Validado (módulo/workspace, host ARM64) | Integrar `ChunkTickets` ao runtime após a extração do tick; a suíte x86_64/Actions continua pendente. |
| 2026-11-24 | Portado `world_spawn` de `horizon-sim-data` para `horizon-world`: espiral vanilla de spawn inicial, busca de spawn/respawn e seleção de candidatos em chunks. Import ajustado de `horizon_world::heightmap` para `crate::heightmap`; demais APIs e testes preservados. `cargo fmt --all -- --check` e teste direcionado passaram (4 testes). A seguir, a validação completa do workspace passou (1.275 testes, 0 falhas, 11 ignorados); árvore Cargo (181 linhas) sem client/mod/plugin/render/GPU. | Validado (módulo/workspace, host ARM64) | Integrar no runtime ao extrair criação/respawn de `Game`; a suíte x86_64/Actions continua pendente. |
| 2026-11-24 | Revisão de escopo localizou e removeu APIs nativas de mod portadas inadvertidamente em `horizon-world`: callbacks de loot/item defaults, contexto/place nativo, módulo de instalação global e testes de hooks. | Validado nos testes direcionados anteriores | Reexecutar workspace completo e auditar os crates portados; não declarar o pacote server-only sem revisar APIs públicas. |
| 2026-11-24 | Removidos `worldgen/native_generate.rs`, callbacks nativos de geração de chunks/far-column e amostragem far-terrain/LOD client de `worldgen/generator.rs`; mantida a geração vanilla, inclusive early return de flat, ruído/aquíferos, regra de superfície, carvers, features e estruturas. Teste de presets foi adaptado para gerar chunks normais; removidas memoização e fixture/benchmark LOD sem consumidores server (`examples/lod_grid_bench.rs`). | Antes da última remoção, `cargo fmt --all -- --check`, `git diff --check`, `cargo check -p horizon-world --lib --locked` passaram; `cargo test -p horizon-world --lib --locked`: 509 passaram, 0 falhas, 4 ignorados. A suíte completa iniciada antes da exclusão do benchmark e a auditoria residual continuam em andamento; repetir a suíte se o alvo example tiver sido construído. | Finalizar/ajustar validação do workspace e auditoria; depois atualizar evidências, rever diff e enviar somente `BorizonV26.3`. |
| 2026-11-24 | Nesta continuação, a remoção em lote dos callbacks não-vanilla de `LevelAccess` revelou callsites e fixtures ainda dependentes deles (47 erros de produção e 64 erros totais no diagnóstico); para não deixar a branch num estado que não compila, a trait foi restaurada ao `HEAD` validado. O corte desses hooks continua pendente e explícito; não afirmar que está concluído. Permaneceram as alterações server-safe já independentes: teste de plates/tripwire baseado em ocupantes vanilla, consultas vanilla `count_entities`, remoção de veto na propagação de fluidos e exclusão do callback de mods de teleporte/consumo. | `cargo fmt --all -- --check`; `cargo check -p horizon-world --tests --locked` passou; `cargo test -p horizon-world --lib --locked`: 505 passaram, 0 falhas, 4 ignorados; `git diff --check` passou. `cargo test --workspace --locked` e auditoria de resíduos ainda estão em andamento nesta execução; não houve build do client. | Completar corte dos hooks em callsites/fixtures com semântica vanilla direta; auditar resíduos client/mod em quatro crates e validar workspace antes de commit/push. |
| 2026-11-24 | A suíte completa da branch terminou com exit code 0; foi consolidada a saída em log para não inferir totais por trecho final. O grafo `cargo tree --workspace --locked` permanece com 181 linhas e não encontrou referências Cargo a `horizon-client`, `horizon-mod*`, plugin, render, `wgpu`, `winit` ou `naga`. A auditoria estática de fontes ainda localiza APIs/prosa de mods no protocolo e hooks extensíveis em `horizon-world`, além de ramos client-side de pistões, `is_client_side` e fixtures de probes. | `cargo test --workspace --locked`: 1.263 passaram, 0 falhas, 11 ignorados (52 sumários de alvos); `cargo tree --workspace --locked`: 181 linhas; `cargo fmt --all -- --check`, `cargo check -p horizon-world --tests --locked`, `cargo test -p horizon-world --lib --locked` (505 passaram, 4 ignorados) e `git diff --check` passaram. Nenhuma compilação client. | Remover APIs/código client e callbacks restantes de forma semântica, limpar fixtures/comentários do protocolo e do world; depois repetir auditoria, revisar diff e publicar só em `origin/BorizonV26.3`. |
| 2026-11-24 | Continuação da auditoria removeu `BlockLandingWorld::trample_allowed` (veto NeoForge), a callback `has_native_block_entity` e ramos client-side de landing; a semântica resultante usa somente regras server/vanilla, flags dos estados e dimensionamento vanilla. Revisados métodos sem consumidores: removido `FluidKind::is_vanilla`; `FluidKind::all()` permanece pois `from_name` o usa. A extensão de componentes com IDs/namespace custom, negociação e instalação no protocolo foi retirada; loot odds de recipe viewer sem callsites também foi removida. Um helper de datafix rotulado como API, usado apenas pelos próprios testes, tornou-se privado e `cfg(test)`, mantendo cobertura do upgrade pré-flattening e eliminando warnings de dead code. | `horizon-world`: `block_landing.rs`, `level/{mod.rs,update.rs,pistons.rs}`, `fluid.rs`, `datafix/{mod.rs,block_state_data.rs,tests.rs}`, `loot.rs`; removidos `loot/odds.rs` e `loot/odds_tests.rs`. `horizon-protocol`: `item.rs`, `registry.rs`; removido `item/custom.rs`. | Última `cargo test --workspace --locked`: 1.210 passaram, 0 falhas, 11 ignorados, 52 alvos e 0 warnings. `cargo fmt --all -- --check`, `git diff --check`, `cargo check --tests --examples -p horizon-world --locked`, testes dos pacotes e `cargo tree --workspace --locked` (181 linhas, sem client/mod/plugin/render/GPU/shader) passaram. Nenhuma compilação client. | Validado (auditoria local) | Prosseguir com extração de runtime server-owned, concluir inventário de módulos mistos e iniciar prova executável; commitar e publicar esta rodada somente em `origin/BorizonV26.3`, sem marcar a migração total concluída. |
| 2026-11-24 | Continuação da auditoria removeu `BlockLandingWorld::trample_allowed` (veto NeoForge), a callback `has_native_block_entity` e ramos client-side de landing; a semântica resultante usa somente regras server/vanilla, flags dos estados e dimensionamento vanilla. Revisados métodos sem consumidores: removido `FluidKind::is_vanilla`; `FluidKind::all()` permanece pois `from_name` o usa. A extensão de componentes com IDs/namespace custom, negociação e instalação no protocolo foi retirada; loot odds de recipe viewer sem callsites também foi removida. Um helper de datafix rotulado como API, usado apenas pelos próprios testes, tornou-se privado e `cfg(test)`, mantendo cobertura do upgrade pré-flattening e eliminando warnings de dead code. | `horizon-world`: `block_landing.rs`, `level/{mod.rs,update.rs,pistons.rs}`, `fluid.rs`, `datafix/{mod.rs,block_state_data.rs,tests.rs}`, `loot.rs`; removidos `loot/odds.rs` e `loot/odds_tests.rs`. `horizon-protocol`: `item.rs`, `registry.rs`; removido `item/custom.rs`. | Última `cargo test --workspace --locked`: 1.210 passaram, 0 falhas, 11 ignorados, 52 alvos e 0 warnings. `cargo fmt --all -- --check`, `git diff --check`, `cargo check --tests --examples -p horizon-world --locked`, testes dos pacotes e `cargo tree --workspace --locked` (181 linhas, sem client/mod/plugin/render/GPU/shader) passaram. Nenhuma compilação client. | Validado (auditoria local) | Rodada já commitada em `339c9195b0` e enviada a `origin/BorizonV26.3`; próximo: prosseguir com extração de runtime server-owned, concluir o inventário de módulos mistos e iniciar prova executável, sem marcar a migração total concluída. |
| 2026-11-24 | Portado o catálogo server-only de game rules vanilla da referência, ajustando imports para APIs locais e `save` para `std::io::Result`. `LevelAccess::game_rule_catalog()` empresta a fonte completa e a simulação projeta os dez campos usados, sem defaults duplicados nem cópia do mapa por consulta. Os sete testes focados cobrem tipos/limites, invariantes do catálogo, NBT/gzip, persistência, feature experimental, fallback inválido e integração da projeção. | `cargo test -p horizon-world --lib gamerules --locked`: 7 passaram, 0 falhas; `cargo test --workspace --locked`: 1.217 passaram, 0 falhas, 11 ignorados em 52 alvos, 0 warnings; `cargo check -p horizon-world --tests --examples --locked` passou; `cargo tree --workspace --locked`: 181 linhas, sem nomes client/mod/plugin/render/GPU/shader; `git diff --check` passou. Nenhuma build de client. | Validado (módulo/workspace, host ARM64) | Avançar para extração do runtime server-owned; revisar submódulos de scoreboard/texto/registries antes de qualquer porte; não copiar section_mask sem consumidor de servidor. |
| 2026-11-24 | Auditoria read-only do scoreboard e do dispatcher: o `scoreboard.rs` tem 1.631 linhas e não é um corte seguro sem extrair component/text/registries; `horizon-command` é vanilla relevante, porém a árvore e o parsing dependem de vários símbolos de `horizon-client-core` e `/pregen` é extensão separada. Nenhum código foi copiado desses módulos. O relatório também rejeitou `section_mask` por ausência de callsites server e confirmou que codecs genéricos de Custom Payload/Login Plugin Query são vanilla; os testes de protocolo foram só neutralizados nos fixtures de terceiros. | Auditoria de imports, módulos e callsites na referência e no destino; alterações de código apenas nos fixtures de teste. | Validado (triagem estática) | Delimitar a próxima fronteira de tipos server-owned (`Text`/SNBT/registries) antes de migrar comandos ou scoreboard; preservar comportamento de protocolo vanilla. |
| 2026-11-24 | Auditoria server-only do protocolo: não foram encontrados handlers de frameworks de mods/plugins na produção; Custom Payload e Login Plugin Query genéricos são parte do protocolo vanilla e permanecem. Os únicos exemplos com identificadores específicos de extensões estavam em testes, então seus fixtures foram neutralizados sem alterar codecs. A auditoria do `scoreboard.rs` identificou um módulo misto de 1.631 linhas dependente de component/text/registries ainda client-owned; nenhum código dele foi copiado. `section_mask` foi descartado como candidato por só ter consumidores no client. | Incluído no `cargo test --workspace --locked` com 1.217 testes aprovados, 0 falhas e 11 ignorados; `git diff --check` passou. | Validado (auditoria/fixtures) | Extrair os tipos vanilla de scoreboard como unidades menores somente após definir component/text/registries neutros; manter codecs genéricos do protocolo. |
| 2026-11-24 | Auditoria de `horizon-client-core::registries::Registries`: o tipo agrega dados de servidor e contexto client-side. `horizon-command/src/args.rs` consome `target_block`, `target_point` e `selected_entities`, documentados como contexto de crosshair; `custom_completions` é estado de sugestões do cliente. A cópia exploratória integral foi descartada; nenhum arquivo/Cargo dep foi mantido. | Bloqueio de recorte: não transportar o agregador. Validação do branch após descarte: `cargo test --workspace --locked` passou (1.217 aprovados, 0 falhas, 11 ignorados, 52 sumários de alvos); `cargo tree --workspace --locked` (181 linhas) sem dependências client/mod/plugin/render/GPU/shader; `cargo fmt --all -- --check` e `git diff --check` passaram. Nenhum client compilado. | Definir tipos server-owned separados para registry snapshot e contexto de execução/sugestões; prosseguir com extração do runtime sem puxar `horizon-client-core`. |
| 2026-11-24 | Auditoria estática de dados/preparer: `HORIZON_RESOURCES` é a interface neutra existente, mas `horizon-world` ainda recorre a `.local/horizon-client/<version>` em datafix e fixtures. Na referência, `horizon-prepare` tem lanes e classpaths client+server (`lib.rs`), coleta artefatos dos dois lados (`artifacts.rs`) e copia assets de apresentação (`sounds.rs`); não há corte pronto para portar. Nenhum path/código foi alterado e nenhuma build do client/preparer foi executada. | Bloqueio de integração/preparação; os arquivos necessários contêm dados vanilla extraídos, mas origem/geração/instalação precisam migrar juntos. | Mapear todos os outputs realmente consumidos pelo futuro engine; definir `horizon-resources/<version>` neutro e implementar só exports server necessários, depois remover nomes/fallbacks herdados. |
| 2026-11-24 | Triagem de `client_side` em `entity_move.rs` e `player_food.rs`: as branches controlam collision/fall-damage/movement-emission e exhaustion conforme autoridade do nível, não usam código de client nem crate externa. Nenhum código removido. | Sem consumer/implementador de `EntityMoveWorld` no workspace server atual; a semântica será exercitada quando o runtime for extraído. | Implementar a interface no engine dedicado com `client_side = false` para mundo server e autoridade local de acordo com a entidade; cobrir transições com testes. |
| 2026-11-24 | Extraído `SOUND_SOURCES` da referência para `horizon-core::sound_source` como catálogo vanilla estático de `/playsound`; enumera exatamente 11 nomes na ordem vanilla e não tem dependências. Primeiro teste direcionado falhou porque o auxiliar tentou `dedup` em array (`E0599`); convertido para `Vec` e teste repetido passou (2 testes). | `cargo test -p horizon-core --lib sound_source --locked`: 2 passaram, 0 falhas; validação workspace: 1.219 passaram, 0 falhas, 11 ignorados, 52 alvos, 0 warnings. `cargo fmt --all -- --check`, `git diff --check` e árvore sem dependências proibidas passaram. | Usar `horizon_core::sound_source::SOUND_SOURCES` ao extrair dispatcher server-owned, sem trazer enum de partículas que depende de implementação/sprites do client. |
| 2026-11-24 | Extraídos para `horizon-core::bossbar` os nomes vanilla de cores e overlays aceitos por `/bossbar` (7 colors, 5 styles), comprovados pelo callsite de `horizon-command/src/tree_misc.rs`. Não foi copiado o modelo misto que usa scoreboard/component/text/NBT/display formatting; as constantes não têm deps. | `cargo test -p horizon-core --lib bossbar --locked`: 2 passaram, 0 falhas; workspace completo: 1.221 aprovados, 0 falhas, 11 ignorados, 52 alvos e 0 warnings. `cargo fmt --all -- --check`, `git diff --check`, `cargo tree --workspace --locked` (181 linhas, sem dependências proibidas) passaram. | Usar `horizon_core::bossbar::{COLORS, STYLES}` na futura extração do dispatcher; manter componentes e estado de bossbar para uma fronteira separada/testada. |
| 2026-11-24 | Auditoria da fronteira Text/Component: a referência acopla parsing de component a `Text`/`Style`, `lang`/translation e named colors; `keybind` também passa por `key_display`. O `horizon-core::text` atual é somente parsing de inteiros JDK, e o `Component::Text(Tag)` do protocolo é componente de item, não chat. A referência `horizon-command/src/args.rs:1450` depende de `horizon-client-core::component::parse_snbt` para Component/Style, mas o branch atual ainda não tem `horizon-command`. Nenhum código foi portado. | Sem módulo neutro atual que garanta round-trip de JSON de chat/unknown fields; não reescrever apresentação como se fosse dado server-owned. | Quando a etapa de comandos chegar, isolar/testar o parser puro SNBT→JSON usado em Component/Style num módulo server-owned (serde_json já existe em crates server); projetar o container/codec para round-trip e migrar formatação/tradução separadamente, sem resolver keybind/teclado. |
| 2026-11-24 | Portado o motor vanilla `NbtPath` (parse/prefix, get/count, set/insert/remove, comparação parcial e erros com translation keys) de `horizon-client-core` para `horizon-world`; a fronteira visual era uma chamada de formatação SNBT, substituída pelo formatter server-owned `snbt_text::to_snbt`. A referência confirma uso em argumentos e comandos `/data`/`/execute`. | `crates/horizon-world/src/nbt_path.rs`, `src/lib.rs`; nenhum Cargo dependency ou código client/mod/plugin foi adicionado. | 12 testes focados passaram; `cargo fmt --all` e `git diff --check`. Os resultados workspace completos ficam pendentes até a validação final após este corte. | Ligar o módulo ao futuro `horizon-command` server-owned e confirmar que parsers/handlers de `/data` e `/execute` usam exatamente esse comportamento; não declarar a migração global concluída. |
| 2026-11-24 | Portado `block_entity_nbt` para `horizon-world`: canonicalização vanilla e merge por campos/listas de slots preservam unknown fields, itens e componentes de sinais como NBT; a referência usa as operações no save/persist/spill/machines. A única fronteira de identifier foi adaptada para a API server-owned `Identifier::parse`; nenhum renderer, idioma ou client API entrou. | `crates/horizon-world/src/block_entity_nbt.rs`, `src/lib.rs`; sem Cargo/lockfile changes. | 15 testes focados passaram; validação completa pós-port: 1.248 aprovados, 0 falhas, 11 ignorados em 52 alvos, 0 warnings; formato e árvore sem pacotes proibidos. | Ligar canonicalização/overlay ao futuro leitor/gravador server-owned de block entities; validar round-trip de chunks quando o subsistema de persistência do engine for migrado. Não marcar engine/migração global concluídos. |

## Ponto de partida verificado

Retrato da `main` em 2026-11-24, commit `f2e5556cbcb8060313f513d1871ccf3e7fb6e6ec`:

- `horizon-server` é o CLI/supervisor que inicializa e administra o servidor.
- `horizon-prepare` importa os arquivos locais de Minecraft e prepara os dados
  usados pelo engine.
- O workflow legado de release (`.github/workflows/server-release.yml:65-70`) ainda compila `crates/horizon-client` como binário e o renomeia para `horizon-engine`, habilitando `dedicated-server,server-plugins`; a release diz não incluir plugin `.so`, mas compila o host/API C ABI. Não reproduzir esse build na branch vanilla.
- O caminho de runtime dedicado já tem entrada própria em
  `crates/horizon-client/src/game/dedicated.rs` e integração de plugins em
  `crates/horizon-client/src/game/server_plugins.rs`, mas o tipo `Game` e vários
  imports ainda pertencem ao código compartilhado do client.
- O grafo Linux desse alvo ainda inclui `horizon-client-lib`,
  `horizon-client-core`, `horizon-client-input`, `horizon-client-render`,
  `horizon-narrator`, `wgpu`, `winit`, `naga`, `horizon-optifine`,
  `horizon-shaders`, `horizon-shader-api` e o gerador da antiga API de trait
  mods. A presença no grafo prova compilação; não prova que cada módulo seja
  necessário em runtime.
- `server-plugins/api`, `server-plugins/sdk` e `server-plugins/loader` são a
  extensão nativa `.so`, fora do marco inicial. A branch vanilla deve deixá-los
  fora do engine/workspace até o servidor vanilla funcionar sozinho.
- O workflow de release monta `horizon-engine`, `horizon-server` e
  `horizon-prepare`. O teste recente do pacote identificou também um requisito
  de empacotamento: uma pasta limpa precisa receber todos os exporters/fontes ou
  classes verificadas que `horizon-prepare` exige para preparar Minecraft 26.3.
  O teste direto só concluiu após colocar fontes auxiliares do checkout na pasta
  de teste; o release ainda precisa ser validado sem essa ajuda manual.

Comando que reproduz o grafo dedicado atual no Linux:

```sh
cargo tree --locked --target x86_64-unknown-linux-gnu \
  -p horizon-client --no-default-features \
  --features dedicated-server
```

Essa saída é o baseline estático do engine legado sem feature `server-plugins`; ela ainda inclui `horizon-mods` e o build-dependency `horizon-mod-api-gen` (o `cargo tree -i` aponta entrada pelo pacote e pelo build script, respectivamente). O workflow de release atual da main habilita `.so` plugins; esse caminho não faz parte do destino vanilla. Medir novamente após cada corte e comparar crates, dependências nativas, duração de build e memória do runner. Não estimar ganhos sem medição reproduzível.

### Auditoria estática do CLI e do preparer (2026-11-24)

| Alvo | Evidência observada | Consequência para a migração |
|---|---|---|
| `horizon-server` (CLI/supervisor) | `cargo tree --locked --edges normal -p horizon-server` na referência: 404 linhas; árvore com `horizon-platform`, `horizon-server-files`, `horizon-world`, `horizon-protocol`, `horizon-threads`, `horizon-core`, `horizon-system`, `horizon-bench`, `horizon-perf-format`, `horizon-hooks`, `horizon-mcp`, `horizon-anticheat`, `horizon-chat-filter` e `horizon-release-sign`. Sem `horizon-client`, renderer, `wgpu`, `winit`, `naga` ou plugin host. | O supervisor pode migrar separadamente do engine; suas dependências extras/serviços ainda precisam ser avaliados contra o marco vanilla. Ele executa o binário do jogo e não é o engine. |
| Seleção do engine pela CLI | `crates/horizon-server/src/engine.rs:14-18, 51-64`: tenta `horizon-engine` primeiro, mas mantém fallback ao executável `horizon-client`. | Após existir engine server-owned, remover o fallback e atualizar mensagens/installer para que o produto não dependa de client. |
| `horizon-prepare` (binário e biblioteca) | `cargo tree --locked --edges normal,build -p horizon-prepare`: 44 linhas; sem dependências Cargo de workspace. Runtime direto: `crc32fast`, `regex`, `sha1`, `sha2`, `zip`, com `libc` em Linux/macOS; build usa `sha2` para hash de fontes (`build.rs:5-37`). | O pacote é independente de Cargo, mas seu comportamento e dados produzidos ainda podem depender de fontes Java e jars do cliente; não basta preservar seu manifest. |
| Dados requeridos pelo engine atual | `crates/horizon-server/src/engine.rs:158-168` valida `facts.json`, `block-tags.json`, `registries.json`, `commands.json` e diretórios de font/textures. `horizon-prepare/src/lib.rs:47-50, 121-159` distingue exporters server/client; worldgen também compara bundles dos dois jars. | Preservar os dados até a nova fronteira de carregamento provar quais arquivos são necessários. Auditar/condicionar as saídas client e empacotar exporters server somente após rastrear consumidores e fallbacks. |

As árvores foram obtidas sem `cargo build/check/test` nesses dois pacotes e sem compilar o client. `cargo tree` resolveu metadata/dependências e baixou alguns crates para o cache; nenhuma alteração foi feita no checkout de referência.

### Auditorias estáticas de command/items/sim-data e core compartilhado (2026-11-24)

As auditorias abaixo foram feitas somente na cópia de referência `/home/ubuntu/BorizonServer-main`. Não houve build, testes, edição ou cópia destes quatro pacotes durante a auditoria. Os achados guiam o recorte; não autorizam arrastar a crate client inteira.

| Pacote/fronteira | Evidência de código e uso | Decisão provisória server-only |
|---|---|---|
| `horizon-client-core` | Manifesto direto: apenas `horizon-world`, `serde` e `serde_json` (`crates/horizon-client-core/Cargo.toml:9-12`), sem dependências de GPU/janela. Exporta NBT/SNBT/path, texto/componentes/idiomas, registries, gamerules, scoreboard, potions, partículas, bossbar, sound source, UUID/Who e `key_display`. `component.rs:421-425` resolve `keybind` através de `key_display`, que contém layout/nome de teclado. | Não copiar como `horizon-client-core`. A camada de tipos compartilhados pode ser extraída com nome neutro; deixar `key_display` no client e manter o `keybind` estruturado no protocolo sem fingir conhecer layout/configuração do usuário. `text`/NBT e `lang` exigem decidir quais semânticas são dados de jogo vs apresentação. |
| `horizon-command` | `vanilla.rs:41-73,574-642` constrói dispatcher e árvore dedicada; `tree_extra` contém registros dedicados; `network.rs:11-12` serializa `Commands` via `horizon-protocol`. Parsing, sugestões, permissões, mensagens de erro, ordem e árvore são parte do comportamento vanilla. `/pregen` vem da extensão Horizon opcional, não do dispatcher vanilla. Depende de módulos do core como gamerules, sound_source, bossbar, text/lang, NBT, registries, scoreboard e particle_type. | Candidato server-side, porém só após substituir a dependência nominal em `horizon-client-core` por APIs comuns extraídas. Preservar semântica e testes dedicados; excluir extensão `/pregen` do dispatcher vanilla. |
| `horizon-items` | Não tem dependência direta de renderer/protocolo; contém regras de ferramenta, mineração, loot/consumíveis, durabilidade/cooldowns, componentes de item e codecs NBT (`item_nbt.rs`). Usa scoreboard/component/NBT e lang via core. `enchant::full_name*` é apresentação; `blocks::StateFacts` mistura dureza/fluido/colisão/suporte/som com `RenderShape`, `solid_render`, `random_offset`, `outline` e `outline_edges`. | Mover regras/codec/dados que o servidor consume e cortar apresentação só com mapa de consumidores. `outline` pode servir picking/colisão; não eliminar sem provar que nenhum cálculo server usa. Não habilitar `test-support`. |
| `horizon-sim-data` | Núcleo valioso: chunks, block entities, world/tickets/spawn, colisão, lighting, block edits, containers/furnaces, brewing, efeitos, raids, sculk/vault, POI e random sources. Mistura HUD, custom sky, pack palettes/hooks, CEM, animação de render, containers/blocos modded, auth/skins, crash/telemetria. Manifesto acopla `horizon-client-core`, `horizon-threads`, `horizon-system`, assets/protocol/items/command e externos (`png`, `ureq`, `glam`, `bytemuck`, compressão). | Não copiar integral: `ticket_levels` e `world_spawn` foram extraídos em separado para `horizon-world`, cada qual com testes próprios; os demais módulos requerem corte por uso/import. Manter fronteiras mistas `chunk/block_entity.rs`, `containers.rs`, `block_entities.rs` e pack data sob investigação. |

**Testes sugeridos após extração:** suíte de comando vanilla/dedicado (árvore, parse, permissões, mensagens); round-trip NBT de stacks e componentes; invariantes de chunks/tickets/colisão/lighting/block entities; nenhuma dessas suítes foi executada nesta rodada.

### Auditoria estática do runtime dedicado legado (2026-11-24)

| Fronteira | Evidência na `main` | Implicação para o porte server-only |
|---|---|---|
| Bootstrap dedicado | `crates/horizon-client/src/game/dedicated.rs:46-105` parte de `run`, usa `load_resources`, cria `Game::new` e termina em `run_dedicated`; passa `UiResources` e `Audio` para a construção. | É um ponto de entrada reutilizável, não um engine autônomo. Preservar lógica vanilla de startup, mas mover a propriedade do estado e o boot para o pacote server-owned antes de retirar `horizon-client`. |
| Bundle de recursos | `resource_load.rs:124-143, 151-257, 342-474` carrega dados headless por branches, mas o tipo `CoreTables` ainda contém `Assets`, `RenderAssets`, `ItemRenderer` e `FaceVisibility`, constrói `MeshTables`/`PackResources` e carrega metadados/language. A versão server deste branch removeu `horizon-world::block_render`; portanto a cópia integral deste loader não compila sem adaptar essa fronteira. | Separar primeiro tabelas de registry/gameplay/biome/collision/behavior necessárias ao servidor dos bundles de meshing/render; não reintroduzir `block_render` para preservar um tipo do loader client. |
| Construção de `Game` | `game.rs:2026-2031` define `UiResources` como atlas de GUI e panorama; `game.rs:2152-2425` contém no mesmo `Game` `World`, `Streaming`, save/ticks/entidades e estado de player/container, junto de input/winit, window/renderer, telas/UI, geometria GPU e áudio/music. | A auditoria confirma que separar server não é trocar nome de binário: extrair estado e métodos por domínio enquanto preserva as implementações de simulação e save. |
| Recursos CPU visuais | `resource_loader.rs:64-113` constrói `GameCpuResources::server` com modelos/variantes vazios para satisfazer o construtor compartilhado; é caminho de compatibilidade do build dedicado. | Não portar bundle visual vazio como API server. Substituir por recursos de simulação realmente usados, sem stubs para tipos do client. |
| Tick/níveis | `game/levels.rs:43-104, 644-795` guarda níveis, mundos, saves, streaming, entidades e tick server; usa tipos de `horizon-client` e também instrumentação/plugin callbacks no loop. | Preservar código vanilla de níveis/tick e remover chamadas de profiling/mod/plugin por corte verificável; mapear dependências por símbolo antes de copiar o módulo integral. |


**Triagem dos submódulos em `game/dedicated.rs` (main, 2026-11-24):** `events.rs` é um buffer para endpoint Horizon `/api/v1/events`, não um requisito vanilla; `predicate_graph.rs` declara ser validação parcial nova, não port de `RegistryDataLoader`/`ValidationContext`; ambos ficam fora do núcleo vanilla. `watchdog.rs` é comportamento server-side pertinente, mas chama `horizon_threads::stack_dump` e `horizon_client::crash`/logging, então requer uma fronteira neutra de stack/crash antes de portar. `management.rs`, `status.rs`, `transfer.rs` e `report.rs` acessam `Game` ou tipos/profiling client; não migrar integralmente nesta etapa.

**Próxima sequência técnica:** a suíte pós-cortes está concluída: `cargo test --workspace --locked` (1.248 aprovados, 0 falhas, 11 ignorados, 52 alvos, 0 warnings), `cargo tree --workspace --locked` (181 linhas, sem dependências client/mod/plugin/render/GPU/shader), formatação e `git diff --check` passaram; nenhum client foi compilado. Não há um corte integral seguro no agregador `Registries` nem no `horizon-prepare`: ambos misturam estado/contexto ou lanes server/client. Próximo: mapear callsites de produção no caminho de `Game` dedicado; integrar o motor NBT path ao futuro parser de comandos e a canonicalização/overlay de block entities ao leitor/gravador de mundo server-owned; estabelecer fronteiras neutras para Text/Component, scoreboard, registries e keybind; rastrear outputs de recursos consumidos antes de desenhar um preparer server-only; só então migrar módulos vanilla acompanhados de seus testes. Excluir endpoints/eventos Horizon e `/pregen`; extrair watchdog somente depois de desacoplar crash-report/stack-dump. A branch ainda não possui engine executável e a migração global não está concluída.

## Inventário completo do workspace atual

Inventário gerado com `cargo metadata --no-deps` em 2026-11-24: **57 pacotes**.
O rótulo é a decisão para o marco inicial **engine vanilla sem plugins/extras**,
não uma decisão de apagar histórico. `MOVER/SPLIT` exige preservar a
implementação vanilla usada pelo servidor antes de remover o pacote de origem.
`ADIAR` significa deixar fora do engine vanilla e rever somente numa etapa
posterior. Esta tabela cobre todos os pacotes membros do workspace atual.

**Prioridade de migração:** `P0` = bloquearia engine vanilla caso não seja
resolvido (simulação, rede, estado ou dependência mista); `P1` = necessário para
preparar/instalar/iniciar o pacote limpo, sem fazer parte da simulação; `P2` =
ferramenta operacional opcional ou trabalho futuro; `P3` = remover da branch
vanilla depois de cortar as referências. Na tabela, MANTER/MOVER/SPLIT em código
de simulação é P0; `horizon-prepare` e o mínimo de `horizon-server` são P1;
ADIAR é P2; client/mod/serviços fora do escopo são P3. As decisões de módulo
marcadas AUDITAR ainda exigem a inspeção da etapa 1 antes de apagar código.

| Pacote atual | Decisão para o marco vanilla | Motivo/ação concreta |
|---|---|---|
| `horizon-addresses` | REMOVER | Catálogo de endpoints de serviços do Horizon; não é parte do protocolo/servidor Minecraft vanilla. |
| `horizon-anticheat` | ADIAR | Extra de gameplay do fork. Não carregar no primeiro engine vanilla; decidir depois se volta como plugin separado. |
| `horizon-assets` | AUDITAR; fora do engine inicial se só decodificar apresentação | Decodifica modelos/recursos do client. Preservar apenas código que uma carga vanilla server realmente consuma; NPC/modelos custom ficam para plugin futuro. |
| `horizon-bench` | ADIAR como ferramenta de desenvolvimento | Formato/apoio de benchmark, não simulação vanilla. Não linkar ao engine sem necessidade comprovada. |
| `horizon-chat-filter` | ADIAR | Filtro/mutes são extra do fork; retirar do primeiro runtime vanilla. |
| `cargo-horizon-mod` | REMOVER | CLI para criar/buildar/executar mods de conteúdo Horizon; não faz parte do servidor vanilla. |
| `horizon-client` | EXTRAIR e depois REMOVER como pacote | A entrada `game::dedicated` é a base existente do engine; movê-la para um binário/pacote server-owned preservando o código vanilla usado. |
| `horizon-client-backends` | REMOVER | Backends de GPU para client com janela. |
| `horizon-client-core` | MOVER/SPLIT | Algumas estruturas compartilhadas (texto, SNBT/NBT, gamerules, registries/partículas) podem ser usadas pelo servidor. Migrar só as partes comprovadas para crates de domínio server/core. |
| `horizon-client-input` | REMOVER | Teclado, mouse, controles, clipboard e janela são client. Desacoplar tipos de evento que ainda cruzem a fronteira. |
| `horizon-client-lib` | MOVER/SPLIT | Biblioteca heterogênea de sistemas client, control tool e probes; transferir apenas simulação e rotinas usadas pelo dedicado. |
| `horizon-client-render` | MOVER módulos server-safe; depois REMOVER do workspace server | O manifest diz render/screens, mas há módulos hoje alcançados pelo engine. Identificar e migrar os imports necessários antes de cortar. |
| `horizon-command` | MANTER | Parsing/árvore de comandos vanilla e serialização do protocolo. |
| `horizon-core` | MANTER | Fundamentos da simulação vanilla. |
| `horizon-cosmetics` | REMOVER | Cosméticos desenhados pelo client; não mudam o servidor vanilla. |
| `horizon-discord` | REMOVER do engine vanilla | Rich Presence do client. |
| `horizon-dualsense` | REMOVER | Saída HID e controles do client. |
| `horizon-friends` | REMOVER | Convites/relay de mundos client e servidor integrado. |
| `horizon-game` | REMOVER | Wrapper de executável que depende de `horizon-client` e do mod index. |
| `horizon-hdr` | REMOVER | Metadados de swapchain/Vulkan. |
| `horizon-heartbeat` | REMOVER | Ping de presença do client para serviços Horizon. |
| `horizon-hooks` | AUDITAR e reduzir | Hooks de integração Horizon/mods. Remover extensões de client e manter somente mecanismos indispensáveis ao servidor vanilla, caso existam. |
| `horizon-items` | MANTER | Itens/blocos e dados usados pela lógica vanilla; confirmar por import e execução do engine. |
| `horizon-mcp` | ADIAR como ferramenta operacional | Interface de administração MCP não é gameplay vanilla. Pode ser adicionada à CLI depois, fora do engine. |
| `horizon-mod-api-gen` | REMOVER | Geração da antiga API Rust trait mods; não será migrada nem usada. |
| `horizon-mod-api-gen-tests` | REMOVER | Testes do runtime/API de mods que não será usado. |
| `horizon-mod-deps` | REMOVER | Dependências para mods nativos; não são parte do engine vanilla nem do plugin atual. |
| `horizon-mods` | REMOVER do engine vanilla, após confirmar datapacks | Conteúdo/modpacks/índice Horizon. Não migrar a API de mods. Preservar a implementação vanilla de datapacks onde estiver; não confundir datapack vanilla com mod executável. |
| `horizon-narrator` | REMOVER | Narração de UI/chat do client, sem uso na simulação server. |
| `horizon-official` | REMOVER | Executável client com integração de serviços Horizon. |
| `horizon-official-servers` | REMOVER | Lista de servidores Horizon na tela multiplayer do client. |
| `horizon-optifine` | REMOVER do primeiro engine; reavaliar só no plugin futuro | Formatos CEM/connected textures são apresentação/customização. Modelo NPC do AuraNPC não deve manter isso dentro do engine vanilla. |
| `horizon-perf` | ADIAR como ferramenta | CLI/leitor de captures de performance; não é runtime vanilla. |
| `horizon-perf-format` | ADIAR | Formato próprio `.hzperf`; retirar do baseline vanilla salvo se houver requisito de diagnóstico decidido separadamente. |
| `horizon-perf-share` | REMOVER | Abrir/compartilhar captures via serviço, sem relação ao servidor vanilla. |
| `horizon-platform` | MOVER/SPLIT | API de integração entre engine e camada Horizon. Deixar no engine somente infraestrutura usada pelo dedicado; remover a fachada destinada a mods/launcher quando sem consumidores. |
| `horizon-polytone` | REMOVER | Formato de modificadores visuais de resource pack do client. |
| `horizon-port-pack` | REMOVER | Ferramenta para portar modpacks Java, sem função no servidor vanilla. |
| `horizon-prepare` | MANTER como ferramenta separada | Importa/verifica os arquivos Minecraft e produz os dados requeridos pelo engine; não é lógica de jogo nem código client UI. Corrigir o pacote para instalação limpa. |
| `horizon-protocol` | MANTER | Protocolo Java Edition e estruturas usadas na conexão/sessão do servidor. |
| `horizon-relay` | REMOVER | Serviço de relay de Play with Friends. |
| `horizon-release-sign` | ADIAR/condicional à CLI | Assinatura/verificação de releases; manter só se a CLI server continuar instalando/atualizando releases. Não linkar ao engine vanilla. |
| `horizon-server` | MANTER separado do engine, depois auditar extras da CLI | CLI/supervisor útil para hospedar o engine. Painel, Discord, MCP, update e demais serviços são decisões operacionais próprias, não lógica vanilla. |
| `horizon-server-api` | ADIAR com plugins | Tipos para comandos/jogadores/extensões; não são necessários ao motor vanilla sem plugins. Extrair tipos essenciais de jogador/comando para a camada vanilla se a auditoria provar uso independente. |
| `horizon-server-files` | MANTER | `server.properties`, EULA, ops, whitelist, bans e cache vanilla. |
| `horizon-server-plugin-api` | ADIAR; não migrar agora | ABI pública de plugins `.so`; preservada na `main`/histórico. Reintegrar só em etapa futura depois do engine vanilla estabilizar. |
| `horizon-server-plugin-loader` | ADIAR; não migrar agora | Loader dinâmico `.so`; não linkar nem compilar no primeiro engine vanilla. |
| `horizon-server-plugin-sdk` | ADIAR; não migrar agora | Wrappers Rust públicos do ABI; não pertence ao marco vanilla puro. |
| `horizon-shader-api` | REMOVER | API de shader packs Horizon e ferramenta de shader, sem função em servidor vanilla. |
| `horizon-shaders` | REMOVER do engine vanilla | Leitura de shader packs custom do client; não confundir com carregamento vanilla de datapack/recursos. |
| `horizon-sim-data` | SPLIT; não migrar como crate inteira | `ticket_levels` e `world_spawn` server-safe já foram migrados para `horizon-world`; extrair os demais módulos por uso/import, mantendo fora HUD/sky/CEM/animações, auth/skins, crash/telemetria, hooks e dados de mods. |
| `horizon-system` | ADIAR/condicional à CLI | Informações do processo/máquina. Não é simulação; manter apenas se status/diagnóstico da ferramenta de hospedagem precisar. |
| `horizon-telemetry` | REMOVER | Consentimento/outbox/upload de telemetria do client. |
| `horizon-theme` | REMOVER | Fontes, ícones e design system de UI; não manter por relatórios visuais da CLI sem decisão explícita. |
| `horizon-threads` | MANTER e auditar deps de plataforma | Runtime de threads/allocator usado pelo engine; remover somente as partes client-specific, mantendo scheduling e alocador requeridos. |
| `horizon-world` | MANTER | NBT, regiões, chunks e armazenamento vanilla do mundo; agora também inclui `ticket_levels` (níveis de chunk tickets) e `world_spawn` (spawn inicial/respawn vanilla). |
| `xtask` | ADIAR/REDUZIR | Ferramentas do repositório. Manter apenas tarefas de empacotamento/checagem server; pode sair do workspace runtime. |

### Pacotes fora do workspace e diretórios

| Caminho atual | Decisão para o marco vanilla |
|---|---|
| `crates/horizon-bots` (excluído do workspace) | Ferramenta de carga/teste separada; não integrar nem publicar com o engine. Não fazer build de bots nesta migração. |
| `crates/horizon-mod-registry` (excluído) | Registro de mods client; remover da branch server-only. |
| `crates/horizon-telemetry-server` (excluído) | Backend de telemetria do projeto, não servidor Minecraft; remover da branch do produto. |
| `launcher/` | Produto client independente; remover da árvore final. |
| `third_party/wgpu-hal`, `third_party/naga` | Patches para render/shaders client; remover após provar que o grafo vanilla não os usa. |
| `third_party/libmimalloc-sys` | Patch do alocador usado por `horizon-threads`; manter enquanto a implementação server depender dele. |
| `server-plugins/` | Não migrar nesta etapa; preservar na `main`/histórico. A futura integração começa depois de o engine vanilla estar isolado. |
| `mods/horizon/src/test/java/dev/horizonlab/parity/` | Fontes Java de referência usadas pelos exporters do `horizon-prepare`; não são a API de mods do jogo. Manter no processo de release ou empacotar exporters compilados e verificados. |
| `mods/horizon/src/bench/` e `mods/horizon/src/neoforge-*` | Harnesses de comparação/benchmark. Não entram no servidor runtime; auditar e remover da branch vanilla se não forem necessários para preparar dados ou validar simulação server. |
| `native/`, `native-hooks.json` | API/hook system antigo de mods nativos; não migrar. Remover junto com gerador e referências depois de confirmar que nenhum recurso vanilla depende dele. |
| `launcher/` e assets de launcher | Produto client; retirar da árvore final. |
| `packaging/server/` | Egg/Docker/instaladores do servidor; manter e adaptar para o novo arquivo de release. |
| `packaging/windows/` | Empacotamento de executáveis Windows/client; remover se o produto final continuar Linux x86_64, como o workflow atual. |
| `research-lock.json` | Manter: fixa versão e hashes de Minecraft usados pelo `horizon-prepare`. |
| `flake.nix` / `flake.lock` | Auditar saídas: manter somente packages server/preparer/CLI; remover saídas client/launcher. |
| `tools/` | Não é dependência de Cargo nem bloqueia o corte do grafo. Como acordado, não fazer limpeza ampla agora; só tirar referências client se atrapalharem docs/workflows server. |
| `.github/workflows/ci.yml` | Manter e reduzir para engine vanilla, CLI e preparer depois de existirem crates próprios. Não compilar SDK/loader nesta etapa. |
| `.github/workflows/server-release.yml` | Manter: construir/empacotar engine vanilla, CLI e preparer e iniciar o pacote limpo em smoke test. |
| `.github/CODEOWNERS`, templates, `release-line.txt`, configs de Actions | Revisar no fechamento; reter só configuração de contribuição/release aplicável ao servidor. |
| `.cargo/config.toml`, `Cargo.toml`, `Cargo.lock`, `.dockerignore`, `.gitignore`, `clippy.toml` | Manter/adaptar para o workspace server. Remover flags/targets exclusivamente client. |
| `README.md`, `docs/`, `LICENSE`, `THIRD_PARTY_NOTICES.md` | Reescrever índice/docs como server-only; preservar avisos/licenças do código/dependências que ficarem. |

O diretório `mods/` é ambíguo pelo nome: parte dele contém exporters de
referência vanilla usados pelo preparer; não apagá-lo junto com a API de mods.
Já `native/` e `native-hooks.json` correspondem ao sistema de extensão antigo e
ficam fora deste projeto server vanilla.

## Decisões de escopo já tomadas

- **Produto inicial:** servidor Horizon vanilla 26.3, sem as extensões do fork.
- **Sem build client:** nenhum `cargo build/check/test -p horizon-client`, build
   da janela ou teste de renderer. O baseline usa o release/Actions existente;
   o primeiro build novo usa o pacote server-owned depois da extração.
- **Lógica existente:** mover o código dedicado e simulação usada. Não
   reimplementar mecânicas vanilla por causa de nomes/pastas antigos.
- **Mods:** remover a antiga API/runtime de trait mods; não migrar modpacks,
   registro Horizon ou loader de mods. Manter datapacks vanilla somente onde a
   lógica do jogo exigir.
- **Plugin API `.so`:** fora desta migração vanilla. Não integrar API, SDK ou
   loader agora; preservar a versão atual em `main`/histórico para uma etapa
   posterior independente.
- **Extras de gameplay:** anticheat e chat filter não entram no engine vanilla
   inicial. Reavaliar depois como plugins separados, sem acoplá-los ao core.
- **Crates:** podem continuar separados quando forem componentes necessários do
   servidor. Nenhum crate será mantido para preservar compatibilidade client.
   Consolidar crates somente quando reduzir dependência/custo ou simplificar uma
   fronteira real.
- **Release:** primeiro provar instalação/preparação limpa com o pacote
   publicado; o release anterior não empacotou tudo que `horizon-prepare`
   precisou no teste local.

### Arquivos e fronteiras a inspecionar primeiro

- `crates/horizon-client/src/game.rs`: `Game` ainda combina estado do mundo e
  simulação com campos de apresentação; identificar campos/métodos usados por
  `game::dedicated` e mover a propriedade do estado para o engine server.
- `crates/horizon-client/src/game/dedicated.rs`: caminho atual de inicialização
  dedicada; é o ponto de partida para preservar o runtime existente.
- `crates/horizon-client/src/game/server_plugins.rs`: código da integração
  `.so` fora do primeiro engine; catalogar as referências para removê-las sem
  carregar o plugin host.
- `crates/horizon-client/src/entry.rs`, `main.rs` e `Cargo.toml`: entrada e
  features que hoje constroem o binário dedicado junto do pacote client.
- `crates/horizon-client/build.rs` e `game/mod_api*`/`native_mods*`: geração e
  referências da antiga API interna; separar de tipos de conteúdo antes de
  retirar o gerador.
- `crates/horizon-client-lib/Cargo.toml`,
  `crates/horizon-client-render/Cargo.toml` e
  `crates/horizon-client-input/Cargo.toml`: dependências que aparecem no grafo
  dedicado; seguir os imports efetivamente usados pelo `Game` do servidor.
- `crates/horizon-prepare/src/exporters.rs` e
  `.github/workflows/server-release.yml`: contrato de fontes/exporters e o
  conteúdo do arquivo de release.
- `Cargo.toml`, `Cargo.lock`, `.github/workflows/ci.yml`,
  `.github/workflows/server-release.yml`, `docs/README.md` e README da raiz:
  workspace, builds permitidos, empacotamento e documentação do produto final.

## Roteiro de migração em checkboxes

### Branch e baseline

- [x] Clonar `main` em checkout separado para referência: `/home/ubuntu/BorizonServer-main`, commit `f2e5556cbc`; manter o trabalho em `BorizonV26.3`.
- [x] Capturar metadados (57 pacotes na origem) e baseline estático do alvo dedicado com `dedicated-server`, sem a feature `server-plugins` `.so` (947 linhas); a saída mostra crates client/GPU/janela e resíduos `horizon-mods`/`horizon-mod-api-gen`; não é uma baseline vanilla limpa. Nenhum build do client foi executado.
- [ ] Verificar relação histórica da branch e registrar tempo/RAM dos Actions e smoke de instalação limpa usando dados existentes; não rebuildar `horizon-client` para baseline.
- [ ] Corrigir o release/preparação até uma pasta vazia conseguir preparar recursos
    usando somente o arquivo publicado e os arquivos locais Minecraft do operador.

- [ ] **Aceite:** existe uma branch isolada, baseline repetível e um teste de release
    que detecta se o pacote depende de fontes/arquivos omitidos.

### Inventário do uso server real

- [x] Gerar `cargo metadata` da origem e `cargo tree` do engine legado dedicado (somente análise estática).
- [x] Capturar árvores estáticas locked do `horizon-server` (normal) e `horizon-prepare` (normal + build); consultar manifestos e identificar crates de workspace/client ausentes.
- [ ] Completar classificação de todas as dependências, incluindo dev-dependencies/target-specific e rotas Cargo indiretas; separar dependência Cargo da dependência de artefatos Java/empacotamento.
- [ ] Para cada dependência de workspace, registrar: quem a importa, símbolos usados
    pelo caminho dedicado, se gera dados/runtime, estado (manter, mover, remover,
    investigar) e evidência em arquivo/linha.
- [ ] Separar dependência de build, dependência de teste e dependência de runtime;
    não tratar todas como custo do binário final, mas retirar as três do workflow
    de servidor quando forem exclusivamente client.
- [ ] Confirmar quais recursos vanilla, dados de mundo, comandos e operações de
    rede o engine usa. Marcar anticheat, chat filter, mod loaders, plugin host,
    telemetria e presentation como extras fora do engine vanilla inicial.

- [ ] **Aceite:** nenhuma remoção planejada depende de uma suposição baseada no nome
    do crate; cada linha do inventário aponta para código ou manifesto verificável.

### Extrair o engine dedicado sem reescrever a simulação

- [ ] Criar um pacote de engine server-owned e transferir a entrada dedicada,
    servidor de rede vanilla e o estado necessário ao tick/save.
- [ ] Mover os módulos/tipos necessários preservando suas implementações Rust;
    adaptar imports e ownership somente onde a fronteira client os impõe.
- [ ] Não carregar `Game` de apresentação, janela/input, telas, fontes, áudio ou
    renderizador. O binário dedicado deve iniciar o runtime server diretamente.
- [ ] Não integrar o ABI/loader `.so` nesta etapa. O engine vanilla deve iniciar
    sem plugin host e sem comandos de gerenciamento de plugins.
- [ ] Não usar `cargo build/check -p horizon-client` como atalho de validação. O
    material antigo pode ser consultado e editado, mas o primeiro build do engine
    deve selecionar apenas o novo pacote server-owned e sua árvore server.

- [ ] **Aceite:** novo binário server roda com recursos preparados, abre mundos e
    aceita conexões; `cargo tree` já não parte do pacote `horizon-client`.

### Migrar o código server que está em módulos mistos

- [ ] Separar estado de simulação/persistência/rede de estruturas de apresentação.
- [ ] Mover código server necessário para crates com fronteira server/core clara;
    nomes antigos podem ser renomeados quando a separação estiver funcional.
- [ ] Deixar código compartilhado somente quando ele representar uma abstração
    necessária ao servidor. Nenhuma API será mantida para o client consumir.
- [ ] Desacoplar tipos de input/render (`wgpu`, `winit`, `glam` visual, fontes,
    HUD, atlas) sem substituir lógica server por stubs usados em runtime.
- [ ] Remover `allow(dead_code)`/stubs condicionais da antiga estratégia de compilar
    client no binário dedicado, assim que as referências deixarem de existir.

- [ ] **Aceite:** engine server constrói sem crates de apresentação/input, e o teste
    de inicialização e as verificações server relevantes passam.

### Remover runtimes e extras do marco vanilla

- [ ] Remover do engine a API de trait mods, `horizon-mod-api-gen`,
    `horizon-server-api` e referências a `native/`/`native-hooks.json` quando não
    forem usados pelo vanilla.
- [ ] Remover do primeiro engine os extras Borizon anticheat e chat filter.
- [ ] Não migrar nem integrar `.so`, API de trait mods, SDK ou loader nesta branch; manter na `main`/histórico, sem etapa de reintegração planejada.
- [ ] Preservar datapack vanilla, recursos e exporters que a simulação/preparer
    comprovadamente precisa; remover carregamento de mods executáveis e conteúdo
    client.
- [ ] Confirmar referências no workspace, build scripts, datasets, workflows e
    documentação antes de excluir cada diretório.

- [ ] **Aceite:** o servidor carrega os dados vanilla necessários; não compila nem
    inicia trait mods, plugin `.so`, anticheat ou chat filter nesta etapa.

### Reduzir workspace, dependências e Actions

- [ ] Remover membros de workspace exclusivamente client e dependências sem caminho
    para os alvos server finais; regenerar `Cargo.lock` apenas com a toolchain
    fixada pelo projeto.
- [ ] Tirar `wgpu`, `winit`, backends gráficos, áudio, input, UI e crates transitivos
    exclusivamente ligados a eles do grafo server. Verificar por `cargo tree`,
    incluindo `--target x86_64-unknown-linux-gnu`.
- [ ] Remover workflows de launcher, binário client, bots, mods antigos e testes
    visuais. Manter CI de engine vanilla, CLI e preparer. não incluir CI de plugins/mods nesta branch.
- [ ] Release constrói apenas os artefatos que mudaram e são distribuídos: engine,
    server CLI e preparer; nenhum build/release do client ou de `horizon-bots`.
- [ ] Atualizar o workflow para empacotar dependências/exporters de `horizon-prepare`
    e validar a extração e inicialização do próprio arquivo produzido.

- [ ] **Aceite:** todos os jobs obrigatórios pertencem ao engine vanilla, CLI e
    preparer; o grafo não contém renderer/input/GPU nem plugin host; o pacote
    publicado passa o smoke test em pasta vazia sem loader.

### Limpar o produto e encerrar a migração

- [ ] Excluir do branch os diretórios e docs exclusivamente client identificados,
    sem apagar referências históricas necessárias para a licença/proveniência.
- [ ] Reescrever README e índice de docs para descrever Borizon como server-only.
- [ ] Remover instruções de build, distribuição ou suporte de client.
- [ ] Integrar a branch em `main` somente depois dos critérios finais e preservar um
    tag/commit de migração para rollback.

- [ ] **Aceite:** a árvore atual da branch vanilla só contém engine vanilla, dados
    upstream necessários, CLI/preparer e documentação server. SDK/loader ainda não
    foi reintegrado.

### Fora do escopo: mods e plugins

Não migrar nem integrar `server-plugins/api`, `sdk`, `loader`, API antiga de trait
mods ou qualquer sistema de plugins. Esses componentes permanecem na `main` e no
histórico e não são critério de conclusão desta branch.

## Regras de execução

- Não preservar client por compatibilidade ou para que o client ainda compile.
- Não compilar/testar o binário client, nem usar `horizon-client` como alvo
  transitório de build. Antes do novo engine estar isolado, usar análise estática
  e o release existente como referência de comportamento.
- Não copiar apenas diretórios chamados `server`; simulação, mundo e protocolo
  podem estar em crates com nomes neutros ou `client` por legado.
- Não apagar um módulo misto antes de mover o código server usado e localizar
  todas as referências de build/runtime.
- Não reimplementar mecânicas vanilla que já funcionam só para renomear crates.
- Preservar o comportamento vanilla; não introduzir funcionalidades client como parte da migração.
- Não deixar stubs de client compilando no engine server para silenciar erros.
- Não incluir builds locais de release, build do client ou Actions de client.
- Cada etapa deve registrar diff, crates removidos/movidos, grafo comparado,
  duração/RAM observados e checagem server que foi executada.
- Corrigir erros antes de avançar para a próxima etapa; não publicar release
  parcial ou branch de migração quebrada como `main`.

## Critérios para concluir o marco vanilla

- [ ] O engine é um pacote server-owned, sem binário em `crates/horizon-client`.
- [ ] Builds server-only não compilam launcher, renderer, input, janela, áudio,
  UI, GPU, narrador nem runtimes de mod client.
- [ ] Crates comuns que permanecem têm uso server documentado e não são mantidos
  por compatibilidade de client.
- [ ] O engine não carrega plugin `.so`, API antiga de mods, anticheat ou chat filter.
- [ ] `horizon-server` inicia uma instalação nova; `horizon-prepare` consegue
  preparar os arquivos locais com o pacote publicado sem fontes omitidas.
- [ ] O smoke test do arquivo de release chega a `Done` e encerra de forma limpa.
- [ ] CI e release só constroem e publicam componentes server-side.
- [ ] O grafo do engine não contém `wgpu`, `winit`, renderer ou input client; a
  checagem é automatizada para evitar regressão.
- [ ] README, docs e workflows descrevem exclusivamente o projeto server-side.

Plugins, SDKs e loaders `.so` não fazem parte do escopo nem dos critérios de conclusão desta branch.

## Referências locais

- [Escopo server-side anterior](server-side-scope.md): histórico e decisões da
  primeira redução do alvo dedicado. Este roadmap passa a ser o plano da
  migração integral; revalidar as anotações antigas antes de usá-las como estado
  atual.
- [SDK/loader de plugins](../server-plugins/README.md)
- [Workflow de CI](../.github/workflows/ci.yml)
- [Workflow de release server](../.github/workflows/server-release.yml)
- [Cargo workspace](../Cargo.toml)


## Continuação da sessão de 2026-11-24 (registro incremental)

| Estado | Alteração / evidência | Próximo passo |
|---|---|---|
| Validado | Removidos do `horizon-protocol` o canal de comandos silenciosos `horizon:quiet_commands` (payload de cliente mod), protocolo custom de attachments/menu e sentinel/codec de partícula custom. Preservados codecs vanilla, packet vanilla de login-plugin-query e compatibilidade/renumeração entre versões. Referências aos módulos removidos não aparecem nos crates consumidores. | Continuar auditoria de identificadores/canais residuais; não remover tráfego vanilla apenas por conter a palavra histórica “plugin”. |
| Validado | No `horizon-world`, removidos `LevelAccess::allow_piston_move` e `is_client_side`, replay/tick client de eventos e block entities de pistão, atraso visual `death_ticks` e modo client do probe. Simplificado `piston_move_blocks` para o resultado de execução do servidor; mantidos verificação vanilla de sinal, resolução, movimento, `should_tick_blocks_at` e `is_ticking_chunk`. | Remover callbacks extensíveis restantes de fluido/ticks, growth, fade, cauldrons, redstone, placement e shapes, com callsites vanilla diretos e cobertura adequada. |
| Validado | Removido fallback de física aérea exclusivo de cliente: em `living_air`, o servidor aplica gravidade sem consultar client/chunk/min-height. Em honey landing, removido o teste de client-side e mantido o broadcast de evento do servidor. Eliminado o modo `piston_event` client do fixture de block-update. | Revisar outras fronteiras misturadas e garantir que os probes compilem no workspace server-only. |
| Validação anterior | `cargo check --tests --examples -p horizon-world --locked` passou; `cargo test -p horizon-world --locked` passou com 478 testes, 0 falhas e 4 ignorados. Nenhum client foi compilado. | Repetir validações após os próximos cortes e executar as verificações finais de workspace antes do commit/push. |
| Validado | Removidos `LevelAccess::count_entities_at` e `has_mod_block_entity` (sem callsites), registry dinâmico `ModFluid`/`horizon-mod-fluids.json`, fluid tags/imersão/current de mods, buckets e liquid blocks custom, ajustes de redstone para blocos custom, o avaliador público de condições NeoForge sem callers, a classe/placement/shape `PipeBlock` e `modify_fall_damage` para blocos custom. Mantidos água/lava, tags vanilla, worldgen de springs, buckets vanilla, redstone vanilla e geometria/teste da chorus plant. | Continuar auditoria de APIs e resíduos mod/plugin em outros módulos e manifests; validar árvore total e critérios de dependências server-only. |
| Validação | `cargo fmt --all`; `cargo check --tests --examples -p horizon-world --locked`; `cargo test -p horizon-world --locked` (573 passaram, 0 falhas, 6 ignorados em todos os bins de teste). `cargo tree --workspace --locked -e normal,build` não mostrou client/mod/plugin/render/GPU. `git diff --check` passou. Nenhum client foi compilado. | Rodar suíte e árvore completas após auditoria adicional; concluir roadmap, revisar diff e, somente no fim da janela, commit/push para `origin/BorizonV26.3`. |

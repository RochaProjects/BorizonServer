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
O primeiro build desta branch ocorrerá quando existir um pacote server-owned que
não dependa de `horizon-client`, renderer, input ou outras dependências client.

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
| Branch e baseline | [ ] | [ ] | [x] | Migração e baseline de build ainda não iniciados. Cópia local de referência da `main` em `/home/ubuntu/BorizonServer-main`, commit `f2e5556cbc`; branch de trabalho `BorizonV26.3` mantida separada. | Registrar a origem da branch; capturar baseline reproduzível de grafo, Actions e instalação limpa; conferir o contrato de empacotamento do preparer. |
| Inventário do uso server real | [ ] | [ ] | [x] | — | Registrar imports/símbolos e evidências por pacote, separando build/test/runtime; completar auditoria dos módulos mistos. |
| Engine server-owned | [ ] | [ ] | [x] | — | Criar e compilar pacote próprio sem `horizon-client`/renderer/input; provar startup com recursos preparados, mundo e conexões. |
| Migração dos módulos mistos | [ ] | [ ] | [x] | — | Migrar estado e operações reais de tick, rede e save; remover dependências client sem stubs; executar testes server. |
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
| — | — | — | — | [ ] | [ ] | [ ] |

## Diário de execução

| Data | Atividade/evidência | Estado | Próximo passo |
|---|---|---|---|
| 2026-11-24 | Clonada a `main` separadamente em `/home/ubuntu/BorizonServer-main`, commit `f2e5556cbc`; checkout de trabalho segue em `BorizonV26.3`. Nenhuma alteração de código server foi migrada ou validada ainda. | Em andamento (preparação) | Conferir a origem da branch, inventariar o código real da `main` e medir o baseline permitido sem compilar o client. |

## Ponto de partida verificado

Retrato da `main` em 2026-11-24, commit `1608bd3d5e4ac60f24e2e8923b51e31454b30316`:

- `horizon-server` é o CLI/supervisor que inicializa e administra o servidor.
- `horizon-prepare` importa os arquivos locais de Minecraft e prepara os dados
  usados pelo engine.
- O executável entregue como `horizon-engine` ainda é compilado pelo pacote
  `crates/horizon-client`, com `--no-default-features` e as features
  `dedicated-server,server-plugins`.
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
  --features dedicated-server,server-plugins
```

Essa saída é o baseline da migração. Medir novamente após cada corte e comparar
crates, dependências nativas, duração de build e memória do runner. Não estimar
ganhos sem uma medição reproduzível.

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
| `horizon-sim-data` | MANTER/MOVER módulos server-safe | Extrair apenas tipos/dados usados por tick, chunks, colisões, entidades e worldgen; a finalidade final é server-owned, sem promessa de compartilhamento com client. |
| `horizon-system` | ADIAR/condicional à CLI | Informações do processo/máquina. Não é simulação; manter apenas se status/diagnóstico da ferramenta de hospedagem precisar. |
| `horizon-telemetry` | REMOVER | Consentimento/outbox/upload de telemetria do client. |
| `horizon-theme` | REMOVER | Fontes, ícones e design system de UI; não manter por relatórios visuais da CLI sem decisão explícita. |
| `horizon-threads` | MANTER e auditar deps de plataforma | Runtime de threads/allocator usado pelo engine; remover somente as partes client-specific, mantendo scheduling e alocador requeridos. |
| `horizon-world` | MANTER | NBT, regiões, chunks e armazenamento vanilla do mundo. |
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

- [ ] Registrar e verificar a relação de origem entre `BorizonV26.3` e a `main`; a cópia local de referência está em `/home/ubuntu/BorizonServer-main` (commit `f2e5556cbc`).
- [ ] Guardar no roadmap a saída do grafo server atual, o tempo/RAM do build do
    workflow e o log de uma instalação limpa do release. Usar dados dos Actions e
    do release já existente; não rebuildar `horizon-client` para criar baseline.
- [ ] Corrigir o release/preparação até uma pasta vazia conseguir preparar recursos
    usando somente o arquivo publicado e os arquivos locais Minecraft do operador.

- [ ] **Aceite:** existe uma branch isolada, baseline repetível e um teste de release
    que detecta se o pacote depende de fontes/arquivos omitidos.

### Inventário do uso server real

- [ ] Gerar `cargo metadata` e `cargo tree` para o engine dedicado, `horizon-server`
    e `horizon-prepare`. O SDK/loader de plugins está fora deste marco.
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
- [ ] Não migrar nem integrar `.so` agora. Preservar código atual em `main` e
    planejar sua volta numa etapa independente depois da validação vanilla.
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
    visuais. Manter CI de engine vanilla, CLI e preparer. CI do plugin fica para
    a etapa futura de reintegração.
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

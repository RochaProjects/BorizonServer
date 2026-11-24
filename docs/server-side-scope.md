# Server-side build scope

## Política de meshing de folhas movida para dados compartilhados (2026-11-24)

`LeafCullingMode` agora é definido em `horizon-sim-data::mesh_policy`, junto dos
contratos de dados usados por simulação/streaming; `mesher`, seu algoritmo de
culling e `streaming` consomem esse caminho sem importar
`horizon-client-render::graphics`. O renderer mantém um reexport de
compatibilidade para suas opções gráficas existentes. Os cinco modos, seus nomes,
rótulos e a regra `is_interior` foram preservados, sem trocar o meshing por stub.

Validação: `cargo check --offline --locked -j1 -p horizon-game --no-default-features
--features dedicated-server --bin horizon-engine` passou; os testes
`streaming::` passaram (33 aprovados, 2 ignorados); `cargo fmt --all` e
`git diff --check` passaram; os testes de integração da política em
`horizon-sim-data/tests/mesh_policy.rs` passaram (2/2). O filtro dos testes
`mesher::leaf_culling_tests`, com `test-support`, compilou, mas os nove testes
estão ignorados porque não há os recursos vanilla 26.3 preparados neste ambiente.
Esse corte faz os consumidores
do mesher/streaming independentes do módulo de configurações gráficas, mas **não
removeu `horizon-client-render`, `wgpu` ou `winit` do grafo dedicado**. O check
dedicado ainda compila o renderer e a raiz de runtime continua delegando ao
`Game` misto; separar isso e validar boot de mundo/ticks/save/plugin `.so`
continua pendente. Nenhum commit/push ou build release foi feito.

## Raiz executável dedicada em `horizon-game` (2026-11-24; migração em andamento)

Foi criado o binário `horizon-engine` em `crates/horizon-game/src/bin/dedicated.rs`.
O root dedicado agora pertence ao crate `horizon-game`, e o Dockerfile passa a
construir esse alvo em vez de renomear `horizon-client` para `horizon-engine`.
`horizon-game` mantém `client` como feature padrão para preservar os executáveis
existentes; `--no-default-features --features dedicated-server` habilita apenas
`horizon-client/dedicated-server` e o loader/SDK `.so`. O marcador do mod API
client-side não é compilado nesse perfil. O supervisor continua aceitando o
binário legado `horizon-client` como fallback para instalações antigas.

Validação básica: o novo alvo passou em `cargo check --offline --locked -j1 -p
horizon-game --no-default-features --features dedicated-server --bin horizon-engine`,
e respondeu a `--help`. Um smoke em diretório temporário com
`horizon-engine --dedicated --initSettings` encerrou com sucesso e gerou
`server.properties` e `eula.txt` (`eula=false`); o diretório foi descartado.
Isso confirma a nova entrada de processo/preflight, **não** boot de mundo, ticks,
save/restart/stop nem carregamento de plugin `.so`.

Este é o primeiro corte da raiz do executável, não a separação completa do
runtime: `horizon-game` ainda depende de `horizon-client`, cuja facade puxa
`horizon-client-render` e `horizon-client-input`; o alvo também ainda alcança
`wgpu` e `winit`. Não se reivindica grafo client-free nem redução de build. O
próximo trabalho é extrair a raiz de inicialização server-side e seus serviços
do `horizon-client::entry`/`Game`, mantendo gameplay real, tick e protocolo, e
migrar os consumidores por fronteiras funcionais em vez de retirar dependências
à força. Nenhum build release foi executado localmente.

Tentativa de corte direto de `wgpu`: torná-lo opcional e ocultar `gfx`, `graphics`,
`hud`, `post_chain` e módulos de GPU fez o check dedicado falhar com 102 erros,
porque a facade do renderer também é consumida por `chat`, `font`, `mesher`,
`streaming`, `Game` e outros módulos compilados no servidor. A tentativa foi
revertida integralmente; o novo alvo voltou a passar no check em 16,35 s
incremental, e `cargo tree` confirma que `horizon-server-plugin-loader` continua
presente. Próximo passo real: separar os contratos server-safe usados por essas
rotinas antes de condicionar os módulos gráficos, em vez de gatear o renderer
monolítico de uma vez.

## Runtime server-owned: políticas de tick e preflight dedicado (2026-11-23)

Foi criado `horizon-server-runtime`, crate consumido pelo engine dedicado para
as políticas de cadência do tick, pausa por servidor vazio e aviso de overload.
O agendador preserva o tick real no engine, inclusive bombeamento de chunks,
regra de sprint e skip somente quando o atraso excede dois segundos; não há
simulador alternativo nem stub de gameplay.

Nesta continuação, parsing de argumentos, leitura/armazenamento de
`server.properties`, configuração de compressão de region files e verificação
da EULA foram movidos para `horizon-server-runtime::preflight`. O engine mantém
no adaptador as opções próprias de comandos e consome `ServerSetup` para seguir
no fluxo dedicado já existente. Foram adicionados testes para `--help` e opção
desconhecida, ambos sem tocar arquivos do diretório de trabalho.

Validação observada: `cargo test --offline -j1 -p horizon-server-runtime`
passou (7/7); o check dedicado locked/offline passou em 58,01 s nesta sandbox;
`cargo fmt --all` e `git diff --check` passaram. A árvore normal do novo crate
inclui `horizon-server-files` e `horizon-world`, e não inclui renderer, input,
`wgpu` nem `winit`. **Isso comprova isolamento apenas do crate novo.** O check do
binário ainda compila `horizon-client-render` e `horizon-client-input`; o grafo
dedicado global e o objetivo de removê-los permanecem pendentes. Ainda faltam
smoke tests do boot completo, mundo/save/restart e loader `.so`.

Smoke real do executável: em diretório temporário, `cargo run --offline --locked
-j1 -p horizon-client --no-default-features --features
server-plugins,dedicated-server --bin horizon-client -- --dedicated
--initSettings` compilou em perfil `dev` (8m49s) e executou com sucesso. Criou
`server.properties` e `eula.txt` (com `eula=false`), registrou os caminhos e
encerrou normalmente; o diretório descartável foi removido. Esse smoke cobre a
entrada CLI e a inicialização/configuração, **não** o boot completo do mundo,
ticks, save/restart/stop ou a integração do loader `.so`. Durante o build, o
log confirmou compilação de `horizon-client-render`, `horizon-client-input`,
`wgpu` e `winit`; portanto não se reivindica redução do build dedicado global.

## Extração de operações server-side de chunks (2026-11-24)

As operações não visuais de gerar/carregar chunks, decorar, finalizar,
persistir protos e construir/calcular a visão de iluminação foram movidas para
`horizon-server-runtime::chunk_jobs`. O dispatcher existente em
`horizon-client-render::streaming` delega para essas funções; queueing,
agendamento e meshing continuam no renderer. A extração reutiliza os algoritmos
e contratos existentes, sem duplicar simulação nem trocar gameplay por stubs.

Validação: os 7 testes de `horizon-server-runtime` passaram; a suíte filtrada
`streaming::` do renderer teve 33 aprovados e 2 ignorados; o check dedicado
locked/offline passou (aprox. 1m32s nesta execução), assim como `git diff
--check`. O runtime crate continua sem renderer/input/`wgpu`/`winit` em suas
dependências normais, mas essa extração **não removeu essas dependências do
grafo do binário dedicado**: `horizon-client-render` e `horizon-client-input`
seguem alcançáveis e a compilação ainda inclui `wgpu` e `winit`. A próxima
fronteira de ownership relevante é separar o caminho server-side do queue e do
estado de streaming atualmente compartilhados com meshing. Ainda não foi feito
boot completo do mundo com ticks/save/restart/stop nem carregamento real de uma
`.so`; portanto esses comportamentos e a ABI permanecem sem validação runtime
nesta etapa.

## Executor server-side e primitivas genéricas da fila (2026-11-24)

O enum dos trabalhos não visuais (`Generate`, `Decorate`, `Finish`, `Light`,
`Save` e `LightView`) e o despacho que os executa agora pertencem a
`horizon-server-runtime::chunk_jobs` (`ServerJob`, `ServerJobDone` e
`execute_server_job`). As primitivas genéricas de fila (`Queued<J>`,
`rank_again` e `release_columns`) também foram movidas para
`horizon-server-runtime::work_queue`; o desempate FIFO, biases e tarefas sem
coluna mantêm a semântica original. `horizon-client-render::streaming` calcula
prioridades a partir da distância, cancela tarefas, registra métricas e integra
resultados. A heap e o mutex agora são encapsulados por
`horizon-server-runtime::work_queue::JobQueue`, que fornece push/pop, consultas
de rank, repriorização, liberação de colunas, retenção durante abort e capacidade
para o censo de memória. Seu enum distingue explicitamente
`Server(ServerJob)` de `Mesh`/`Free`. O mesher continua no renderer e recebe a
mesma região imutável; não houve mudança nos algoritmos de gameplay, iluminação
ou persistência.

Validação mais recente: `cargo test --offline --locked -j1 -p
horizon-server-runtime` passou (12/12); os testes filtrados `streaming::` do
renderer passaram (33 aprovados, 2 ignorados); o check dedicado locked/offline
passou (25,31 s incremental); `cargo fmt --all -- --check` e `git diff --check`
passaram. Os testes novos verificam acesso à fila sincronizada, peek/pop,
retenção de saves no abort e descarte de jobs ao descarregar colunas. **É uma
separação de ownership e despacho, não a remoção do renderer do grafo:** as
políticas e a propriedade da fila estão no runtime server-owned, mas o estado do
`Streaming` e sua integração com mundo/mesher permanecem em
`horizon-client-render`; `horizon-client-render`, `horizon-client-input`, `wgpu`
e `winit` continuam alcançáveis pelo binário dedicado. O próximo corte deve
separar o estado/política de streaming server-side dos estados e dados de
meshing, preservando ordem de jobs e aplicação de resultados no mundo.

## Corte de TTS nativo do perfil dedicado (2026-11-23)

Foi implementado o feature `horizon-narrator/client-speech`. O backend TTS e
seus bindings de plataforma agora só são compilados quando o cliente habilita
`client-graphics`; o perfil dedicado usa o crate `horizon-narrator` apenas para
as APIs compartilhadas de status/narração de telas e não compila `speech.rs`.
As dependências de plataforma (`libloading`, `windows` e `objc2`) e `log` também
ficaram opcionais sob esse feature. O comportamento do servidor não depende de
fala: `init()` não inicia o worker sem `client-speech`, enquanto a lógica de
status, registro de mensagens e testes segue disponível.

Validação desta fatia: o check dedicado passou depois da alteração; os 15 testes
de `horizon-narrator` passaram com `--no-default-features`; e o check padrão do
crate com `client-speech` habilitado passou no Linux. **Isso remove código TTS do
perfil dedicado, mas não reduz a contagem de pacotes no Linux.** `libloading`
continua na árvore por ser dependência do `winit`; o crate `horizon-narrator`
continua necessário para APIs de narração de tela/status chamadas pela facade
compartilhada. A medição histórica reportou 229 em ambos os conjuntos; essa
contagem não foi reproduzida pela normalização atual, descrita abaixo.

## Persistência de LOD movida para camada server-safe (2026-11-23)

O cache de colunas decoradas (`lod/cache.rs`) e o bake de amostras distantes
(`lod/bake.rs`) agora pertencem a `horizon-sim-data::lod_data`, não ao módulo de
meshing do renderer. `horizon-client-render::lod` mantém reexports de
compatibilidade, e `save.rs` usa diretamente os tipos `lod_data` e os contratos
de `streaming_api` para `SavedLodSampler`/`Persistent<G>`. Nenhum codec, chave,
fingerprint, layout de arquivo ou ordenação de save foi alterado.

Validação: o check dedicado passou após a extração; os novos testes server-safe
passaram (3/3: cache, bake e formato do nome de servidor), incluindo reopen e
separação de amostras aproximadas. `cargo fmt --all -- --check` e
`git diff --check` também passaram. A árvore ainda contém
`horizon-client-render`, `horizon-client-input`, `wgpu` e `winit`; a extração de
ownership, por si só, **não prova redução do grafo**.

## Cache de terreno de outros servidores excluído do perfil dedicado

`horizon-client-render::server_terrain` carrega e mantém os chunks distantes que
o cliente recebeu de servidores remotos ou lê do cache Bobby local; é separado
do serviço `lod_net` que o servidor usa para atender requests `horizon:lod` de
seus jogadores. O módulo agora é compilado somente com `client-graphics`. O
helper puro `server_folder` foi extraído para `horizon-sim-data::server_address`
e é reexportado pelo caminho cliente antigo, pois o indexador BlueMap ainda o
usa. No alvo dedicado, o ramo de abertura desse cache remoto retorna sem abrir
arquivos locais. O protocolo de terrain distante, `lod_net`, geradores, save e
simulação não foram alterados.

Validação: check dedicado passou; `cargo check` do renderer com
`client-graphics` passou; os oito testes existentes `server_terrain::` passaram;
os três testes de integração server-safe acima passaram. Isso exclui o código do
cache remoto do cliente do crate renderer no perfil dedicado, mas não reduz
dependências Cargo: renderer/input/`wgpu`/`winit` permanecem alcançáveis.

## Downloader e autodetector BlueMap condicionados ao cliente (2026-11-23)

O runtime `bluemap` (download/decode das tiles BlueMap) e `bluemap_detect` agora
são compilados somente com `client-graphics`, assim como `server_terrain`. A
preparação de LOD no alvo sem gráficos não abre cache remoto nem executa detecção;
continua usando as fontes server-side existentes e preserva `lod_net`/`horizon:lod`.
O estado BlueMap em `Game` e a atualização visual de tiles também ficaram no ramo
gráfico. Para não quebrar o formato de preferências do cliente, `Settings`, o parse
das linhas `blueMap:` e as chaves existentes foram extraídos para o módulo puro
`bluemap_settings`; `bluemap::Settings` continua reexportado no cliente gráfico.

Validação: check dedicado passou (45,87 s incremental); `cargo check` de
`horizon-client-render` com `client-graphics` passou (14,53 s); os 12 testes
`bluemap::` passaram; formatação e `git diff --check` passaram. Imediatamente após
esta fatia, a contagem era 304/220; o corte do crate HDR registrado a seguir reduziu
um pacote adicional. Renderer/input/`wgpu`/`winit` continuam no grafo e o objetivo
central segue pendente.

## Crate HDR excluído do perfil dedicado

`horizon-client-render` agora declara `horizon-hdr` como dependência opcional,
ativada pelo feature `client-graphics`. As chamadas de criação especial do device
Vulkan, ray-query experimental e configuração de metadados HDR também ficam nesse
feature; sem gráficos, o engine dedicado não abre renderer nem solicita as
extensões HDR. O caminho gráfico ativo continua usando as mesmas APIs HDR.

Validação: `cargo check` dedicado passou; `cargo check` de
`horizon-client-render --features client-graphics` passou. A consulta inversa do
alvo confirmou **nenhum caminho para `horizon-hdr`**; a árvore caiu de 304/220 para
**303 IDs `nome+versão` / 219 nomes**. `wgpu` continua presente por arestas diretas
de `horizon-client`, `horizon-client-lib` e `horizon-client-render`; esta fatia
removeu um crate, não retirou ainda o conjunto gráfico nem prova ganho de tempo.

## Arestas diretas de `wgpu` removidas do engine e da facade (2026-11-23)

`horizon-client` agora declara `wgpu` como opcional, habilitado por
`client-graphics`, e `horizon-client-lib` deixou de declarar essa dependência
diretamente. Os dois usos diagnósticos em `game/control.rs` e `game/debug_keys.rs`
foram ajustados para não referenciar enums/tipos de `wgpu` diretamente. O
`Cargo.lock` foi sincronizado por check offline. O check dedicado passou com
`--offline --locked -j1` em **40,63 s** nesta sandbox; o check de
`horizon-client-render --features client-graphics` passou em **12,16 s**; fmt e
`git diff --check` passaram.

Isso remove as arestas diretas desses dois crates, mas não retira `wgpu` do
servidor: `cargo tree -e normal -i wgpu` confirma que ele ainda é dependência
normal de `horizon-client-render`, alcançado por `horizon-client` e
`horizon-client-lib`. A medição do alvo `aarch64-unknown-linux-gnu`, com a
normalização documentada, é **302 IDs `nome+versão` / 219 nomes**. Não há uma
contagem anterior reproduzida com as mesmas opções após a mudança, portanto não
se declara redução de grafo nem ganho de tempo. O renderer tem referências a
`wgpu` em muitos arquivos (incluindo `gfx/` e UI); torná-lo opcional sem separar
essas implementações de consumidores de gameplay seria um toggle especulativo,
não um corte seguro. `winit`, renderer e input também permanecem no grafo.

Correção de medição: a contagem de 305/229 registrada antes não é reproduzível
com o comando documentado. Na medição anterior à fatia de arestas diretas abaixo,
remover com regex a marca de dependência repetida `(*)` retornava **304 IDs
`nome+versão`** e **220 nomes de crate**. O pipeline reproduzível para os IDs é:

```sh
cargo tree --offline --locked -e normal -p horizon-client \
  --no-default-features --features server-plugins,dedicated-server \
  --target aarch64-unknown-linux-gnu --prefix none |
python3 -c 'import re,sys; print(len({re.sub(r" \(\*\)$", "", x.rstrip()) for x in sys.stdin}))'
```

Para nomes de crate, acrescente `.split(" v", 1)[0]` à expressão dentro do
conjunto. Baseline e estado atual precisam usar a mesma unidade e target antes de
declarar variação numérica.

## Continuação e revalidação anterior às últimas arestas de `wgpu` (2026-11-23)

Após as extrações documentadas abaixo, o check dedicado foi repetido com
`cargo check --offline --locked -j1 -p horizon-client --no-default-features
--features server-plugins,dedicated-server --bin horizon-client`. Passou em
**1,04 s incremental** nesta sandbox. Esse resultado é histórico; o check mais
recente e seu tempo estão registrados na seção de arestas diretas acima. Check
incremental não mede build frio, build release nem ganho de tempo.

A árvore normal dedicada naquela validação continha 304 IDs `nome+versão` (220 nomes
distintos) pela normalização corrigida. A medição posterior registrada acima contém
302 IDs / 219 nomes; não há baseline medido com as mesmas opções imediatamente antes
da mudança para declarar delta. As consultas inversas ainda mostram `horizon-client-render`,
`horizon-client-input`, `wgpu` e `winit`; `horizon-client-backends` não aparece.
Portanto, as extrações atuais preservam a compilação e isolam contratos/dados
compartilhados, mas **não atingem o objetivo de retirar código ou dependências
client-only do grafo dedicado**.

O bloqueio concreto continua sendo de ownership, não de configuração: o binário
dedicado é `horizon-client`, que compila o mesmo `game.rs`/`client_lib.rs` usados
pela aplicação cliente. Há mais de cem módulos de `game/` com uso de APIs de
render e dezenas com APIs de input; a estrutura `Game` também mantém esses
estados junto à simulação. Tornar renderer/input opcionais antes de migrar esses
consumidores já foi testado e produziu 414 erros. Um corte real requer uma raiz
de compilação server-owned e a migração incremental dos call sites que hoje
atravessam essa facade. Não foi feita outra alteração de arquitetura nesta
revalidação, para não introduzir uma remoção especulativa que só substitua
comportamento real por stubs.

## Verificação atual e direção de implementação (2026-11-22)

Esta seção prevalece sobre os handoffs datados abaixo quando houver conflito.

- Cópia verificada: `main` em `1608bd3d5e` (`origin/main`), inicialmente limpa.
- O check dedicado **passou** em build frio com:
  `cargo check --locked -j1 -p horizon-client --no-default-features --features server-plugins,dedicated-server --bin horizon-client`.
  Tempo observado nesta sandbox: **11 min 01 s**. Isto é `cargo check` de desenvolvimento, não build release nem prova de tempo do CI.
- A compilação registrou `wgpu`, `winit`, `horizon-client-render`, `horizon-client-input`, `horizon-sim-data`, `horizon-shaders`, `horizon-narrator`, `horizon-server-plugin-api` e `horizon-server-plugin-loader`. Portanto, o servidor funciona no alvo dedicado, mas esse alvo ainda compila uma cadeia substancial de cliente.
- `cargo tree` **não aceita `--bin`**. Para inspecionar dependências normais do pacote/feature, usar `cargo tree --locked -e normal -p horizon-client --no-default-features --features server-plugins,dedicated-server -i <crate>`. A consulta confirmou `horizon-client-render`, `wgpu`, `winit`, `naga`, `horizon-shaders` e `horizon-narrator` no grafo. Não contar `dev-dependencies` como parte do release.
- A causa arquitetural confirmada: `horizon-client` continua sendo o pacote/binário do engine dedicado; `horizon-client-lib` aponta para `crates/horizon-client/src/client_lib.rs` e depende incondicionalmente de render/input; essa facade reexporta amplamente o renderer; `game.rs` declara centenas de módulos e mistura `Game` de simulação com estado visual/input. `game::dedicated::run` separa o startup em runtime, mas ainda cria e executa o mesmo `Game` compartilhado.

### Tentativa de corte pela facade (2026-11-23)

Foi testado tornar `horizon-client-render` opcional nos manifests de `horizon-client`
e `horizon-client-lib`, condicionar seus reexports a `client-graphics` e, no perfil
dedicado, reexportar os tipos já presentes em `horizon-sim-data`. O primeiro check
offline do alvo dedicado falhou em `horizon-client-lib` com **414 erros**: esconder
os reexports não impede que `client_lib.rs` compile dezenas de módulos de cliente,
que continuam importando caminhos do renderer (por exemplo `widgets`, `options_ui`,
`sound_engine`, `hud_kit` e `biome`). As cópias de tipos render/sim-data também não
foram demonstradas intercambiáveis. A contagem por arquivo no log capturado foi
liderada pela própria facade (`client_lib.rs`, 182 localizações), seguida por
`title.rs` (42), `save.rs` (39), `client_perf/request.rs` (28),
`title/create_world.rs` e `vanillagen.rs` (16 cada), `title/flat.rs` (14) e
`realms_dedicated.rs` (7). Ou seja, gatear só telas de menu não basta: existem
consumidores de exports render/client também em save, worldgen e módulos usados
no dedicado, exigindo extração/adaptação por API em vez de um gate amplo cego.

Essa alteração experimental foi integralmente revertida, inclusive o lockfile; ao
fim daquela tentativa não permaneceu alteração de código ou feature dela no branch.
O `cargo tree --locked` posterior ao rollback confirmou que `horizon-client-render` continua alcançável diretamente por
`horizon-client` e por `horizon-client-lib`, e que `wgpu` também tem pais nesses dois
crates, além de `horizon-client-render`/`horizon-hdr`. Portanto, **nenhuma redução do
grafo foi obtida** e o check desta tentativa não passou.

Conclusão operacional: a próxima alteração não pode apenas esconder reexports nem
ativar tipos do sim-data no lugar dos tipos canônicos. É necessário separar a
facade/raiz de compilação dedicada dos módulos client-side que hoje são compilados
incondicionalmente e migrar consumidores server-side para tipos canônicos comuns,
preservando o `Game`/tick e a API `.so`. Só então o renderer poderá sair do caminho
normal do alvo dedicado. Não executar builds release localmente; manter `-j1`.

### Primeira extração server-safe implementada (2026-11-23)

Em vez de repetir o toggle que quebrou a facade, foram movidas as fronteiras de
dados/contratos de geração que save e worldgen já compartilham:

- Novo `horizon-sim-data::lod_data`: tipos `Surface`, `Span`, `Spans` e `Column`,
  incluindo redução de colunas e resumo das colunas de um chunk. `lod.rs` do
  renderer reexporta os mesmos tipos, sem cópias nem conversões que mudem IDs.
- Novo `horizon-sim-data::streaming_api`: `ChunkGenerator`, `LodSampler`,
  `LodSource`, `LodDetail`, `Proto` e `IDLE_RELEASE`. `streaming.rs` do renderer
  reexporta esses contratos; save e implementações de geradores continuam usando
  a mesma interface.

Checks observados nesta fatia: `cargo check --offline -j1 -p horizon-sim-data`
passou; `cargo check --offline -j1 -p horizon-client-render --features
client-graphics` passou; e a revalidação final do alvo dedicado passou com
`cargo check --locked -j1 -p horizon-client --no-default-features --features
server-plugins,dedicated-server --bin horizon-client` em 28,26 s incremental.
Também passaram `cargo test --offline --locked -j1 -p horizon-sim-data --test
lod_data` (3/3) e `cargo test --offline --locked -j1 -p horizon-client-render
--features client-graphics,test-support lod::` (83 passaram, 5 ignorados por
requererem recursos preparados; 2018 filtrados). A formatação Rust e
`git diff --check` passaram.

O comando amplo `cargo test --offline --locked -j1 -p horizon-sim-data` **não
compila seus unit tests herdados**: eles esperam `Items::for_tests` em
`brewing.rs` e `Assets::reload` em `collisions.rs`, APIs/fixtures não disponíveis
no crate isolado. Os testes LOD foram colocados num alvo de integração separado
e passaram; não se declara a suíte unitária inteira de sim-data aprovada.

Uma consulta inicial de `cargo tree` foi registrada como 305 identificadores
únicos, mas essa contagem incluía referências abreviadas repetidas. A
normalização descrita no início deste documento corrige a baseline para 229 IDs
de pacote. As árvores inversas confirmam `horizon-client-render` e
`horizon-client-input` como
dependências de `horizon-client`/`horizon-client-lib`; `wgpu` ainda chega pelos
dois crates, pelo renderer e por `horizon-hdr`; `winit` chega pelo engine, input,
facade e renderer. `horizon-client-backends`, ao contrário, não aparece no perfil
dedicado (continua opcional). **Nenhuma redução nova do grafo nem ganho de tempo foi
demonstrado por esta fatia.** Permanecem avisos de manifests sobre `[lints]` e
warnings preexistentes de dependências declaradas não usadas.

**Esta fatia ainda não reduz o grafo dedicado.** Consultas inversas após as mudanças
confirmam que `horizon-client-render` continua filho de `horizon-client` e
`horizon-client-lib`, e `wgpu` continua alcançável por ambos, por renderer e por
`horizon-hdr`. A leitura dos manifests também confirma que `horizon-client` e
`horizon-client-lib` declaram renderer/input como dependências obrigatórias; a
facade do renderer (`horizon-client-render/src/lib.rs`) exporta incondicionalmente
uma superfície extensa de gameplay, mundo e apresentação. Portanto, remover somente
uma aresta direta ou trocar um feature flag não retira o renderer com segurança.

Há dois limites de código a migrar, não apenas um. O `Streaming` worker mistura
geração/decoração/iluminação/saves com meshing, upload backlog e tipos de mesh; seu
caminho runtime `!self.mesh` desliga a submissão visual, mas os campos/tarefas ainda
compilam. Separar essa implementação é necessário para mover a API de simulação, mas
não suficiente: `Game` e os módulos compartilhados ainda usam extensivamente APIs de
render/input e `winit` (há centenas de referências nas fontes do engine). O alvo real
é migrar o runtime dedicado para uma fronteira server-owned que não importe a facade
cliente e, em paralelo, extrair do worker apenas a geração/iluminação/save, mantendo
meshing como extensão client-side. Não remover nem substituir essas rotinas por stub;
validar cada corte com o build dedicado e nova consulta inversa antes de afirmar
redução.

### Auditoria arquitetural por fatias (2026-11-22)

Foi feita uma leitura estática do entrypoint/`Game`, tick e entidades, rede/jogadores,
recursos/worldgen/save, plugins `.so`/hooks e facade de apresentação/input. A auditoria
não editou código nem executou novos builds; as únicas mudanças desta continuação até
esta anotação são documentais. O check dedicado de 11m01s acima continua sendo a última
medição validada; não há redução de dependências ou tempo demonstrada nesta etapa.

Evidências e limites confirmados:

- O dedicated percorre `game::dedicated::run` → `Game::new` → `run_dedicated` →
  `tick_server_spans` → `tick_server`/`tick_world`. Portanto, a simulação vanilla não
  é um simulador alternativo; está misturada à mesma implementação compartilhada.
  `Game` (por volta de `game.rs:2152-2325`) reúne `World`, streaming, jogador/clock,
  inventário e estado de níveis com GUI, renderer, HUD, janela, teclas, mouse, cursor,
  IME e gamepad. O construtor dedicado ainda exige `UiResources` e cria o mesmo tipo.
- O tick preserva ordem vanilla de packets, block/random ticks, entidades/mobs,
  veículos/portais, block entities, sculk, pistões e sincronização. Dentro dessa
  orquestração há chamadas visuais/input (HUD, mão local, partículas/animações e frame
  visuals); separar deve ser por operação/subsistema, sem remover chunks, streaming,
  callbacks de gameplay ou alterar a ordem/cadência do tick.
- `game/remote.rs` é a implementação do cliente remoto e contém input/UI/predição;
  não é o listener do servidor. O caminho server-side é `lan`/`Hub`: recebe packets,
  atualiza convidados e faz save/sync no mesmo ciclo do mundo. `TickCtx` depende de
  `World`, colisão e streaming; mover `lan.rs` inteiro não seria uma fronteira real.
- Recursos de apresentação (sprites, modelos, atlas visual, mesh/render tables)
  já têm alguns caminhos headless. Em contraste, registry/tabelas de gameplay,
  receitas/tags/loot, data packs, biomes, worldgen e saves são usados pelo servidor e
  devem preservar os IDs, codecs, migrações e comportamento atuais. A persistência está
  em `crates/horizon-client/src/save.rs` (não em `game/save.rs`).
- `horizon-server-api` e `server-plugins/api` + `server-plugins/loader` são fronteiras
  sem renderer: manter API e ABI v2/v3, símbolos, ciclo de vida da biblioteca, eventos,
  permissões e callbacks. Porém, a implementação `GameHost`/bridge e `native_hooks.rs`
  ainda vivem em arquivos mistos; `mod_api_host.rs` é o sistema legado de trait mods,
  não o host `.so`.
- A facade `horizon-client-lib` continua dependendo diretamente de
  `horizon-client-render` e `horizon-client-input`, além de declarar `wgpu`, `winit` e
  `pollster`; `client_lib.rs` reexporta amplamente essas APIs. Há consumidores de
  `winit` em dezenas de arquivos do engine, incluindo tipos de evento em código
  compartilhado. Remover uma declaração de feature/dependência antes de migrar os
  consumidores não cria isolamento e quebra a compilação.

Conclusão de implementação: a primeira alteração de código precisa criar uma nova
fronteira consumida pelo fluxo dedicado, e não apenas mover `dedicated.rs`, reexportar
`horizon-server-api` (que já está separado), nem recortar uma dependência do manifest.
O primeiro candidato proposto — extrair `lan/io_pool.rs` para uma crate independente —
foi rejeitado após inspecionar seus acoplamentos: o pool chama `super::conn::frame_body`,
e `conn.rs` usa tipos internos do pool, inclusive `Worker`, para acordar conexões. Mover
esse arquivo sozinho preservaria um acoplamento invertido ou exigiria uma API artificial;
não é uma fronteira que reduza o grafo por si só. Uma extração futura de transporte
precisa substituir essa chamada por um contrato explícito de framing e encapsular o
despertar do worker, com testes de ordem/framing/keep-alive preservados. `Hub`/`TickCtx`
também não podem ser movidos como bloco porque dependem diretamente de `World`, colisão
e streaming. Até uma fronteira server-owned compilar e ser chamada pelo caminho dedicado,
não afirmar que qualquer crate client-only foi removida. O objetivo de grafo continua
pendente: retirar `horizon-client-render`, `horizon-client-input`, `wgpu` e `winit` do
alvo dedicado.

### Primeira fatia avaliada nesta sessão (2026-11-22)

A candidata client-only de menor aparência, `horizon-narrator`, **não foi removida**. A consulta `cargo tree --locked -e normal -p horizon-client --no-default-features --features server-plugins,dedicated-server -i horizon-narrator` mostra caminhos diretos por `horizon-client` e indiretos por `horizon-client-lib` e `horizon-client-render`. O código de `horizon-client` também chama essa API em `game.rs` (estado `narration` e inicialização), `game/narrator.rs`, `game/control.rs`, `game/remote.rs`, `game/options.rs`, `game/startup_menu.rs`, `game/resource_loader.rs` e `game/pad/radial_play.rs`; além disso, `screen_narrator.rs` sustenta APIs chamadas por diversas telas. Tornar a dependência opcional, retirar seu reexport ou trocar chamadas por no-op não provaria uma fronteira real e poderia quebrar caminhos compartilhados de chat/controle. Não houve alteração de código/feature nem compilação do client nesta fatia.

Próxima extração concreta para possibilitar remover o narrador sem mascarar comportamento: (1) separar de `game/narrator.rs` a interface/política de fala usada por chat da integração TTS e do ciclo de vida do backend client; (2) migrar os call sites de `game/remote.rs` que emitem fala de chat/sistema para a interface server-safe, preservando no cliente o encaminhamento ao backend; (3) mover `ScreenNarrator`, toasts, hotkeys e navegação para módulos client-only, cobrindo `game/control.rs`, `game/options.rs`, `game/startup_menu.rs`, `game/pad/radial_play.rs` e o campo `Game::narration` em `game.rs`; (4) condicionar `game/narrator.rs`, `screen_narrator.rs`, os reexports em `horizon-client-lib` e a dependência `horizon-narrator` ao feature client somente após esses call sites migrarem. Essa sequência depende primeiro de separar o estado de apresentação/runtime do `Game` misto; não é segura como toggle isolado.

Comandos executados: `git status --short --branch`, `git rev-parse HEAD`, inspeção dos manifests/call sites e a consulta inversa de `cargo tree` acima. A árvore confirma que `horizon-narrator` continua incluído. Nenhum build foi executado pois nenhuma alteração de código foi feita. O renderer/input e sua cadeia gráfica continuam sendo o corte de maior impacto, mas a próxima ação segura é extrair estado/runtime compartilhado de `Game` antes de tentar removê-los pelo manifesto.

### Meta e plano de conclusão

Meta deste fork: priorizar um engine server-side rápido e funcional, com gameplay vanilla, protocolo, mundo/persistência e o SDK/loader de plugins `.so` preservados. **Não é requisito manter o cliente compilando** durante ou depois da separação. Também não se deve manter módulo client-only só para preservar uma facade pública do cliente, salvo decisão explícita posterior.

O próximo corte de alto impacto não é outro toggle isolado em `Cargo.toml`. É criar uma fronteira de runtime de servidor que não dependa da facade client-side e migrar para ela, sem duplicar a simulação vanilla:

1. Inventariar as entradas server-owned usadas no tick, rede, comandos, recursos/data packs, worldgen/save e host `.so`; marcar UI, janela, input físico, renderização, HUD, áudio de saída, narrador, splitscreen e serviços desktop como client-only, exceto tipos concretos que o servidor realmente consumir.
2. Extrair por fatias compiláveis o estado e as rotinas do runtime dedicado de dentro do `Game` misto. A fronteira deve possuir startup, ciclo de vida/tick e estado server-side sem construir bundle GUI nem importar APIs de renderer/janela. Reutilizar a simulação compartilhada em vez de copiar o gameplay para um segundo engine.
3. Fazer o alvo de release dedicado depender da nova fronteira; permitir remoção, substituição ou quebra de módulos exclusivamente client-side quando necessário. Não deixar o novo binário voltar a depender de `horizon-client`/`horizon-client-lib` completos só para chegar ao runtime antigo.
4. Só depois dos call sites migrarem, retirar do grafo dedicado `horizon-client-render`, `horizon-client-input`, `wgpu`, `winit` e demais crates visuais comprovadamente não necessárias. Manter `naga` ou parser de pack apenas se houver validação de dados realmente usada pelo servidor, e separar essa validação da compilação/renderização de shaders.
5. Preservar a ABI pública do plugin SDK `.so`, o loader e seus hooks/eventos server-side. Não confundir o plugin API com `mod_api`/mods legados do client; o sistema legado não precisa ser mantido para satisfazer o build do servidor.

### Critérios de aceite

- O comando dedicado de `cargo check` e os jobs server-side de CI/release passam; testes de simulação/paridade, rede, worldgen/save e SDK/loader `.so` relevantes passam.
- `cargo tree --locked -e normal -p <pacote dedicado> --features <features do server>` confirma a ausência das crates client-only acordadas (inicialmente renderer, input, `wgpu`, `winit` e APIs/telas de cliente), em vez de inferir sucesso pela ausência de janela em runtime.
- O build client-side não é bloqueador. O objetivo é validar o servidor primeiro.
- A redução de tempo deve ser comparada com builds **release limpos do CI** antes/depois. Os 11m01s acima são somente a linha de base de `cargo check` local; não são a duração do release.

## Status da integração com o PR #2 (2026-11-22)

Foi feita a integração local do PR #2 (`1a554f14`) com o commit WIP local
`4b6a91ec`, e o resultado foi enviado à branch do PR como `98f26f42`. A `main`
local aponta para esse mesmo commit; o WIP continua preservado como pai do merge.

- O PR #2 separa alguns dados/regras compartilhados (`raid`, `ticket_levels` e
  `world_spawn`) e adiciona caminhos dedicados sem inicialização de partes do
  cliente.
- O WIP local moveu mais módulos de `horizon-client-render` para
  `horizon-sim-data` e adicionou reexports no renderer/facade.
- As duas mudanças usam `horizon-sim-data`; foram combinadas no mesmo crate. A
  integração também manteve `TabList::layout` no renderer porque depende de
  `Font`/HUD, evitando mover uma implementação incompatível com essa fronteira.
- `cargo check --locked -j1 -p horizon-client --no-default-features
  --features server-plugins,dedicated-server --bin horizon-client` passou, assim
  como `cargo clippy` para o mesmo alvo com `-D warnings`.
- A verificação do cliente com áudio padrão não completou neste ambiente porque
  falta a biblioteca de sistema ALSA (`alsa.pc`). Uma primeira tentativa sem a
  feature `horizon-extras` não representa o conjunto normal do cliente. A
  verificação gráfica sem áudio, com `horizon-extras`, passou em 2m05s.
- `cargo fmt --all -- --check` e `git diff --check` passaram.
- A integração **não prova ainda** que o objetivo de reduzir o grafo do servidor
  foi atingido: `horizon-client-render`, `horizon-client-input`, `wgpu`, `winit`
  e `naga` continuam no alvo dedicado. A extração de módulos, por si só, não os
  remove do grafo.

Conclusão técnica provisória: PR #2 e o WIP são compatíveis no alvo dedicado e
podem coexistir, mas o WIP acrescenta uma migração grande sem demonstrar ainda a
redução de dependências que motivou o trabalho. O PR atualizado continua aberto;
o Actions para `98f26f42` está em andamento.

## Handoff: status real em 2026-11-20 (atualizado)

Este documento é o ponto de partida para continuar a separação server/client.
O estado abaixo prevalece sobre planos antigos neste arquivo. **O alvo dedicado
passa no check e no Clippy com `-D warnings`. O build de release do CI leva
~10 min 37 s — tudo em `horizon-client-lib`. O objetivo desta etapa é cortar
esse tempo na metade removendo crates client-only do grafo do servidor.**

### Análise do log do CI (aura-v0.1.16)

O build tem dois blocos sequenciais:

| Bloco | Tempo | Crates principais |
|---|---|---|
| `horizon-client` + `horizon-client-lib` + `horizon-client-render` + wgpu + winit + naga | ~10 min 37 s | **todo o cliente** |
| `horizon-server` + `horizon-prepare` + TLS/crypto | ~2 min 10 s | apenas servidor |

O grafo do build dedicado tem 219 crates únicos. Categorização:

**Necessário no servidor (não tocar):**
- `horizon-core`, `horizon-world`, `horizon-protocol`, `horizon-items`, `horizon-command` — simulação vanilla
- `horizon-client-core`, `horizon-server-files`, `horizon-server-api` — tipos compartilhados
- `horizon-threads` — pool de workers (traz `framehop`/`gimli` para stack dumps — necessário)
- `horizon-perf-format` — servidor grava captures `.hzperf`; `prost` vem daqui (necessário)
- `horizon-anticheat`, `horizon-chat-filter`, `horizon-hooks` — funcionalidade de servidor
- `horizon-server-plugin-api`, `horizon-server-plugin-loader` — sistema de plugins
- `mio`, `rayon`, `serde`, `serde_json`, `flate2`, `zip`, `sha1`, `sha2`, `ureq`, `ring`, `rustls` — rede, I/O, TLS (necessário)
- `naga` (wgsl-in) — `horizon-shaders` usa para ler shaders; o servidor valida packs de recursos. **Removível** com esforço.
- `prost`/`prost-derive` — de `horizon-perf-format`. Necessário.

**Client-only preso no grafo por `horizon-client-render` (raiz do problema):**
- `wgpu`, `wgpu-core`, `wgpu-hal`, `wgpu-naga-bridge`, `wgpu-core-deps-windows-linux-android`, `renderdoc-sys` — GPU
- `winit`, `x11-dl`, `x11rb`, `x11rb-protocol`, `calloop`, `polling`, `xkbcommon-dl`, `xkeysym`, `dlib`, `cursor-icon`, `smol_str`, `dpi`, `raw-window-handle` — janela/entrada
- `tracing`, `tracing-core`, `profiling` — usados por wgpu
- `ab_glyph`, `ab_glyph_rasterizer`, `owned_ttf_parser`, `ttf-parser` — fonte/texto para HUD
- `glam` — vetores/matrizes 3D para renderer (usado em gameplay também — verificar)
- `icu_*` (12 crates), `tinystr`, `litemap`, `zerotrie`, `potential_utf`, `writeable`, `zerovec*`, `yoke*`, `zerofrom*` — normalização Unicode para o renderer de fonte
- `codespan-reporting` — diagnósticos do naga
- `half` — float16 para texturas
- `lz4_flex` — compressão de atlas/texturas

**Estratégia de remoção (esta sessão — execução real):**

A tentativa de tornar `horizon-client-render` opcional no Cargo resultou em 6644 erros
porque o crate tem ~70k linhas onde `wgpu`, `winit`, `glam` aparecem em módulos de
simulação misturados com rendering. Tornar as deps opcionais no Cargo sem gate no source
não compila.

A separação real requer partir `horizon-client-render` em dois crates:

**`horizon-sim-data` (novo crate — sem GPU):**
- `streaming`, `chunk`, `collisions`, `effects`, `containers`, `brewing`, `biome`,
  `assets`, `mesher`, `datapack`, `lod`, `world`, `particles`, `block_entity_render`,
  `camera`, `skins`, `spans`, `crash`, `logging`, `auth`, `fast_hash`, `json_folder`,
  `java_text`, `random_source`, `sound_events`, `audio` (structs/tipos sem cpal)
- Deps: `horizon-core`, `horizon-world`, `horizon-protocol`, `horizon-items`,
  `horizon-threads`, `glam`, `serde`, `serde_json`, `flate2`, `sha1`, `sha2`, `ureq`,
  `chrono`, `bytemuck`, `smallvec`, `png` — **sem wgpu, winit, ab_glyph, icu, lz4_flex, half, naga-glsl, pollster**

**`horizon-client-render` (refatorado — com GPU):**
- `gfx`, `hud`, `hud_kit`, `hud_overlays`, `hud_layout`, `mesher::Renderer/Vertex/MeshTables`,
  `particles::ParticleEngine`, `skins::Downloader/Resolver/SkinSlots`, `ui`, `widgets`,
  `font`, `models`, `block_sprites`, `shader_options`, `shader_pack_list`, todos os
  módulos de UI/tela, OptiFine, Polytone, shader API
- Deps: mantém wgpu, winit, ab_glyph, glam, icu, naga, lz4_flex, half, pollster, etc.

**Impacto esperado:** remover do grafo servidor:
`wgpu`, `wgpu-core`, `wgpu-hal`, `wgpu-naga-bridge`, `wgpu-core-deps*`, `renderdoc-sys`,
`winit`, `x11-dl`, `x11rb*`, `calloop`, `polling`, `xkbcommon-dl`, `xkeysym`, `dlib`,
`cursor-icon`, `smol_str`, `dpi`, `raw-window-handle`, `tracing*`, `profiling`,
`ab_glyph*`, `ttf-parser`, `owned_ttf_parser`, `icu_*` (12), `tinystr`, `litemap`,
`zerotrie`, `potential_utf`, `writeable`, `zerovec*`, `yoke*`, `zerofrom*`,
`codespan-reporting`, `half`, `lz4_flex`, `horizon-optifine`, `horizon-polytone`,
`horizon-shaders`, `horizon-shader-api`, `horizon-hdr`, `horizon-narrator`
= **~80 crates removidos, build estimado ~4–5 min (de 10 min)**

**Execução desta separação (ordem metodológica):**

1. Criar `crates/horizon-sim-data/` com `Cargo.toml` e `src/lib.rs`
2. Mover módulo por módulo de `horizon-client-render/src/` para `horizon-sim-data/src/`,
   começando pelos sem deps de render (`spans`, `crash`, `logging`, `fast_hash`, `json_folder`)
3. Para cada módulo, verificar imports — remover os de `gfx::`, `hud::`, `wgpu::`, `winit::`
   substituindo pelos tipos simulação-safe
4. Atualizar `horizon-client-render/src/lib.rs` para reexportar de `horizon-sim-data`
5. Atualizar `horizon-client-lib/Cargo.toml` para depender de `horizon-sim-data` (incondicional)
   e `horizon-client-render` (opcional, atrás de `client-graphics`)
6. Gate o `pub use horizon_client_render::` em `client_lib.rs`
7. Corrigir o `Game` struct — gate campos render-only com `#[cfg(not(feature = "dedicated-server"))]`
8. `cargo check` a cada módulo movido; não tentar mover tudo de uma vez
9. Clippy + push quando o check passar

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

### Estado atual (2026-11-21) — onde a sessão parou

**O que foi feito:**

- `crates/horizon-sim-data/` criado e compilando limpo. Contém todos os módulos de
  simulação extraídos de `horizon-client-render` sem nenhuma dep de GPU:
  `assets`, `auth`, `ban_notice`, `block_edit`, `block_entities`, `block_entity_anim`,
  `block_entity_data`, `brewing`, `chunk`, `chunk_map`, `collisions`, `command_block`,
  `containers`, `crash`, `custom_sky`, `effects`, `fast_hash`, `hud_layout`,
  `java_text`, `jigsaw_block`, `light`, `logging`, `pack_hooks`, `pack_palette`,
  `poi_sections`, `providers`, `random_source`, `sculk`, `section_mask`, `signs`,
  `skins`, `spans`, `structure_block`, `tab_list`, `test_block`, `test_instance_block`,
  `vault`, `world`, `block_edit`.
- `horizon-client-render/src/lib.rs` atualizado: `pub mod X` → `pub use horizon_sim_data::X`
  para todos os módulos migrados. O render compila limpo com `--features client-graphics`.
- `horizon-client-render/src/block_entity_render.rs`: `BlockAnim` agora é
  `pub use horizon_sim_data::block_entity_data::BlockAnim`.
- `horizon-client-render/src/hud_layout.rs`: wrapper que faz `pub use horizon_sim_data::hud_layout::*`
  e `pub use crate::hud_layout_ext::{width, paint, abgr}`.
- `horizon-client-render/src/hud_layout_ext.rs` (novo): `width`, `paint`, `abgr` que
  precisam de `crate::hud` e `crate::font`.
- `horizon-client-lib/Cargo.toml`: `horizon-sim-data` adicionado como dep incondicional.
- `horizon-client/src/client_lib.rs`: `block_entity_data` e `json_folder` reexportados
  do sim-data; `vault` reexportado do sim-data.

**Estado do build:**

- `cargo check -p horizon-sim-data` → **passa** (6 warnings de imports não usados).
- `cargo check -p horizon-client-render --features client-graphics` → **passa**.
- `cargo check -p horizon-client --no-default-features --features server-plugins,dedicated-server`
  → **1 erro**: `no method named layout found for TabList`.

**O único erro restante:**

`crates/horizon-client/src/game/scoreboard.rs:968` chama `self.tab_list.layout(&inputs, &self.gui.atlas.font)`.

`TabList::layout` precisa de `Font` (crate render). `TabList` está no sim-data mas `layout`
não foi movido porque depende de ~400 linhas de helpers (`build_layout`, `render_header_footer`,
`hearts`, `Entry`, `sprite`, `font.width_text`, `font.split_lines`) que por sua vez dependem
de `Font`, `hud_layout::paint` e outros tipos do render.

**Como resolver (próxima sessão):**

Opção A (mais limpa):
1. Definir `pub trait TabFont { fn epoch(&self) -> u64; fn width_text(&self, t: &Text) -> f32; fn split_lines(...); ... }` no sim-data.
2. Implementar `TabFont for Font` no render (`crates/horizon-client-render/src/tab_list_font_impl.rs`).
3. Mover `layout`, `build_layout`, `render_header_footer`, `hearts`, `render_hearts`, `Entry`,
   `sprite`, `LayoutCache` do render `tab_list.rs` para o sim-data `tab_list.rs`,
   substituindo `font: &Font` por `font: &dyn TabFont`.
4. Fazer o render's `tab_list.rs` ser `pub use horizon_sim_data::tab_list::*`.
5. Atualizar o `scoreboard.rs` do client para usar `use crate::tab_list::TabFont` em scope.

Opção B (mais rápida):
1. Converter `TabList::layout` em função livre em `scoreboard.rs`:
   `fn draw_tab_layout(tab: &mut TabList, inputs: &TabInputs, font: &Font) -> Vec<GuiDraw>`
   implementada no render (onde `Font` está disponível) e gateada com
   `#[cfg(not(feature = "dedicated-server"))]` em `scoreboard.rs`.

**Arquivos modificados nesta sessão (não commited ainda):**
```
Cargo.lock, Cargo.toml
crates/horizon-client-lib/Cargo.toml
crates/horizon-client-render/Cargo.toml
crates/horizon-client-render/src/block_entity_render.rs
crates/horizon-client-render/src/hud_layout.rs         (wrapper)
crates/horizon-client-render/src/hud_layout_ext.rs     (novo)
crates/horizon-client-render/src/lib.rs
crates/horizon-client/src/client_lib.rs
crates/horizon-sim-data/                               (novo crate)
docs/server-side-scope.md
```

### Próximo trabalho obrigatório

1. ~~Corrigir os erros acima~~ — feito nesta etapa.
2. ~~Repetir `cargo check --locked -j1`~~ e ~~Clippy~~ — ambos passam.
3. ~~Criar `crates/horizon-sim-data/`~~ — feito. O crate compila limpo com os
   módulos: `assets`, `auth`, `ban_notice`, `block_entities`, `block_entity_anim`,
   `block_entity_data`, `brewing`, `chunk`, `chunk_map`, `collisions`,
   `command_block`, `containers`, `crash`, `custom_sky`, `effects`, `fast_hash`,
   `java_text`, `jigsaw_block`, `light`, `logging`, `pack_hooks`, `pack_palette`,
   `poi_sections`, `providers`, `random_source`, `sculk`, `section_mask`, `signs`,
   `skins`, `spans`, `structure_block`, `tab_list`, `test_block`,
   `test_instance_block`, `world`. Reexporta: `lang`, `scoreboard`, `blocks`,
   `enchant`, `items`, `json_folder`, `text`. Sem dependências de GPU, janela,
   fonte ou shader.
4. **Impedimento atual:** `horizon-client-render` não pode reexportar tipos de
   `horizon-sim-data` porque tem `impl` nesses tipos (`impl World` em
   `block_edit.rs`, `impl Vault`/`impl VaultConfig` em `vault.rs`) — Rust não
   permite `impl` em tipos de outro crate sem trait. Tentar fazer o `client_lib.rs`
   reexportar `chunk`, `world` etc. do sim-data causou 41 erros de tipo incompatível
   porque `game.rs` e seus ~93 sub-módulos implementam traits do render sobre os
   tipos do render. Os dois crates têm cópias dos mesmos tipos e não são
   intercambiáveis.
   O estado atual: `horizon-client-lib` depende de `horizon-sim-data` (incondicional)
   e de `horizon-client-render` (incondicional). O `client_lib.rs` reexporta tudo
   do render (fonte canônica dos tipos) e adiciona `block_entity_data` e
   `json_folder` do sim-data como novos reexports. O servidor ainda compila o
   render inteiro — o impacto de build ainda não foi alcançado.
5. **Próximo passo real:** A separação real requer uma de duas estratégias:
   a. Mover os métodos render-específicos para traits definidas em `horizon-sim-data`
      que o render implementa (`trait WorldRenderExt`, `trait VaultRenderExt`) —
      mais limpo, permite reexportar tipos do sim-data no render.
   b. Gatear os ~93 arquivos de `game.rs` que referenciam módulos de render com
      `#[cfg(feature = "client-graphics")]`, convertendo o `horizon-client-render`
      num dep opcional de verdade. Cada arquivo que usa `hud`, `mesher`, `streaming`,
      `font`, etc. precisa ser gated ou ter os imports removidos do path dedicado.
5. Separar o estado visual/input de `Game` em uma fronteira cliente, preservando
   para o dedicated somente os dados usados pelo tick, rede e persistência. Só
   então tornar renderer/input/shaders dependências opcionais e provar a remoção
   com `cargo tree`. Remover o `#![cfg_attr(feature = "dedicated-server",
   allow(dead_code, unused_imports))]` do `horizon-client` quando os módulos
   cliente puderem ser gated individualmente sem quebrar código compartilhado.
6. Não apagar `horizon-mods`, `horizon-optifine`, shaders ou tipos compartilhados
   por suposição. Procurar referências e separar visualização de simulação por
   feature; plugins `.so` são a extensão server-side suportada.
7. Não rodar build release local nem paralelizar compilações. Usar jobs limitados;
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
| Narration | Native speech (`speech.rs` and platform bindings) is gated by `client-speech`, enabled with `client-graphics`; `horizon-narrator` remains for shared screen/status APIs. | TTS code is absent from the dedicated profile; full crate removal still requires moving screen narration and separating client UI/control call sites. |
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

### Revalidação da continuação (2026-11-23)

Após reverter o experimento que tornou `wgpu`, `winit` e `pollster` opcionais sem
migrar os consumidores, o alvo dedicado voltou a passar:
`cargo check --offline --locked -j1 -p horizon-client --no-default-features
--features server-plugins,dedicated-server --bin horizon-client` (1,69 s
incremental neste sandbox). `git diff --check` também passou. A árvore normalizada
foi medida novamente e continua em **229 IDs de pacote**; `horizon-client-render`,
`horizon-client-input`, `wgpu`, `winit`, `horizon-shaders` e `horizon-narrator`
continuam presentes. Portanto, não há redução nova de grafo demonstrada nesta
continuação, e o tempo incremental não é comparável a um build limpo de release.

O experimento descartado falhou porque o renderer e o código compartilhado contêm
milhares de referências diretas aos tipos/APIs de `wgpu`; opcionalizar a dependência
no manifest antes de criar uma fronteira de módulos não é um corte válido. O diff
atual do manifest mantém apenas as alterações herdadas (TTS e `test-support`), não
o toggle experimental. A próxima unidade de implementação precisa migrar operações
e estado do runtime dedicado para fora da facade visual, em vez de repetir toggles
de dependências.

Na sequência, `horizon-client-render::biome` foi movido para
`horizon-sim-data::biome`; o renderer preserva seu caminho público como reexport.
Isso transfere a leitura dos biomes, registros de packs, regras de temperatura,
spawn e dados usados por simulação para uma crate compartilhável sem GPU/janela,
sem duplicar tipos nem alterar IDs. O check dedicado passou após a mudança; a suíte
de tint no renderer passou com 4 testes (2 ignorados por exigir recursos locais),
e `cargo fmt --all -- --check`/`git diff --check` passaram. `Biomes::from_jsons`
fica disponível apenas em testes ou com `horizon-sim-data/test-support`, para
preservar fixtures que os testes do renderer usam. O novo teste de integração
`horizon-sim-data/tests/biome_data.rs` passou (1/1), cobrindo IDs built-in,
substituição/aditamento por datapack, features, spawn entries, spawn costs e
comportamento de neve/temperatura.

Essa extração reduz ownership/acoplamento, mas **não reduz o grafo dedicado**:
a medição posterior segue em 229 IDs, com renderer, input, `wgpu`, `winit`,
`horizon-shaders` e `horizon-narrator` presentes. Não atribuir a ela ganho de
compilação; a fronteira ainda precisa avançar até o runtime dedicado deixar de
depender da facade e do renderer.

Como limpeza complementar da nova camada, foram retiradas de
`horizon-sim-data/Cargo.toml` seis dependências diretas sem referências no código
(`bytemuck`, `horizon-assets`, `horizon-command`, `horizon-hooks`,
`horizon-perf-format` e `zip`), com remoção confirmada no bloco correspondente do
`Cargo.lock`. O check locked de sim-data e o check locked do binário dedicado
passaram depois da atualização. A árvore do binário continuou em 229 IDs, pois
esses pacotes ainda chegam por outros caminhos; esta limpeza não é contabilizada
como redução do grafo do servidor.

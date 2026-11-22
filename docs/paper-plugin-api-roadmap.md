# Roadmap de API de plugins do Borizon — referência funcional Paper 26.3

> Documento de acompanhamento. Paper é uma **referência funcional** para identificar capacidades e semânticas relevantes; não é obrigação copiar Java, reproduzir a arquitetura interna, nem promessa de compatibilidade binária ou de executar plugins Bukkit/Paper. A direção pretendida é uma API **Rust idiomática, segura e performance-first para Horizon 26.3**, com contratos próprios explicitamente documentados.

## Referência e critérios de leitura

- PaperMC/Paper, branch `main`, commit `1eda666f33904ba17fd5c14973a3ea62eca10b0c`; `mcVersion=26.3`, `apiVersion=26.3`, canal `STABLE`, conforme os resultados fornecidos.
- [Árvore `paper-api/src/main/java` no commit de referência](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java). Os links de classes e pacotes citados abaixo apontam para esse mesmo permalink.
- Classificação inicial restrita à superfície de plugins do Borizon examinada: [README do runtime](../server-plugins/README.md), [ABI pública](../server-plugins/api/src/lib.rs), [loader](../server-plugins/loader/src/lib.rs) e [adaptador no engine](../crates/horizon-client/src/game/server_plugins.rs). “Ausente” significa não demonstrado na superfície revisada; não prova inexistência de mecanismo interno não acessível a plugins.
- Não se infere equivalência por nome parecido, operação JSON, callback genérico ou funcionalidade de jogo. Um hook pós-ação, por exemplo, não equivale a um evento pré-ação cancelável.
- A granularidade é por capacidade/contrato implementável, agrupando famílias de classes; não é uma listagem método a método de toda assinatura Java. Use o Javadocs 26.3 e a árvore de fontes fixada acima para decompor uma linha em tarefas menores quando a implementação começar.

**Legenda:** `[x]` já existe (a fonte confirma); `[~]` parcial; `[ ]` pendente; `[?]` precisa decisão. Os status são um retrato inicial, não compromisso de implementação.

## Estado atual do Borizon

O sistema de plugins documentado é de **bibliotecas nativas dinâmicas**, com ABI C versionada v2/v3 e arquivos `.so` em `plugins/`; é distinto de um carregador de plugins Bukkit/Paper Java/JAR. O engine define `server-plugins` como feature opcional que habilita as crates de API/loader e `dedicated-server` como feature separada para o build dedicated; a receita de servidor ativa ambas. Há APIs de commands, events, permissions, scheduler, storage, world, entity, economy/transactions e messaging em níveis diversos. Os callbacks são síncronos no contexto/thread do servidor. O código nativo é confiável, sem sandbox contra corrupção de memória, `abort`, crash ou exaustão de recursos. A instalação de bibliotecas exige reinício; hot-unload/reload nativo não deve ser presumido seguro. O próprio callback pode consumir orçamento do tick, e trabalho bloqueante pode degradar a simulação.

O material examinado descreve criação/destruição de instâncias, callbacks de comando/menu/evento, tarefas por ticks, storage JSON e operações selecionadas. A documentação indica captura de panic em callbacks e desativação do plugin, mas isso não contém falhas de memória ou crash de código nativo. A existência de operações não implica APIs Bukkit equivalentes. Mods de conteúdo — dados, recursos ou alterações de conteúdo do jogo — devem permanecer conceitualmente separados do runtime de extensões nativas: não tratar mod de conteúdo como plugin executável, nem presumir que registries/datapacks e ABI de servidor tenham o mesmo lifecycle ou modelo de confiança.

## 1. Lifecycle, metadata, carregamento e segurança

Referências: [`org.bukkit.plugin`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/plugin), [`org.bukkit.plugin.java`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/plugin/java), [`io.papermc.paper.plugin`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/plugin), [`io.papermc.paper.plugin.lifecycle.event`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/plugin/lifecycle/event), [Lifecycle API docs](https://docs.papermc.io/paper/dev/lifecycle/).

- [~] Manter o sistema nativo atual como extensão própria: ABI v2/v3, entrada create/destroy, callbacks e estado `enabled` são evidência de ciclo básico, não de lifecycle Paper completo.
- [x] Disponibilizar um crate Rust SDK v3 com trait de plugin, macro de entry point e wrappers seguros para os callbacks, UUID e `call_json`; ABI binária v2/v3 permanece inalterada. Coberto por documentação de uso e testes unitários do SDK. Isso melhora a ergonomia Rust, mas não implementa lifecycle Paper nem sandbox.
- [ ] Especificar estados/transições, validação, falha parcial, idempotência e callbacks explícitos de load/enable/disable; garantir limpeza de comandos, listeners, tarefas e serviços ao desativar.
- [ ] Definir descriptor nativo, identidade/versão, capacidades, permissões, dependências e validação; resolução de dependências/ciclos e ordem de ativação/desativação.
- [ ] Definir operações administrativas e política de instalação: restart requerido, autorização, rollback, dependências e distinção entre reload de configuração, reload de registrars/dados e reload real do plugin.
- [ ] Definir semântica de callbacks de enable/disable e notificações de lifecycle, incluindo quem observa transições e como falhas de callback afetam o plugin.
- [ ] Definir mecanismo extensível e seguro de resolução/carregamento de bibliotecas nativas, política de trust/allowlist, compatibilidade ABI e limites de recursos; não prometer sandbox.
- [ ] Separar explicitamente mods de conteúdo e assets do runtime de código nativo no desenho e na documentação.
- [?] Decidir se o alvo continuará exclusivamente nativo ou se haverá bridge Java/JAR; não inferir suporte a `plugin.yml`, `paper-plugin.yml`, `JavaPlugin`, bootstrap Paper ou classloaders Java.
- [?] Avaliar bootstrap e lifecycle registrars Paper como inspiração funcional para fases/registradores, sem adotar literalmente tipos Java nem hot-unload implícito.

## 2. Eventos, listeners e hooks de jogo

Referências: [`org.bukkit.event`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/event), [`org.bukkit.plugin`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/plugin), [`io.papermc.paper.event`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/event), [Event listeners docs](https://docs.papermc.io/paper/dev/event-listeners/).

- [~] Eventos atuais incluem join/quit, break/place, menu close, mudança de dimensão, teleport, death, chat, status de resource pack e tarefa. SDK oferece `decode_server_notification_event` tipado para join/quit (incluindo snapshot de localização opcional), tentativa de morte, block break/place, menu close, chat, mudança de dimensão, teleport e status bruto de resource pack, com payload validado e testes; `ResourcePackStatusCode` opcional mapeia os códigos atuais 0–7 e deixa códigos desconhecidos disponíveis como `i32`. O callback Rust `notification_event` é opt-in por `TYPED_NOTIFICATION_EVENTS`; com default false, não adiciona decode específico ao fluxo dos plugins JSON existentes. ABI e callback genérico de eventos permanecem inalterados.
- [~] Preservar/documentar distinção de fase: `BLOCK_BREAK` permite veto pelo retorno do callback após checks e antes da destruição; `PLAYER_DEATH` pode ser vetado para manter o jogador em 1 de saúde; place e chat são notificações pós-ação, `PLAYER_TELEPORT` é observacional e não impede a decisão do motor. Constantes `PLAYER_MOVE` e `PLAYER_INTERACT_ENTITY` não comprovam, isoladamente, emissão efetiva.
- [x] `ServerNotificationEvent::is_cancellable()` reflete os handlers que atualmente consomem o retorno de cancelamento (`BLOCK_BREAK` e `PLAYER_DEATH`); o resultado vem do dispatch real do host, não de inferência pelo nome.
- [ ] Definir contrato de payload versionado, fase pré/pós, mutabilidade, thread, erro, ordem, prioridade e cancelamento por evento; resolver semântica de múltiplos plugins (hoje cancelamento descrito como OR, sem prioridades).
- [ ] Implementar lifecycle de registro/desregistro e remoção automática de callbacks na desativação, além de filtros/prioridade se requeridos.
- [ ] Avaliar hooks pré-commit para conexão/login/kick/respawn, movimento/teleporte/interação, chat/comandos, block place/interação e inventário; permitir cancelamento/redirecionamento somente quando aplicado antes da decisão do motor.
- [ ] Avaliar eventos de dano, spawn, morte, transformação, target, projétil, entidades, veículos, inventários, chunks/mundos, clima, estruturas e administração conforme casos de uso; cada família deve ter contrato e teste próprio.
- [ ] Distinguir eventos assíncronos e síncronos; callbacks atuais são no contexto do servidor, sem garantia de thread pool ou segurança off-thread.
- [?] Escolher API estreita de eventos nativos JSON ou abstração Rust tipada de handlers; compatibilidade nominal Bukkit não é requisito presumido.

## 3. Commands, permissões e ajuda

Referências: [`org.bukkit.command`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/command), [`org.bukkit.permissions`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/permissions), [`org.bukkit.help`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/help), [`io.papermc.paper.command.brigadier`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/command/brigadier), [Paper command registration docs](https://docs.papermc.io/paper/dev/command-api/basics/registration/).

- [~] ABI registra comandos em startup; v3 descreve usage, fallback de nível op e nó opcional de permissão. Loader constrói árvores Brigadier a partir de usages textuais, verifica colisões/aliases vanilla e dá suporte a sugestões literais e callback dinâmico.
- [x] SDK Rust v3 oferece `CommandSpec` e `PluginApi::register_command`, com fallback op level normalizado a 0–4, permission node opcional validado localmente em 1–128 bytes, e retorno tipado para erro de argumento/transporte/rejeição do host; payload, clamp, validação local e rejeição do estágio de registro são cobertos por testes. Isso não adiciona parsers tipados, aliases, remoção dinâmica nem cleanup por lifecycle.
- [x] SDK v3 oferece `add_command_suggestion(PlayerId, value)` sobre `command.suggestions.add`, validando localmente 1–256 bytes UTF-8 sem caracteres de controle. O host só aceita a chamada no contexto temporário em que consulta sugestões; fora dele retorna `suggestions_unavailable`. Não altera ABI nem garante filtragem, ordenação, deduplicação ou limite agregado de candidatos; testes cobrem envelope, validação e rejeição fora de contexto.
- [x] SDK v3 oferece `execute_player_command(PlayerId, command)` com limite de 2.048 bytes e retorno `ran`, result e componentes estruturados de feedback. A execução preserva permissões do jogador, pode alterar o mundo/servidor e pode retornar `ran: false` fora de um contexto válido; cobertura inclui payload, feedback, false e limites. Não adiciona console/command sender genérico.
- [~] Documentar limites de argumentos e sugestões: `word`/`word?` via ask-server não é catálogo de argumentos tipados, resolvers, parsers, contexto e sugestões completas do Paper Brigadier. SDK fornece `command_completion_input(&str) -> Option<&str>` sem alocação para reconhecer o prefixo e validar o limite de 512 bytes do callback host, além de `add_command_suggestion`; parsing da gramática, contexto, filtros e teste de ponta a ponta em servidor continuam pendentes.
- [x] Precedência confirmada: o `permission node`, quando configurado para um usage, substitui a checagem pelo nível op; sem node, aplica-se o nível op. SDK/README documentam que `PermissionDecision::Defer` usa esse fallback. Fonte: `ServerCommand` e seleção de acesso no engine.
- [~] Permanecem a formalização completa de visibilidade da árvore, mudanças de permissões em runtime e diferenças de autorização entre v2/v3; não há API geral de attachments ou gestão dinâmica de permissões.
- [ ] Definir registro, remoção, ownership, namespace, aliases de plugin e colisões; lifecycle de comando ao desativar plugin.
- [ ] Avaliar permissões hierárquicas/dinâmicas e attachments, com atualização eficiente, sem presumir equivalência com `Permission`/`Permissible`.
- [ ] Definir contextos de execução e origem além do jogador, se necessários: console, bloco, remote/proxy e respectivos dados de localização/identidade.
- [ ] Definir hooks de preprocess, comando desconhecido, visibilidade de comandos e comandos remotos, com ordem/fase/cancelamento explícitos.
- [ ] Definir sistema de ajuda/documentação ou declarar que ajuda é derivada da árvore e não tem API extensível.
- [ ] Testar completions por fonte/contexto, filtragem, limites, ordenação, deduplicação e latência; evitar reconstrução da árvore por tecla/consulta.
- [?] Decidir se a árvore textual atual é suficiente como API Horizon ou se builders/tipos Rust mais expressivos são necessários. Paper `CommandMap`, `CommandSender`, `HelpMap` e builders Java são referências de comportamento, não desenho obrigatório.

## 4. Players, conexões e administração do servidor

Referências: [`org.bukkit.Server`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit), [`org.bukkit.entity.Player`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/entity), [`org.bukkit.profile`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/profile), [`org.bukkit.ban`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/ban), [`org.bukkit.plugin.messaging`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/plugin/messaging), [`io.papermc.paper.connection`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/connection).

- [~] Snapshot/lista/lookup cobrem jogadores online e campos selecionados (UUID, nome, dimensão, posição/rotação, gamemode, permission level, saúde); não equivalem a API geral de `Player` nem a `OfflinePlayer`.
- [~] Operações selecionadas de teleport, gamemode, mensagem, inventário, estatísticas e resource pack existem em graus distintos; definir contratos e limites individualmente.
- [x] SDK v3 oferece `PluginApi::send_message(PlayerId, &str)` para emitir texto simples como componente estruturado ao jogador do callback; rejeições do host são tipadas e cobertas por teste. Não oferece audiência geral, renderer, interceptação pré-broadcast nem componentes tipados completos.
- [x] SDK v3 oferece `teleport_player(PlayerId, dimension, [x,y,z], yaw, pitch) -> bool` e `set_game_mode(PlayerId, GameMode) -> bool`; os wrappers validam números não finitos localmente e mantêm `false` como resultado sem sucesso/mudança reportado pelo host. Teleport entre dimensões pode ser enfileirado pelo host; testes cobrem payload, modos válidos, false e respostas inválidas. Isso não cria referências mutáveis de jogador nem paridade total com `Player.teleport`.
- [x] SDK v3 oferece `set_player_display(&PlayerDisplaySpec) -> PlayerDisplayResult`, expondo resultado agregado e por campo. `chat`/`tab` são sempre enviados (None limpa); o nametag é opcional e pode ser preservado. Testes cobrem componentes, clears e aplicação parcial; a operação ainda pode falhar em hooks nos quais o estado de jogador está trocado.
- [x] SDK v3 oferece `send_resource_pack`, `send_resource_packs` (1–8, com resultado parcial explícito) e `remove_resource_pack(PlayerId, Option<u128>)`, com UUID tipado, prompt opcional e limites locais alinhados ao host; `None` remove todos os packs de servidor. Testes cobrem payload, lote parcial, remoção individual/global e cotas. Isso não é plugin messaging nem uma API geral de configuração/conexão.
- [x] SDK v3 oferece `get_online_player(PlayerId)`, `online_players()`, `find_online_player_by_name()` e `find_online_player_by_id()` tipados como `PlayerSnapshot`; UUIDs hexadecimais são convertidos para `PlayerId`, e a lista conserva a ordem momentânea do host. Testes cobrem desserialização dos campos, lista vazia, rejeições, dados inválidos e consultas por nome/UUID. São snapshots online read-only, não referências vivas, `OfflinePlayer` ou API Paper completa; a lista síncrona permanece sujeita ao limite de 64 KiB da resposta JSON.
- [x] SDK v3 oferece `online_player_stat(PlayerId, type, key) -> i32`, com identificadores limitados a 128 bytes e retorno zero para estatística ainda não registrada; testes cobrem payload, resultado, rejeição do host, validação de limite e resposta inválida. Leitura individual não implementa mutação, API ampla de estatísticas ou eventos de award.
- [x] SDK v3 também oferece `online_player_stats(PlayerId) -> PlayerStatistics`, com mapa tipado por tipo/ID e `DataVersion`; testes cobrem payload, contagens, versão, resposta inválida e rejeição para jogador offline. O JSON completo pode atingir o limite de 64 KiB, então consultas frequentes devem usar a leitura individual.
- [~] `maintenance.begin/status/end` é manutenção temporária que recusa login e pode expulsar sessões; não equivale a ban, whitelist, operador persistente ou kick individual.
- [x] SDK v3 oferece `server_maintenance_status()`, `begin_server_maintenance()` e `end_server_maintenance()` com resultados tipados. Begin ativa o gate e pede kick imediato das sessões; end retorna `PlayersOnline(n)` até esvaziar. Testes cobrem payload, contagens, validação de mensagem e rejeições; é manutenção temporária, não ban/whitelist.
- [x] SDK v3 oferece `prepare_player_data_reset(&[PlayerDataCategory])` e `commit_player_data_reset(plan, backup)` em duas fases. O preview normaliza categorias, lista paths, expiração de 60 s, `backup_required`/`requires_force` e mantém token privado; o commit consome o plan por valor e valida categorias/resultado. Testes cobrem normalização, payload, resultado e backup. Commit é destrutivo, faz backup/deleção síncronos com tamanho variável e não é atômico; não desconecta jogadores à força e falha se `playerdata` for escolhido enquanto houver sessões. Não é API de reset/ban Paper abrangente.
- [~] Chat é notificação após broadcast segundo os resultados; `chat.send` não é broadcast geral nem hook editável/cancelável.
- [~] Envio/status de resource pack Java não é canal de plugin messaging, cookies ou protocolo arbitrário.
- [ ] Definir necessidades de lookup offline, identidade persistente, perfil/texturas, kick individual, permissões, locale e operações adicionais de player.
- [ ] Projetar ban por perfil/IP, whitelist e operadores como políticas distintas, incluindo persistência, expiração, motivo, consulta e eventos.
- [ ] Definir broadcast estruturado, audiência, retorno e eventuais hooks filtráveis.
- [ ] Definir conexão por fases de login/configuração/jogo e callbacks de validação, kick e cookies, se fizerem parte do escopo.
- [ ] Avaliar plugin messaging com canais/direções, payload, limites e lifecycle; separar de resource packs.
- [?] Selecionar conscientemente um subset de API Horizon, em vez de anunciar paridade ampla de `Server`/`Player`.

## 5. World, blocos, chunks, biomas e geração

Referências: [`org.bukkit.World`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit), [`org.bukkit.block`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/block), [`org.bukkit.generator`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/generator), [`org.bukkit.structure`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/structure), [`io.papermc.paper.world.chunk`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/world/chunk).

- [~] Operações de dimensão list/load/unload/create/import/delete e gamerules oferecem gestão selecionada, não interface geral `World`/`WorldCreator` ou lifecycle completo.
- [x] SDK v3 oferece `world_dimensions()` e leitura `world_gamerule(dimension, rule)` de regras vanilla `bool`/`int` em dimensões carregadas, mais `set_world_gamerule` para mutação tipada nesse escopo. Testes cobrem payload, tipos, resposta inválida e rejeições; não equivale a uma API geral de `World`.
- [x] SDK v3 oferece `load_dimension(dimension, ticket_owner)` e `unload_dimension(...)`; `load` retorna dimensão, loaded e pinned, enquanto ticket namespaced fixa chunk `[0,0]` no host. `unload` retorna false quando o host não pode fechar a dimensão. Testes cobrem payload, owner inválido, estado incompleto e rejeição; isso não implementa chunk API geral.
- [x] SDK v3 oferece `create_custom_dimension(id, CustomDimensionPreset)` para presets `Overworld`, `Nether`, `End` e `Flat`, exigindo que ID e preset retornados coincidam com o pedido, com resultado tipado de ID/preset/estado loaded e testes para payload, rejeição e resposta incompleta. A validação semântica do ID segue no host. A chamada cria diretórios e prepara o nível sincronamente no thread do servidor, então deve ser rara e não pode rodar em cada tick. Importação de saves (cópia recursiva) e exclusão recursiva sem backup continuam sem wrappers, pois podem bloquear por tempo proporcional ao tamanho dos dados; isso não oferece API geral de `WorldCreator`.
- [x] SDK v3 oferece `custom_dimension_gamerule(dimension, rule)` para leitura efetiva de regra `bool`/`int` sem carregar a dimensão, além de `set_custom_dimension_gamerule` e `clear_custom_dimension_gamerule` para overrides persistidos. Testes cobrem valores/payload/rejeições; custom set/clear não carregam a dimensão e não substituem o lifecycle de `World`.
- [~] `world.location.find_safe` consulta colunas/chunks carregados com limites próprios; não é busca global nem API genérica de `Location`/ray trace.
- [x] SDK v3 oferece `find_safe_location(PlayerId, radius, attempts, center) -> Option<SafeLocation>` com raio de 8–512 blocos e 1–256 tentativas, consultando apenas chunks já carregados da dimensão atual do jogador. Testes cobrem payload, retorno, ausência de resultado, rejeição e cotas; a busca síncrona não carrega chunks e não deve ser chamada a cada tick.
- [~] Criação por presets/metadados e ticket de chunk em `[0,0]` não equivalem a geração extensível nem gestão geral de chunks.
- [~] Eventos selecionados de bloco incluem break com posição/state e cancelamento, e place pós-ação; não assumir bloqueio de place.
- [ ] Definir identidade/consultas de mundo (ambiente, seed, altura, spawn, estado) e lifecycle de create/load/save/unload.
- [ ] Adicionar, se necessário, leitura/escrita tipada de bloco, `BlockData`/estado, material, física, drops, block entities e mutações em lote.
- [ ] Projetar acesso a chunk, carga/geração/unload e tickets por owner/coordenadas; operações devem explicitar se carregam chunks e oferecer limites/batching.
- [ ] Definir snapshots/heightmaps, biomas e busca, estruturas/locate/place, ray tracing e world border.
- [ ] Se houver geração de plugins, especificar fases, `ChunkData`, biome provider, populator/região limitada, determinismo, seed e concorrência.
- [ ] Criar hooks de mundo/chunk/estrutura e de bloco pré/pós, com timing e cancelabilidade definidos.
- [?] Avaliar extensões Paper de chunk state, ray tracing e predicates como inspiração para subset Horizon versionado.

## 6. Entities, entidades vivas, mobs e movimento

Referências: [`org.bukkit.entity`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/entity), [`org.bukkit.attribute`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/attribute), [`org.bukkit.event.entity`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/event/entity), [`io.papermc.paper.entity`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/entity).

- [~] Spawn/move/remove de entidades, displays, item displays e text displays cobrem CRUD limitado; não equivalem a entidade Bukkit geral, estado tipado ou API por UUID abrangente.
- [x] SDK v3 expõe `TextDisplayCreateSpec`/`create_text_display` para a operação host `entity.text_display.create`, retornando `EntityRef` com ID runtime dimension-scoped removível por `remove_entity`; não fornece UUID estável nem substituição. Em separado, `TextDisplaySpec`, `upsert_text_display` e `delete_text_display` usam UUID estável; upsert remove o display anterior antes de spawnar, exige dimensão carregada e não oferece atomicidade. Testes cobrem payload e limites.
- [x] SDK v3 expõe `EntityRef`, `move_entity` e `remove_entity` para IDs runtime escopados por dimensão. Movimento se restringe a entidades não simuladas (displays, markers e interactions), não mobs/jogadores; remoção usa comportamento nativo semelhante a `/kill`. Testes cobrem payload, escopo e posições não finitas.
- [x] SDK v3 expõe `EntitySpawnSpec`/`spawn_entity` com UUID estável, tipo namespaced, posição e opções host-native de persistência/AI/nome; `minecraft:interaction` inclui dimensões e resposta de hitbox. Retorna UUID mais `EntityRef`; `remove_entity_by_uuid` remove por identidade estável. Testes cobrem payload, hitbox e validações locais. Não é handle geral Bukkit nem estado completo de mob.
- [x] SDK v3 expõe `ItemDisplaySpec`/`upsert_item_display` com resource key e transformação (translation, quaternion normalizado pelo host e scale), validando formato, finitude e limites antes do envio. Substituição por UUID não é atômica; não equivale a API completa de `Display`/`ItemStack`. Testes cobrem payload e rejeições locais.
- [~] Teleport/eventos de player e configurações limitadas de persistência/AI não equivalem a movimento/teleport de toda entidade, flags, colisão, pathfinding ou AI.
- [~] `PLAYER_MOVE`/interação têm limitações de confirmação e hooks pré-commit segundo os resultados; display não é API completa de `Display`/`Hanging`.
- [ ] Definir consultas de entidade e estado essencial por UUID, mundo e tipo; distinguir entidade carregada de persistida/descarregada.
- [ ] Definir teleport/velocidade/movimento, passageiros, colisão, flags e falhas de transferência de mundo.
- [ ] Projetar saúde, dano, cura, absorção, morte/loot/XP e atributos/modificadores, se necessários.
- [ ] Avaliar API de mob AI/target/pathfinding com limites de custo e hooks de spawn/target/transform.
- [ ] Avaliar projéteis, veículos, mounts, hanging, displays/interactions e persistência/remoção com semântica por tipo.
- [ ] Implementar hooks de dano, knockback, spawn, death, teleport, transformação e mount/dismount apenas com ponto de decisão efetivo; especificar momento/cancelamento.
- [?] Definir subset de entidades apropriado à API nativa; não mapear `entity.move` automaticamente para física, velocidade ou `PlayerMoveEvent`.

## 7. Items, inventários, containers, recipes e economy

Referências: [`org.bukkit.inventory`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/inventory), [`org.bukkit.inventory.meta`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/inventory/meta), [`org.bukkit.event.inventory`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/event/inventory), [`org.bukkit.inventory.recipe`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/inventory), [`io.papermc.paper.datacomponent.item`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/datacomponent/item).

- [x] SDK v3 oferece `open_menu(PlayerId, &MenuSpec)` com título estruturado, até 6 linhas/54 itens, slots únicos, contagem limitada e componentes namespaced validados. Retorna `Option<window_id>` e reutiliza o callback de clique existente; não demonstra paridade com inventários/containers vanilla ou transações Bukkit.
- [~] ItemStack/nome/lore/components JSON e inspeção de stack cobrem subconjunto; não presumir criação/mutação completa de ItemMeta ou todos os data components 26.3.
- [~] Views de inventário live/file, slots plugin-owned e delivery/claim têm contratos próprios; não equivalem a `PlayerInventory`, todos os holders ou mutações gerais.
- [x] SDK v3 oferece `open_inventory_view(viewer, &InventoryViewSpec) -> Option<window_id>` para inventário Player/Ender de outro jogador, incluindo dados salvos quando o alvo está offline. Valida capacidade, layout, duplicatas e slots; a view é interativa e cliques podem mover itens usando regras vanilla, portanto deve ser aberta fora de um turno com estado de jogador trocado. Não implementa view genérica de container.
- [x] SDK Rust expõe `get_balance(PlayerId, currency)` e `transact(PlayerId, currency, balance_delta, &[ItemDeltaSpec])` sobre callbacks ABI v2 existentes, sem alterar layout binário. Moeda vazia + delta de saldo zero permite transação item-only; o wrapper valida currency, item IDs, deltas e limite de 16 entradas localmente e mapeia saldo insuficiente, rejeição de itens, storage e request inválido em `EconomyError`. Testes cobrem leitura, callback ausente, status e contrato item-only. O host exige jogador online fora de turno ativo; coordena saldo e inventário no thread do servidor e reverte saldo se a aplicação de inventário falhar durante processamento normal, mas não garante commit atômico entre arquivo de saldo e save do mundo em caso de crash. Não cobre metadata, receitas, stack inspection/delivery genéricos nem transações Paper completas.
- [ ] Definir modelo Rust tipado/versionado para stacks, metadados/components, similaridade, limites, serialização e componentes desconhecidos.
- [ ] Definir leitura/mutação de inventários, slots/equipamento/Ender e inventários de blocos; snapshots vs referências vivas e revisão/conflito.
- [ ] Especificar protocolo pré-commit de click/drag, cursor, ação, slots e cancelamento; prevenir dessync, duplicação e reentrância.
- [ ] Avaliar hooks de pickup/transport/hopper/processamento e containers especializados (furnace, brewing, anvil, smithing, loom, cartography, etc.).
- [ ] Projetar registro/consulta de recipes e crafting, preparação/resultado/consumo/remainder/shift-click, com sync ao cliente.
- [ ] Avaliar encantamentos, loot tables/contexts e trades/merchant com operações e hooks próprios.
- [x] Documentar economy e testar validação, erros e limites no wrapper SDK; a callback host faz validação/coordenação de inventário/saldo, mas não deve ser descrita como transação Paper completa nem como crash-atomic entre arquivos.
- [?] Priorizar paridade de inventário/gameplay versus manter menus e transações simplificados como abstração nativa.

## 8. Comunicação, presentation e gameplay APIs

Referências: [`org.bukkit.conversations`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/conversations), [`org.bukkit.boss`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/boss), [`org.bukkit.scoreboard`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/scoreboard), [`org.bukkit.map`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/map), [`io.papermc.paper.chat`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/chat), [`io.papermc.paper.dialog`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/dialog).

- [~] Envio de chat estruturado, menu, sidebar limitada, display/hologram e leitura de stats existem em diferentes níveis; não equivalem a todas as APIs Paper correspondentes.
- [~] Chat é pós-broadcast; texto JSON/legacy não comprova suporte total a Adventure (componentes aninhados, estilos, click/hover, renderer por audiência).
- [~] Sidebar tem limite documentado de 15 linhas; não é sistema completo de objectives, scores, teams e formatos.
- [~] Display entities oferecem presentation selecionada; não implicam boss bars, sons, partículas, mapas ou advancements.
- [x] SDK v3 oferece `set_sidebar`/`clear_sidebar` com `TextComponent` e `SidebarLine`, limite local de 15 linhas e retorno do host. Texto legacy e JSON são encaminhados, sem prometer todos os recursos Adventure/Bukkit; testes cobrem payload, limite e clear.
- [ ] Definir subconjunto de componentes textuais aceitos, serialização, escaping, estilos e compatibilidade de clientes.
- [ ] Avaliar chat pré-broadcast editável, audiência por destinatário e formatação, além de conversas/sessões de prompts ou dialogs.
- [ ] Adicionar, se necessário, som, partículas e efeitos com payloads tipados e limites por frequência/destinatário.
- [ ] Avaliar boss bars, scoreboards completos, advancements e estatísticas com leitura/escrita/hook tipado.
- [ ] Avaliar mapas com lifecycle, canvas/renderers, cursores e atualizações incrementais.
- [ ] Especificar resource pack, mensagens e apresentação por viewer; controlar tráfego, culling e custo por tick.
- [?] Decidir quais capacidades de UI são centrais ao Horizon 26.3 e quais serão deliberadamente não suportadas.

## 9. Scheduler, thread safety, storage, registries e serviços

Referências: [`org.bukkit.scheduler`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/scheduler), [`io.papermc.paper.threadedregions.scheduler`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/threadedregions/scheduler), [`org.bukkit.plugin.ServicesManager`](https://github.com/PaperMC/Paper/blob/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/plugin/ServicesManager.java), [`org.bukkit.persistence`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/persistence), [`io.papermc.paper.registry`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/io/papermc/paper/registry), [`org.bukkit.plugin.messaging`](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java/org/bukkit/plugin/messaging), [Scheduler docs](https://docs.papermc.io/paper/dev/scheduler/).

- [~] Scheduler por ticks com schedule/cancel, IDs e evento `SERVER_TASK`; tarefa executa no contexto do servidor. Não equivale a scheduler async/global/region/entity Paper/Folia.
- [x] SDK fornece wrappers tipados para agendar/cancelar tarefas por ticks, valida seus limites, serializa payload e encaminha `SERVER_TASK` apenas ao callback SDK cujo ID corresponde. `scheduler.cancel` aceita owner opcional para impedir cancelamento acidental de tarefa com ID de outro namespace; chamadas antigas sem owner continuam aceitas por compatibilidade. O ID é organização para código nativo confiável, não autorização.
- [x] SDK também decodifica payloads `SERVER_TASK` via `decode_server_task_event(&Value) -> Option<ServerTaskEvent>`, verificando `plugin_id`, convertendo `task_id` e mantendo o payload `data` como JSON extensível; testes cobrem owner correto/incorreto e formatos de ID válidos/inválidos. Não altera a ABI nem o formato emitido pelo host.
- [x] SDK v3 oferece `write_log(ServerLogLevel, message)` com enum de nível e validação local de mensagem entre 1 e 2.048 bytes; testes cobrem payload, níveis, limites e rejeições. Continua sendo uma operação de escrita no log, não um serviço geral de logging assíncrono ou rate-limited.
- [~] Storage JSON por plugin com get/set/delete/list, persistência atômica e limites é configuração/armazenamento próprio; não é PDC tipado ligado a objetos, YAML Bukkit ou migração automática.
- [x] SDK fornece CRUD/list tipados com `serde`, namespace derivado de `Plugin::ID`, erros explícitos e paginação; `storage.get` agora inclui `found` para distinguir ausência de JSON `null`. Compatibilidade com host v3 antigo mantém fallback ambíguo para `null`.
- [~] Envio/status de resource packs não é ciclo completo de datapacks/registries nem plugin messaging.
- [ ] Definir semântica exata de atraso/repetição, cancelamento, shutdown, lag/pausa e ownership de tarefa; limpeza ao desativar plugin.
- [ ] Definir estratégia para I/O/CPU assíncronos sem permitir acesso inseguro ao estado de jogo; fila de retorno ao servidor e limites de execução.
- [ ] Medir e impor orçamento de callback/tarefa, tamanho de payload JSON, número de tarefas, frequência e custo de serialização.
- [x] SDK oferece `PluginApi::call_with_response_buffer` e `call_with_buffers` para reuso opt-in dos bytes de resposta e de request/resposta, preservando a reserva mínima/limite de 64 KiB da resposta e a semântica síncrona; testes verificam capacidade/endereço estáveis após aquecimento. Reuso não elimina alocações internas dos valores `serde`, não inclui benchmark e não altera a ABI.
- [ ] Projetar registry tipado/extensível e lifecycle de composição/reload apenas se necessário; estabelecer chaves, ownership, conflitos e estabilidade ABI.
- [ ] Avaliar services/provider registry, unregister automático, prioridades e identidade de tipos compartilhados entre bibliotecas nativas.
- [ ] Definir persistência tipada por holder, namespaces, schema/versionamento e migração, separada do storage JSON atual.
- [ ] Definir API própria de configuração e resource/data reload; separar de configuração interna do servidor.
- [ ] Avaliar plugin messaging com canais, direções, limites e lifecycle distintos de resource packs.
- [?] Decidir se threading por região/entidade é requisito para Horizon; não transplantar Folia como simples troca de executor, pois exige ownership e arquitetura de estado compatíveis.

## Decisões transversais: APIs Java-only, obsoletas e internals

Os tipos Paper/Bukkit aqui citados orientam capacidades e semânticas, não uma lista de nomes para portar. Muitos dependem de classes Java, annotations/reflection, classloaders, `HandlerList`, `CommandMap`, interfaces de `Plugin`, `JavaPlugin`, serializers Adventure, generics e registry/object lifecycle Java. Alguns pontos de entrada são legados ou carregam compatibilidade histórica; outros pertencem a implementação interna e não são contrato público estável. Antes de adotar qualquer ideia, confirmar a API pública e escolher um contrato Rust que não exponha detalhes internos ou obsoletos.

- [?] Não prometer executar JARs, `JavaPlugin`, `plugin.yml`/`paper-plugin.yml`, classloaders, `PluginLoader` Java ou ABI binária Bukkit/Paper.
- [?] Não espelhar tipos Java método a método; preferir ownership, lifetimes, enums versionáveis e erros Rust adequados, mantendo C ABI explícita somente onde necessária.
- [?] Tratar Paper-specific/Folia, registrars reexecutáveis e reload de recursos como inspiração até haver requisito, contrato e testes Horizon.
- [?] Avaliar risco de unload/hot reload de `.so`; exigir restart enquanto segurança de referências, tarefas e callbacks não estiver demonstrada.
- [?] Definir política de compatibilidade por versão do Horizon/ABI e protocolo, sem sugerir que `apiVersion=26.3` Paper seja promessa de compatibilidade.

## Critérios para considerar uma capacidade concluída

Uma linha só deve passar a `[x]` quando a implementação e as fontes do Borizon confirmarem o contrato — não basta existir nome/operação semelhante. Para cada capacidade, registrar:

1. **Contrato público:** documentação de tipos/payloads, ownership, erros, limites, permissões, lifecycle e versionamento; diferença explícita frente à referência Paper quando aplicável.
2. **Semântica do hook:** fase pré/pós, mutabilidade, cancelamento, ordem/prioridade, thread, comportamento em falha e remoção ao desativar; provar que veto ocorre antes do commit.
3. **Testes:** unitários e integração no dedicated server, casos positivos/negativos, dependências/colisões, unload/restart, falhas de callback, persistência e compatibilidade ABI v2/v3.
4. **Edge cases:** jogador desconectado, entidade/chunk indisponível, IDs desconhecidos, payload malformado/grande, callback lento/panic, cancelamentos múltiplos, mudança de versão e estado concorrente.
5. **Segurança e operação:** trust explícito para nativo, autorização, quotas e limites; sem alegar sandbox nem contenção de crash nativo sem mecanismo demonstrado.
6. **Orçamento de tick:** benchmark de custo por callback/evento/tarefa e carga representativa; contabilizar serialização JSON, mutex/dispatch, número de plugins e frequência. Sem regressão inaceitável na simulação.
7. **Versionamento e migração:** política ABI/protocolo, compatibilidade, atualização de metadata/storage e comportamento quando plugin/versão não corresponde.
8. **Paridade reivindicada:** se houver alegação de equivalência funcional, mapear comportamento e casos de teste correspondentes; declarar lacunas e não usar igualdade nominal como prova.

## Fontes locais do Borizon usadas na classificação

- [server-plugins/README.md](../server-plugins/README.md)
- [server-plugins/api/src/lib.rs](../server-plugins/api/src/lib.rs)
- [server-plugins/loader/src/lib.rs](../server-plugins/loader/src/lib.rs)
- [crates/horizon-client/src/game/server_plugins.rs](../crates/horizon-client/src/game/server_plugins.rs)
- [crates/horizon-client/src/game/native_server.rs](../crates/horizon-client/src/game/native_server.rs)

## Referência Paper usada nos resultados fornecidos

- [`gradle.properties` no Paper 26.3](https://github.com/PaperMC/Paper/blob/1eda666f33904ba17fd5c14973a3ea62eca10b0c/gradle.properties)
- [Javadocs Paper API 26.3 — índice de pacotes](https://jd.papermc.io/paper/26.3/allpackages-index.html)
- [Paper API source tree no commit fixado](https://github.com/PaperMC/Paper/tree/1eda666f33904ba17fd5c14973a3ea62eca10b0c/paper-api/src/main/java)
- [Paper plugin development overview](https://docs.papermc.io/paper/dev/how-do-plugins-work/)

O clone local de referência fica em `/home/ubuntu/Paper`. Este roadmap acompanha capacidades por domínio e classes representativas, sem copiar código upstream; consulte os pacotes/Javadocs 26.3 antes de abrir tarefas method-by-method.
- [x] SDK v3 oferece `server_maintenance_status()`, `begin_server_maintenance()` e `end_server_maintenance()` com estados/contagens tipados, kick imediato ao iniciar e bloqueio enquanto há sessões ao encerrar; cobertos por testes de contrato e rejeições.
- [x] SDK v3 oferece `world_dimensions() -> WorldDimensions` para consultar IDs existentes no save e dimensões carregadas, sem carregamento ou mutação; testes cobrem as duas listas, erro do host e resposta incompleta.
- [x] SDK v3 oferece `world_gamerule(dimension, rule) -> GameRuleValue` para leitura de regras vanilla em dimensões carregadas; bool e i32 são desserializados como variantes distintas e regra/dimensão desconhecida retorna rejeição tipada. Testes cobrem ambos os tipos, payload, rejeição e resposta incompatível. A API não carrega dimensões nem altera regras.

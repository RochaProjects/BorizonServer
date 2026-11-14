# Evolução da API de plugins para AuraEssentials

Este plano usa como referência o AuraEssentials Java/Paper em `CommandTpa`,
`CommandRtp`, `CommandBack`, `CommandAuction`, `CommandMundo`, `WorldManager`,
`HomeManager`, `WarpManager` e `HologramManager`. O objetivo é colocar regras e
configurações no plugin Rust e ampliar o Horizon somente com operações genéricas
de servidor.

## Direção

Manter `HostApiV3` e ampliar `call_json` com operações aditivas. Não trocar a
versão binária da ABI por cada operação JSON nova. Só criar uma nova ABI se o
layout C ou a forma dos callbacks mudar. Todo pedido continua com
`{ "operation": "...", "data": { ... } }`; toda resposta traz `ok` e um
código estável de erro. Chamadas que alteram mundo, inventário ou jogadores
executam no thread autoritativo do servidor. Trabalho de disco/generation que
pode demorar deve ser assíncrono e devolver um `job_id`.

Operações devem descrever capacidades (`player.teleport`, `world.dimension.create`,
`inventory.transaction`), nunca sistemas do AuraEssentials (`aura.home`,
`aura.auction`). O plugin escolhe comandos, permissões, cooldowns, textos,
políticas e arquivos em `plugins/AuraEssentials/`. O storage genérico existente
continua disponível para pedidos TPA, cooldowns e configuração; dados grandes
de anúncio usam paginação/arquivos do próprio plugin em vez de um JSON gigante.

## Estado local desta implementação

Já há handlers para lookup/lista de jogadores online, teleporte para outro UUID,
posição completa no evento de morte, busca RTP limitada a chunks carregados,
criação de dimensão com presets, exclusão e reset global em duas etapas com
backup obrigatório, além de gamerules customizadas. O plugin registra e
encaminha homes/warps, TPA, back, RTP e comandos básicos `/mundo`. Dados e
arquivos do plugin usam `plugins/AuraEssentials/`, migrando do nome anterior sem
substituir arquivos existentes.

Esta implementação ainda não tem scheduler de ticks, geração assíncrona de
chunks, transações completas de inventário entre vários jogadores, mailbox,
CRUD persistente de displays/hologramas ou fake-player/NPC. Constantes desses
contratos não tornam as operações disponíveis: o host devolve
`unsupported_operation` até que exista um handler. `/rtp` só considera chunks
carregados; reset global exige nenhum jogador online. Ainda não há paridade
completa com Paper.

## O que já existe no Horizon

| Capacidade | Estado | Consequência |
|---|---|---|
| `player.get`, `player.teleport` | consulta e teleporte do próprio UUID do callback | homes, warps e back simples; não teleporta outro jogador por nome |
| `world.dimensions.list`, gamerules vanilla/custom e unload | dimensões vanilla e custom já registradas/carregáveis | não cria preset, não renomeia nem apaga dimensão |
| `plugin.storage.get/set/delete/list` | storage JSON por plugin | persistência de casas, warps e estado pequeno |
| `menu.open` e transação de moeda/itens simples | loja GUI e compra/venda básica | não captura nem preserva ItemStack com componentes/NBT completo |
| `entity.text_display.create`, move/remove | primitive de texto flutuante | falta identidade estável/CRUD persistente e atualização de grupo por linhas |
| Eventos de join/quit/death/teleporte | callbacks já existentes | death atualmente não fornece posição decimal, rotação nem posição anterior completa |

## Operações genéricas a acrescentar

### 1. Jogadores, movimento e tarefas

- `player.online.list` e `player.lookup`: UUID, nome e estado online; filtro/paginação
  para completar comandos sem expor objeto interno do engine.
- Estender `player.teleport` para aceitar um `target` UUID explícito, com validação
  de alvo online, posição finita, dimensão existente/carregada e retorno de erro
  preciso. O UUID do executor continua sendo o destino padrão quando `target` é omitido.
- Evento de movimento com posição `from` e `to`; evento de morte com posição,
  yaw/pitch e dimensão capturados antes do respawn. O plugin usa isso para impedir
  TPA em combate, cancelar aquecimento se o jogador se mover e salvar `/back`.
- `scheduler.schedule/cancel`: tarefas one-shot ou repetidas em ticks, entregues
  como evento do plugin com `task_id` e payload. Usado para expiração de TPA,
  warmup/cooldown, atualização de scoreboard e placeholders de holograma.
- `world.location.find_safe`: busca genérica e limitada por raio, centro,
  dimensão, borda, bloco de apoio, espaço livre e líquidos; pode carregar/gerar
  chunks fora do thread de jogo e devolver `job_id`. O plugin decide raio,
  cooldown, warmup e dimensões permitidas; não implementa geração de chunks.

### 2. Inventário e transações entre participantes

- `inventory.stack.inspect`: captura uma stack selecionada/da mão em formato
  canônico versionado, incluindo quantidade e componentes necessários para
  recriar o item. Não retornar ponteiros nem tipos Rust.
- `transaction.commit`: transação única sobre vários jogadores, moedas e stacks.
  Validar todos os participantes primeiro, reservar/retirar itens e saldos,
  aplicar os créditos/entregas e desfazer a operação inteira se uma etapa falhar.
  Usar journal durável para sobreviver a queda no meio da transação.
- `inventory.delivery.claim/list`: caixa postal do servidor para itens que não
  couberam no inventário, em vez de derrubar itens no chão como fallback.

Isso atende `/ah` e `/leilao` com anúncio de item real, compra, cancelamento,
pagamento ao vendedor e recuperação. Também é a base para `/trade` e `/trocar`:
os dois jogadores oferecem itens/dinheiro, ambos confirmam a mesma revisão da
oferta, e a troca só fecha quando a transação conjunta valida os dois inventários.

O plugin Java atual implementa leilão com lista limitada, item serializado,
pagamento e saldo pendente. Não há comando Java de troca direta entre jogadores
com o nome `/trade` ou `/trocar`; esse sistema será novo no AuraEssentials Horizon.

### 3. Dimensões e dados globais

- `world.dimension.create`: recebe ID namespaced validado e preset genérico
  (`overworld`, `nether`, `end`, `flat`), seed opcional, estruturas e propriedades
  de geração. Retorna `job_id`; criar registro, arquivos, gamerules e nível de
  maneira consistente. A geração de Nether/End/flat precisa existir de verdade no
  engine; não basta criar uma pasta com o nome da dimensão.
- `world.dimension.load/unload`: estado explícito e motivo de falha; proteger
  contra descarregar dimensão com jogadores/tickets. O carregamento explícito de
  dimensões customizadas ainda não está exposto como operação separada.
- `world.dimension.delete`: requer dimensão customizada, sem jogadores, unload
  bem-sucedido e confirmação preparada; backup antes de remover os dados.
- Usar ID namespaced estável como identidade interna. `/mundo renomear` muda o
  nome de exibição/alias. Renomear o ID e a pasta em disco é migração de dados e
  só deve ser oferecido depois de migrar homes, warps, gamerules e referências de
  todos os plugins.
- `world.player_data.reset.prepare/commit`: escolha global entre estatísticas,
  conquistas, dados de jogador ou combinação explícita. `prepare` informa o que
  será afetado e devolve token curto; `commit` exige token, jogadores offline,
  saves drenados e backup padrão. Limpar cache/sessões depois da operação. Não
  remover dados de moeda/plugin quando a seleção for só estatísticas/conquistas.

O Java `/mundo` inclui criação por ambiente/tipo/seed/estruturas, carregar,
descarregar, apagar com backup opcional, teleporte, gamerules, rename/import e
reset global de conquistas/estatísticas/dados de jogador. Para evitar perda
irreversível, o contrato Horizon deve exigir backup e confirmação para apagar ou
resetar, mesmo que o comando Java permita escolher sem backup.

### 4. Hologramas e NPCs

- Evoluir o display atual para operações de entidade com chave estável pertencente
  ao plugin: criar/atualizar/remover/listar por `owner` e `key`, com dimensão e
  posição. O servidor resolve IDs de runtime após reinício.
- Texto por componente vanilla, várias linhas, alinhamento, billboard, largura,
  fundo/transparência, sombra e quebra de linha. Persistência de título/descrição
  e estilo continua em `plugins/AuraEssentials/holograms.json`.
- Placeholder dinâmico pode ser renderizado pelo plugin via scheduler; o engine
  apenas entrega texto/componentes e envia as atualizações.
- NPC de modelo/skin precisa de API separada de fake-player: perfil/skin,
  spawn/tracking para todos os clientes, atualização, remoção e evento de
  interação. Um `text_display` ou manequim não substitui jogador fake.

## Mapa de comandos para um teste integrado

| Sistema do plugin | Operações necessárias |
|---|---|
| `/sethome`, `/home`, `/homes`, `/delhome`; `/warp`, `/warps`, `/setwarp`, `/delwarp` | storage, player snapshot, player teleport; já há suporte parcial |
| `/tpa`, `/tpahere`, `/aceitar`, `/negar` | online lookup/list, mensagem a qualquer UUID, teleport target, quit/move event e scheduler |
| `/back` | death location completo, teleport target-self, carregar dimensão e storage |
| `/rtp` | safe-location async, chunk generation/carga controlada, movimento/cancelamento e scheduler |
| `/ah`, `/leilao` | menu, stack inspect/capture, transação multijogador, mailbox, storage/config |
| `/trade`, `/trocar` | lookup de jogadores, GUI, revisão/confirmar oferta, transação de inventário e moeda entre os dois |
| `/mundo` | create/load/unload/delete, list/info, presets de geração, gamerule, alias, reset global preparado |
| `/holograma` | display entity upsert/list/remove, componentes multilinha, storage e scheduler |
| `/npc` modelo | API fake-player/profile/tracking/interação; etapa posterior aos hologramas |

## Uma entrega para testar em conjunto

1. Acrescentar todos os contratos JSON acima à API/documentação e implementar
   handlers no host em grupos independentes: jogadores/tarefas, mundo/dados,
   inventário/transações e displays.
2. Organizar `AuraEssentials` em módulos `teleport/`, `worlds/`, `auction/`,
   `trade/`, `holograms/`, `homes/`, `warps/` e `shop/`; cada módulo mantém seus
   arquivos de configuração em `plugins/AuraEssentials/` e registra seus comandos.
3. Implementar comandos e configuração em português de uma vez, com valores
   seguros por padrão e permissões explícitas. O teste integrado deve cobrir
   teleportes dentro/entre dimensões, reconexão, expiração/cancelamento de TPA,
   RTP sem local inseguro, compra/rollback, troca cheia/confirmada, reinício com
   hologramas e reset global com jogadores conectados/backup.
4. Validar primeiro no GitHub Actions o engine/fork e depois compilar apenas o
   `.so` localmente. Instalar no servidor isolado e testar comandos em jogo. Nada
   desta proposta foi enviado ao GitHub.

## O que não fazer

- Não colocar regras específicas (`tpa`, leilão, loja) dentro do engine.
- Não inventar `/trade`/`/trocar` como se existisse no Paper de referência.
- Não tratar uma pasta `dimensions/<namespace>/<id>` como dimensão criada se o
  gerador/preset não foi registrado e o mundo não abre com as regras escolhidas.
- Não apagar world/player data com jogadores conectados, estado em memória sem
  salvar, caminho não canônico ou sem backup/segunda confirmação.
- Não retornar sucesso antes de operação assíncrona concluir; retornar `job_id`
  e emitir evento de conclusão/falha.

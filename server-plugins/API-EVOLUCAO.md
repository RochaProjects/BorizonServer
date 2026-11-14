# API pública para AuraEssentials

O plugin mantém comandos, regras, permissões, textos e arquivos. O engine expõe
operações genéricas de servidor; a API não contém conceitos como loja Aura,
homes ou cargos específicos. A ABI continua `HostApiV3`: novas operações JSON
aditivas não exigem trocar a versão binária enquanto o layout C dos callbacks
permanecer igual.

## Estado implementado nesta árvore

- `/npc model`: importa geometria do pacote AuraNPC e renderiza cada cubo como
  `item_display` vanilla usando os modelos `aura:npc/...` do resource pack Java.
  Não cria zumbi, mob vanilla, armor stand nem fake-player. Os NPCs, modelos
  importados e hologramas são persistidos em `plugins/AuraEssentials/`.
- Interação com a peça do NPC identifica o UUID estável e pode executar o
  comando configurado como o jogador que clicou. NPC, holograma e teleporte
  guardam o ID completo da dimensão, portanto não assumem `minecraft:overworld`.
- `/holograma` persiste título, descrição, cor/fundo e posição. Texto aceita
  códigos Aura `&` e quebra de linha; o fundo pode ser separado por linha ou
  compartilhado no display multilinha. NPCs usam o mesmo conversor de texto
  para o nome flutuante.
- `/mundo importar <nome> <URL>` baixa em background, limita tamanho e caminhos
  ZIP, valida `level.dat`, generator e regiões, e importa a dimensão como
  `minecraft:<nome>`. O mundo de origem e seu `level.dat` não substituem o mundo
  ativo nem os dados de jogadores. As pastas de dimensão `region`, `entities`
  e `poi` são copiadas para o formato de dimensão do Horizon.
- `/entregarcargo <jogador> <cargo>` atribui cargo explicitamente; na primeira
  entrada o jogador recebe `membro`. OP e permissões não concedem cargo. A tab
  completa jogadores online e os nomes dos cargos. O chat mantém o nick sem
  saldo; a tab e nametag usam glyph + nick, e apenas a nametag recebe o saldo
  `aura:coins` em verde.
- Dimensões usam IDs namespaced persistentes. APIs para criar, carregar,
  descarregar, remover, teleportar e definir gamerules preservam a dimensão
  completa em homes, warps, NPCs e hologramas.

## Contratos genéricos relevantes

| Operação | Uso pelo plugin |
|---|---|
| `entity.display.upsert/delete` | Holograma e rótulo textual de NPC por UUID estável |
| `entity.item_display.upsert` | Cubo de modelo Java (`item_model`, transformação e dimensão) |
| Evento `PLAYER_INTERACT_ENTITY` | Identidade UUID da peça e jogador que interagiu |
| `player.command.execute` | Ação configurada do NPC, com contexto de permissão do jogador |
| `world.dimension.import` | Validar e registrar uma dimensão importada de staging |
| `player.display.set` | Componentes independentes de chat, tab e prefixo/sufixo da nametag |

## Limites conhecidos

- Os modelos 3D AuraNPC aparecem corretamente quando o cliente Java tem o
  resource pack AuraNPC correspondente ativo. O plugin verifica que os modelos
  Java referenciados existem no ZIP, mas não força o cliente a aceitar o pack.
- A conversão de geometria Bedrock para `item_display` segue os dados staged do
  AuraNPC; animações contínuas de bones não são reproduzidas por esses displays.
- `/rtp` trabalha dentro dos chunks já carregados. Geração assíncrona segura
  ainda não está disponível como serviço geral.
- Importação aceita ZIPs com `level.dat` e dados de dimensões vanilla. O nome
  escolhido cria `minecraft:<nome>` preservando posições dentro desse ID; não
  transforma estruturas de mundo de terceiros em um preset de geração.
- Plugins nativos são código confiável do servidor, não sandbox. `catch_unwind`
  captura panic Rust, mas não corrupção de memória, abort ou falha de processo.

## Compilação e publicação

O release do Horizon publica engine/servidor e mantém o plugin fora do pacote.
O `.so` é compilado separadamente e instalado em `plugins/`. Esta alteração
está apenas na árvore local: não houve build, teste, commit ou envio ao GitHub.

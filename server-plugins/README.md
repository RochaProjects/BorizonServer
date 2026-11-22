# Plugins de servidor Horizon (ABI v2/v3 e SDK Rust)

Plugins Linux são bibliotecas Rust `.so` carregadas no início do servidor a partir de `plugins/`. O host valida a ABI, chama os callbacks no thread do servidor e os mantém carregados até o processo terminar. Para atualizar um plugin, substitua a biblioteca e reinicie o servidor.

O contrato C público está em `api/`; o SDK Rust seguro fica em `sdk/`; o carregador e a fronteira FFI ficam em `loader/`. O SDK de plugins é separado dos trait mods internos compilados no executável e não é o `horizon-mod-sdk`. ABI v2 continua carregando sem alteração. Plugins novos devem usar o crate `horizon-server-plugin-sdk` sobre ABI v3: ele preserva o contrato binário existente e fornece callbacks Rust, identidade de jogador e chamadas JSON tipadas. A camada é segura para uso Rust sob o contrato do host, mas plugins nativos continuam sendo código confiável sem sandbox.

## 1. Crie o crate

Plugins independentes devem viver fora deste repositório e consumir o SDK público como dependência versionada. Crie um projeto Rust próprio com este `Cargo.toml`:

```toml
[package]
name = "meu-plugin"
version = "0.1.0"
edition = "2024"
license = "MIT"
rust-version = "1.95"

[lib]
crate-type = ["cdylib"]

[dependencies]
horizon-server-plugin-sdk = { path = "../BorizonServer/server-plugins/sdk" }
serde_json = "1.0"
```

O exemplo usa o checkout local do BorizonServer. Para plugins em repositórios externos, substitua `path` por uma dependência Git com `rev` fixado em um commit/tag que já contenha `server-plugins/sdk`; o hash antigo do exemplo ABI manual não inclui este crate. O código do plugin e seus arquivos de dados/configuração pertencem ao projeto do plugin.

## 2. Exemplo manual ABI v2 (baixo nível)

Este exemplo registra `/ola` e envia uma mensagem. Os callbacks usam `C-unwind` para que o host possa capturar panics Rust e desativar o plugin que falhou.

```rust
use horizon_server_plugin_api::{ABI_VERSION, HostApiV2, PluginV2};
use std::ffi::c_void;

static NAME: &[u8] = b"Meu plugin";

unsafe extern "C-unwind" fn create() -> *mut c_void {
    Box::into_raw(Box::new(())) as *mut c_void
}

unsafe extern "C-unwind" fn destroy(state: *mut c_void) {
    if !state.is_null() {
        unsafe { drop(Box::from_raw(state.cast::<()>())) };
    }
}

unsafe extern "C-unwind" fn register_commands(_: *mut c_void, host: *const HostApiV2) {
    let Some(host) = (unsafe { host.as_ref() }) else { return };
    if let Some(register) = host.register_command {
        let command = b"ola";
        unsafe { register(host.context, command.as_ptr(), command.len()) };
    }
}

unsafe extern "C-unwind" fn command(
    _: *mut c_void,
    host: *const HostApiV2,
    player_high: u64,
    player_low: u64,
    _: *const u8,
    _: usize,
) -> u8 {
    let Some(host) = (unsafe { host.as_ref() }) else { return 0 };
    if let Some(send) = host.send_message {
        let text = "Olá! Plugin carregado.";
        unsafe {
            send(
                host.context,
                player_high,
                player_low,
                text.as_ptr(),
                text.len(),
            );
        }
    }
    1
}

unsafe extern "C-unwind" fn menu_click(
    _: *mut c_void, _: *const HostApiV2, _: u64, _: u64, _: i32, _: i32, _: i32, _: i32,
) {}

static PLUGIN: PluginV2 = PluginV2 {
    abi_version: ABI_VERSION,
    name: NAME.as_ptr(),
    name_len: NAME.len(),
    create: Some(create),
    destroy: Some(destroy),
    register_commands: Some(register_commands),
    command: Some(command),
    menu_click: Some(menu_click),
    event: None,
};

#[unsafe(no_mangle)]
pub extern "C-unwind" fn horizon_server_plugin_v2() -> *const PluginV2 {
    &PLUGIN
}
```

The descriptor and its name must remain static while the library is loaded. Callback strings, menu item arrays, event payloads and the API pointer are borrowed only for the callback; copy data a plugin needs to keep. Return nonzero from `command` only when it handled the command. The v2 command helper remains public to all players; v3 command registration accepts a permission node and an operator-level fallback.

## 3. SDK Rust recomendado para plugins v3

Com o SDK, a macro gera o descriptor estático e os callbacks FFI. O plugin implementa apenas a trait Rust; a ponte JSON mantém a chamada para o host síncrona e limitada a 64 KiB.

```rust
use horizon_server_plugin_sdk::{
    declare_plugin, operation, CommandSpec, PlayerId, Plugin, PluginApi,
};

#[derive(Default)]
struct Ola;

impl Plugin for Ola {
    const NAME: &'static str = "Ola";
    const ID: &'static str = "ola";

    fn register_commands(&mut self, api: &PluginApi<'_>) {
        if let Err(error) = api.register_command(
            CommandSpec::new("ola")
                .default_level(0)
                .permission("ola.usar"),
        ) {
            let _ = api.call(
                PlayerId::NONE,
                operation::SERVER_LOG,
                &serde_json::json!({"level":"ERROR","message":error.to_string()}),
            );
        }
    }

    fn command(&mut self, api: &PluginApi<'_>, player: PlayerId, line: &str) -> bool {
        if line != "ola" {
            return false;
        }
        api.send_message(player, "Olá! Plugin carregado.").is_ok()
    }
}

declare_plugin!(Ola);
```

`NAME` é o nome exibido em `/plugins`; `ID` é um namespace estável de até 64 caracteres ASCII alfanuméricos, `_`, `-` ou `.` usado por storage e tarefas. `CommandSpec` registra o comando com nível op padrão (normalizado para 0–4) e nó de permissão opcional (1–128 bytes UTF-8); se houver nó, a decisão de permissão substitui o fallback de nível op (não é uma condição adicional combinada com ele). O helper retorna erros locais de argumento, transporte e rejeições explícitas do host. `PluginApi::send_message` envia texto simples ao jogador associado ao UUID do callback usando o componente estruturado do host. `PluginApi::get_online_player` consulta um `PlayerSnapshot` tipado; falha com `player_not_online` caso não haja sessão online. O snapshot contém identidade, nome, dimensão, posição, rotação, modo de jogo, nível de permissão e saúde, não uma referência mutável nem uma API geral de jogador. O callback de permissão usa `PermissionDecision::{Grant,Deny,Defer}`; `Defer` respeita o nível padrão do comando. Eventos chegam como JSON; tarefas `SERVER_TASK` só são entregues ao callback SDK cujo `ID` corresponde ao proprietário. O retorno só cancela eventos que o host já declara canceláveis. `PluginApi` não deve ser guardada nem usada em outra thread: todos os callbacks atuais rodam sincronamente no contexto do servidor. Mantenha callbacks curtos, evite I/O bloqueante e limite a frequência/tamanho de operações para proteger o orçamento do tick. A API pública aceita qualquer tipo `serde::Serialize`, mas não torna o JSON do host mais estável que a versão/contrato documentado.

`PluginApi::call()` é a forma simples e cria buffers de request e resposta para a chamada. Para operações frequentes, `call_with_response_buffer(player, operation, data, &mut response)` reutiliza a resposta e `call_with_buffers(player, operation, data, &mut request, &mut response)` reutiliza ambos os buffers `Vec<u8>` no estado do plugin. O helper de dois buffers limpa o request antes da serialização, limpa a resposta antes de entrar no callback host, serializa o envelope no request reutilizável e garante à resposta pelo menos 64 KiB, mantendo os limites do host. A chamada continua síncrona, a resposta decodificada é um `serde_json::Value` independente dos buffers, e nenhuma operação host ganha semântica de retry. Aquecer e reusar os buffers evita novas alocações dos bytes de request/response enquanto couberem na capacidade atual; serialização/deserialização de valores `serde` ainda pode alocar memória própria.

```rust
use horizon_server_plugin_sdk::{operation, PlayerId, PluginApi};

#[derive(Default)]
struct PluginState {
    request_buffer: Vec<u8>,
    response_buffer: Vec<u8>,
}

impl PluginState {
    fn log(&mut self, api: &PluginApi<'_>, message: &str) {
        let _ = api.call_with_buffers(
            PlayerId::NONE,
            operation::SERVER_LOG,
            &serde_json::json!({"level":"INFO", "message":message}),
            &mut self.request_buffer,
            &mut self.response_buffer,
        );
    }
}
```

`PluginApi::online_players()` retorna um snapshot tipado de cada jogador online; `find_online_player_by_name()` busca por nome sem diferenciar caixa ASCII e `find_online_player_by_id()` aceita um UUID `PlayerId`. A lista segue a ordem momentânea do host, não garantida entre chamadas. Os resultados são cópias de leitura, não referências vivas, e a lista inteira é limitada a 64 KiB pela ponte JSON. Reutilize snapshots e evite enumerar jogadores em todo tick. `online_player_stat()` lê uma estatística vanilla individual como `i32`, retorna zero para uma estatística ainda não registrada e rejeita localmente identificadores com mais de 128 bytes.
`online_player_stats()` retorna `PlayerStatistics` com mapas ordenados de tipo/ID e `DataVersion`; o snapshot completo é limitado a 64 KiB pela ponte JSON. Para polling frequente, prefira a consulta individual `online_player_stat()`.
`teleport_player(PlayerId, dimension, [x, y, z], yaw, pitch)` envia uma transferência tipada; coordenadas/ângulos não finitos são rejeitados localmente e `false` significa que o host não conseguiu executá-la ou enfileirá-la. A transferência entre dimensões pode ser enfileirada pelo host. `set_game_mode(PlayerId, GameMode)` aceita `Survival`, `Creative`, `Adventure` ou `Spectator` e retorna se o host mudou o modo; jogador offline ou mudança cancelada resulta em `false`.
`set_sidebar(PlayerId, &TextComponent, &[SidebarLine])` e `clear_sidebar(PlayerId)` gerenciam a sidebar do jogador do callback, com limite local de 15 linhas. Componentes aceitam texto com formatação legacy ou JSON Minecraft; a interpretação final continua no host.
`send_resource_pack(PlayerId, &ResourcePackSpec)` oferece um pack com UUID, URL, hash SHA-1 opcional, prompt e flag required; o SDK valida URL de 1–32.767 caracteres e hash de até 40 bytes. `remove_resource_pack(PlayerId, Some(uuid))` remove um pack, e `None` pede ao host remover todos os packs de servidor. O host continua responsável por enfileirar a operação e o retorno `false` indica falha.
`send_resource_packs(PlayerId, &[ResourcePackSpec])` oferece de 1 a 8 packs. O host envia em ordem; uma falha pode ocorrer depois de um prefixo já enviado, então `ResourcePackBatchResult` expõe quantos foram aceitos, `complete` e o erro reportado.
`open_inventory_view(viewer, &InventoryViewSpec)` abre uma view interativa do inventário `Player` ou `Ender` de outro jogador, online ou com dados salvos. O retorno é `Some(window_id)` ou `None`; o SDK valida layout, duplicatas, faixa de slots e capacidade da janela. Cliques usam as regras vanilla e podem mover itens do alvo; chame fora de um turno com estado de jogador trocado. Isto não é uma API geral de inventários.
`set_player_display(&PlayerDisplaySpec)` configura nome de chat/lista e, opcionalmente, prefixo/sufixo do nametag. O host sempre aplica os campos `chat` e `tab`, então `None` os limpa; `nametag: None` deixa o estilo existente intacto. O retorno expõe sucesso agregado e resultado por campo, pois pode haver aplicação parcial.
`open_menu(PlayerId, &MenuSpec)` abre um baú virtual com `MenuItemSpec`, nome/lore estilizados e componentes de item JSON. O SDK valida de 1 a 6 linhas, slots únicos, contagem de 1–64, até 54 entradas e IDs namespaced para components; o retorno é `Some(window_id)`/`None`. Os cliques chegam ao callback de menu já existente.
`create_text_display(&TextDisplayCreateSpec)` chama a operação existente `entity.text_display.create`, preserva suporte a texto legacy e JSON e retorna `EntityRef` (ID runtime escopado pela dimensão), que pode ser removido por `remove_entity`. Diferente de `upsert_text_display`, essa chamada não usa UUID estável nem substitui um display existente; os IDs runtime não são persistentes. Ambos validam posição, largura e view range; faça spawn fora de callbacks/ticks frequentes.

`PluginApi::world_dimensions()` retorna os IDs de todas as dimensões do save e das que estão carregadas em memória, sem carregá-las nem alterá-las. `create_custom_dimension(id, CustomDimensionPreset)` cria uma dimensão com preset `Overworld`, `Nether`, `End` ou `Flat`, retornando ID/preset/estado loaded tipados; o host valida IDs protegidos/duplicados e prepara o gerador no thread do servidor. A preparação faz I/O e abertura/fechamento de nível síncronos, então não a chame em cada tick nem em rotinas frequentes. A SDK ainda não encapsula importação de saves nem exclusão recursiva sem backup. `server_maintenance_status()` lê o estado do gate de manutenção e a contagem de sessões admitidas. `begin_server_maintenance()` recusa novas conexões e imediatamente pede kick de quem está conectado; `end_server_maintenance()` só reabre quando não há sessões, retornando `PlayersOnline(n)` caso contrário. Use begin com cautela, pois afeta os jogadores online. `world_gamerule(dimension, rule)` lê uma regra vanilla apenas em dimensão já carregada e retorna `GameRuleValue::Bool` ou `GameRuleValue::Int`; `set_world_gamerule` altera a regra tipada e exige dimensão carregada. Dimensão descarregada ou regra desconhecida resulta em rejeição/`false`, conforme o helper. `write_log(level, message)` usa os níveis `Info`, `Warn` e `Error` e aceita mensagens de 1 a 2.048 bytes UTF-8.

`prepare_player_data_reset(&[PlayerDataCategory])` é somente preview: retorna categorias normalizadas, paths afetados, janela de expiração (atualmente 60 s), `backup_required`, `requires_force` e um token privado de uso único. Revise o plano antes de prosseguir. `commit_player_data_reset(plan, backup)` consome o plano e torna explícita a decisão de manter/dispensar backup; prefira `backup: true`. O commit remove stats/advancements/playerdata de modo síncrono e pode copiar/remover muitos arquivos no thread do servidor, então não o execute durante ticks ou callbacks de alta frequência. `PlayerDataCategory::PlayerData` não pode ser aplicado enquanto houver jogadores online; `requires_force` sinaliza essa condição, mas o commit não força desconexões e retorna erro até o servidor estar vazio. O host pode rejeitar o token expirado ou retornar falha parcial; esse wrapper não torna a operação atômica.

`get_balance(PlayerId, currency)` lê unidades `i64` pela callback ABI v2 e diferencia moeda inválida, storage indisponível e callback ausente via `EconomyError`. `transact(PlayerId, currency, balance_delta, &[ItemDeltaSpec])` aplica um delta de saldo e até 16 deltas de itens simples; quantidade zero, IDs malformados e limites são rejeitados antes do FFI. Para troca apenas de itens use `currency: ""` e `balance_delta: 0`. O jogador precisa estar online e fora de um turno ativo com estado de jogador alterado; a operação ocorre sincronamente no thread do servidor, não faça I/O no callback. O host desfaz o saldo se a aplicação do inventário falhar durante processamento normal, mas arquivo de saldo e save do mundo não são um commit atômico frente a crash do processo/máquina. Os erros tipados distinguem saldo insuficiente, itens rejeitados, storage e request inválido. Esses helpers não implementam inspeção/delivery genéricos de stacks, metadata, receitas nem transações Paper completas.

Exemplo compacto de uma troca; o host revalida saldo e inventário no momento da chamada:

```rust
use horizon_server_plugin_sdk::{ItemDeltaSpec, PlayerId};

let player = PlayerId::from_u128(player_uuid);
let balance = api.get_balance(player, "aura:coins")?;
let cost = 25_i64;
if balance >= cost {
    let items = [ItemDeltaSpec::new("minecraft:diamond", -1)];
    api.transact(player, "aura:coins", -cost, &items)?;
}
```

Para reset, prepare e revise o plano antes do commit; esta forma mantém o backup:

```rust
use horizon_server_plugin_sdk::PlayerDataCategory;

let plan = api.prepare_player_data_reset(&[PlayerDataCategory::Stats])?;
let _affected_paths = plan.affected_paths();
let _outcome = api.commit_player_data_reset(plan, true)?;
```

`custom_dimension_gamerule(dimension, rule)` lê o valor efetivo bool/int de uma regra vanilla em dimensão custom existente, mesmo sem carregá-la. `set_custom_dimension_gamerule` grava override persistido e `clear_custom_dimension_gamerule` o remove para restaurar o padrão efetivo; ambas aceitam apenas o tipo bool/int correto do host e retornam `false` se dimensão ou regra forem inválidas.

`PluginApi::schedule_task` e `cancel_task` encapsulam tarefas por ticks; o payload de `SERVER_TASK` inclui `task_id`, `plugin_id` e `data`. `storage_get`, `storage_set`, `storage_delete` e `storage_list` oferecem valores `serde` tipados, chaves paginadas e namespace automático por `ID`. Os limites atuais do host são 16 KiB por payload de tarefa e 8 KiB por valor persistido. `storage_get` distingue chave ausente de JSON `null` nos servidores que retornam `found`; em versões anteriores de ABI v3 sem esse campo, `null` armazenado é ambíguo. O `ID` organiza tarefas e dados, mas não é boundary de segurança: qualquer biblioteca nativa carregada pode chamar a ponte JSON diretamente ou indicar outro namespace.
`decode_server_task_event(payload)` converte o evento JSON em `ServerTaskEvent`, rejeita payload malformado e retorna `None` para tarefa de outro `Plugin::ID`; o callback ABI e o formato genérico do evento permanecem inalterados.
Plugins que preferem despacho Rust tipado podem definir `const TYPED_NOTIFICATION_EVENTS: bool = true` e implementar `notification_event(&ServerNotificationEvent)`. Para kinds conhecidos e payload válido, o hook tipado substitui o callback JSON `event`; kinds desconhecidos ou payloads inválidos continuam pelo caminho JSON. O padrão é `false`, evitando decodificação específica adicional para plugins que usam somente JSON. Como no callback anterior, `true` só veta `BLOCK_BREAK` e `PLAYER_DEATH` no host atual.
`decode_server_notification_event(kind, payload)` adiciona views tipadas para join/quit (com última localização opcional), tentativa de morte, block break/place, menu fechado, chat (`ChatAfterBroadcast`), mudança de dimensão, teleport e status de resource pack, com campos conferidos nas emissões atuais. O evento mantém `status: i32` cru; `ResourcePackStatusCode::from_raw` converte os códigos atuais conhecidos (0–7) e retorna `None` para códigos futuros/desconhecidos, sem descartar o valor original. `BLOCK_BREAK` ocorre depois das verificações de permissão, mas antes da destruição; o retorno `true` do callback genérico pode manter o bloco. `PLAYER_DEATH` representa uma tentativa antes da morte: o retorno `true` pode mantê-lo vivo em 1 de saúde, conforme o motor. `BLOCK_PLACE` só é notificação depois de colocar o bloco e não cancela a ação. Chat é entregue em `after_broadcast`, portanto não pode ser cancelado ou reescrito; teleport é observacional e não veta a decisão do motor. Tipos/kinds sem decoder retornam `None`; payload conhecido malformado retorna `InvalidResponse`. `PLAYER_MOVE` e `PLAYER_INTERACT_ENTITY` não são considerados emitidos só por existirem constantes, e o decoder não altera o callback genérico nem a ABI. O dispatch e os decoders são síncronos no contexto do servidor; os modelos retornados copiam strings para `String`, portanto filtre previamente por `kind` se não precisar de decodificação e evite chamar operações adicionais de I/O no callback.
`ServerNotificationEvent::is_cancellable()` informa, para um evento já decodificado, se o host atual consome o `true` do callback (`BLOCK_BREAK` e `PLAYER_DEATH`); isso é verificado no dispatch, não inferido do nome do evento.
Se vários plugins retornarem `true`, o loader combina os resultados por OR; ainda não há prioridade de listener.

Exemplo de veto condicionado no callback existente (retorne `false` para continuar o fluxo nos demais eventos):
```rust
use horizon_server_plugin_sdk::{decode_server_notification_event, PluginApi, ServerNotificationEvent};
use serde_json::Value;

fn event(&mut self, _api: &PluginApi<'_>, kind: u32, payload: &Value) -> bool {
    match decode_server_notification_event(kind, payload) {
        Ok(Some(ServerNotificationEvent::BlockBreakAttempt { block_state_id, .. })) => {
            block_state_id == 1234 // exemplo: preserve esse state específico
        }
        Ok(Some(ServerNotificationEvent::PlayerDeathAttempt { damage_type, .. })) => {
            damage_type == "minecraft:fall" // exemplo: impedir morte por queda
        }
        _ => false,
    }
}
```

No `impl Plugin` para esse exemplo, habilite o opt-in e implemente o hook tipado:

```rust
const TYPED_NOTIFICATION_EVENTS: bool = true;

fn notification_event(
    &mut self,
    _api: &PluginApi<'_>,
    event: &ServerNotificationEvent,
) -> bool {
    matches!(
        event,
        ServerNotificationEvent::BlockBreakAttempt { block_state_id: 1234, .. }
    )
}
```

## 4. Build e instale

From the repository root:

```sh
cargo build --release -p meu-plugin
mkdir -p plugins
cp target/release/libmeu_plugin.so plugins/
```

The server and plugin must target the same OS, architecture and compatible native runtime. Linux is the packaged server target today. The loader also recognizes `.dylib` and `.dll` on their respective platforms.

## APIs v2 and v3

`HostApiV2` provides generic command registration, player messages, vanilla chest menus, menu clicks, player lifecycle and block-break events, balance reads, and transactions. Menu titles, names and lore accept vanilla `§` formatting codes; a color code explicitly clears inherited decorations such as italic. An empty item color leaves the component's color unset. ABI v3 menus also accept arbitrary item `components`, including Java resource-pack components such as `minecraft:item_model` and `minecraft:custom_model_data`.

ABI v3's `call_json` accepts an UTF-8 JSON object `{ "operation": "...", "data": { ... } }` and returns JSON. Requests and responses are capped at 64 KiB. UUIDs are the two `u64` callback arguments (high then low); world and dimension IDs use namespaced strings such as `minecraft:overworld` and `example:sky`.

| Operation | Purpose |
|---|---|
| `command.register` | Register usage, operator-level fallback, and optional named permission node during plugin startup. |
| `command.suggestions.add` | SDK exposes `PluginApi::add_command_suggestion(player, value)`, bounded to 1–256 bytes without control characters; the host accepts it only during the temporary suggestions context of a completion callback. |
| `chat.send` | Send a structured Minecraft text component to the callback player. |
| `menu.open` | Open a virtual chest with structured title, item names and lore; clicks go to `menu_click`. |
| `player.get` | Read online player's UUID, name, dimension, coordinates, rotation, mode, permission level and health. |
| `player.command.execute` | SDK `execute_player_command` dispatches up to 2,048 bytes under the online player's own permission context and returns ran/result/structured output. It may execute commands that mutate the world/server; call only when the player is authorized. The host can return `ran: false` outside a valid player command turn. |
| `player.lookup`, `player.online.list` | Find one online player by UUID/name or return snapshots of all online players. Listing is synchronous, host-ordered, and subject to the 64 KiB JSON response limit. |
| `player.teleport` | Teleport to coordinates in a built-in or valid custom dimension; pending cross-level moves are queued by the server. |
| `player.game_mode.set` | Change the player's vanilla game mode. |
| `player.stat.get` | Read one vanilla statistic as a signed 32-bit count; returns zero when the statistic is unrecorded and rejects identifiers longer than 128 bytes. |
| `resource_pack.send`, `resource_pack.send_batch`, `resource_pack.remove` | Send/remove Java Edition server resource packs; batch sends 1–8 pack pushes in order during one plugin API call, and pack status is delivered as an event. This does not turn separate packs into a single archive or make the current Play-stage send happen before spawn. |
| `world.dimensions.list` | List built-in/custom dimensions that exist in the save and `loaded_dimensions` currently open in memory; the Rust SDK exposes both lists without loading worlds. |
| `world.location.find_safe` | Find a safe position for an online player's current dimension by checking already-loaded chunks only. The SDK bounds radius to 8–512 blocks and attempts to 1–256; no chunks are loaded, and frequent tick polling is discouraged. |
| `world.dimension.create` / `world.dimension.load` / `world.dimension.unload` | SDK exposes `create_custom_dimension` with typed presets (`Overworld`, `Nether`, `End`, `Flat`), plus `load_dimension`/`unload_dimension` with typed load result and optional namespaced ticket owner. Creation prepares the level synchronously on the server thread; use rarely, not from recurring tick work. The current host pins chunk `[0,0]` for a load ticket; unload may return false while a player or another ticket still uses the dimension. Import/delete remain generic host operations, not SDK wrappers. |
| `world.gamerule.get/set` | Read/change a vanilla gamerule for a loaded built-in level. Rust SDK v3 exposes typed bool/int reads and `set_world_gamerule`; it does not load dimensions. |
| `world.custom_gamerule.get/set/clear` | Read, override or clear a gamerule override for an existing custom dimension. Rust SDK v3 exposes typed bool/int reads plus `set_custom_dimension_gamerule` and `clear_custom_dimension_gamerule`; these do not load the dimension. |
| `world.dimension.unload` | Save and unload an idle custom level when no player or ticket uses it. |
| `world.player_data.reset.prepare/commit` | SDK exposes typed `prepare_player_data_reset` and a consuming one-shot `commit_player_data_reset(plan, backup)`. Review the short-lived plan before commit; backup is preferred. Commit may perform large synchronous backup/deletion I/O and can fail partially. Player data cannot be committed while sessions remain online; `requires_force` is informational and does not kick players. |
| `server.maintenance.begin/status/end` | Temporarily refuse new logins, kick connected players, inspect admitted sessions, and reopen logins only after the server is empty. SDK exposes typed status/begin/end outcomes; begin is immediately disruptive and end reports the online count when blocked. |
| `scheduler.schedule/cancel` | Schedule a plugin callback (`event::SERVER_TASK`) after a bounded number of server ticks, optionally repeat it with `period_ticks`, or cancel it before the next run. The Rust SDK sends the task owner ID when cancelling; legacy raw requests without `plugin` retain global-ID cancellation behavior. |
| `server.log` | Write a bounded INFO, WARN, or ERROR line to the server log; the SDK's `write_log` accepts 1–2,048 UTF-8 bytes. |
| `scoreboard.sidebar.set/clear` | Show a title and up to 15 styled lines (optional right-side value) to one player. |
| `player.stat.get` | Read one vanilla statistic for an online player (`type` and `key`, such as `custom` + `player_kills`). Returns zero for an unrecorded statistic. |
| `player.stats.get` | Read all vanilla statistics for one online player as `PlayerStatistics` (category/key maps plus `DataVersion`); subject to the 64 KiB JSON response limit. Prefer `player.stat.get` for frequent lookups. |
| `entity.text_display.create` / `entity.display.upsert/delete` | `create_text_display` spawns a new vanilla text display and returns a runtime `EntityRef` scoped to its dimension, removable with `remove_entity`; it does not take a stable UUID or replace an existing display. Separately, `TextDisplaySpec`, `upsert_text_display` and `delete_text_display` use stable UUIDs; upsert removes the old display before spawning its replacement, requires a loaded dimension, and validates finite positions, width 1–2,000 and view range 0–16. |
| `entity.item_display.upsert` | Rust SDK exposes `ItemDisplaySpec`/`upsert_item_display` with stable UUID, item model resource key and bounded translation/quaternion/scale. Upsert replaces by removing then spawning, so it is not atomic and requires a loaded dimension. |
| `entity.move/remove` | Rust SDK v3 exposes `EntityRef`, `move_entity` and `remove_entity` for dimension-scoped runtime IDs. Movement applies only to non-simulated entities (displays, markers, interactions), not mobs/players; removal follows the host's native `/kill`-like behavior. |
| `entity.spawn`, `entity.remove_uuid` | Rust SDK v3 exposes `EntitySpawnSpec` for namespaced entity types, stable UUID, position, selected vanilla flags/name and optional `minecraft:interaction` hitbox settings. `spawn_entity` returns both UUID and dimension-scoped runtime ID; `remove_entity_by_uuid` removes by stable UUID. This is not a generic Bukkit entity handle or a full mob configuration API. |
| `plugin.storage.get/set/delete/list` | Persist JSON values under `plugins/<PluginId>/storage.json`, atomically, for homes, warps, requests, listings, and other plugin state. Values are capped at 8 KiB each and 4 MiB per plugin file. `get` returns `found` and `value`; existing `plugins/data/<PluginId>.json` data is moved on first access. |

Registered literal subcommands are suggested directly from the Brigadier tree. Every `word` or `word?` argument is marked `minecraft:ask_server`, so required and optional values such as IDs, names, presets, and roles reach the plugin's existing `command` callback for dynamic completion. The callback receives `COMMAND_COMPLETION_PREFIX` followed by the typed command line; `command_completion_input(line)` returns the borrowed input (including the partial final token) only for recognized requests up to 512 bytes. Add filtered values with `PluginApi::add_command_suggestion(player, value)` (or the generic operation). The host accepts additions only while the loader is querying suggestions; elsewhere it returns `suggestions_unavailable`. The helper validates each value, but filtering, sorting, deduplication and limiting the total candidate set remain plugin responsibilities. The loader does not execute the typed command for a completion request, and this keeps the C ABI layout unchanged.

```rust
if let Some(input) = horizon_server_plugin_sdk::command_completion_input(line) {
    let partial = input.rsplit(' ').next().unwrap_or("");
    for candidate in ["home", "spawn", "warp"] {
        if candidate.starts_with(partial) {
            let _ = api.add_command_suggestion(player, candidate);
        }
    }
}
```

Example request for a sidebar:

```json
{"operation":"scoreboard.sidebar.set","data":{"title":{"text":"Servidor","color":"gold"},"lines":[{"left":{"text":"Saldo: ","color":"gray"},"right":{"text":"120","color":"green"}}]}}
```

Periodic tasks can include `period_ticks` (1–120000) alongside `delay_ticks`; their `SERVER_TASK`
event repeats until cancelled. The Rust SDK associates them with `Plugin::ID` and routes them only to
that SDK callback; this is an ownership convention for trusted native plugins, not a sandbox. A
legacy raw `scheduler.cancel` call that omits `plugin` is still unscoped. Plugins can use
`player.stat.get` for vanilla kills, deaths and
playtime, and the generic economy API to read balances, then update their own sidebar configuration
and placeholders on the interval they choose.

Storage requests include the plugin's lowercase ID as `plugin` and a `key`; `set` also takes any JSON `value`. `list` may omit `key` or use it as a prefix, and supports `limit` (1–256) plus the previous response's `next` value as `after`. Keep each plugin on its own ID. Plugins are trusted native code, so this namespace is organization and collision avoidance, not a security boundary.

Events currently include player join/quit, block break/place, menu close, dimension change, teleport, death, chat, Java resource-pack status, and scheduled server tasks. ABI v3 callbacks receive `{player,name,dimension,window_id,position,data_id,data}`. Current place events and chat events are notifications after the action; block-break events can be canceled. For plugin features that need cancellable place, chat formatting/interception, movement/interaction protection, or entity damage events, the engine still needs a hook before the action is committed.

Dimension APIs expose built-in and custom dimensions, runtime creation with Overworld/Nether/End/flat presets, persistent per-custom-dimension seeds, loading/unloading, custom gamerules and direct deletion without a backup. Plugins can also open interactive views of another player's inventory or Ender Chest through `inventory.view.open`. Holograms use vanilla `text_display`. AuraNPC model NPCs use groups of vanilla `item_display` entities with the imported Java cube models from the AuraNPC resource pack; they are not mobs, mannequins or fake players. Java resource packs are sent by Horizon; Bedrock packs and Geyser extensions belong to the separate Velocity/Geyser proxy and are outside this backend API.

Transactions may name a shared currency ID (for example `aura:coins`), a signed balance delta, and signed plain-item deltas. To transact items without changing a balance, pass an empty currency ID and a zero balance delta. The host checks the balance and inventory, commits the balance file, then applies inventory changes on the server thread; it rolls the balance back if the inventory update fails. A transaction needs the player online and outside a player's active command turn, as menu clicks are. This is atomic during normal server processing, but the balance file and Minecraft's world save are separate files, so a process or machine crash at the precise moment of a purchase is not a cross-file atomic commit. Balance values are signed 64-bit integer units; the plugin that owns a currency defines its precision.

Balances are saved atomically in `plugins/.horizon/balances.json`, keyed by currency ID and player UUID. Existing version-1 balance files in `plugins/balances.json` and plugin folders are consolidated there on first startup. To seed a test account, stop the server and edit the file, using the player's 32-digit UUID without hyphens:

```json
{
  "version": 1,
  "accounts": [
    { "currency": "aura:coins", "player": "00000000000000000000000000000000", "units": 100000 },
    { "currency": "aura:credits", "player": "00000000000000000000000000000000", "units": 10000 }
  ]
}
```

Replace the example UUID with the real player UUID. The IDs are global so a separate economy plugin can share a currency intentionally. Native plugins are trusted code: any loaded plugin can call the generic transaction API for any currency ID. Treat plugin files as executable server code and restrict who can install them.

The host catches Rust panics at plugin callbacks and disables a plugin that panics. This does not contain memory corruption, segmentation faults, explicit aborts, or process-wide resource exhaustion; native plugins are not sandboxed. `C-unwind` is required for panic capture across the ABI.

The GitHub release workflow builds the server and generic plugin host; independent plugins are built and released by their own projects. Install a compatible `.so` under the server's `plugins/` directory. The loader preserves locally installed plugins when installing a server release.

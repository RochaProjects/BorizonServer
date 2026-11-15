# Plugins de servidor Horizon (ABI v2 e v3)

Plugins Linux são bibliotecas Rust `.so` carregadas no início do servidor a partir de `plugins/`. O host valida a ABI, chama os callbacks no thread do servidor e os mantém carregados até o processo terminar. Para atualizar um plugin, substitua a biblioteca e reinicie o servidor.

O contrato público está em `api/`; o carregador e a fronteira FFI ficam em `loader/`. Não há crate `horizon-mod-sdk` nem trait `ServerMod` neste checkout. Os trait mods documentados em `docs/native-mods.md` são compilados dentro de um executável; os plugins deste SDK são compilados separadamente. ABI v2 continua carregando sem alteração. Plugins novos devem usar ABI v3: ela preserva as funções comuns de v2 e acrescenta uma chamada JSON extensível, eventos com payload JSON e permissões nomeadas. Novas operações genéricas podem ser acrescentadas sem alterar o layout da ABI.

## 1. Crie o crate

Dentro de `server-plugins/`, crie `meu-plugin/Cargo.toml`:

```toml
[package]
name = "meu-plugin"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
horizon-server-plugin-api = { path = "../api" }
```

Adicione `"server-plugins/meu-plugin"` aos membros do workspace em `Cargo.toml`. Para um projeto fora deste repositório, publique ou espelhe o crate `horizon-server-plugin-api` e troque a dependência de caminho pela versão correspondente.

## 2. Implemente a entrada e os callbacks

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

## 3. Build and install

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
| `command.suggestions.add` | Add one dynamic completion while the loader is querying a plugin's command callback. |
| `chat.send` | Send a structured Minecraft text component to the callback player. |
| `menu.open` | Open a virtual chest with structured title, item names and lore; clicks go to `menu_click`. |
| `player.get` | Read online player's UUID, name, dimension, coordinates, rotation, mode, permission level and health. |
| `player.teleport` | Teleport to coordinates in a built-in or valid custom dimension; pending cross-level moves are queued by the server. |
| `player.game_mode.set` | Change the player's vanilla game mode. |
| `resource_pack.send`, `resource_pack.send_batch`, `resource_pack.remove` | Send/remove Java Edition server resource packs; batch sends 1–8 pack pushes in order during one plugin API call, and pack status is delivered as an event. This does not turn separate packs into a single archive or make the current Play-stage send happen before spawn. |
| `world.dimensions.list` | List built-in/custom dimensions that exist in the save and `loaded_dimensions` currently open in memory. |
| `world.dimension.load` / `world.dimension.unload` | Open or close an existing dimension. `load` can take a namespaced `owner` to keep a chunk ticket and keep the dimension ticking until `unload` releases that owner. |
| `world.gamerule.get/set` | Read/change a vanilla gamerule for a loaded built-in level. Vanilla rules remain shared according to Horizon's current vanilla level model. |
| `world.custom_gamerule.get/set/clear` | Read, override or clear a gamerule override for an existing custom dimension. |
| `world.dimension.unload` | Save and unload an idle custom level when no player or ticket uses it. |
| `world.player_data.reset.prepare/commit` | Prepare a short-lived global reset plan and then remove its selected player-data categories. Commit creates a backup by default; `backup: false` skips it. Player data cannot be committed while sessions remain online. |
| `server.maintenance.begin/status/end` | Temporarily refuse new logins, kick connected players, inspect admitted sessions, and reopen logins only after the server is empty. |
| `scheduler.schedule/cancel` | Schedule a plugin callback (`event::SERVER_TASK`) after a bounded number of server ticks, optionally repeat it with `period_ticks`, or cancel it before the next run. |
| `server.log` | Write a bounded INFO, WARN, or ERROR line to the server log. |
| `scoreboard.sidebar.set/clear` | Show a title and up to 15 styled lines (optional right-side value) to one player. |
| `player.stat.get` | Read one vanilla statistic for an online player (`type` and `key`, such as `custom` + `player_kills`). Returns zero for an unrecorded statistic. |
| `player.stats.get` | Read the complete vanilla statistics JSON for an online player. |
| `entity.text_display.create` / `entity.display.upsert` | Create or update a vanilla text display in a loaded custom or built-in level, with alignment, billboard, background, shadow, width and `view_range` settings; useful for holograms. |
| `entity.move/remove` | Move or remove an entity Horizon keeps as a non-simulated display/marker, by dimension and entity ID. |
| `plugin.storage.get/set/delete/list` | Persist JSON values under `plugins/<PluginId>/storage.json`, atomically, for homes, warps, requests, listings, and other plugin state. Values are capped at 8 KiB each and 4 MiB per plugin file. Existing `plugins/data/<PluginId>.json` data is moved on first access. |

Registered literal subcommands are suggested directly from the Brigadier tree. Every `word` or `word?` argument is marked `minecraft:ask_server`, so required and optional values such as IDs, names, presets, and roles reach the plugin's existing `command` callback for dynamic completion. The callback receives `COMMAND_COMPLETION_PREFIX` followed by the typed command line; detect and strip this public constant, then add filtered values with `command.suggestions.add`. The loader does not execute the typed command for a completion request, and this keeps the C ABI layout unchanged. Plugins should handle a completion request centrally by splitting completed words from the partial token, then return candidates for that command path; the shared AuraEssentials helper applies prefix filtering, sorting, and deduplication.

Example request for a sidebar:

```json
{"operation":"scoreboard.sidebar.set","data":{"title":{"text":"Servidor","color":"gold"},"lines":[{"left":{"text":"Saldo: ","color":"gray"},"right":{"text":"120","color":"green"}}]}}
```

Periodic tasks can include `period_ticks` (1–120000) alongside `delay_ticks`; their `SERVER_TASK`
event repeats until cancelled. The AuraEssentials sidebar uses a 40-tick interval (2 seconds),
reads vanilla kills, deaths and playtime through `player.stat.get`, and reads online balances through
the generic economy API. Its title, lines, refresh interval, currency IDs and glyph catalog are
editable in `plugins/AuraEssentials/scoreboard.json`; there are no scoreboard editing commands.
Placeholders include `%kills%`, `%kills_short%`, `%deaths%`, `%deaths_short%`, `%saldo%`,
`%saldo_short%`, `%credito%`, `%credito_short%`, `%online%`, `%playtime%`, `%player%`, `%world%`,
`%x%`, `%y%`, `%z%`, `%rank_glyph%`, and `%glyph_<nome>%` for the Java resource-pack glyph catalog, including `time_vermelho`, `time_azul` and `time_sem_time`.

Storage requests include the plugin's lowercase ID as `plugin` and a `key`; `set` also takes any JSON `value`. `list` may omit `key` or use it as a prefix, and supports `limit` (1–256) plus the previous response's `next` value as `after`. Keep each plugin on its own ID. Plugins are trusted native code, so this namespace is organization and collision avoidance, not a security boundary.

Events currently include player join/quit, block break/place, menu close, dimension change, teleport, death, chat, Java resource-pack status, and scheduled server tasks. ABI v3 callbacks receive `{player,name,dimension,window_id,position,data_id,data}`. Current place events and chat events are notifications after the action; block-break events can be canceled. For plugin features that need cancellable place, chat formatting/interception, movement/interaction protection, or entity damage events, the engine still needs a hook before the action is committed.

Dimension APIs expose built-in and custom dimensions, runtime creation with Overworld/Nether/End/flat presets, persistent per-custom-dimension seeds, loading/unloading, custom gamerules and direct deletion without a backup. Plugins can also open interactive views of another player's inventory or Ender Chest through `inventory.view.open`. Holograms use vanilla `text_display`. AuraNPC model NPCs use groups of vanilla `item_display` entities with the imported Java cube models from the AuraNPC resource pack; they are not mobs, mannequins or fake players. Java resource packs are sent by Horizon; Bedrock packs and Geyser extensions belong to the separate Velocity/Geyser proxy and are outside this backend API.

Transactions may name a shared currency ID (for example `aura:coins`), a signed balance delta, and signed plain-item deltas. To transact items without changing a balance, pass an empty currency ID and a zero balance delta. The host checks the balance and inventory, commits the balance file, then applies inventory changes on the server thread; it rolls the balance back if the inventory update fails. A transaction needs the player online and outside a player's active command turn, as menu clicks are. This is atomic during normal server processing, but the balance file and Minecraft's world save are separate files, so a process or machine crash at the precise moment of a purchase is not a cross-file atomic commit. Balance values are signed 64-bit integer units; the plugin that owns a currency defines its precision. `AuraEssentials` uses hundredths, so `100` means `1,00` moeda or Crédito.

Balances are saved atomically in `plugins/AuraEssentials/balances.json`, keyed by currency ID and player UUID. Existing `plugins/balances.json` files are moved there at startup. The former `AuraEssentialsHorizon` folder is migrated without overwriting files already at the new path. To seed a test account, stop the server and edit the file, using the player's 32-digit UUID without hyphens:

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

## AuraEssentials

`aura-loja/` is the legacy crate directory and Cargo package name; the plugin's public name in `/plugins` is `AuraEssentials`. Feature code is split under `aura-loja/src/`: `essentials/` for player commands, `shop/` for the chest shop, and `plugin_api.rs` for shared SDK calls. Persistent files live in `plugins/AuraEssentials/`; files from the former `AuraEssentialsHorizon/` folder migrate without overwriting newer files. The shop creates `config.json` and `shop.json` without overwriting files already present. `shop.json` contains the eight Portuguese categories and their products; edit a category array to add/remove products, set `icon`, `lore`, `itemModel` or `components`, and products are paginated automatically to fit the chest. `config.json` controls the menu title and the normal/VIP currency IDs. An operator can run `/loja recarregar` to validate and apply both files without restarting. Buying and selling use the generic transaction API. `aura:coins` and `aura:credits` start at zero; an economy/admin plugin or an operator-managed `balances.json` entry must credit a player before purchases succeed. Selling still requires the corresponding plain items in the online player's inventory.

Essentials currently exposes homes and warps (including delete/list), `/tpa`, `/tpahere`, `/aceitar`, `/negar`, `/back`, `/rtp`, `/invsee`, `/endersee`, and `/mundo` for listing, creating, loading/unloading, teleporting, custom gamerules, display aliases, URL import and global data reset. TPA requests expire after 30 seconds and are discarded on quit. RTP searches only already loaded chunks; it does not generate chunks asynchronously and reports failure when it cannot find a safe loaded location. Dimension IDs remain stable; built-in IDs stay `minecraft:overworld`, `minecraft:the_nether` and `minecraft:the_end`, while `/mundo criar <nome> <preset>` creates `minecraft:<nome-em-minusculas>` with its own persisted generation seed. `/mundo importar <nome> <URL>` downloads and validates a world archive before registering its vanilla Overworld data as `minecraft:<nome>`; it does not overwrite the active world's `level.dat` or player data. `/mundo definirprincipal <nome>` loads and keeps that dimension open at startup; it does not require a previously defined spawn. `/mundo tp <nome>` uses the saved spawn or enters a new dimension at a safe-height starting position; `/mundo definirspawn` saves its arrival point. `/mundo remover <nome>` immediately unloads and deletes a custom dimension without a backup. `/mundo reset conquistas` and `/mundo reset estatisticas` run immediately with a backup. `/mundo reset jogadores force` immediately blocks new logins, kicks active players, waits for their sessions to save and close, then deletes player data without creating a backup. The engine reset token remains internal to the scheduled operation; players do not need a token or second confirmation command. `/holograma` stores multiline text displays with configurable background/color. AuraNPC models import automatically on startup by default; `/npc modelos` lists them and `/npc criar` creates 3D item-display NPCs. The matching Java resource pack must be active for the custom item models to render. First-time players receive the `membro` role; `/entregarcargo` grants listed roles explicitly. Glyph + nickname appear in tab and nametag, with green balance only in the nametag; chat remains the player's plain nickname. Auctions and direct player trade remain future work.

Menu item models are Java-side item components. They can point at models/textures supplied by a Java resource pack; the plugin can request several Java packs in one API batch, but clients still receive and report each pack separately. Bedrock item textures are controlled by Geyser/Floodgate's proxy resource-pack setup, not by the Horizon backend plugin API.

The client supports Minecraft dialogs, and Horizon's protocol layer can encode `show_dialog` during both configuration and play. The public server-plugin API does not yet expose dialogs or a pre-placement login callback. Its resource-pack operation currently sends a Play-stage packet to an already placed/online player. Sending packs or dialogs before spawn requires a new configuration-stage hook and a way to hold the connection through the client's configuration exchange; the current login hook belongs to the native mod API and only allows accepting/refusing the join. So a plugin cannot yet guarantee a pre-spawn pack prompt or a dialog while holding a player in pre-login.

The old `native/auraessentials` mixin sources are legacy. The GitHub release workflow builds only the vanilla engine with the public plugin host, plus `horizon-server` and `horizon-prepare`; it does not build or package `aura-loja`. Build the plugin locally when needed and place its `.so` in the server's `plugins/` folder. The loader preserves that local plugin when installing a release that has no bundled plugin.

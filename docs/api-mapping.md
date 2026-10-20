# API MAPPING — los 64 comandos del bridge y su destino en Rust

> F3 de `repo-migrate`. **Propiedad del lote B-P2** (documento en curso).
> La lista de abajo es la de partida, verificada contra
> `tauri/src/lib/bridge.ts` (líneas 438-812) el 20/10/2026. P2 la completa con
> la firma Rust destino y el tipo de la respuesta.

## Por qué esta tabla es el gate de la UI

La UI nativa tiene que cubrir **64 comandos**. El crate GPUI que ya existe
(`desktop/`, hoy en `whatfast`) cubre **22**. Faltan 42, que es el 66 % de la
superficie: ese es el trabajo real de la fase de UI, y por eso el orden de los
lotes de UI no es "componente por componente" sino "dominio por dominio".

## Las tres categorías

| Categoría | Qué es | Cómo se porta |
|---|---|---|
| **RPC de protocolo** | viajan al sidecar por protobuf | el crate `whatfast-protocol` los expone como funciones Rust async; la UI los llama directo |
| **Comandos locales** | los resuelve el núcleo (staging, edición de imagen, aperturas, rutas) | funciones Rust del crate `whatfast-core` o del crate de la app |
| **Efectos de plataforma** | abrir archivos, portapapeles, notificaciones, selector de archivos | crate de plataforma con la implementación por SO |

## RPC de protocolo (los que ya existen en `proto/bridge.proto`)

| # | Comando TS | Destino Rust | Nota |
|---|---|---|---|
| 1 | `subscribe_backend` | `protocol::subscribe()` → `Receiver<FrontendEvent>` | stream de eventos, orden FIFO |
| 2 | `hello` | `protocol::hello()` | handshake; falla cerrado |
| 3 | `get_auth_state` | `protocol::auth_state()` | |
| 4 | `start_pairing` | `protocol::start_pairing()` | emite el QR por el stream |
| 5 | `reconnect` | `protocol::reconnect()` | **no cubierto hoy por GPUI** |
| 6 | `get_sync_status` | `protocol::sync_status()` | **no cubierto** |
| 7 | `request_older_history` | `protocol::request_older_history()` | **no cubierto** |
| 8 | `get_chat_history_coverage` | `protocol::history_coverage()` | **no cubierto** |
| 9 | `get_history_overview` | `protocol::history_overview()` | **no cubierto** |
| 10 | `get_integrity_status` | `protocol::integrity_status()` | **no cubierto** |
| 11 | `repair_local_cache` | `protocol::repair_cache()` | **no cubierto** |
| 12 | `list_chats` | `protocol::list_chats()` | |
| 13 | `list_messages` | `protocol::list_messages()` | |
| 14 | `open_message_window` | `protocol::open_message_window()` | |
| 15 | `list_messages_after` | `protocol::list_messages_after()` | |
| 16 | `search_local` | `protocol::search_local()` | |
| 17 | `open_contact` | `protocol::open_contact()` | |
| 18 | `list_messages_around` | `protocol::list_messages_around()` | |
| 19 | `send_text` | `protocol::send_text()` | |
| 20 | `forward_message` | `protocol::forward_message()` | **no cubierto** |
| 21 | `edit_message` | `protocol::edit_message()` | **no cubierto** |
| 22 | `delete_message` | `protocol::delete_message()` | **no cubierto** |
| 23 | `list_statuses` | `protocol::list_statuses()` | **no cubierto** |
| 24 | `subscribe_newsletter` | `protocol::subscribe_newsletter()` | **no cubierto** |
| 25 | `list_newsletters` | `protocol::list_newsletters()` | **no cubierto** |
| 26 | `list_newsletter_messages` | `protocol::list_newsletter_messages()` | **no cubierto** |
| 27 | `unsubscribe_newsletter` | `protocol::unsubscribe_newsletter()` | **no cubierto** |
| 28 | `set_chat_pin` | `protocol::set_chat_pin()` | **no cubierto** |
| 29-30 | `archive_chat` / `unarchive_chat` | `protocol::set_chat_archive()` | **no cubierto** (el RPC real es `set_chat_archive`) |
| 31-32 | `mute_chat` / `unmute_chat` | `protocol::set_chat_mute()` | **no cubierto** |
| 33-34 | `block_chat` / `unblock_chat` | `protocol::set_chat_block()` | **no cubierto** |
| 35 | `list_stickers` | `protocol::list_stickers()` | **no cubierto** |
| 36 | `send_sticker_from_library` | `protocol::send_sticker_from_library()` | **no cubierto** |
| 37 | `send_image` | `protocol::send_image()` | |
| 38 | `send_sticker` | `protocol::send_sticker()` | |
| 39 | `send_attachment` | `protocol::send_attachment()` | |
| 40 | `get_message_image` | `protocol::get_message_image()` | |
| 41 | `get_message_attachment` | `protocol::get_message_attachment()` | **no cubierto** |
| 42 | `open_media_path` | `platform::open_path()` | efecto de plataforma |
| 43 | `save_media_as` | `platform::save_as()` | efecto de plataforma |
| 44 | `mark_read` | `protocol::mark_read()` | |
| 45 | `get_chat_avatar` | `protocol::chat_avatar()` | |
| 46 | `send_reaction` | `protocol::send_reaction()` | |
| 47 | `get_chat_info` | `protocol::chat_info()` | |
| 48 | `get_participant_avatar` | `protocol::participant_avatar()` | |
| 49 | `repair_recent_reactions` | `protocol::repair_reactions()` | **no cubierto** |
| 50 | `set_typing` | `protocol::set_typing()` | |
| 51 | `create_poll` | `protocol::create_poll()` | **no cubierto** |
| 52 | `vote_poll` | `protocol::vote_poll()` | **no cubierto** |
| 53 | `set_message_pin` | `protocol::set_message_pin()` | **no cubierto** |
| 54 | `list_pinned_messages` | `protocol::list_pinned_messages()` | **no cubierto** |
| 55 | `logout` | `protocol::logout()` | borra bases y cachés |
| 56 | `restart_app` | `app::restart()` | efecto de plataforma |

## Comandos locales (los resuelve el núcleo, no el sidecar)

| # | Comando TS | Destino Rust | Nota |
|---|---|---|---|
| 57 | `stage_clipboard_image` | `core::stage_clipboard_image()` | |
| 58 | `discard_staged_image` | `core::discard_staged_image()` | |
| 59 | `stage_voice_note` | `core::stage_voice_note()` | |
| 60 | `stage_outgoing_media` | `core::stage_outgoing_media()` | **la regla de seguridad**: solo rutas de `staged-outgoing` llegan al backend |
| 61 | `edit_image` | `core::edit_image()` | recorte en píxeles de la imagen ya rotada |
| 62 | `list_desktop_applications` | `platform::desktop_applications()` | |
| 63 | `open_external_url` | `platform::open_url()` | |
| 64 | `reveal_media_path` | `platform::reveal_path()` | |

## Resumen de cobertura

| | cantidad |
|---|---|
| Comandos totales | 64 |
| Cubiertos por el crate GPUI existente | 22 |
| RPC de protocolo que **no** cubre hoy | 28 |
| Comandos locales que **no** cubre hoy | 2 (`stage_outgoing_media`, `edit_image`) |
| Efectos de plataforma pendientes | 6 |

## Reglas de la API en Rust

1. **Un comando = una función.** Sin `invoke("nombre", args)`: el nombre se
   compila.
2. **Respuesta = tipo, no `any`.** El origen devuelve
   `{ code, message, retryable }` en `BridgeError`; en Rust es
   `thiserror::Error` con un enum de códigos.
3. **Los eventos son un `Receiver`, no un `Channel` de callbacks.** El
   `Channel` de Tauri es FIFO con backpressure; el equivalente natural es
   `async_channel::Receiver<FrontendEvent>`.
4. **Los medios son rutas, no blobs.** El origen pasa `imagePath`; en Rust
   también: la UI no carga bytes en memoria, el backend los sirve desde el
   perfil.
5. **Nada de comandos que el renderer pueda usar para leer fuera del perfil.**
   La lista blanca de rutas se aplica en el crate, no en la UI.

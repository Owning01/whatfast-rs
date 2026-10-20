# API MAPPING - los comandos del bridge y su destino en Rust

> Contrato vivo. Las dos mitades: los **50 RPC** del sidecar (el contrato de
> `proto/bridge.proto`) y los **56 comandos** que la app web le pedia al
> nucleo de Tauri. Verificado el 20/10/2026 contra `proto/bridge.proto` y
> `tauri/src/lib/bridge.ts` del repositorio de origen.

## Por que esta tabla es el gate de la UI

La UI nativa tiene que cubrir **50 RPC**. El cliente GPUI que ya existe en el
repositorio de origen (`desktop/`, hoy `whatfast`) pide **22**. Faltan **28**:
ese es el trabajo real de la fase de interfaz.

La diferencia con el cliente web no es el protocolo: es el mismo. La
diferencia es que el cliente web tiene 56 comandos, de los cuales **6 no
viajan al sidecar** (los locales). El cliente GPUI los resuelve en el
proceso, en `whatfast-core`.

## Las tres categorias

| Categoria | Que es | Como se porta |
|---|---|---|
| **RPC de protocolo** | viajan al sidecar por protobuf | `whatfast-protocol` los expone como el enum `rpc_request::Request`; la UI los manda por el transporte |
| **Comandos locales** | los resuelve el proceso (edicion de imagen, recortes, rutas, permisos) | funciones de `whatfast-core` o del crate de la UI |
| **Efectos de plataforma** | abrir archivos, portapapeles, notificaciones, selector de archivos | la UI los pide directo a GPUI |

## Los 50 RPC del sidecar

﻿| # | Campo del proto | Peticion | Estado | Donde vive la logica |
|---|---|---|---|---|
| 1 | `hello` | Hello | cubierto | el handshake; `whatfast_protocol::transport::PROTOCOL_VERSION` |
| 2 | `get_auth_state` | GetAuthState | cubierto | el estado del enlace; `whatfast_model::model::Screen` |
| 3 | `start_pairing` | StartPairing | cubierto | el QR; `whatfast_model::model::Screen::Pairing` |
| 4 | `list_chats` | ListChats | cubierto | la lista; `whatfast_model::model::Store::upsert_chat` |
| 5 | `list_messages` | ListMessages | cubierto | la pagina; `Store::prepend_messages` |
| 6 | `send_text` | SendText | cubierto | el envio; `whatfast_core::send_idempotency` |
| 7 | `mark_read` | MarkRead | cubierto | eliado; `Store::mark_selected_chat_read_locally` |
| 8 | `logout` | Logout | cubierto | desloguear; `Store::clear_account_state` |
| 9 | `shutdown` | Shutdown | **falta** | apagar |
| 10 | `get_chat_avatar` | GetChatAvatar | cubierto | el avatar; `whatfast_core::participant_avatar_queue` |
| 11 | `send_reaction` | SendReaction | cubierto | la reaccion; `whatfast_core::send_idempotency` |
| 12 | `get_participant_avatar` | GetParticipantAvatar | cubierto | el avatar; `whatfast_core::participant_avatar_queue` |
| 13 | `repair_recent_reactions` | RepairRecentReactions | cubierto | la reparacion; `whatfast_core::send_idempotency` |
| 14 | `send_image` | SendImage | cubierto | el envio con medio; `whatfast_model::sticker::prepare` |
| 15 | `get_message_image` | GetMessageImage | cubierto | la imagen del mensaje; `whatfast_model::model::set_message_image_paths` |
| 16 | `search_local` | SearchLocal | cubierto | la busqueda; `whatfast_core::spotlight` |
| 17 | `open_contact` | OpenContact | cubierto | abrir un contacto |
| 18 | `list_messages_around` | ListMessagesAround | cubierto | el salto a un mensaje; `Store::replace_message_window` |
| 19 | `send_sticker` | SendSticker | cubierto | el sticker; `whatfast_model::sticker` |
| 20 | `open_message_window` | OpenMessageWindow | cubierto | el buffer de un chat; `Store::replace_message_window` |
| 21 | `list_messages_after` | ListMessagesAfter | cubierto | la cola; `Store::append_newer_messages` |
| 22 | `get_chat_info` | GetChatInfo | cubierto | la ficha; `whatfast_model::model::ChatInfoState` |
| 23 | `set_typing` | SetTyping | cubierto | el indicador; `whatfast_core::notification_routing` |
| 24 | `get_message_attachment` | GetMessageAttachment | **falta** | el adjunto |
| 25 | `send_attachment` | SendAttachment | **falta** | el adjunto |
| 26 | `list_stickers` | ListStickers | **falta** | la biblioteca de stickers |
| 27 | `send_sticker_from_library` | SendStickerFromLibrary | **falta** | el sticker de la biblioteca |
| 28 | `create_poll` | CreatePoll | **falta** | la encuesta |
| 29 | `vote_poll` | VotePoll | **falta** | el voto |
| 30 | `set_message_pin` | SetMessagePin | **falta** | fijar un mensaje |
| 31 | `list_pinned_messages` | ListPinnedMessages | **falta** | los fijados |
| 32 | `forward_message` | ForwardMessage | **falta** | reenviar |
| 33 | `edit_message` | EditMessage | **falta** | editar |
| 34 | `get_sync_status` | GetSyncStatus | **falta** | el progreso; `Store::apply_sync_progress` |
| 35 | `request_older_history` | RequestOlderHistory | **falta** | pedir historial viejo; `whatfast_core::sync` |
| 36 | `get_chat_history_coverage` | GetChatHistoryCoverage | **falta** | la cobertura; `whatfast_protocol::fake::fake_history_coverage` |
| 37 | `get_history_overview` | GetHistoryOverview | **falta** | el resumen del historial |
| 38 | `reconnect` | Reconnect | **falta** | reconectar |
| 39 | `get_integrity_status` | GetIntegrityStatus | **falta** | la integridad; `fake::fake_integrity_status` |
| 40 | `repair_local_cache` | RepairLocalCache | **falta** | reparar la cache local |
| 41 | `delete_message` | DeleteMessage | **falta** | borrar un mensaje |
| 42 | `list_statuses` | ListStatuses | **falta** | los estados |
| 43 | `subscribe_newsletter` | SubscribeNewsletter | **falta** | suscribirse a un newsletter |
| 44 | `list_newsletters` | ListNewsletters | **falta** | los newsletters |
| 45 | `list_newsletter_messages` | ListNewsletterMessages | **falta** | los mensajes del newsletter |
| 46 | `unsubscribe_newsletter` | UnsubscribeNewsletter | **falta** | darse de baja |
| 47 | `set_chat_pin` | SetChatPin | **falta** | fijar un chat |
| 48 | `set_chat_archive` | SetChatArchive | **falta** | archivar un chat |
| 49 | `set_chat_mute` | SetChatMute | **falta** | silenciar un chat |
| 50 | `set_chat_block` | SetChatBlock | **falta** | bloquear un chat |

CUBIERTOS: 22
FALTAN: 28

**22 cubiertos, 28 para escribir.** El lado del sidecar ya existe (es el mismo
codigo Go de siempre): lo que falta es la parte de la UI que los pide y lo
que hace con la respuesta.

### Los 28 que faltan, agrupados por dominio

| Dominio | RPC | Que hay que escribir |
|---|---|---|
| **Sincronizacion e historia** | `get_sync_status`, `request_older_history`, `get_chat_history_coverage`, `get_history_overview` | el progreso de la sincronizacion y el boton de "pedir mas historia" |
| **Estado y salud** | `get_integrity_status`, `repair_local_cache`, `reconnect` | el aviso de "la cache local esta da?ada" y el reintento |
| **Medios** | `send_attachment`, `get_message_attachment`, `list_stickers`, `send_sticker_from_library` | la bandeja de stickers y los adjuntos que no son imagen |
| **Mensajes** | `forward_message`, `edit_message`, `delete_message`, `set_message_pin`, `list_pinned_messages` | el menu de cada mensaje y la lista de fijados |
| **Encuestas** | `create_poll`, `vote_poll` | el compositor de encuestas y la votacion |
| **Newsletters** | `subscribe_newsletter`, `list_newsletters`, `list_newsletter_messages`, `unsubscribe_newsletter` | el panel de newsletters |
| **Estados** | `list_statuses` | la tira de estados |
| **Ajustes del chat** | `set_chat_pin`, `set_chat_archive`, `set_chat_mute`, `set_chat_block` | las acciones del menu de la ficha del chat |
| **Cierre** | `shutdown` | apagar el sidecar al salir de la app |

## Los 56 comandos del cliente web

De los 56, **50 son RPC** (la tabla de arriba, con nombres en camelCase) y
**6 son locales**:

| Comando local | Que hace | Donde vive en Rust |
|---|---|---|
| `openMediaPath` | abrir un archivo con el programa del sistema | la UI (GPUI abre con el shell del sistema) |
| `saveMediaAs` | guardar un medio en otra carpeta | la UI |
| `listStatuses` (local) | leer los estados del perfil | `whatfast-core` |
| `restartApp` | reiniciar la app | la UI |
| `subscribeBackend` | el stream de eventos | `whatfast_protocol::transport::EventSequence` |
| `shutdown` | apagar el sidecar | `transport::BackendClient::shutdown` |

Los comandos que el cliente web expone y **el sidecar no tiene** (los de la
app web, no del protocolo) son los de edicion de imagen: el recorte y la
rotacion. Esos ya estan portados: `whatfast_core::media_crop`.

## El mapa de los tipos

Los DTOs canonicos viven en `crates/whatfast-core/src/types.rs` y los tipos
del contrato en `crates/whatfast-protocol`. El detalle de que tipo va donde
esta en [`type-map.md`](type-map.md).

## Como se verifica esta tabla

`scripts/parity.py` compara los casos del oraculo (los 153 de
`tauri/tests/`) con los de Rust. La paridad actual es **153/153 (100 %)**:
el nucleo de dominio esta completo. Lo que esta tabla marca como "falta" es
la superficie de la **interfaz**, que no tiene caso de prueba en el oraculo
de la logica: su verificacion es visual, con el backend fake.

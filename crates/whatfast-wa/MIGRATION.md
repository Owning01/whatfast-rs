# Migración del sidecar: Go (whatsmeow) → Rust (whatsapp-rust)

Plan operativo completo. Un agente de IA que arranca de cero debe poder
ejecutarlo con solo este documento, `proto/bridge.proto` y el repo.

## 0. Objetivo y contexto

Este crate (`whatfast-wa`) reemplaza al sidecar Go de `backend/` manteniendo
el contrato de `proto/bridge.proto` byte a byte. La UI no cambia: mismo
framing por stdio (`--stdio --data-dir <dir> --log-file <f>`), mismas
respuestas. El motivo de la migración: **llamadas y videollamadas**. El stack
`voip` (señalización MLOW + motor de media + relay UDP) vive en whatsapp-rust
y exige que la sesión entera viva ahí. Sin doble vinculación: un solo
dispositivo, y el usuario re-vincula una vez el día del swap.

Dato clave (verificado 2026-10-23): whatsapp-rust tiene paridad funcional con
whatsmeow para TODO lo que el Go usa. La migración es un mapeo función por
función, no una reinvención. El árbol completo con `voip` compila en Windows
con rustc 1.98.

## 1. Reglas de oro

1. El contrato `proto/bridge.proto` no se rompe ni se amplía hasta la ola 5.
2. Lo no portado responde `not_implemented` con el nombre del RPC (ya así).
3. stdout del proceso: solo frames. Logs a stderr o `--log-file`.
4. Del fork crmne/zapfast y del fork de whatsapp-rust se **consume y se lee**;
   código propio se escribe acá. Si algún día se porta un bloque literal de
   zapfast (MIT), va con su aviso de copyright. No se disfraza procedencia.
5. La dependencia va pineada por revisión exacta en `Cargo.toml` +
   `Cargo.lock`. Re-pin deliberado, nunca flotante.
6. Credenciales solo en este proceso; a la UI nunca le llegan.
7. Puede haber otros agentes trabajando en el repo (ver `AGENTS.md` de
   opencode / `~/.zcode/AGENTS.md`): un escritor por archivo; antes de tocar
   `whatfast-protocol/src/fake.rs` o docs raíz, mirar si hay WIP ajeno.
8. Cada ola termina con `cargo check --workspace` en 0 y
   `cargo test -p whatfast-wa` en verde, más la prueba manual con la UI.
9. Commit al final de cada ola, mensaje en español como el resto del repo.
10. Antes de escribir un dato en `PROJECT_MAP.md`, verificarlo con un comando.

## 2. Dónde está todo

| Qué | Dónde |
|---|---|
| Contrato (fuente de verdad de la API) | `proto/bridge.proto` (v21, 50 RPCs, 15 eventos) |
| **La especificación funcional** | `backend/internal/wa/client.go` (~10.7k líneas: cada RPC implementado) |
| **El esquema de datos propio** | `backend/internal/store/store.go` (20 CREATE TABLE: chats, mensajes, reacciones, cobertura) |
| Semántica del handshake/errores | `backend/internal/app/server.go` (ya espejada en `src/server.rs`) |
| Fuente del fork whatsapp-rust | cache de cargo: `G:\Dev\cargo\git\checkouts\whatsapp-rust-*/94f7c13\` (o re-clonar crmne/whatsapp-rust a la misma rev) |
| Docs del fork para agentes | `<fork>/agent_docs/architecture.md`, `testing.md`, `codegen.md` y `<fork>/CLAUDE.md` |
| Eventos del protocolo | `<fork>/wacore/src/types/events.rs` (enum `EventKind`, línea ~240) |
| Referencia de uso (leer, no copiar) | github.com/crmne/zapfast — `src/backend/worker.rs` muestra cada API en uso real |
| Mapa RPC↔comandos del origen | `docs/api-mapping.md` (raíz del repo) |
| Mapa del repo | `PROJECT_MAP.md`; decisiones `RULEBOOK.md` |

## 3. Correspondencia whatsmeow → whatsapp-rust

Verificada contra `client.go` (superficie real usada) y el fuente del fork.
Los `?` significan: confirmar el nombre exacto en el archivo indicado antes
de programar.

| whatsmeow (Go) | whatsapp-rust (fork 94f7c13) |
|---|---|
| `NewClient(device, store)` + sqlstore | `SqliteStore::open(path)` → `Bot::builder().with_backend(store)` … `.build().await` → `bot.spawn()` → `handle.client()` |
| `GetQRChannel` / `QRChannelEvent*` | `client.pair_with_qr_code()` — ver `<fork>/src/pair.rs` (el QR llega por el event handler) |
| `AddEventHandler` + `events.*` | `.with_event_handler(impl EventHandler)`; llega `Arc<Event>` con `EventKind` — ver `<fork>/wacore/src/types/events.rs` y el `handle_wa_event` de zapfast |
| `wa.SendMessage` | `client.send_message(to, msg)` (builder) / `client.send_text(...)` — `<fork>/src/client/messaging.rs` |
| `wa.SendPeerMessage` (respuesta privada) | parámetro de reply del builder de send — ver `send/` y cómo zapfast hace private reply |
| `wa.GenerateMessageID` | la librería genera el ID al armar `MessageInfo` — verificar en messaging.rs |
| `wa.Upload` / `UploadReader` | `client.upload(...)` / `upload_stream(...)` — `<fork>/src/upload.rs` |
| `wa.DownloadToFile` / `DownloadMediaWithPathToFile` | `client.download(...)` / `download_to_writer(...)` — `<fork>/src/download.rs` |
| `wa.GetGroupInfo` | `<fork>/src/features/groups.rs` (`fetch_metadata`, etc.) |
| `wa.GetUserInfo` | `<fork>/src/features/contacts.rs` |
| `wa.GetProfilePictureInfo` | `<fork>/src/features/pictures.rs` |
| `wa.BuildPollCreation` / `BuildPollVote` / `DecryptPollVote` | `<fork>/src/features/polls.rs` (`CreatedPoll`, `decrypt_vote`, `aggregate_votes`) |
| `wa.BuildReaction` / `DecryptReaction` | `<fork>/src/features/reaction.rs` (`send_reaction`, `react`) |
| `wa.BuildEdit` | `client.edit_message(...)` — `<fork>/src/client/messaging.rs` + `features/message_edit.rs` |
| `wa.RevokeMessage` | delete-for-everyone — ver `chat_actions.rs` / `message/` |
| `wa.SendChatPresence` (tipeo) | `<fork>/src/features/presence.rs` + `chatstate.rs` |
| `wa.GetNewsletter*`, `NewsletterSubscribeLiveUpdates` | `<fork>/src/features/newsletter.rs` |
| `wa.BuildHistorySyncRequest` / `BuildUnavailableMessageRequest` | historial on-demand — `<fork>/src/history_sync.rs` + `features/group_history.rs`; uso real: zapfast worker ("on_demand") |
| `wa.IsConnected` / `Disconnect` / `Connect` | accessors + `connect` / `pause` / `shutdown` / `reconnect` — `<fork>/src/client/lifecycle.rs`, `accessors.rs` |
| `Logout` | `client.logout()` |
| eventos: Message/Receipt/ChatPresence/HistorySync/GroupInfo/AppState/Mute/Archive/Connected/Disconnected/LoggedOut/Contact/PushName/BusinessName/Picture/MarkChatAsRead/JoinedGroup | variantes de `EventKind`: message-batch (`MessageBatch`/`InboundMessage` en el prelude), receipt (ver enum), `ChatPresence`, `HistorySync`, `GroupUpdate`, `ArchiveUpdate` + dirty/label, `Connected`, `Disconnected`, `LoggedOut`, `ContactUpdated`, picture/pushname (ver enum), `MarkChatAsReadUpdate` |

Llamadas (nuevo, no existe en el Go): `IncomingCall`, `CallEndedElsewhere`,
`CallLog*` en `EventKind`; API `client.voip()` — ver `<fork>/src/voip/`
(facade.rs, mod.rs) y la sección 8.

## 4. Arquitectura del sidecar

```
crates/whatfast-wa/src/
  main.rs      ✅ proceso: flags, lock de perfil, hilos de pipe, runtime
  bridge.rs    ✅ framing servidor (4 bytes BE + Envelope)
  server.rs    ✅ handshake + despacho + secuencia de eventos
  session.rs   ⬜ store sqlite del dispositivo + Bot + ciclo de vida (ola 1)
  pairing.rs   ⬜ QR y eventos PairingQr (ola 1)
  events.rs    ⬜ EventKind del fork → BackendEvent del bridge (ola 1-2)
  db.rs        ⬜ rusqlite: esquema propio de chats/mensajes (ola 2)
  chats.rs     ⬜ ListChats + búsqueda + ajustes de chat (ola 2/4)
  messages.rs  ⬜ ListMessages*/ventanas/cobertura de historial (ola 2)
  send.rs      ⬜ texto/ediciones/reacciones/encuestas/pins (ola 3)
  media.rs     ⬜ caché de medios: upload/download, rutas (ola 3)
  meta.rs      ⬜ avatares, ChatInfo, contactos, estados, newsletters (ola 4)
  calls.rs     ⬜ señalización y plano de media (ola 5)
```

Datos: `<data-dir>/session.db` (store del dispositivo, lo abre la librería),
`<data-dir>/chats.db` (esquema propio, portar las 20 tablas de
`backend/internal/store/store.go` con rusqlite), `<data-dir>/media/` (caché),
`<data-dir>/backend.lock` (✅ ya). La idempotencia de envíos
(`client_message_id`) es tabla propia, como en el Go.

## 5. Olas, entregables y gates

### Ola 0 — Proceso y puente ✅ (hecha 2026-10-23)

Hecho: crate compila con el árbol completo del fork + `voip` en Windows;
handshake idéntico al Go; `not_implemented` por RPC; 3 tests de humo en
`tests/bridge_smoke.rs` (handshake+estado+shutdown, pre-Hello rechazado,
lock de perfil).

### Ola 1 — Sesión y vinculación

- RPCs: `StartPairing`, `GetAuthState` real, `Reconnect`, `Logout`,
  `GetSyncStatus` (fase inicial).
- Eventos: `PairingQr`, `ConnectionChanged`.
- Archivos nuevos: `session.rs`, `pairing.rs`, arranque del `events.rs`.
- Especificación: `client.go` — `StartPairing`, `OwnID`, `IsPaired`,
  `IsConnected`, `Reconnect`, `Logout`, y el mapeo `authConnectionState`.
- Gate: vincular un teléfono real por QR contra el sidecar Rust, ver el
  estado `CONNECTED` en la UI (con `WHATFAST_BACKEND`), cerrar, reabrir y
  que la sesión persista. La UI muestra el QR que hoy ya sabe dibujar.

### Ola 2 — Historial, chats y mensajes (la más pesada)

- RPCs: `ListChats`, `ListMessages`, `ListMessagesAfter`,
  `ListMessagesAround`, `OpenMessageWindow`, `SearchLocal`, `OpenContact`,
  `GetSyncStatus` completo, `RequestOlderHistory`,
  `GetChatHistoryCoverage`, `GetHistoryOverview`, `GetIntegrityStatus`,
  `RepairLocalCache`, `SetMessagePin`, `ListPinnedMessages`.
- Eventos: `SyncProgress`, `SyncStatusChanged`, `ChatUpserted`,
  `MessageUpserted`, `ReceiptUpdated`, `ChatMerged`, `StickersChanged`,
  `PinnedMessagesChanged`, `HistoryCoverageChanged`, `TypingChanged`.
- Archivos: `db.rs` (portar las 20 tablas), `chats.rs`, `messages.rs`.
- Especificación: `client.go` + `store.go` completos; el consumo de
  `HistorySync` del Go es el modelo del `EventKind::HistorySync` del fork.
- Gate: la UI contra el Rust carga la lista de chats y conversaciones con
  historial real, recibe mensajes en vivo, y el backfill de historial viejo
  anda al scrollear arriba.

### Ola 3 — Envíos y medios

- RPCs: `SendText`, `SendImage`, `SendAttachment`, `SendSticker`,
  `SendStickerFromLibrary`, `ListStickers`, `SendReaction`, `MarkRead`,
  `SetTyping`, `CreatePoll`, `VotePoll`, `EditMessage`, `DeleteMessage`,
  `ForwardMessage`, `GetMessageImage`, `GetMessageAttachment`.
- Eventos: `ReactionUpdated`, `RecentReactionsRepaired`.
- Archivos: `send.rs`, `media.rs`.
- Especificación: `client.go` (idempotencia por `client_message_id`,
  validaciones de tipo de archivo, caché de medios y sus rutas).
- Gate: mandar texto/foto/sticker/encuesta a un teléfono real desde la UI
  contra el Rust; reaccionar; editar; borrar para todos; descargar un medio
  viejo.

### Ola 4 — Metadatos

- RPCs: `GetChatAvatar`, `GetParticipantAvatar`, `GetChatInfo`,
  `ListStatuses`, `ListNewsletters`, `ListNewsletterMessages`,
  `SubscribeNewsletter`, `UnsubscribeNewsletter`, `SetChatPin`,
  `SetChatArchive`, `SetChatMute`, `SetChatBlock`.
- Especificación: `client.go` (bloqueos locales device-side, caché de
  avatares con `GetProfilePictureInfo` + fallbacks de privacidad).
- Gate: paridad visual de la UI contra el Go: avatares, info de grupo,
  estados, newsletters.

### Ola 5 — Llamadas y videollamadas (el objetivo)

Recién acá se amplía `bridge.proto` (versión v22):
- Eventos: `CallOffer` (entrante con chat/video), `CallUpdate`
  (fases: placing→dialing→ringing→connecting→active→ended/failed + motivo),
  `CallEnded`, `CallDevicesChanged`.
- RPCs: `CallPlace` (chat, video), `CallAccept`, `CallReject`, `CallHangup`,
  `CallMute`, `CallCameraOn/Off`, `ListCallDevices` (micós/parlantes/cámaras).
- Media por el bridge: audio i16 16 kHz mono en frames de 60 ms (32 KB/s por
  dirección); video H.264 Annex B en access units (unos Mbps). Captura y
  playback viven en la UI (cpal ya está en `recording.rs`, rodio en
  `audio.rs`); el motor vive en el sidecar (`client.voip()`).
- Cancelación de eco: `sonora` + `sonora-aec3` en el proceso UI (AEC3 de
  WebRTC portado a Rust). Cámara: nokhwa (`input-msmf` en Windows).
  Encode/decode: openh264. Tonos: portar la idea de `call_tones` de zapfast.
- UI: los glifos `WhatfastIcon::Phone`/`Video` ya existen (`icons.rs:438,454`)
  y el hueco del header está documentado en `render.rs:2373-2435`.
- Referencia de arquitectura (leer): `<fork>/src/voip/mod.rs` (dos mitades:
  runtime portable + transporte nativo) y zapfast `src/calls.rs` (fases,
  dispositivos, pipeline de video) — de nuevo: leer, no copiar.
- Gate: llamada de voz 1:1 con un teléfono real, luego videollamada, 30
  segundos estables cada una, con mute y cambio de parlante.

## 6. El día del swap

1. Ola 5 en verde → el instalador/packaging pasa a empaquetar
   `whatfast-wa.exe` junto a (o en lugar de) `whatfast-backend.exe`; la UI lo
   resuelve igual (`rpc.rs:1005 backend_executable`, env `WHATFAST_BACKEND`).
2. Primera ejecución tras actualizar: sesión incompatible → la UI ofrece el
   QR de re-vinculación (una sola vez en la vida del usuario).
3. El Go `backend/` se congela (no se borra) durante un par de releases como
   rollback; después se archiva.

## 7. Trampas ya encontradas

- `fs4::try_lock_exclusive` devuelve `Ok(false)` si el lock está tomado, no
  `Err` (`main.rs` ya lo maneja).
- prost expone el oneof del Envelope como campo `body`
  (`envelope::Body::Request/Response/Event`), no como campo por mensaje.
- El fork exige rustc reciente; el repo usa 1.98 y alcanza. Si aparecen
  errores de MSVC rarispos, verificar que la versión del toolchain no bajó.
- `whatfast-ui` NO lleva `rev` en gpui (ver su Cargo.toml): el pin vive en
  `Cargo.lock`. No imitar ese patrón acá: el fork de whatsapp-rust SÍ lleva
  rev explícito.
- Otro agente puede tener WIP en `fake.rs`/docs raíz; si no compila por
  causa ajena, corregir solo lo mecánico y avisar en `PROJECT_MEMORY.md`.

## 8. Verificación de cada ola

```
cargo check --workspace          # 0 errores
cargo test  -p whatfast-wa       # humo del bridge + lo nuevo de la ola
cargo clippy -p whatfast-wa      # sin warnings nuevos
set WHATFAST_BACKEND=target\debug\whatfast-wa.exe
cargo run   -p whatfast-ui       # prueba manual contra el sidecar Rust
```

Al cerrar cada ola: entrada append-only en `PROJECT_MEMORY.md`, estado en
este archivo (tabla de abajo), y commit.

## Estado

| Ola | Contenido | Estado |
|-----|-----------|--------|
| 0 | Proceso, framing, handshake, Shutdown, GetAuthState placeholder | ✅ 2026-10-23 |
| 1 | Sesión: store, QR (`PairingQrCode`), conexión, Reconnect, Logout | ✅ código 2026-10-23; gate: escanear QR con un teléfono |
| 2 | Historial, chats, mensajes, sync | 🔶 casi completa: + SearchLocal (grupos+mensajes), GetSyncStatus real, reacciones encriptadas (decrypt via secrets del store), SetChatPin/Archive/Mute/Block. Falta: coverage/integrity, journal de outbound, history on-demand |
| 3 | Envíos y medios | 🔶 base: SendText (idempotente), SendImage (upload+sniff), SendReaction, EditMessage, DeleteMessage (local+forall), MarkRead con receipts reales, SetTyping. Falta: stickers, adjuntos no-imagen, encuestas, forward |
| 4 | Metadatos y ajustes | ✅: contactos (push names de mensajes + history sync), títulos con la regla del Go, backfill de nombres de grupo, GetChatInfo (participantes/admins), GetChatAvatar/GetParticipantAvatar (caché determinista + marcador de privacidad), ListNewsletters local. VotePoll queda visible como not_implemented |
| 5 | Llamadas y videollamadas | ✅ (voz 1:1): sidecar completo + UI (barra de llamada con contestar/rechazar/mutear/cortar, botón Phone en el header, captura cpal con remuestreo a 16 kHz, playout rodio, pump de frames a 20 ms). El video queda para la siguiente etapa; sin AEC (usar auriculares) |

Pruebas: `cargo test -p whatfast-wa` → 9/9 (4 smoke del bridge + 5 del store).
El crate es lib+bin: los tests de integración ejercitan la librería directo.

# Spike B-R1 — ¿existe una librería Rust que reemplace al sidecar Go (`whatsmeow`)?

> Lote B-R1 de `PLAN.md`. Entregable de texto. **No se escribió código de producción.**
> Fecha de la investigación: **20/10/2026**. Todas las fuentes se consultaron ese día.
> Oráculo (solo lectura): `G:\Proyectos\16-whatsapp2`.

---

## 1. Veredicto

**Serviría parcialmente, y el gate de B-R1 no pasa: el sidecar Go se queda.**
La capa de protocolo está cubierta al 100 % por `whatsapp-rust` 0.7.0 (upstream, 48 de 50 RPC quedan cubiertas o son triviales; 2 exigen construir encima), pero **no hay forma de reusar la sesión ya emparejada**: el esquema SQLite es incompatible con el de `whatsmeow` y no existe importador, así que un usuario emparejado tendría que volver a escanear el QR.
Aun así, reescribir el sidecar en Rust son 13.526 líneas de Go que **no** mueven la meta de RAM (31,8 MB de 120 MB; el 85 % de la RAM es el WebView2), mientras que sí rompen el requisito más duro del charter.

---

## 2. Tabla comparativa de los candidatos

Dimensiones pedidas: (1) madurez, (2) cobertura de protocolo, (3) contrato exacto de Whatfast, (4) almacenamiento y reanudación de sesión, (5) licencia, (6) vía de salida.

| | **`whatsapp-rust` 0.7.0** (upstream, `oxidezap/whatsapp-rust`) | **`wa-rs` 0.2.0** (fork, `homun-app/wa-rs`) | **`whatsmeow` (Go, sidecar actual)** |
|---|---|---|---|
| **1. Madurez** | Último release **0.7.0, 07/08/2026**; 10 versiones desde 07/10/2025 [C-wr]. **1.386 PRs mergeadas**, 5 abiertas, 1 issue abierta, ~1.923 commits, 35+ contributors (top: `jlucaso1` 1.692 commits) [G-wr-pr]. **928 estrellas**, 152 forks [G-wr]. Commit más reciente **20/10/2026** (el mismo día de este spike) [G-wr-commits]. MSRV declarado **1.94**, edition 2024 [C-wr]. `rust-toolchain.toml` del repo fija `nightly-2026-06-16` para desarrollo [R-toolchain]. CI: 14 workflows, incluidos `miri`, `supply-chain`, `binary-size`, `e2e`, `signal-durability-nightly`, `consumers` [T-wr]. Dependencias: rangos `^` mayormente, **4 pines exactos**, 42 `workspace = true`, `Cargo.lock` commiteado (155 KB) [R-wr-cargo, R-wr-lock]. **3 dependientes** en crates.io [C-wr-rev] | **Una sola versión: 0.2.0, 17/02/2026** [C-wa-rs]. **0 PRs, 0 issues, 0 releases, 0 tags**; 26 commits de 1 contributor (`fabiocantone`) [G-wa-rs, G-wa-rs-pr]. **8 estrellas**, 3 forks, 0 issues [G-wa-rs]. Commit más reciente **14/10/2026** [G-wa-rs-commits]. **Sin MSRV declarado** (`rust_version: null`) [C-wa-rs]. El repo declara `fork: false`: es una copia, no un fork sincronizado [G-wa-rs]. Dependencias: 0 pines exactos, 19 `workspace`, `Cargo.lock` commiteado (84 KB) [R-wa-rs-cargo]. **9 dependientes**, todos de la familia "claw" (yantrikclaw, zeptoclaw, zeroclawlabs, gloamy, chat-system, kumiho-construct, velaclaw, vw-agent, revka) [C-wa-rs-rev] | `go.mau.fi/whatsmeow v0.0.0-20260718134955-fac667d55293` [O-go]. Upstream `tulir/whatsmeow`: **7.617 estrellas**, 83 issues abiertas, push 17/10/2026 [G-whatsmeow]. Licencia **MPL-2.0** [G-whatsmeow] |
| | **Veredicto de madurez: el más maduro de los dos.** Nota crítica: `wa-rs` está **8 meses detrás** del upstream y su propio README lo admite: *"This fork tracks it selectively"* [R-wa-rs] | | **Veredicto: maduro, pero MPL-2.0** (ver dimensión 5) | |
| **2. Cobertura de protocolo** | **Completa.** Noise: `wacore/noise` (handshake 52 KB, framing 34,9 KB, verificación de cadena de certificados) [T-wr]. Signal: implementación **propia** en `wacore-libsignal` (X3DH + Double Ratchet + sender keys; `session_cipher.rs` 119 KB, `sender_keys.rs` 100 KB, `state/session.rs` 126 KB) [T-wr, R-ls-mod]. App-state: `wacore/appstate` (`processor.rs` 111 KB, `hash.rs` 73,5 KB, `patch_decode.rs` 32,5 KB, `schemas.rs` 52,5 KB) con los schemas `Archive`, `Unarchive`, `Mute`, `Pin`, `Star`, `Label`, `FavoriteSticker` y los scopes `regular`, `regular_high`, `regular_low`, `critical_block`, `critical_unblock_low` [R-appstate]. Pairwise: QR + código de emparejamiento (`pair.rs`, `pair_code.rs`, `client_profile.rs`) [T-wr, D-client]. Newsletters: `iq/newsletter.rs` + API `list_subscribed`, `get_metadata_by_invite`, `join`, `leave`, `subscribe_live_updates` [D-newsletter]. Multimedia: `upload.rs`, `download.rs`, `media_retry.rs` + reupload con clave de medios [D-media, D-reupload]. Polls, reacciones, ediciones, revocaciones, pin/keep de mensajes, status, blocklist, presencia, chatstate, stickers, history sync [D-send, D-polls, D-status, D-blocking, D-presence, D-chatstate]. Extras que Whatfast no necesita: VoIP completo (Opus/MLOW/H.264) [D-voip] | **Copia de la arquitectura vieja del upstream (~0.4.x)**: el árbol del repo tiene `wacore/src/iq` y `wacore/src/stanza` [T-wa-rs], layout que el upstream ya no usa. Hereda el mismo set de features del upstream de febrero. Sin verificación de que cubra `newsletter`, `poll` o `voip` en la versión publicada | **Cubre todo lo que usa Whatfast hoy.** `client.go` llama 28 APIs de `whatsmeow` [O-client]: `SendMessage`, `SendPeerMessage`, `Upload`, `UploadReader`, `DownloadToFile`, `DownloadMediaWithPathToFile`, `RevokeMessage`, `BuildReaction`, `DecryptReaction`, `BuildPollCreation`, `BuildPollVote`, `DecryptPollVote`, `BuildEdit`, `BuildHistorySyncRequest`, `GetNewsletterInfoWithInvite`, `GetNewsletterMessages`, `NewsletterSubscribeLiveUpdates`, `GetGroupInfo`, `GetUserInfo`, `GetProfilePictureInfo`, `SendChatPresence`, `ParseWebMessage`, `GenerateMessageID` |
| **3. Contrato exacto (50 RPC de `bridge.proto`)** | **48 de 50 servibles** (38 sin trabajo extra de contrato, 10 con ajuste de contrato) y 2 a construir encima; el detalle RPC por RPC está en la sección 3. Las 17 RPC puramente locales y las 4 híbridas no las cubre ninguna librería de protocolo: son el store de producto de Whatfast [O-server] | Idéntico al upstream en lo que cubre, pero con 8 meses de atraso de protocolo | **Cubre las 29 RPC de protocolo**; las 17 puramente locales y las 4 híbridas son consultas al store de producto (`client.db`) o de ciclo de vida local [O-server] |
| **4. Almacenamiento y reanudación de sesión** | **Trae su propio store SQLite** (`whatsapp-rust-sqlite-storage`, Diesel + SQLite, 5.767 líneas, 34 migraciones) [C-wr-sq, T-wr]. **Esquema incompatible con `session.db` de `whatsmeow`**: tablas `identities`, `sessions`, `prekeys`, `sender_keys`, `app_state_keys`, `app_state_versions`, `app_state_mutation_macs`, `device`, `signed_prekeys` [R-mig] contra `whatsmeow_identity`, `whatsmeow_session`, `whatsmeow_pre_key`, `whatsmeow_sender_key`, `whatsmeow_app_state`, `whatsmeow_device`, `whatsmeow_legacy_session` de `whatsmeow`. **Cero menciones de "whatsmeow"** en los 433 KB de `sqlite_store.rs` [R-sqlstore]: **no hay importador**. El `SessionRecord` de `wacore-libsignal` usa su propio protobuf (`buffa`), no el de libsignal-go [R-ls-session]. El feature `legacy-session-interop` es para el formato **libsignal v1** y su doc dice *"Container decoding is intentionally out of scope"* [R-legacy] | Mismo esquema (es el mismo código, renombrado): `wa-rs-sqlite-storage` con las migraciones del upstream de 2025-08 a 2026-04 [T-wa-rs]. Mismo problema de importación | **`session.db` (whatsmeow) + `client.db` (producto, con FTS5)** [O-client:262, O-store:693]. La sesión **se reanuda sin re-loguear**: `container.GetFirstDevice(ctx)` + `device.Store.ID != nil` [O-client:291, O-client:525] |
| **5. Licencia** | **MIT** [C-wr, G-wr]. Compatible con `MIT OR Apache-2.0` de `whatfast-rs` [O-rs-cargo]. Sin copyleft | **MIT** [C-wa-rs]. Compatible, pero hay que conservar el aviso de copyright original: *"Original copyright (c) 2025 João Lucas de Oliveira Lopes"* [R-wa-rs] | **MPL-2.0** [G-whatsmeow]. El sidecar es un **proceso separado** que habla protobuf por stdio: no hay enlace, así que la copyleft por archivo no se extiende al proyecto. El riesgo real: si la reescritura en Rust **derivara** código de `whatsmeow`, MPL-2.0 aplicaría al archivo derivado. `whatsapp-rust` es implementación independiente (*"Inspired by whatsmeow and Baileys"*) [R-wr] → sin obligación MPL |
| **6. Vía de salida (coste de NO usarlo)** | Se paga el coste de reescribir el sidecar: **13.526 líneas** de Go de producción (+ 6.372 de tests en 13 archivos) [O-gocount] | Ídem, y encima arrastra un fork stagnant | **Se paga 31,8 MB de RAM** (7,4 % de los 432 MB medidos) [O-charter] + el toolchain Go en el build + un segundo lenguaje. **La RAM no es el factor decisivo**: el 85 % de la RAM es el WebView2 (366,8 de 432 MB) [O-charter] |

### Otros crates de WhatsApp en Rust que existen hoy (descartados)

| Crate | Versión / fecha | Descargas | Por qué no sirve |
|---|---|---|---|
| `wayrs` (`wayrs-client`, `wayrs-protocols`) | 1.3.1 (19/05/2025) / 0.14.11 (16/06/2025) | 156.784 / 136.315 | Sin actividad desde hace ~16 meses [C-search] |
| `wa-agent` (`lordmacu/whatsapp-rs`) | 0.1.7 (26/06/2026) | 2.929 | 0.1.x, 2.929 descargas totales [C-search] |
| `bailey` (`QaidVoid`) | 0.2.4 (19/10/2026) | 209 | 209 descargas [C-search] |
| `ruwa` | 1.0.0 (29/03/2026) | 179 | 179 descargas [C-search] |
| `ogawa-rs` | 0.4.0 (14/04/2023) | 10.267 | Muere en 2023 [C-search] |
| `corewa_rs` | 0.0.2 (07/10/2020) | 3.397 | Muere en 2020 [C-search] |
| `solwalrs`, `swaprs`, `walrs` | 2022–2024 | < 11.000 | Muertos [C-search] |
| `oxidezap/client` | creado 24/08/2026, push 16/10/2026 | 29 estrellas | **Cliente de escritorio en Rust sobre `whatsapp-rust`** — el precedente más cercano a Whatfast. MIT, pero tiene 2 meses de vida y 29 estrellas [G-client] |

**Conclusión de la dimensión 1 y 2:** hay una sola librería que sirve: `whatsapp-rust` upstream 0.7.0. `wa-rs` es una copia congelada de febrero de 2026 cuyo único delta declarado es *"Removed `#![feature(portable_simd)]` … enabling compilation on stable Rust"* [R-wa-rs]. El upstream **ya no usa `portable_simd`** en `wacore/appstate`, `wacore-libsignal`, `wacore` ni `waproto` [R-grep-simd], así que **la razón de ser del fork ya no aplica** y su costo es la congelación.

---

## 3. Mapa de cobertura RPC (los 50 de `proto/bridge.proto`)

Fuente del contrato: `G:\Proyectos\16-whatsapp2\proto\bridge.proto` (50 peticiones, 51 respuestas: 50 de carga útil + `RpcError`).
Fuente de la implementación actual: `G:\Proyectos\16-whatsapp2\backend\internal\app\server.go`, función `dispatch` (líneas 283-988).

**Leyenda**
- `soportado` — la parte de WhatsApp la da `whatsapp-rust` 0.7.0 y el resto es estado local de Whatfast.
- `parcial` — la librería da la operación base pero falta algo del contrato exacto.
- `no soportado` — la librería no da la capacidad; hay que construirla encima (con la librería como insumo).
- La columna "lógica" dice dónde vive el trabajo: `WhatsApp` (protocolo), `local` (store de producto `client.db` de Whatfast) o `híbrido`.

| # | RPC | Marca | Dónde vive la lógica | Motivo |
|---|---|---|---|---|
| 1 | `hello` | soportado | local | Handshake del bridge, no toca WhatsApp [O-server:285] |
| 2 | `get_auth_state` | soportado | WhatsApp | `IsPaired`/`IsConnected`/`OwnID` → store de device, `is_connected`, `logout` [D-client] |
| 3 | `start_pairing` | soportado | WhatsApp | QR + código de emparejamiento: `pair_with_code`, evento de QR [D-client] |
| 4 | `list_chats` | soportado | híbrido | Lista del store local + presentación (nombre, avatar) vía `contacts`/`groups` [D-contacts, D-groups] |
| 5 | `list_messages` | soportado | local | `store.MessagesBefore` [O-server:406] |
| 6 | `send_text` | **parcial** | WhatsApp | La librería da `send`/`SendOptions`/menciones, pero el contrato exige un `client_message_id` (UUID) persistido **antes** de enviar y nunca repetido, y además "reply privada" (`reply_to_chat_id` distinto del `chat_id`). No verificado que la API acepte un ID de mensaje elegido por el cliente |
| 7 | `mark_read` | soportado | híbrido | `mark_chat_as_read` [D-chat-actions]; el backend resuelve timestamp y contexto de grupo desde el mensaje almacenado [O-client] |
| 8 | `logout` | soportado | híbrido | `logout` → `LogoutReport` [D-client]; las 3 etapas (aislamiento de trabajos de medios → desvinculación remota → borrado local) se arman arriba [O-client:Logout] |
| 9 | `shutdown` | soportado | local | [O-server:982] |
| 10 | `get_chat_avatar` | soportado | WhatsApp | `pictures().lookup(ProfilePictureRequest)` [D-pictures] |
| 11 | `send_reaction` | soportado | WhatsApp | `send_reaction` [D-send] |
| 12 | `get_participant_avatar` | soportado | WhatsApp | [D-pictures] |
| 13 | `repair_recent_reactions` | **no soportado** | local + app-state | La librería no reenvía app-state ni lleva cursores. Es un trabajo propio de Whatfast sobre las tablas `reaction_repair_jobs` y `legacy_reaction_replays` [O-store:466, O-store:478]. El motor de app-state (`wacore-appstate`) sí sirve de insumo |
| 14 | `send_image` | soportado | WhatsApp | Upload + envío de imagen [D-media, D-upload] |
| 15 | `get_message_image` | soportado | híbrido | Descarga y descifrado [D-download]; la miniatura es local |
| 16 | `search_local` | soportado | híbrido | FTS5 de `client.db` + búsqueda de contactos de WhatsApp [O-store:693, D-contacts] |
| 17 | `open_contact` | soportado | WhatsApp | Contact lookup [D-contacts] |
| 18 | `list_messages_around` | soportado | local | `store.MessagesAround` [O-server:516] |
| 19 | `send_sticker` | **parcial** | WhatsApp | Hay `sticker_pack.rs` [T-wr], pero el contrato exige enviar un WebP completo preparado por el escritorio, con reply. No verificado |
| 20 | `open_message_window` | soportado | local | `store.InitialMessageWindow` [O-server:421] |
| 21 | `list_messages_after` | soportado | local | `store.MessagesAfter` [O-server:435] |
| 22 | `get_chat_info` | **parcial** | WhatsApp | La librería da metadata de grupo y `user info`, pero el contrato pide 13 campos (`about`, `verified_name`, `description`, `created_at_ms`, `created_by`, `participant_count`, `announce_only`, `locked`, `disappearing_timer_seconds`, `is_community`, `join_approval_required`): los de comunidad y de disappearing timer no están verificados en la API pública |
| 23 | `set_typing` | soportado | WhatsApp | Chatstate [D-chatstate] |
| 24 | `get_message_attachment` | soportado | WhatsApp | Descarga [D-download] |
| 25 | `send_attachment` | **parcial** | WhatsApp | Upload genérico [D-upload]; la nota de voz exige Ogg Opus válido y streaming sin cargar el archivo entero. No verificado |
| 26 | `list_stickers` | **parcial** | híbrido | La biblioteca da el app-state `favoriteSticker` [R-appstate]; el contrato arma el pack combinando favoritos + historial local y devuelve un ID estable por hash [O-proto:943-976]. La primera mitad es de la librería, la segunda es de Whatfast |
| 27 | `send_sticker_from_library` | **parcial** | WhatsApp | Reenviar un favorito por ID: depende de (26) |
| 28 | `create_poll` | soportado | WhatsApp | Polls [D-polls] |
| 29 | `vote_poll` | **parcial** | WhatsApp | `vote` + `decrypt_vote`; el secreto del creador lo debe persistir el cliente, la API da `decrypt_vote_raw` para frontera de interoperabilidad [D-polls] |
| 30 | `set_message_pin` | soportado | WhatsApp | `pin_message` / `unpin_message` / `keep_message` [D-send] |
| 31 | `list_pinned_messages` | soportado | local | `store.PinnedMessages` [O-server:899] |
| 32 | `forward_message` | **parcial** | WhatsApp | `forward_message` existe [D-current-api]; el contrato exige `client_message_id` idempotente: misma salvedad que (6) |
| 33 | `edit_message` | **parcial** | WhatsApp | `edit_message` + `EditOptions` + `with_secret` para ediciones cifradas [D-current-api]; el contrato devuelve el mensaje editado, con lo que hay que persistir el secret |
| 34 | `get_sync_status` | soportado | local | `store.sync_state` [O-store:408] |
| 35 | `request_older_history` | **no soportado** | local + app-state | Mecanismo propio de Whatfast: cursor de app-state, reintentos, tabla `history_coverage` [O-store:438]. La librería da history sync **entrante**, no "pedir más historia bajo demanda" con ese contrato |
| 36 | `get_chat_history_coverage` | soportado | local | `store.history_coverage` [O-store:438] |
| 37 | `get_history_overview` | soportado | local | [O-server:314] |
| 38 | `reconnect` | soportado | WhatsApp | `reconnect` / `reconnect_immediately` [D-client] |
| 39 | `get_integrity_status` | soportado | local | `store.storage_health` [O-store:409] |
| 40 | `repair_local_cache` | soportado | local | [O-server:329] |
| 41 | `delete_message` | **parcial** | WhatsApp | `revoke_message` (para todos) y `delete_message_for_me` (local) [D-send, D-chat-actions]; el contrato distingue los dos con `delete_for_everyone` |
| 42 | `list_statuses` | soportado | local | `store.StatusMessagesSince` sobre `status@broadcast` [O-server:601-605]; **no consulta a WhatsApp** |
| 43 | `subscribe_newsletter` | soportado | WhatsApp | `get_metadata_by_invite` + `join` [D-newsletter] |
| 44 | `list_newsletters` | soportado | WhatsApp | `list_subscribed` [D-newsletter] |
| 45 | `list_newsletter_messages` | soportado | WhatsApp | `NewsletterMessage` + `subscribe_live_updates` [D-newsletter] |
| 46 | `unsubscribe_newsletter` | soportado | WhatsApp | `leave` [D-newsletter] |
| 47 | `set_chat_pin` | soportado | local | `store.SetChatPinned` [O-server:671]. Sin app-state |
| 48 | `set_chat_archive` | soportado | local | `store.UpsertChatMetadata` [O-server:687]. Sin app-state |
| 49 | `set_chat_mute` | soportado | local | `store.SetChatMute` [O-server:706]. Sin app-state |
| 50 | `set_chat_block` | soportado | local | `store.SetChatBlocked` [O-server:721]. El proto lo dice: *"Device-local block list. The linked-device transport does not expose WhatsApp app-state, so the block is owned by this desktop profile."* [O-proto:644-646] |

**Recuento:** 38 `soportado` · 10 `parcial` · 2 `no soportado`.

**Lectura del mapa:**

1. **De los 50 RPC, 17 son puramente locales** (no hacen ninguna llamada a WhatsApp) y **4 son híbridos** (mezclan store local y protocolo). Las 17 puramente locales: `hello`, `shutdown`, `list_messages`, `open_message_window`, `list_messages_after`, `list_messages_around`, `list_pinned_messages`, `get_sync_status`, `get_chat_history_coverage`, `get_history_overview`, `get_integrity_status`, `repair_local_cache`, `list_statuses`, `set_chat_pin`, `set_chat_archive`, `set_chat_mute`, `set_chat_block`. Las 4 híbridas: `list_chats`, `search_local`, `request_older_history`, `repair_recent_reactions`. **Quedan 29 de protocolo.**
2. Las 17 puramente locales no le interestan a ninguna librería de protocolo: son el store de Whatfast, con su FTS5 y sus 25 tablas [O-store:295-693]. Cambiar de librería no las resuelve: hay que escribirlas en Rust igual.
3. **Los 10 `parcial` no son un hueco de protocolo.** Todos son ajustes de contrato: idempotencia por UUID del cliente, "reply privada", voz Opus, semántica exacta de los 13 campos de `get_chat_info`, y armado de packs de stickers. Son trabajo de integración, no de la librería.
4. **Los 2 `no soportado` (`repair_recent_reactions`, `request_older_history`) son mecanismos de Whatfast**, no de WhatsApp. El motor de app-state de la librería (`wacore-appstate`, 4.409 líneas) es exactamente el insumo que necesitan.

**Conclusión del mapa:** la librería cubre el 100 % de la capa de WhatsApp. El contrato de 50 RPC no se cumple "con la librería": se cumple con **la librería + un store de producto en Rust**, y ese store es el mismo trabajo que hay que hacer en cualquier escenario de migración del sidecar.

---

## 4. El riesgo del pairing

**Este es el requisito más duro del charter: un usuario ya emparejado no puede perder su sesión.**

**Lo que pasa hoy con `whatsmeow`:** el sidecar abre `session.db`, corre `container.Upgrade(ctx)`, y recupera el device con `container.GetFirstDevice(ctx)`. Si hay device, `IsPaired()` da `true` y el cliente reconecta sin pedir nada al usuario [O-client:262-296, O-client:525]. La clave de Noise, la identity key, el signed pre-key, la `adv_secret_key`, el `registration_id` y el JID de la cuenta viven en la fila `whatsmeow_device`.

**Lo que pasa si se cambia la librería:**

| Aspecto | `whatsmeow` (hoy) | `whatsapp-rust` 0.7.0 |
|---|---|---|
| Tabla de device | `whatsmeow_device` | `device` con columnas `lid, pn, registration_id, noise_key, identity_key, signed_pre_key, signed_pre_key_id, signed_pre_key_signature, adv_secret_key, account, push_name` [R-mig] |
| Formato de sesión Signal | `go.mau.fi/libsignal` v0.2.2 (protobuf `SessionRecord` v3 de libsignal) [O-go] | Protobuf propio sobre `buffa` (`wacore-libsignal`, `state/session.rs` 126 KB) [R-ls-session] |
| Tablas de identidad y app-state | `whatsmeow_identity`, `whatsmeow_session`, `whatsmeow_app_state`, `whatsmeow_legacy_session` | `identities`, `sessions`, `app_state_keys`, `app_state_versions`, `app_state_mutation_macs` [R-mig] |
| Importador de `session.db` | — | **No existe.** Cero menciones de "whatsmeow" en los 433.682 bytes de `sqlite_store.rs` [R-sqlstore]; cero paths con nombre "whatsmeow" o "import" en los 1.302 archivos del repo [T-wr] |
| Feature `legacy-session-interop` | — | Es para el `SessionRecord` **v1** de libsignal, y su doc dice *"Container decoding is intentionally out of scope. Callers provide owned bytes and typed roles"* [R-legacy]. El comentario del `Cargo.toml` lo define como *"Typed interop with the decoded legacy SessionRecord v1 model, for migrating externally produced auth state"*. **No trae el device row**, que es lo que el servidor reconoce como este dispositivo |

**Consecuencia:** con la librería, un usuario con la app ya emparejada **tiene que volver a emparejar** (escanear el QR otra vez o usar código de emparejamiento). WhatsApp admite varios dispositivos vinculados, así que técnicamente se puede: la sesión vieja queda en el teléfono y aparece otro dispositivo. Pero:

1. Es una **regresión de producto** en un cliente de escritorio ya instalado, y no hay forma de automatizarla.
2. El device row incluye la `noise_key` y la `identity_key`: **no son transferibles** porque la Noise handshake es la que autentica el dispositivo ante el servidor. Reconstruirlos a mano no es una opción.
3. El chart de `docs/api-mapping.md` y los 153 tests TS del oráculo no tienen caso de "usuario ya emparejado" salvo por el de `start_pairing` cuando `IsPaired()` es `true` [O-server:294-296]. Es decir: **el oráculo no cubre el escenario que se rompería**. El riesgo es invisible para el juez de paridad.

**Veredicto de esta sección: el riesgo de pairing es el que decide.** La librería no falla por cobertura del protocolo; falla por continuidad de sesión, y ese requisito no tiene atajo hoy.

---

## 5. Decisión y siguiente paso

### 5.1 Lo que decide el spike

**El gate de B-R1 ("¿`wa-rs` o `whatsapp-rust` cubren los 50 RPC que necesita el cliente?") se responde: no, en el sentido de que la librería no puede, por sí sola, reemplazar al sidecar Go**, porque:

- las 17 RPC puramente locales y las 4 híbridas no las cubre ninguna librería de protocolo, y
- la sesión ya emparejada no se puede migrar.

**Consecuencia: se aplica la salida prevista en el charter.** El punto 1 de "Qué NO se migra" dice: *"Si ninguno cubre lo que necesita el cliente, el sidecar Go (`whatsmeow`) **se queda** y el trabajo termina en 'UI en Rust, protocolo en Go'. Eso es un outcome válido, no un fracaso: el ahorro de RAM (el WebView2) ya está hecho."* [O-charter:33-38]

**Pero la decisión no es binaria entre "reescribir el sidecar" y "no hacer nada":**

| Opción | Qué significa | Coste | Riesgo |
|---|---|---|---|
| **A. Sidecar Go se queda** (la del charter) | `whatfast-ui` (GPUI) habla el framing protobuf sobre stdio con el sidecar Go que ya existe | 0 líneas nuevas de protocolo; F4 no cambia de arquitectura (el transporte sobre stdio ya es el que usa Tauri) [O-plan:70-77] | Toolchain Go en el build; 31,8 MB de RAM extra; dos lenguajes |
| **B. Sidecar en Rust con `whatsapp-rust`** | Reescribir `server.go` (1.593) + `client.go` (7.442) + `store.go` (3.296) + el resto (1.195) = **13.526 líneas** de Go, con 6.372 de tests Go como oráculo [O-gocount] | 13.526 líneas nuevas + los 6.372 de test; ~4-6 semanas extra sobre las 14 del plan | Sesión: re-emparejamiento forzoso; dependencia de un solo mantenedor; MSRV 1.94 |

### 5.2 Lo que NO cambia la decisión: la RAM

El baseline medido en esta máquina el 20/10/2026, con 10.000 chats cargados [O-charter:15-28]:

```
Árbol completo                      432,0 MB
├─ WebView2 (6 procesos)            366,8 MB   ← 85 %
├─ Sidecar Go (whatsmeow)            31,8 MB   ← 7,4 %
└─ Núcleo Rust (Tauri)               25,6 MB
Meta F8                             120,0 MB
```

- Quitar el WebView2 libera 366,8 MB. **Eso es lo que decide la meta.** Opciones A y B se llevan lo mismo.
- El sidecar Go son 31,8 MB: el 26 % de la meta. Sacarlo **no** es ganar 31,8 MB netos: en la opción B entra el stack de protocolo en Rust (Tokio multi-thread + Diesel/SQLite + `wacore` + `wacore-libsignal` + `wacore-appstate` + `waproto`, unas 230.000 líneas de Rust) dentro del proceso propio. **No hay ninguna medición pública de su RSS: no verificado.** Cualquier cifra en ese sentido sería inventada.
- El riesgo real de la meta es Skia (GPUI), que el charter ya marca como gap accepted [O-charter:67].

**La consecuencia concreta: la decisión entre A y B no se toma por la RAM.** Se toma por riesgo y esfuerzo.

### 5.3 Recomendación

**Opción A, y anotarlo como excepción explícita al objetivo "100 % Rust".** El razonamiento, en orden de peso:

1. **La sesión manda.** En la opción B, cada usuario instalado pierde su emparejamiento. Ninguna de las dos libraries resuelve eso hoy, y no es un trabajo de días: es trabajo sobre formatos criptográficos y el device row.
2. **El esfuerzo de la opción B excede el plan.** 13.526 líneas de Go de backend (más 6.372 de test) es un lote más grande que todo el Grupo 1 del plan (15 módulos de lógica pura) y comparable con la traducción del grupo 3.
3. **La opción B no compra RAM.** Ver 5.2.
4. **La opción A no bloquea nada de F4 a F8.** `crates/whatfast-protocol` habla el mismo framing protobuf sobre stdio con el sidecar Go que ya existe. El corte vertical de F4 (el sidebar) se puede hacer igual. La UI GPUI es lo que cambia, y eso es el objetivo real del charter.

**Consecuencias documentales para el orquestador** (no las aplico yo: el charter es del humano):

1. `CHARTER.md`, sección "Qué NO se migra" punto 1: cambiar *"Si ninguno cubre lo que necesita el cliente, el sidecar Go se queda"* por el resultado del spike, con la razón técnica (no es cobertura de protocolo; es continuidad de sesión + tamaño del store de producto).
2. `CHARTER.md`, objetivo: la frase *"cliente de escritorio de WhatsApp 100 % Rust"* pasa a *"UI 100 % Rust, protocolo en Go"* o similar. Hoy el charter promete algo que este spike muestra que no se puede cumplir sin re-emparejar a los usuarios.
3. `PLAN.md`, grupo 4 / lote B-R1: cerrar B-R1 con este documento como entregable y **abrir** un lote nuevo y opcional para la opción B, con su propio gate. No se ejecuta ahora.
4. `PROJECT_MAP.md`: una línea que diga que el backend es un proceso Go externo y que `crates/whatfast-protocol` es el cliente de ese contrato.

### 5.4 Decisiones que le corresponden al humano

| # | Decisión | Por qué no la toma un agente |
|---|---|---|
| D1 | ¿Se firma el charter con la excepción "UI en Rust, protocolo en Go"? | El objetivo medible del charter cambia. El charter es del humano (F0 está "pendiente de firma") |
| D2 | ¿Se acepta re-emparejar a los usuarios para poder reemplazar el sidecar? | Es una regresión de producto sobre usuarios instalados. Decisión de producto, no técnica |
| D3 | ¿Se abre el lote opcional de la opción B, con qué presupuesto y con qué gate? | Es un lote de ~14 semanas extra sobre las 14 del plan |
| D4 | Si se hace la opción B, ¿se acepta la dependencia de un solo mantenedor con MSRV 1.94 y toolchain de desarrollo nightly? | Es una decisión de riesgo de supply chain del proyecto |

**Si el humano quiere el dato antes de decidir (D2/D3), el spike mínimo de 2 días es este**, y no requiere tocar el oráculo:

1. Crear un crate de prueba aparte, fuera de `whatfast-rs`, con `whatsapp-rust = "0.7"` y `default-features = false, features = ["sqlite-storage", "tokio-transport", "ureq-client"]`.
2. Emparejar contra un número de prueba y medir el RSS del proceso en reposo con 10.000 chats cargados. Eso convierte "no verificado" en un número y responde D2 y la mitad de D3.
3. Con eso, probar si el `SessionRecord` de libsignal-go (el que está en `session.db`) se puede decodificar con `wacore-libsignal` a mano. Es el blocker real: si el formato se puede leer, la importación es un trabajo acotado; si no, D2 es inevitable.

### 5.5 Lo que este spike **no** verificó

- El **RSS** del stack de protocolo en Rust con 10.000 chats. No medido. No hay cifra pública.
- Si `wacore-libsignal` puede decodificar el `SessionRecord` v3 de `go.mau.fi/libsignal` que hoy está en `session.db`. No probado: los bytes difieren (protobuf propio sobre `buffa` vs. protobuf de libsignal-go).
- Si la API de envío acepta un **ID de mensaje elegido por el cliente** (la idempotencia de `client_message_id` que exigen 6 RPC). No verificado en la documentación pública.
- Si la API de envío soporta **"reply privada"** (`reply_to_chat_id` distinto del `chat_id`). No verificado.
- El envío de **WebP de sticker arbitrario** y de **notas de voz Opus** con las restricciones del contrato. No verificado.
- Si el upstream mantiene el ritmo de releases que tenía antes de 0.7.0 (07/08/2026): hay commits hasta el 20/10/2026 pero sin release nueva. No verificado si es trabajo de estabilización o si la release viene lento.
- La **frescura del protocolo**: `waproto/src/whatsapp.proto` (232 KB) y `waproto/api.snapshot` (9,2 MB) están commiteados y se regeneran con la herramienta `whatspec` de otro repo, con una dependencia git pineada `wa-store` en el workspace [T-wr, R-wr-cargo]. El CI tiene `waproto/tests/build_freshness.rs` y un commit reciente *"fix(waproto): keep the crate fresh after the first build (#1762)"*, así que hay automatismo. La consecuencia: **el protocolo de Rust puede quedar viejo respecto del de `whatsmeow` sin que nadie se entere hasta que WhatsApp rompe el cliente.**

---

## Anexo · Fuentes

Todas consultadas el **20/10/2026**.

### crates.io
| Clave | URL | Qué dio |
|---|---|---|
| C-wr | `https://crates.io/api/v1/crates/whatsapp-rust` | versión 0.7.0 (07/08/2026), MIT, MSRV 1.94, 146.955 descargas, 10 versiones, líneas por crate |
| C-wacore | `https://crates.io/api/v1/crates/wacore` | 122.249 líneas / 173 archivos, 151.093 descargas |
| C-wr-sq | `https://crates.io/api/v1/crates/whatsapp-rust-sqlite-storage` | 5.767 líneas / 6 archivos |
| C-wa-rs | `https://crates.io/api/v1/crates/wa-rs` | 0.2.0 (17/02/2026), MIT, `rust_version: null`, 142.051 descargas, 1 versión |
| C-wa-rs-family | `https://crates.io/api/v1/crates/{wa-rs-core,wa-rs-libsignal,wa-rs-noise,wa-rs-appstate,wa-rs-binary,wa-rs-proto,wa-rs-sqlite-storage,wa-rs-tokio-transport,wa-rs-ureq-http}` | líneas por crate (total familia ≈ 60.935) |
| C-wr-rev | `https://crates.io/api/v1/crates/whatsapp-rust/reverse_dependencies` | 3 dependientes |
| C-wa-rs-rev | `https://crates.io/api/v1/crates/wa-rs/reverse_dependencies` | 9 dependientes, familia "claw" |
| C-deps-wr | `https://crates.io/api/v1/crates/whatsapp-rust/0.7.0/dependencies` | 54 deps, ninguna git |
| C-search | `https://crates.io/api/v1/crates?q=whatsapp&sort=downloads` (y `q=wa-rs`, `q=baileys`) | censo de los otros crates |

### GitHub
| Clave | URL | Qué dio |
|---|---|---|
| G-wr | `https://api.github.com/repos/oxidezap/whatsapp-rust` | 928 estrellas, 152 forks, 6 issues abiertas (1 issue + 5 PRs), MIT, push 20/10/2026 |
| G-wr-pr | `https://api.github.com/search/issues?q=repo:oxidezap/whatsapp-rust+type:pr&is:merged` | 1.386 mergeadas / 5 abiertas / 1.553 totales |
| G-wr-commits | `https://api.github.com/repos/oxidezap/whatsapp-rust/commits` | último commit 20/10/2026 14:54:38Z, 1.923 commits, 35+ contributors |
| G-wa-rs | `https://api.github.com/repos/homun-app/wa-rs` | 8 estrellas, 3 forks, 0 issues, `fork: false`, push 14/10/2026 |
| G-wa-rs-pr | `https://api.github.com/search/issues?q=repo:homun-app/wa-rs+type:pr` | 0 PRs de cualquier estado |
| G-whatsmeow | `https://api.github.com/repos/tulir/whatsmeow` | 7.617 estrellas, 83 issues, MPL-2.0 |
| G-client | `https://api.github.com/repos/oxidezap/client` | cliente de escritorio Rust sobre la librería, 29 estrellas, creado 24/08/2026 |
| T-wr | `https://api.github.com/repos/oxidezap/whatsapp-rust/git/trees/main?recursive=1` | 1.302 archivos; árbol de `wacore/`, `waproto/`, `storages/`, 14 workflows |
| T-wa-rs | `https://api.github.com/repos/homun-app/wa-rs/git/trees/main?recursive=1` | 309 archivos; layout viejo del upstream |

### Código fuente (raw.githubusercontent.com)
| Clave | Archivo | Qué dio |
|---|---|---|
| R-wr | `README.md` (upstream) | capacidades declaradas; *"Inspired by whatsmeow and Baileys"* |
| R-wa-rs | `README.md` (wa-rs) | *"This fork tracks it selectively"*; *"Removed `#![feature(portable_simd)]`"*; aviso de copyright |
| R-wr-cargo | `Cargo.toml` (upstream) | 4 pines exactos, 42 `workspace`, `wa-store` git pineado, features `legacy-session-interop`, `voip`, `plugins` |
| R-wa-rs-cargo | `Cargo.toml` (wa-rs) | 0 pines exactos, 19 `workspace`, sin `rust-version` |
| R-toolchain | `rust-toolchain.toml` | `channel = "nightly-2026-06-16"` |
| R-mig | `storages/sqlite-storage/migrations/2025-08-14-035031_initial/up.sql` | esquema de tablas del store Rust |
| R-sqlstore | `storages/sqlite-storage/src/sqlite_store.rs` (433.682 bytes) | 0 menciones de "whatsmeow" |
| R-legacy | `wacore/libsignal/src/protocol/legacy_session.rs` | *"Container decoding is intentionally out of scope"*; `LegacySessionRecordV1` |
| R-ls-mod | `wacore/libsignal/src/protocol/mod.rs` | exports; `#![feature]` ausente |
| R-ls-session | `wacore/libsignal/src/protocol/state/session.rs` | `serialize`/`deserialize` propios (protobuf `buffa`) |
| R-appstate | `wacore/appstate/src/schemas.rs` | schemas `Archive`, `Unarchive`, `Mute`, `Pin`, `Star`, `Label`, `FavoriteSticker`; scopes `regular*`, `critical_*` |
| R-grep-simd | `wacore/appstate/src/lib.rs`, `wacore/libsignal/src/lib.rs`, `wacore/Cargo.toml`, `waproto/Cargo.toml` | 0 marcadores `portable_simd` / `#![feature]` |

### Documentación del proyecto (whatsapp-rust.jlucaso.com)
`llms.txt` (índice), `concepts/storage.md`, `api/store.md`, `api/client.md`, `api/send.md`, `api/chat-actions.md`, `api/newsletter.md`, `api/polls.md`, `api/contacts.md`, `api/groups.md`, `api/pictures.md`, `api/chatstate.md`, `api/presence.md`, `api/status.md`, `api/blocking.md`, `api/download.md`, `api/upload.md`, `api/media-reupload.md`, `guides/current-api.md`, `guides/media-handling.md`, `guides/newsletters.md`, `guides/polls.md`.

### Oráculo (`G:\Proyectos\16-whatsapp2`, solo lectura)
| Clave | Archivo | Qué dio |
|---|---|---|
| O-proto | `proto/bridge.proto` | 50 peticiones, 51 respuestas, 15 eventos; comentarios del contrato (`client_message_id` idempotente, `blocked` local, IDs de sticker) |
| O-server | `backend/internal/app/server.go` | `dispatch` con los 50 handlers; qué RPC va a `s.wa.*` y cuál a `s.store.*` |
| O-client | `backend/internal/wa/client.go` | 7.442 líneas; 28 APIs de `whatsmeow` usadas; `session.db` + `sqlstore.Container` + `GetFirstDevice` |
| O-store | `backend/internal/store/store.go` | 3.296 líneas; 25 tablas + `message_search` FTS5 |
| O-go | `backend/go.mod` | `go.mau.fi/whatsmeow v0.0.0-20260718134955-fac667d55293`, `go.mau.fi/libsignal v0.2.2`, `modernc.org/sqlite` |
| O-gocount | conteo de líneas de `backend/**/*.go` | 13.526 de producción + 6.372 de test en 13 archivos |
| O-charter | `G:\Proyectos\17-whatfast-rs\CHARTER.md` | baseline de RAM y punto 1 de "Qué NO se migra" |
| O-plan | `G:\Proyectos\17-whatfast-rs\PLAN.md` | lote B-R1, F4 |
| O-rs-cargo | `G:\Proyectos\17-whatfast-rs\Cargo.toml` | `license = "MIT OR Apache-2.0"`, MSRV 1.85, edition 2024 |

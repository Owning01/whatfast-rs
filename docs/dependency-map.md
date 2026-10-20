# DEPENDENCY MAP — generado por script

NO editar a mano: `python scripts/dependency-map.py` lo regenera.

## Totales

| Métrica | Valor |
|---|---|
| archivos_frontend | 58 |
| loc_ts | 16753 |
| loc_css | 4439 |
| suites_origen | 24 |
| casos_origen | 153 |
| modulos_destino | 19 |
| tests_destino | 0 |

## Módulos del origen

| Origen | LOC | Bucket | Destino Rust | Nota |
|---|---|---|---|---|
| `App.tsx` | 279 | reescritura | `whatfast-ui` |  |
| `components/Avatar.tsx` | 52 | reescritura | `whatfast-ui` |  |
| `components/ChatSwitcher.tsx` | 54 | reescritura | `whatfast-ui` |  |
| `components/Composer.tsx` | 787 | reescritura | `whatfast-ui` |  |
| `components/Conversation.tsx` | 698 | reescritura | `whatfast-ui` |  |
| `components/EmojiPicker.tsx` | 428 | reescritura | `whatfast-ui` |  |
| `components/MediaEditor.tsx` | 290 | reescritura | `whatfast-ui` |  |
| `components/MemberPanel.tsx` | 44 | reescritura | `whatfast-ui` |  |
| `components/MessageBubble.tsx` | 1353 | reescritura | `whatfast-ui` |  |
| `components/NewsletterPanel.tsx` | 154 | reescritura | `whatfast-ui` |  |
| `components/Panels.tsx` | 883 | reescritura | `whatfast-ui` |  |
| `components/ParticipantList.tsx` | 78 | reescritura | `whatfast-ui` |  |
| `components/ParticipantRow.tsx` | 40 | reescritura | `whatfast-ui` |  |
| `components/Primitives.tsx` | 49 | reescritura | `whatfast-ui` |  |
| `components/Screens.tsx` | 374 | reescritura | `whatfast-ui` |  |
| `components/Sidebar.tsx` | 595 | reescritura | `whatfast-ui` |  |
| `components/SpotlightSearch.tsx` | 376 | reescritura | `whatfast-ui` |  |
| `components/StatusStrip.tsx` | 174 | reescritura | `whatfast-ui` |  |
| `components/StickerTray.tsx` | 122 | reescritura | `whatfast-ui` |  |
| `components/Tabs.tsx` | 204 | reescritura | `whatfast-ui` |  |
| `components/ThemeIcon.tsx` | 219 | reescritura | `whatfast-ui` |  |
| `components/TitleBar.tsx` | 165 | reescritura | `whatfast-ui` |  |
| `index.tsx` | 31 | reescritura | `whatfast-ui` |  |
| `lib/bridge.ts` | 732 | reescritura | `whatfast-protocol` | la fachada de comandos desaparece |
| `lib/browser-mock.ts` | 1411 | eliminable | `-` | backend fake en Rust |
| `lib/chat-merge.ts` | 11 | reemplazo | `chat_merge` |  |
| `lib/emoji-autocomplete.ts` | 123 | reemplazo | `whatfast-ui` | offsets UTF-16 |
| `lib/file-drop.ts` | 30 | reemplazo | `file_drop` |  |
| `lib/format.ts` | 157 | reemplazo | `message_format` | sin suite: tests de caracterizacion |
| `lib/i18n.ts` | 282 | reemplazo | `whatfast-ui` | datos puros |
| `lib/media-crop.ts` | 217 | reemplazo | `media_crop` | geometria pura |
| `lib/notification-policy.ts` | 44 | reemplazo | `notification_policy` |  |
| `lib/notification-routing.ts` | 129 | reescritura | `whatfast-ui` | timers |
| `lib/notifications.ts` | 78 | reescritura | `whatfast-ui` | plugin de Tauri |
| `lib/participant-avatar-queue.ts` | 178 | reescritura | `whatfast-ui` | concurrencia |
| `lib/participant-roster.ts` | 39 | reemplazo | `whatfast-ui` |  |
| `lib/performance.ts` | 80 | reemplazo | `whatfast-ui` |  |
| `lib/polls.ts` | 27 | reemplazo | `polls` |  |
| `lib/send-idempotency.ts` | 43 | reemplazo | `whatfast-ui` |  |
| `lib/spotlight.ts` | 166 | reescritura | `whatfast-ui` | ranking + localStorage |
| `lib/theme.ts` | 292 | reescritura | `whatfast-ui` | variables CSS -> tokens de GPUI |
| `lib/types.ts` | 490 | directa | `proto/bridge.proto` | el contrato ya vive en Rust |
| `lib/unsupported.ts` | 15 | reemplazo | `unsupported` |  |
| `lib/voice-note.ts` | 162 | reescritura | `whatfast-ui` | binario |
| `lib/whatsapp-formatting.ts` | 434 | reescritura | `whatsapp_formatting` | no hay soporte en GPUI |
| `media-editor.css` | 105 | reescritura | `whatfast-ui` |  |
| `perf/harness.ts` | 249 | reescritura | `whatfast-ui` |  |
| `state/app-helpers.ts` | 169 | reemplazo | `app_helpers` | sin suite: tests de caracterizacion |
| `state/app.ts` | 3023 | reescritura | `whatfast-ui` | store reactivo -> Entity<Store> |
| `state/backend-lifecycle.ts` | 83 | reemplazo | `backend_lifecycle` |  |
| `state/pairing.ts` | 27 | reemplazo | `pairing` |  |
| `state/preferences.ts` | 194 | reescritura | `whatfast-ui` | localStorage -> archivo JSON |
| `state/scroll-restoration.ts` | 48 | reemplazo | `scroll_restoration` |  |
| `state/sync.ts` | 71 | reescritura | `whatfast-ui` |  |
| `state/tab-keyboard.ts` | 57 | reemplazo | `tab_keyboard` |  |
| `state/unread.ts` | 11 | reemplazo | `unread` |  |
| `state/workspace.ts` | 232 | reemplazo | `workspace` | 31 tests en el origen |
| `styles.css` | 4334 | reescritura | `whatfast-ui` |  |

## Suites del oráculo

| Suite | Módulo | Casos |
|---|---|---|
| `authoritative-resync.test.ts` | authoritative-resync | 1 |
| `backend-lifecycle.test.ts` | backend-lifecycle | 8 |
| `chat-merge.test.ts` | chat-merge | 1 |
| `emoji-autocomplete.test.ts` | emoji-autocomplete | 10 |
| `file-drop.test.ts` | file-drop | 3 |
| `media-crop.test.ts` | media-crop | 20 |
| `notification-routing.test.ts` | notification-routing | 7 |
| `notifications.test.ts` | notifications | 3 |
| `pairing.test.ts` | pairing | 3 |
| `participant-avatar-queue.test.ts` | participant-avatar-queue | 9 |
| `performance.test.ts` | performance | 5 |
| `polls.test.ts` | polls | 4 |
| `scroll-restoration.test.ts` | scroll-restoration | 6 |
| `send-idempotency.test.ts` | send-idempotency | 3 |
| `spotlight.test.ts` | spotlight | 4 |
| `sync.test.ts` | sync | 4 |
| `tab-keyboard.test.ts` | tab-keyboard | 5 |
| `theme.test.ts` | theme | 4 |
| `unread.test.ts` | unread | 3 |
| `unsupported.test.ts` | unsupported | 2 |
| `voice-note.test.ts` | voice-note | 3 |
| `whatsapp-formatting-spoiler.test.ts` | whatsapp-formatting-spoiler | 7 |
| `whatsapp-formatting.test.ts` | whatsapp-formatting | 7 |
| `workspace.test.ts` | workspace | 31 |

## Estado del destino

| Archivo Rust | LOC | Tests |
|---|---|---|
| `crates/whatfast-core/src/app_helpers.rs` | 0 | 0 |
| `crates/whatfast-core/src/backend_lifecycle.rs` | 0 | 0 |
| `crates/whatfast-core/src/chat_merge.rs` | 0 | 0 |
| `crates/whatfast-core/src/file_drop.rs` | 0 | 0 |
| `crates/whatfast-core/src/lib.rs` | 15 | 0 |
| `crates/whatfast-core/src/media_crop.rs` | 0 | 0 |
| `crates/whatfast-core/src/message_format.rs` | 0 | 0 |
| `crates/whatfast-core/src/notification_policy.rs` | 0 | 0 |
| `crates/whatfast-core/src/pairing.rs` | 0 | 0 |
| `crates/whatfast-core/src/polls.rs` | 0 | 0 |
| `crates/whatfast-core/src/scroll_restoration.rs` | 0 | 0 |
| `crates/whatfast-core/src/tab_keyboard.rs` | 0 | 0 |
| `crates/whatfast-core/src/unread.rs` | 0 | 0 |
| `crates/whatfast-core/src/unsupported.rs` | 0 | 0 |
| `crates/whatfast-core/src/whatsapp_formatting.rs` | 0 | 0 |
| `crates/whatfast-core/src/workspace.rs` | 0 | 0 |
| `crates/whatfast-protocol/build.rs` | 7 | 0 |
| `crates/whatfast-protocol/src/lib.rs` | 1 | 0 |
| `crates/whatfast-protocol/src/transport.rs` | 0 | 0 |

## Violaciones del rulebook (gate de F7)

```
crates/whatfast-protocol/build.rs:2: let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc unavailable");
crates/whatfast-protocol/build.rs:6: .expect("failed to compile bridge.proto");
```

# DEPENDENCY MAP — inventario generado por script

> F1 de `repo-migrate`. **Se genera con `scripts/dependency-map.py`, no a
> mano.** El script no está todavía: lo escribe el agente de inventario.

## Estado actual (medido a mano el 20/10/2026, pendiente de automatizar)

### Totales

| Grupo | Archivos | Líneas de código |
|---|---:|---:|
| `tauri/src/components/` | 21 | 7.139 |
| `tauri/src/lib/` | 22 | 5.140 |
| `tauri/src/state/` | 10 | 3.915 |
| `tauri/src/` (resto) | 3 | 559 |
| CSS (`styles.css` + `media-editor.css`) | 2 | 4.439 |
| **Total origen** | **58** | **21.192 (+4.439 CSS)** |
| `desktop/src/` (destino parcial, GPUI) | 9 | 10.187 |

### Buckets de dependencias

| Bucket | Contenido | Destino |
|---|---|---|
| **Directa** | `lib/types.ts` (490) | ya existe en Rust: `proto/bridge.proto` |
| **Reemplazo** | `solid-js` → GPUI · `@tanstack/solid-virtual` → `v_virtual_list` · `qrcode` → crate `qrcode` · `lucide-solid` → `gpui-component` `IconName` · catálogo de emoji → crate `emojis` | |
| **Reescritura** | `state/app.ts` (3.023) · `lib/whatsapp-formatting.ts` (434) · `lib/theme.ts` (292) + `state/preferences.ts` (194) · CSS (4.439) | |
| **Eliminable** | `lib/browser-mock.ts` (1.411) | backend fake en Rust |
| **Con estado o I/O** | `lib/participant-avatar-queue.ts` (178) · `lib/voice-note.ts` (162) · `lib/notification-routing.ts` (129) | separar decisión (puro) de efecto |

### Grafo de dependencias (módulos del grupo 1)

```
whatsapp_formatting  ─┐
media_crop           ─┤
message_format       ─┼─> (sin dependencias internas)
polls                ─┤
chat_merge           ─┤
unsupported          ─┤
file_drop            ─┤
unread               ─┤
pairing              ─┤
workspace            ─┤
app_helpers          ─┘
tab_keyboard
notification_policy
backend_lifecycle
scroll_restoration
```

Los 15 módulos del grupo 1 **no dependen entre sí**: por eso los tres
lotes pueden correr en paralelo sin pisarse.

### Módulos que quedan fuera del grupo 1

| Módulo TS | LOC | Por qué |
|---|---:|---|
| `state/app.ts` | 3.023 | store reactivo con 113 acciones: rediseño, no porte |
| `lib/bridge.ts` | 732 | fachada de comandos: se reemplaza por el cliente RPC del crate `whatfast-protocol` |
| `lib/theme.ts` | 292 | escribe variables CSS en el DOM: no existe en GPUI |
| `state/preferences.ts` | 194 | `localStorage` + `document.documentElement` |
| `lib/i18n.ts` | 282 | datos: se portan tal cual (batch posterior) |
| `lib/spotlight.ts` | 166 | ranking con `localStorage` |
| `lib/send-idempotency.ts` | 43 | reuso de `clientMessageId` |
| `lib/emoji-autocomplete.ts` | 123 | offsets UTF-16 (trampa documentada en el rulebook) |
| `lib/performance.ts` | 80 | índice de quotes en una pasada |
| `lib/notification-routing.ts` | 129 | timers y colas |
| `lib/participant-avatar-queue.ts` | 178 | concurrencia y backoff |
| `lib/voice-note.ts` | 162 | conversión WebM→Ogg: binario, va al crate de medios |
| `lib/browser-mock.ts` | 1.411 | se elimina (backend fake en Rust) |

# PLAN — migración a Rust nativo, con_AGENTES y gates

> Ejecución del charter (`CHARTER.md`) con la secuencia de fases de
> `repo-migrate`. Este documento es el mapa de lotes: quién es dueño de qué,
> en qué orden, y qué gate cierra cada lote.

## Reglas de ejecución (no negociables)

1. **Un lote = una partición de archivos.** Dos agentes nunca tocan el mismo
   archivo en el mismo momento. `crates/whatfast-core/src/lib.rs` lo mantiene
   el orquestador.
2. **Main siempre verde.** Cada lote termina en un commit que compila, con
   lints limpios y sus tests en verde.
3. **Nadie edita el oráculo.** Los tests TS de `whatfast` están congelados.
   Los agentes escriben tests de Rust **nuevos**, traducidos.
4. **Nada de stubs.** Un `todo!()` o un `unwrap()` para que compile es
   rechazo inmediato.
5. **Cada error que se repite 3 veces se sube al rulebook**, no se parchea
   en el archivo.
6. **Los lotes de un mismo grupo corren en paralelo; los grupos, en
   secuencia.**

---

## F0 · Charter — ✅ hecho, falta firma

`CHARTER.md`. Objetivo medible, qué no se migra, riesgos aceptados.

## F1 · Inventario — ✅ hecho (explorador, solo lectura)

- Origen: 25.600 líneas (21.192 TS/TSX + 4.439 CSS) en 58 archivos.
- Destino parcial: `desktop/` (GPUI) con 9 archivos, 10.187 líneas, 64 tests.
- Superficie RPC: 64 comandos; el crate GPUI hoy cubre 22 → **faltan 42**.
- Componentes: 21; 3 con equivalente utilizable, 7 parciales, 11 inexistentes.
- Tests: 3/24 suites con equivalente directo, 11 parciales, 10 sin nada.
- Rewrites reales: parser de formato (434 líneas, 0 soporte en GPUI), sistema
  de tema/CSS (4.439 líneas), `state/app.ts` (3.023 líneas, 113 acciones).

Pendiente de esta fase (se hace con scripts, no a mano):
`docs/dependency-map.json` + `docs/dependency-map.md`.

## F2 · Oráculo — el judge

**El judge es el repo de origen, congelado.** No se toca.

| Pieza | Dónde | Rol |
|---|---|---|
| 153 tests TS (`tauri/tests/`) | `whatfast` | truth de comportamiento del frontend |
| 64 tests Rust (`desktop/src/`) | `whatfast` | truth del modelo, transporte y RPC |
| 50 RPC del proto v21 | `whatfast/proto` | contrato de la API |
| 9 paquetes de tests Go | `whatfast/backend` | truth del sidecar |

**Regla de paridad, por módulo**: para cada test TS `X.test.ts` con N casos,
el puerto tiene un test Rust con N casos de los mismos nombres traducidos y
las mismas aserciones. El juez compara **nombre por nombre** la lista de
casos (no el texto de las aserciones, que cambia de lenguaje).

**Validación del juez** (gate de F2, antes de escribir una línea de destino):
1. Correr el juez contra el origen → verde.
2. Romper el origen a propósito (cambiar un `>=` por `>` en un módulo de
   lógica pura) → el juez tiene que fallar. Si no falla, el juez no sirve y
   hay que reforzarlo **antes** de migrar.

## F3 · Rulebook — ✅ redactado, falta firma

`RULEBOOK.md` + `docs/type-map.md` + `docs/api-mapping.md`.

## F4 · Interop + corte vertical

**Puente elegido: proceso propio con framing protobuf sobre stdio.** Es el
mismo que ya usa el sidecar Go, pero el cliente y el backend son los dos
Rust. No hay FFI, no hay NAPI, no hay servicio HTTP.

```
crates/whatfast-protocol   ← proto/bridge.proto + framing + cliente RPC
crates/whatfast-core       ← lógica pura (este repo)
crates/whatfast-ui         ← GPUI (aún vacío)
```

El **primer componente real** que se porta detrás del puente es el
**sidebar**: lista de chats con filtro, no leídos, fijados y búsqueda local.
Mide: same data, mismo orden, mismo click.

## F5 · Mini-migración stress (slice duro)

**Slice: el parser de formato de WhatsApp** (`whatsapp_formatting.rs`).
Es el módulo más difícil de la lista: 434 líneas, 14 tests, 6 estilos inline,
6 tipos de bloque, reglas de delimitadores, URLs, menciones, spoilers anidados.
Si el slice duro pasa, el proceso sirve.

Spec del trial-run (3 archivos, 1 implementador + 2 revisores + 1 fixer): el
output se descarta, el proceso mejorado es el entregable.

## F6 · Traducción paralela — el mapa de lotes

### Grupo 1 · lógica pura (sin GPUI, sin red) — **corre en paralelo**

| Lote | Agente | Archivos (propiedad exclusiva) | Origen |
|---|---|---|---|
| **B-A1** | A1 | `crates/whatfast-core/src/whatsapp_formatting.rs` + `tests/whatsapp_formatting.rs` | `lib/whatsapp-formatting.ts` + 2 suites |
| **B-A2** | A2 | `src/media_crop.rs`, `src/message_format.rs`, `src/polls.rs`, `src/chat_merge.rs`, `src/unsupported.rs`, `src/file_drop.rs`, `src/unread.rs`, `src/pairing.rs` + sus tests | 8 módulos TS + 8 suites |
| **B-A3** | A3 | `src/workspace.rs`, `src/app_helpers.rs`, `src/tab_keyboard.rs`, `src/notification_policy.rs`, `src/backend_lifecycle.rs`, `src/scroll_restoration.rs` + sus tests | 6 módulos TS + 6 suites |

Los tres lotes son disjuntos por archivo. El orquestador mantiene
`src/lib.rs`.

### Grupo 2 · transporte y contrato

| Lote | Agente | Archivos | Origen |
|---|---|---|---|
| **B-P1** | P1 | `crates/whatfast-protocol/src/transport.rs` + `tests/transport.rs` | `tauri/src-tauri/src/bridge.rs` (framing, ciclo de vida de requests) |
| **B-P2** | P2 | `docs/api-mapping.md` (documento) | `tauri/src/lib/bridge.ts` (64 comandos) → acciones del crate UI |

### Grupo 1 bis · reparación (nace de un gap entre agentes)

| Lote | Agente | Archivos | Por qué |
|---|---|---|---|
| **B-A4** | A4 | `src/types.rs` (nuevo) + `src/message_format.rs` + `src/app_helpers.rs` + `src/notification_policy.rs` | A2 y A3 definieron **dos** familias paralelas de `Chat`, `Message`, `TextContent`, `LinkPreview` y compañía. El rulebook no decía dónde viven los DTOs compartidos: es un gap del rulebook, no un error de un agente. La UI (F4) necesita un solo tipo `Chat` |

### Grupo 3 · UI nativa (GPUI) — arranca cuando el grupo 2 está verde

**Hallazgo que cambió el plan (20/10/2026):** el repo de origen **ya tiene
una implementación GPUI** en `desktop/` (9 archivos, 10.187 líneas, 64 tests):
`main.rs` con 6.503 líneas de `render_*`, más `model.rs`, `bridge.rs`,
`rpc.rs`, `paths.rs`, `settings.rs`, `sticker.rs`, `emoji_picker.rs`.

O sea: la interfaz **no se porta desde SolidJS**, se separa, se limpa y se
completa. Lo que falta en esa implementación_gpuI (11 componentes, 42 RPC) es
lo que hay que escribir de verdad, y sale de los componentes SolidJS.

| Lote | Agente | Alcance | Origen |
|---|---|---|---|
| **B-U0** | U0 | **Crate `whatfast-model`**: el modelo de datos (`model.rs`, 1.267 l., 23 tests) + `paths.rs` + `settings.rs` + `sticker.rs` + `emoji_picker.rs`, sin dependencia de GPUI | `desktop/src/` del oráculo |
| **B-U1** | U1 | **El backend fake** (`fake_loop`, los 50 RPC con datos de demostración) en `whatfast-protocol/src/fake.rs` | `desktop/src/bridge.rs` (la parte fake) |
| **B-U2** | U2 | **Shell + sidebar** (corte vertical del F4): ventana, layout de 3 paneles, lista de chats con filtros, no leídos y fijados | `desktop/src/main.rs` |
| **B-U3** | U3 | **Tokens del tema**: las 14 custom properties del CSS y los 30 tokens del tema, como sistema de estilos de GPUI | `tauri/src/styles.css` + `whatfast-core/theme.rs` (ya portado) |
| **B-U4** | U4 | Lista de mensajes virtualizada + burbuja (reacciones, replies, adjuntos, polls, quoted, link preview) | `desktop/src/main.rs` + `MessageBubble.tsx` |
| **B-U5** | U5 | Compositor: texto, adjuntos, voz, emoji, menciones, polls | `Composer.tsx` |
| **B-U6** | U6 | Barra de título, strip de tabs, panel de ficha del chat, ajustes, miembros, newsletters, estados | `TitleBar.tsx`, `Tabs.tsx`, `Panels.tsx`, `MemberPanel.tsx`, `NewsletterPanel.tsx`, `StatusStrip.tsx` |
| **B-U7** | U7 | Buscador global, switcher de chats, bandeja de stickers, visor de imagen | `SpotlightSearch.tsx`, `ChatSwitcher.tsx`, `StickerTray.tsx`, `ImageViewer` |
| **B-U8** | U8 | Editor de imagen (recorte libre + rotación) en GPUI | `MediaEditor.tsx` + `whatfast-core/media_crop.rs` (ya portado) |

**Orden**: U0 y U1 en paralelo (son la base y no se tocan), después U2 (el
corte vertical), después U3, y de ahí U4 a U8 en el orden del riesgo.

### Grupo 4 · protocolo en Rust (spike con gate)

| Lote | Agente | Pregunta | Gate | Resultado |
|---|---|---|---|---|
| **B-R1** | R1 | ¿`wa-rs` o `whatsapp-rust` cubren los 50 RPC que necesita el cliente? | Si no cubren: el sidecar Go se queda y el trabajo termina en "UI en Rust" | **CERRADO**: no se puede reusar la sesión ya emparejada (esquema SQLite incompatible, cero importadores). Decisión D1 del charter: el sidecar Go se queda |

**B-R1 no bloquea los grupos 1 a 3**: es un spike en paralelo, con su propio
presupuesto.

## F7 · Repair + paridad

Por lote: L1 (sintaxis, del diagnóstico del compilador) → L2 (compilación +
lints) → L3 (paridad: comparación caso por caso contra el test TS) → mutation
≥ 80 % sobre lo cambiado (`cargo install cargo-mutants`; `cargo mutants
--in-diff`) → `grep` cero-patrones-viejos → revisión firmada.

## F8 · Cutover

1. Apagar el webview componente por componente, del borde al núcleo.
2. Cada apagado parcial corre la e2e completa.
3. Borrar el origen muerto en el mismo PR.
4. Medir contra el baseline: **ventana < 0,5 s, RAM < 120 MB**.
5. Publicar los números (README de `whatfast-rs`).

---

## Orden de ejecución y quién hace qué

| Momento | Quién | Qué |
|---|---|---|
| ahora | orquestador | esqueleto, charter, rulebook, plan, repo nuevo |
| ahora | A1, A2, A3 (en paralelo) | grupo 1: 15 módulos de lógica pura + sus tests |
| ahora | R1 (en paralelo) | spike del protocolo Rust |
| después | P1, P2 | transporte + mapa de API |
| después | U1 → U8 | UI nativa, en batches con gates |
| después | F7, F8 | repair, paridad, cutover |

## Cómo se mide el avance

- **No en líneas.** En lotes cerrados con L1+L2+L3 verdes.
- La paridad se cuenta: `casos de paridad verdes / casos de paridad del
  oráculo`. Hoy: 0/153 (el destino tiene 0 casos). Meta F6: ≥ 90 %. Meta F8:
  100 %.

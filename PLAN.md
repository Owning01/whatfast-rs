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

### Grupo 3 - UI nativa (GPUI) - el corte vertical

**Hallazgo que cambio el plan (20/10/2026):** el repo de origen **ya tiene
una implementacion GPUI** en `desktop/` (9 archivos, 10.187 l., 64 tests):
`main.rs` con 6.503 l. de `render_*`, mas `model.rs`, `bridge.rs`, `rpc.rs`,
`paths.rs`, `settings.rs`, `sticker.rs`, `emoji_picker.rs`.

O sea: la interfaz **no se porta desde SolidJS**, se separa, se limpia y se
completa. Lo que falta en esa implementacion GPUI (11 componentes, 42 RPC) es
lo que hay que escribir de verdad, y sale de los componentes SolidJS.

#### Tanda 1 - el corte vertical (20/10/2026)

El `main.rs` del oraculo son 6.503 lineas en un solo archivo. Se parte en
dos mitades que se pueden portar en paralelo, porque los dos agentes leen
el mismo archivo del oraculo y respetan los mismos nombres de campo.

| Lote | Agente | Archivos | Alcance |
|---|---|---|---|
| **B-G1** | G1 | `backend/**` | El sidecar en Go, copiado del oraculo al repo nuevo. El modulo ya esta renombrado (`github.com/Owning01/whatfast/backend`). |
| **B-U2a** | U2 | `crates/whatfast-ui/src/{app.rs,rpc.rs,text.rs}` | El estado y las acciones: el `struct Whatfast` completo, el ciclo de vida de los pedidos (`PendingRequest` semantico + `RequestClass`) y los helpers de texto. |
| **B-U2b** | U4 | `crates/whatfast-ui/src/{render.rs,helpers.rs}` | El dibujo: los `render_*`, el `impl Render` y los helpers libres de dibujo. |
| **B-U3** | U3 | `crates/whatfast-ui/src/tokens.rs` | Los tokens del tema: las medidas del CSS multiplicadas por la escala, la conversion de los 30 tokens a color y la aplicacion del tema sobre `gpui_component`. |

El orquestador owns: `crates/whatfast-ui/src/main.rs` (la entrada, la ventana
y las acciones globales) y el `Cargo.toml` de la UI.

**El contrato entre los tres agentes de la UI**: el `struct Whatfast` lo
define U2 con **todos los campos `pub(crate)`** y **los mismos nombres y
tipos** que en el oraculo; U4 escribe un `impl Whatfast` aparte (Rust
permite varios bloques `impl` del mismo tipo en archivos distintos) y
accede a los campos como `self.<campo>`.

#### Tanda 2 - lo que el cliente GPUI del oraculo no tiene

Once componentes y 42 RPC que el cliente de Tauri si tiene, y que salen de
los componentes SolidJS:

| Lote | Agente | Alcance | Origen |
|---|---|---|---|
| **B-U4** | U4 | Lista de mensajes virtualizada + burbuja (reacciones, replies, adjuntos, polls, quoted, link preview) | `desktop/src/main.rs` + `MessageBubble.tsx` |
| **B-U5** | U5 | Compositor: texto, adjuntos, voz, emoji, menciones, polls | `Composer.tsx` |
| **B-U6** | U6 | Barra de titulo, strip de tabs, panel de ficha del chat, ajustes, miembros, newsletters, estados | `TitleBar.tsx`, `Tabs.tsx`, `Panels.tsx`, `MemberPanel.tsx`, `NewsletterPanel.tsx`, `StatusStrip.tsx` |
| **B-U7** | U7 | Buscador global, switcher de chats, bandeja de stickers, visor de imagen | `SpotlightSearch.tsx`, `ChatSwitcher.tsx`, `StickerTray.tsx`, `ImageViewer` |
| **B-U8** | U8 | Editor de imagen (recorte libre + rotacion) en GPUI | `MediaEditor.tsx` + `whatfast-core/media_crop.rs` (ya portado) |

##### Tanda 2 en ejecucion (20/10/2026): los lotes reales

La tabla de arriba es el reparto teorico; en la ejecucion los lotes se
partieron por **archivos disjuntos** (regla del charter: un archivo, un
dueno) y el estado es este:

| Lote | Worktree | Rama | Archivos que posee | Que hace |
|---|---|---|---|---|
| **r1** | `wf/r1` | `mig/r1` | `crates/whatfast-model/src/model.rs`, `crates/whatfast-ui/src/{rpc.rs,app.rs}` | Los **28 RPC** que faltaban: la peticion, el estado del store y la accion que la dispara. **Cerrado**: 50/50 RPC |
| **r2** | `wf/r2` | `mig/r2` | `crates/whatfast-ui/src/{icons.rs,media_editor.rs}` | El juego de iconos del cliente web y el editor de imagen (recorte + rotacion). **Cerrado** |
| **r3** | `wf/r3` | `mig/r3` | `crates/whatfast-ui/src/{navrail.rs,tabs.rs}` | La columna de iconos y el strip de pestañas. **Cerrado** |
| **r4** | `wf/r4` | `mig/r4` | `crates/whatfast-ui/src/{newsletters.rs,statuses.rs,members.rs,stickers.rs}` | Los cuatro paneles que dependen del estado nuevo del store. **Cerrado** |
| **r5** | `wf/r5` | `mig/r5` | `crates/whatfast-ui/src/message_actions.rs` | El menu del mensaje, el reenvio, la encuesta y el clip de adjuntos. **Cerrado** |

**El orquestador** integra: `crates/whatfast-ui/src/main.rs` (los `mod` y la
entrada), los campos nuevos del `struct Whatfast`, el layout de `render.rs`
(la columna de iconos a la izquierda, el strip de pestañas arriba de la
conversacion, los paneles al lado) y las medidas.

**La puerta de calidad** es `scripts/funcionalidad.py`: cuenta los RPC que la
interfaz pide (sobre 50) y los componentes del cliente web que tienen
equivalente en Rust (sobre 21). Sale con codigo 1 mientras falte algo.
`scripts/parity.py` sigue siendo el juez de los 153 casos del dominio.

#### Tanda 3 - el cierre

- El empaquetado: el binario de Rust con el sidecar en Go al lado, los
  iconos y el instalador.
- Las medidas: arranque y RAM, contra el cliente viejo, en el README.
- El corte: publicar y apuntar la pagina del proyecto.

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

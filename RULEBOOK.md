# RULEBOOK — quéfast TS/TSX → Rust

> Fase F3 de `repo-migrate`. **Firma pendiente del humano.**
> Si dos agentes podrían responder distinto a una pregunta, la respuesta está
> acá. Cuando aparezca una pregunta nueva: se responde acá primero y se
> vuelve a portear el módulo.

## 0. Alcance de este rulebook

Cubre la traducción del **frontend** (`tauri/src/`) al crate de lógica pura
`crates/whatfast-core`. El núcleo Tauri, el sidecar Go y la UI GPUI tienen
rulebooks propios cuando entren en juego (`whatfast-core` no depende de GPUI
a propósito: es el loop rápido).

## 1. Los 5 ejes invariantes

### 1.1 Números

| Origen (TS) | Destino (Rust) | Regla |
|---|---|---|
| `number` entero de timestamps (ms) | `i64` | milisegundos epoch, nunca `f64` |
| `number` de conteo / tamaño | `u32` o `usize` según el dominio | si el origen lo compara con `>= 0`, es `u32` |
| `number` fraccionario (posición, escala, alpha) | `f64` | |
| `number` de duración en segundos | `f64` | |
| `number` de tamaño de archivo (bytes) | `u64` | |

- **Trampa**: `Math.round` de JS redondea **hacia +∞** en los empates
  (`Math.round(-0.5) === -0`), mientras que `f64::round` de Rust redondea
  **lejos de cero** (`(-0.5f64).round() == -1.0`). Cuando el origen use
  `Math.round` sobre un valor que puede ser negativo, el puerto debe
  reproducir el comportamiento de JS, no el de Rust.
- **Trampa**: `Math.floor`/`Math.ceil`/`Math.trunc` tienen el mismo
  comportamiento en ambos lenguajes. No hay que hacer nada.
- Enteros de JS son `f64`: un `u32` de Rust es un subconjunto. Si el origen
  puede ver `NaN` o `Infinity` en un campo numérico, ese campo es `Option<f64>`
  o `f64`, no un entero. Revisar módulo por módulo.

### 1.2 Nulabilidad

| Origen | Destino |
|---|---|
| `T \| undefined` | `Option<T>` |
| `T \| null` | `Option<T>` |
| `T \| null \| undefined` | `Option<T>` |
| `T` (siempre presente, a veces `""` o `[]`) | `T` (no `Option`) |
| `Record<string, T>` | `HashMap<String, T>` o `BTreeMap` si el orden importa |
| `T[]` | `Vec<T>` |
| `readonly T[]` | `Vec<T>` (la API pública puede usar `&[T]`) |

- `null` y `undefined` **no son lo mismo** en JS (`null == undefined` es
  `true`, pero `null !== undefined` con `===` es `false`… salvo que el origen
  use `==`). Si el origen usa `==`, hay que fijar cuál de los dos es.
  **Regla**: en el puerto, `null` y `undefined` se unifican en `None` salvo
  que el test del origen demuestre que se distinguen.
- Un campo presente con valor `null` **se conserva** en el `Option`: no se
  borra la clave. (Aplica a `serde` con `#[serde(skip_serializing_if)]`: no
  usarlo en el puerto, salvo que el origen borre la clave.)

### 1.3 Errores

| Situación | Destino |
|---|---|
| El origen devuelve `null` para "no hay" | `Option<T>` |
| El origen lanza `throw` | `Result<T, E>` con `thiserror` |
| El origen ignora el error | el puerto también lo ignora, pero con un `// TODO(port): el origen ignora este error` |

- **Prohibido** `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()`
  fuera de `#[cfg(test)]`. Es verificable con `grep` y es gate de F7.
- En `whatfast-core` (lógica pura) no debería haber ningún `Result` que no sea
  de validación de entrada: si hace falta, es que el módulo tiene estado o
  I/O, y no es lógica pura.
- Los errores de tipo van con `thiserror` (`#[derive(Error)]`), no con
  `anyhow`, en este crate. `anyhow` es para binarios.

### 1.4 Concurrencia

- `whatfast-core` es **sincrónico y puro**: sin `async`, sin canales, sin
  `Arc<Mutex>`, sin tiempo. Si un módulo del origen tiene timers o colas
  (por ejemplo `participant-avatar-queue.ts`), se separa en dos: la lógica
  de decisión (pura, en `whatfast-core`) y el efecto (en el crate de UI).
- Donde el origen use `setTimeout` / `Date.now()`, el puerto expone una
  función que **recibe el instante** como parámetro, para que sea testeable
  sin reloj real.

### 1.5 Memoria

- `String`/`Vec` por defecto (own). Pedir prestado (`&str`, `&[T]`) solo
  cuando no cuesta crear el dato.
- No `Rc<RefCell<>>` en `whatfast-core`.
- Las estructuras de dominio que se copian mucho (mensajes, chats) usan
  `#[derive(Clone)]` y campos `String`, no `&'a str` con lifetimes
  complicados. **Regla de lifetimes: no se usan.** Si hace falta un lifetime,
  el diseño está mal: devolvé datos poseídos.

## 2. Toolchain

| | Origen | Destino |
|---|---|---|
| Build | `pnpm build` (Vite) | `cargo build --workspace` |
| Package manager | pnpm 10.28.2 | cargo (fichero `Cargo.lock` versionado) |
| Test | `node --test tests/*.test.ts` | `cargo test --workspace` (wrapper `scripts/test.cmd`) |
| Lint | `tsc --noEmit` + ESLint | `cargo clippy --all-targets --locked -- -D warnings` (wrapper `scripts/lint.cmd`) |
| Formatter | prettier | `cargo fmt` |
| Watch | `vite` | `cargo watch` (opcional, no bloquea nada) |
| fast-check | — | `cargo test` + `proptest` solo si el módulo lo pide |

- **Siempre** pasar `--config "build.rustc-wrapper=''"`: el volumen `G:` de
  esta máquina no tiene `sccache` instalado. Los wrappers
  `scripts/test.cmd` y `scripts/lint.cmd` ya lo hacen. **Usar los wrappers.**
- Rust stable, edición 2024, `rust-version = "1.85"`.
- `gpui` y `gpui-component` van **pineados** al rev
  `031555662e99a1b5a549990b47f246d475b8288a`. No se actualizan a mitad de la
  migración: cambiar el pin invalida los lotes ya mergeados.

## 3. Convenciones de traducción

1. **Nombres**: `camelCase` → `snake_case`. `PascalCase` (tipos) → `PascalCase`.
   Los nombres de los tests también.
2. **Dos pasos, dos diffs**:
   - **Paso 1 (literal)**: mismo comportamiento, mismos nombres de función
     traducidos, mismo orden de lógica, sin reescrituras idiomáticas. El
     objetivo es que el diff con el origen sea legible.
   - **Paso 2 (idiomático)**: refactor a idioms de Rust (`iterators`, `?`,
     `let else`, `match` exhaustivo). **Nunca en el mismo diff.**
   Cada lote de F6 es un PR por módulo, y un módulo necesita dos PRs (o dos
   commits claramente separados) para pasar de literal a idiomático.
3. **Fidelidad al original, incluidas las rarezas**: si el origen tiene un
   `"Yesterday"` hardcodeado, el puerto lo hardcodea. Si el origen ordena con
   un comparador raro, se reproduce. El objetivo es paridad, no mejora.
4. **Sin dependencias nuevas** sin pasar por el humano. Si un módulo necesita
   un crate que no está en `Cargo.toml`, se usa código estándar o se anota
   en "Decisiones pendientes".
5. **Los tests se portan 1:1**: cada test de TS tiene un test de Rust con el
   mismo nombre traducido, el mismo caso y las mismas aserciones. El test TS
   es el oráculo: **nunca se modifica el test TS para que el puerto pase**.
6. **Comentarios**: el puerto explica las rarezas del original
   (`// port: el original usa X y por eso Y`). No se documenta lo obvio.

### 3.1 Trampas específicas de TypeScript → Rust

| Trampa | Regla |
|---|---|
| `Array.prototype.sort` es estable desde ES2019 | usar `sort_by`, **nunca** `sort_unstable_by` |
| Los strings de JS son UTF-16; los de Rust UTF-8 | las posiciones de caret (emoji-autocomplete) son offsets UTF-16 en el origen: convertirlas a byte offsets de Rust y devolver byte offsets. Los tests del origen.assertan posiciones UTF-16: el test de Rust debe traducir la expectativa |
| `Date.now()` es milisegundos epoch UTC | `i64` milisegundos; si el origen usa `new Date()` sin argumentos, eso es "ahora" y no es testeable: se inyecta el instante |
| `JSON.parse` / `JSON.stringify` | `serde_json`; `JSON.stringify` de `undefined` es `undefined` (no `null`), y de un objeto con `undefined` **omite la clave**: `serde` con `skip_serializing_if = "Option::is_none"` |
| `Object.keys` sobre un objeto con claves numéricas las ordena numéricamente | si el origen depende de ese orden, se documenta en el test |
| `Number("")` es `0`, `Number("abc")` es `NaN` | los puertos no parsean strings salvo que el origen lo haga explícitamente |
| `parseInt` sin base | base 10 en ambos |
| `x ?? y` (nullish) vs `x || y` (falsy) | `??` → `Option`, `||` → depende: `""` y `0` son falsy en JS y **verdaderos** en `Option` |
| `!` (truthiness) | `Option::is_some()` es el equivalente de `!= null`; el truthiness de JS (`""`, `0`, `false`, `NaN`) **no** existe en Rust: hay que decidir el caso explícitamente |

## 4. Módulos del lote B (reglas por módulo)

| Módulo Rust | Origen TS | Decisión |
|---|---|---|
| `whatsapp_formatting.rs` | `lib/whatsapp-formatting.ts` | Parser de formato. Devuelve un árbol de bloques y segmentos. **Sin `String` mal formados: se ignoran como en el original.** Sin `regex` crate: el original usa regex; `regex` es una dependencia nueva → **se implementa a mano o se autoriza `regex`**. Default: a mano con tests del original como guía |
| `media_crop.rs` | `lib/media-crop.ts` | Geometría pura. Already tested (20 tests). El crop se expresa en píxeles de la imagen **ya rotada** |
| `message_format.rs` | `lib/format.ts` | Fechas, `messageText`, iniciales, `hueFor` (FNV-1a), bytes, duración. `formatChatTime` tiene `"Yesterday"` hardcodeado: se conserva |
| `polls.rs` | `lib/polls.ts` | Reducer optimista de votos |
| `chat_merge.rs` | `lib/chat-merge.ts` | 1 función |
| `unsupported.rs` | `lib/unsupported.ts` | Fallback de poll |
| `file_drop.rs` | `lib/file-drop.ts` | Clasificación de drops |
| `unread.rs` | `state/unread.ts` | Contador optimista |
| `pairing.rs` | `state/pairing.ts` | Decisiones de arranque y fases del QR |
| `workspace.rs` | `state/workspace.ts` | Panes, tabs, switcher, eviction, snapshot. Ya tiene 31 tests |
| `app_helpers.rs` | `state/app-helpers.ts` | `sortChats`, `mergeChats`, `mergeMessages`, `upsertSortedMessage`, `trimMessages`, `encodeMentions`, `mediaKey`, recientes |
| `tab_keyboard.rs` | `state/tab-keyboard.ts` | Mapeo de teclas → comando |
| `notification_policy.rs` | `lib/notification-policy.ts` | `shouldNotify` |
| `backend_lifecycle.rs` | `state/backend-lifecycle.ts` | Epochs, generaciones, reintentos acotados |
| `scroll_restoration.rs` | `state/scroll-restoration.ts` | Ancla de scroll |

## 5. Prohibido (gate de F7)

- `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()` fuera de tests.
- `unsafe` (no hay necesidad en este crate).
- Tests omitidos, comentados o "skip".
- Dependencias nuevas no autorizadas.
- Tocar archivos fuera de la partición asignada del lote.
- Modificar `crates/whatfast-core/src/lib.rs` (lo mantiene el orquestador).

## 6. Definición de "listo" (por módulo)

1. El módulo está portado (paso literal) y sus tests pasan.
2. Los tests de Rust cubren **todos** los casos del test TS equivalente,
   con los mismos nombres traducidos.
3. `cargo clippy --all-targets --locked -- -D warnings` limpio.
4. `cargo fmt` aplicado.
5. Cero ocurrencias del patrón viejo en el módulo (grep determinístico).
6. `git diff` limitado a la partición asignada.

## 7. Decisiones pendientes (se resuelven acá, no en el código)

| # | Pregunta | Dueño | Estado |
|---|---|---|---|
| D1 | ¿Se autoriza `regex` como dependencia de `whatfast-core`? | humano | abierta — por defecto: **no**, se implementa a mano |
| D2 | ¿`hueFor` (FNV-1a) se porta o se cambia? | humano | por defecto: **se porta igual** (el color del avatar es paridad) |
| D3 | ¿Los offsets de caret del emoji-autocomplete se exponen en bytes UTF-8 o en "unidades de carácter"? | humano | por defecto: **bytes UTF-8**, documentado |
| D4 | ¿El snapshot del workspace sigue siendo JSON en `localStorage`? | humano | abierta — en GPUI el equivalente es un archivo JSON en el perfil; a definir en F4 |
| D5 | ¿Los nombres de los chats mock (`Fast friend N`) se conservan? | humano | **sí**: son parte del oráculo de los tests |
| D6 | ¿El sidecar del protocolo se reescribe en Rust? | humano | **no** (decisión D1 del charter, con el spike B-R1 como evidencia): `whatsapp-rust` no puede reusar una sesión emparejada y `wa-rs` está parado. El sidecar Go se queda |
| D7 | ¿El transporte entre el binario Rust y el sidecar Go sigue siendo protobuf sobre stdio? | humano | **sí** (decisión D4): ya existe, ya está versionado y falla cerrado |
| D8 | ¿Dónde viven los DTOs compartidos (`Chat`, `Message`, `TextContent`, `LinkPreview`…)? | orquestador | **`src/types.rs`**, módulo nuevo. El rulebook no lo decía y dos agentes definieron dos familias paralelas de los mismos tipos: gap del rulebook, corregido por el lote B-A4. **Regla para los lotes que siguen: si un tipo lo va a usar más de un módulo, se declara en `types.rs` y no en el módulo que lo necesita primero** |
| D9 | ¿Dónde viven los efectos (almacenamiento, timers, red)? | orquestador | **fuera del crate de lógica pura**: `src/` solo decide, el crate de UI ejecuta. `readWorkspaceSnapshot`/`readRecentChats` quedaron como `parse_*` puros, con el acceso al almacén para la capa de UI |

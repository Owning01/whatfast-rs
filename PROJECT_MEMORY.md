# PROJECT_MEMORY — bitácora de whatfast-rs

Historial append-only de qué cambió y por qué. El estado actual vive en
`PROJECT_MAP.md`. Nunca se reescribe ni se borra una entrada.

## 2026-10-20 — Arranque de la migración: charter, rulebook, plan y primeros lotes

**Qué se cambió**

- Repo nuevo `G:\Proyectos\17-whatfast-rs` → `github.com/Owning01/whatfast-rs`
  (público). El repo de origen `G:\Proyectos\16-whatsapp2` queda congelado
  como oráculo y no se toca.
- `CHARTER.md` (F0): objetivo medible (< 0,5 s de ventana, < 120 MB de RAM,
  153/153 de paridad), qué no se migra, riesgos aceptados por escrito.
- `RULEBOOK.md` (F3): los 5 ejes invariantes resueltos, toolchain, trampas de
  TypeScript → Rust, los patrones prohibidos y las decisiones pendientes.
- `PLAN.md`: el mapa de lotes con propiedad exclusiva de archivos y los gates.
- `docs/type-map.md`, `docs/api-mapping.md` (F3), `docs/oracle.md` (F2).
- `scripts/dependency-map.py`: genera el inventario F1 (58 módulos, 24 suites,
  153 casos del oráculo) y detecta las violaciones del rulebook por `grep`.
- `scripts/parity.py`: el juez de paridad. Compara, módulo por módulo, los
  nombres de caso del oráculo contra los del puerto.
- Esqueleto de crates: `whatfast-core` (15 módulos de lógica pura ya
  declarados), `whatfast-protocol` (protobuf + transporte), `whatfast-ui`
  (placeholder, arranca en F4).

**Por qué**

El webview es el 85 % de la RAM: 367 de los 432 MB del árbol de procesos son
seis procesos de WebView2 que existen solo para pintar el DOM. La migración
no es cosmética: es el mayor single win de recursos que tiene el proyecto.

**Evidencia**

- Baseline medido: ventana visible 0,25 s; núcleo Rust 25,6 MB; sidecar Go
  31,8 MB; WebView2 366,8 MB; total 432 MB; UI 411 KB de JS.
- Inventario del origen: 21.192 líneas de TS/TSX + 4.439 de CSS en 58
  archivos; 24 suites, 153 tests. El crate GPUI que ya existe tiene 10.187
  líneas, 64 tests, y cubre 22 de los 64 comandos de la API.
- `scripts/dependency-map.py` corre y da: 58 módulos, 24 suites, 153 casos
  del oráculo, 0 tests en el destino (arranque), 2 violaciones (los `todo!()`
  del esqueleto, que se van con los lotes).
- `scripts/parity.py` corre y da 0/153 (0,0 %): el juez lee correctamente los
  153 casos del oráculo, o sea que está validado contra el origen.

**Trampas**

- El volumen `G:` no tiene `sccache` y el linker del MSVC no está en el PATH:
  hay que cargar `msvc.ps1` y usar los wrappers `scripts\test.cmd` /
  `scripts\lint.cmd` (ya llevan `--config "build.rustc-wrapper=''"`).
- Los worktrees de los agentes van en `C:` (`...\Temp\opencode\wf\<lote>`),
  no en `G:`, porque en `G:` no se pueden borrar archivos y `git worktree
  remove` falla.
- Los ficheros escritos por el `write` de PowerShell quedan con CRLF: un
  script Python que compare bloques exactos tiene que abrir con
  `newline=""` o comparar sin fin de línea.

**Pendiente**

- Los tres lotes de lógica pura (B-A1, B-A2, B-A3) están corriendo.
- Falta el gate de F2 validado contra una rotura deliberada del origen.
- Falta decidir D4 (dónde vive el snapshot del workspace en GPUI).

## 2026-10-20 — Spike del protocolo en Rust: el sidecar Go se queda

**Qué se cambió**

- `docs/protocol-spike.md` (290 líneas), el entregable del lote B-R1.
- El charter, el rulebook y el plan absorben el veredicto (decisiones D1, D4,
  D6, D7).

**Por qué**

El pedido original era "todo en Rust". El spike probó que el protocolo no se
puede migrar sin romper lo más duro del producto: la sesión ya emparejada.

**Evidencia (verificada de forma independiente del informe del agente)**

- `whatsapp-rust` 0.7.0: MIT, 928 estrellas, 146.955 descargas, actualizado el
  07/08/2026, commit el 20/10/2026. Cubre Noise, Signal, app-state,
  newsletters, multimedia y polls.
- Su esquema SQLite es incompatible con el de `whatsmeow`: tablas
  `device`, `identities`, `sessions`, `prekeys` contra
  `whatsmeow_device`, `whatsmeow_session`. Y su store (423 KB de
  `sqlite_store.rs`) tiene **0 menciones de `whatsmeow`**: no hay importador.
- `wa-rs` 0.2.0: parado desde el 17/02/2026, 8 estrellas, un solo
  mantenedor.
- 13.526 líneas de Go de producción en el sidecar.

**Trampas**

- Reescribir el sidecar no mueve la meta: son 31,8 MB de los 120 MB. La RAM
  que duele (367 MB) es el WebView2, que se elimina sin tocar el protocolo.

**Pendiente**

- D5 (spike de 2 días con `whatsapp-rust` para medir RSS y probar si el
  `SessionRecord` de libsignal-go se decodifica) queda para atrás, salvo que
  el humano quiera ese dato.

## 2026-10-20 ? Lotes B-A2 y B-A3 mergeados, y un gap del rulebook que se hizo lote

**Qu? se cambi?**

- B-A2 (8 modulos puros) y B-A3 (6 modulos de estado) mergeados en `main`.
  134 tests en verde, clippy limpio, cero `unwrap`/`panic`/`todo!` fuera de
  los tests.
- Paridad: de 0/153 a **89/153 (58,2 %)** mas 45 tests de caracterizacion.
- El juez de paridad (`scripts/parity.py`) tuvo dos bugs que lohacian mentir:
  buscaba `media-crop.rs` con guion (los archivos usan `media_crop.rs`) y
  solo miraba el archivo homonimo de cada suite. Arreglados: ahora busca en
  todos los archivos de test del destino y distingue los tests de
  caracterizacion (modulos sin suite en el oraculo) de los inventados.
- `lib.rs` ordenado en alfanumerico: `cargo fmt --all` reordena los
  `pub mod` (regla `reorder_modules` de rustfmt) y los agentes lo estaban
  revirtiendo en sus worktrees. Ahora `cargo fmt --all` es inocuo.
- Lote **B-A4** creado: consolidar los DTOs duplicados en `src/types.rs`.
- Rulebook: D8 (donde viven los DTOs compartidos) y D9 (donde viven los
  efectos) resueltos y escritos.

**Por qu?**

A2 y A3 definieron **dos familias paralelas e incompatibles** de `Chat`,
`Message`, `TextContent`, `LinkPreview` y companhia: el rulebook no decia
donde viven los tipos compartidos, asi que cada agente invento los suyos.
No es un error de un agente: es un gap del rulebook. La UI de la fase F4
necesita **un** `Chat` y **un** `Message`, no dos.

**Evidencia**

- B-A2: 36/36 casos del oraculo (20 de media-crop, 4 de polls, 3 de pairing,
  3 de unread, 2 de unsupported, 1 de chat-merge, 3 de file-drop) + 14 de
  caracterizacion para `message_format`.
- B-A3: 53/53 casos del oraculo (31 de workspace, 8 de backend-lifecycle,
  6 de scroll-restoration, 5 de tab-keyboard, 3 de notification-policy) + 31
  de caracterizacion para `app_helpers`.
- Las caracterizaciones seSacaron con un script Node temporal **fuera** del
  repo de origen, congelando `new Date()` y forzando `TZ=UTC`: el oraculo
  quedo intacto.

**Trampas**

- `localeCompare` de JS no es orden de bytes: el oraculo ordena
  `["a","A","b","B"]` donde Rust ordenaria `["A","B","a","b"]`. El puerto
  reproduce dos niveles de colacion (primario sin distinguir mayusculas,
  terciario minuscula antes que mayuscula).
- La verdad de JavaScript: cadena vacia es falsy. Aparece en seis ramas
  distintas de los modulos de estado y hay que decidirla en cada una.
- `runBoundedRetry` era `async` con `window.setTimeout`: en el crate puro
  pasa a ser sincrono con un callback `wait` que recibe el delay, y el plan
  de delays aparte (`[0,100,200]`). Los tests del oraculo se adaptan sin
  cambiar sus valores.

**Pendiente**

- B-A1 (parser de formato) y B-A4 (consolidacion de DTOs) en curso.
- Sigue sin resolverse donde vive el snapshot del workspace en GPUI (D4).

## 2026-10-20 ? B-A1 (el slice duro) mergeado y gate de F2 validado

**Qu? se cambi?**

- B-A1 mergeado: `whatsapp_formatting.rs` (el parser de formato de WhatsApp)
  y sus 14 tests, traducidos 1:1 de las dos suites del oraculo.
- Paridad: de 89/153 a **103/153 (67,3 %)**. 154 tests en verde.
- Tercer bug del juez: partia "WhatsApp" en dos tokens al normalizar, y el
  nombre en Rust lo tiene junto ("whatsapp"). Ahora compara la forma sin
  separadores. Con control negativo: con solo `media_crop` presente, el juez
  reporta 20/153 y da FALTAN en el resto.
- **Gate de F2 cerrado**: el juez (a) pasa contra el origen ? los 14 tests
  TS del oraculo corren verdes en esta maquina ? y (b) atrapa una rotura
  deliberada.

**Por qu?**

El gate de F2 de `repo-migrate` exige dos cosas: que el juez pase contra el
origen y que falle contra una rotura deliberada. Sin la segunda, el juez
puede estar mirando cualquier cosa.

**Evidencia**

- B-A1: 14/14 casos del oraculo, mas 6 tests internos de la capa de URL.
  El agente corrio un **harness diferencial contra el oraculo congelado
  (fuera del repo): 160.272 casos, 0 diferencias**, cubriendo fuzz de
  delimitadores, spoilers anidados, URLs, IPv4/IPv6, IDN, menciones y code
  blocks.
- Gate F2, rotura deliberada: en una COPIA del oraculo se cambio la regexp de
  las listas numeradas de `. ` a `) `; el juez salio con codigo 1 (falla).
  El repo de origen quedo con 0 cambios.
- `cargo clippy --all-targets --locked -- -D warnings`: limpio.

**Desviaciones que el agente documento (y quedan aceptadas)**

- El IDNA del parser de URLs propio esta aproximado con el lowercase de
  Unicode: no reproduce la descomposicion de compatibilidad ni los criterios
  de validez (NFC, Bidi, code points no asignados). Un host que solo falla
  por eso queda como link, mientras el original rechaza la URL entera. El
  harness diferencial de 160.272 casos dio 0 diferencias sobre el corpus
  usado, asi que la diferencia esta en casos que el corpus no cubre.
- El agente escribio ~1.100 lineas de URL Standard de WHATWG a mano porque la
  crate `url` no esta autorizada. Es codigo grande en un solo archivo: la
  capa de URL deberia pasar a su propio modulo cuando se refine.

**Trampas**

- `cargo fmt --all` reordena los `pub mod` de `lib.rs`: los agentes lo
  revierten en sus worktrees. `lib.rs` ya esta ordenado, asi que el comando
  es inocuo en `main`.

**Pendiente**

- B-A4 (consolidacion de DTOs en `types.rs`) en curso.
- Sigue sin resolverse donde vive el snapshot del workspace en GPUI (D4).

## 2026-10-20 ? B-A5, B-A6, B-P1 mergeados, y GPUI?? compila con una sola copia

**Que se cambio**

- B-A5 (5 modulos) y B-A6 (4 modulos) mergeados: paridad de 103/153 a
  **150/153 (98,0 %)**. 219 tests en verde en todo el workspace.
- B-P1: el transporte protobuf en `crates/whatfast-protocol/src/transport.rs`.
  Framing (4 bytes BE + payload, tope de 8 MiB), `classify_exit` con las seis
  agujas del oraculo, `BackendClient` con los bucles de lectura y escritura,
  `RequestTracker` con ids monotonicos desde 1 y tope de 512 en vuelo,
  `RequestClass` con sus plazos (Control 15 s, Read 30 s, Write 60 s,
  Attachment 22 min) y `EventSequence` para el orden del stream. 18 casos del
  oraculo.
- El crate `whatfast-ui` (GPUI) entra al workspace y **compila**: ventana con
  `gpui_component::init`, tema oscuro y layout centrado.

**Por que**

El transporte era el "puente" del F4: sin el, el crate de UI no puede hablar
con el sidecar Go. Y GPUI era el desconocido grande: si no compilaba, todo el
plan de la interfaz caia.

**Evidencia**

- `cargo test --workspace`: 219 en verde. `cargo clippy --workspace
  --all-targets --locked -- -D warnings`: limpio.
- B-A5: 23/23 casos (send-idempotency 3, emoji-autocomplete 10, performance 5,
  sync 4, authoritative-resync 1). B-A6: 24/24 (notification-routing 7,
  participant-avatar-queue 9, spotlight 4, theme 4).
- `voice_note` (el ultimo modulo, 3 casos) quedo en el lote B-A7.
- El shell de GPUI compila incremental en 4,4 s.

**Trampas (las dos mas caras de la sesion)**

1. **Dos copias de gpui en el grafo.** Pinear `gpui` con `rev = ...` hace que
   cargo resuelva una copia con `?rev=` y otra sin, porque los crates internos
   del repo de Zed se declaran entre si por URL plana. Dos copias significa
   dos `AssetSource` distintos y el error "the trait bound `Assets:
   AssetSource` is not satisfied". **Solucion**: declarar `gpui` y
   `gpui_platform` SIN `rev` (como hace el crate `desktop/` del origen) y
   dejar que el `Cargo.lock` versionado fije el commit
   (`zed#1a246efd7e1b83ab568ec5e3e6c1a43a42e1abba`).
2. **`gpui-component` pineado no compila contra el HEAD de Zed**: la firma de
   `wrap_line` cambio y el componente llama con los argumentos viejos. De ahi
   el rev de `gpui` que se uso: el del `Cargo.lock` del crate viejo.
3. Editar el `Cargo.lock` a mano con un reemplazo de texto deja el lock con
   paquetes duplicados y cargo deja de poder parsearlo. La forma correcta de
   resolver un lock pegado es copiar uno que ya funciona y dejar que cargo lo
   adjusting.
4. `is_multiple_of` pide MSRV 1.87 y el crate declara 1.85: clippy lo rechaza
   con `incompatible_msrv`. Se usa `%`.

**Pendiente**

- B-A7 (voice_note) en curso: con el, la paridad llega a 153/153.
- Faltan los lotes de UI (B-U1 tokens, B-U2 shell, B-U3 sidebar como corte
  vertical, y B-U4..B-U8).
- Deuda anotada: el parser de URLs de `whatsapp_formatting.rs` son ~1.100
  lineas de WHATWG dentro de un solo archivo; deberia pasar a su modulo.

## 2026-10-20 ? B-U0: el modelo de datos sale del cliente GPUI a un crate propio

**Que se cambio**

- Crate nuevo `crates/whatfast-model`: `model.rs` (1.267 l.), `paths.rs`,
  `settings.rs`, `sticker.rs` y `emoji_picker.rs`, portados del cliente GPUI
  del oraculo, con **cero GPUI** en el arbol de dependencias (verificado con
  `cargo tree`).
- 31 tests en verde (32 en el oraculo: uno es `#[cfg(unix)]`).
- 250 tests en verde en el workspace.

**Por que**

El modelo de datos es la base de la UI nativa y no deberia arrastrar el motor
de render: un crate que depende de GPUI no se puede testear sin levantar una
ventana, y no se puede reusar desde el binario ni desde una herramienta de
linea de comandos.

**Evidencia**

- `model.rs` usaba GPUI solo por `Pixels`, `Size`, `px` y `size`. El crate
  nuevo define un modulo `geom` con `pub type Pixels = f32`,
  `pub struct Size<T = Pixels> { width: T, height: T }`, `px()` y `size()`.
  No hubo que reimplementar ningun metodo: el modelo solo guardaba
  `Size<Pixels>` y estimaba alturas con `f32`.
- Los DTOs del bridge salen de `whatfast-protocol` (tipos generados en la raiz
  del crate): no hay copia de los tipos.

**Desvios que el agente documento**

1. Cuatro funciones pasaron de `pub(crate)` a `pub`: en el oraculo eran un
   binario, aca la UI es otro crate.
2. `sticker.rs`: el `try_into().unwrap()` de produccion del oraculo se
   lee byte a byte. Mismo valor, sin `unwrap`.
3. `model.rs`, caso `cached_sidebar_ids_allocate_only_when_chats_mutate`: el
   oraculo contaba alocaciones con un `#[global_allocator]` propio, que
   exige `unsafe`, y F7 lo prohibe. Se reescribio con un contador
   `#[cfg(test)] chat_id_rebuilds` (el patron que el propio oraculo ya usa
   para `full_message_reindexes`): conserva el nombre, el montaje y las
   comprobaciones de puntero `Rc`.

**Trampa (nueva, de disco)**

- Los worktrees de los agentes viven en `C:`, y cada uno compilaba su
  propio `target/`. Con el stack de GPUI (que son 3 GB por worktree) **C:
  quedo con 5,8 GB libres** y una compilacion se mato por falta de espacio.
  **Solucion**: `scripts/test.cmd` y `scripts/lint.cmd` ahora fijan
  `CARGO_TARGET_DIR` a un unico target compartido en `G:` (si la carpeta
  existe; si no, cargo usa el local). Con eso, N agentes no llenan el disco
  con N copias de la compilacion.

**Pendiente**

- B-U1 (backend fake) y B-A7 (voice_note) en curso.
- Sigue sin resolverse donde vive el snapshot del workspace en GPUI (D4).

## 2026-10-20 ? B-A7 cierra la paridad del oraculo: 153/153

**Que se cambio**

- `voice_note.rs`: el remuxer de WebM a Ogg Opus. La paridad del oraculo pasa
  de 150/153 a **153/153 (100 %)**. 253 tests en verde en el workspace.

**Por que**

WhatsApp exige contenedores Ogg Opus (OggS con OpusHead) para las notas de voz
push-to-talk, y el grabador del sistema produce `audio/webm`. El modulo
reempaqueta los frames Opus ya codificados: no necesita ningun encoder, asi
que es portable sin dependencias nuevas.

**Evidencia**

- **Paridad byte a byte con el oraculo**: el modulo TS congelado corrido con
  Node 24 y el modulo Rust corrido con `rustc` sobre las mismas entradas
  producen salida identica (hex y longitud): el WebM sintetico del test da
  131 bytes, y un WebM con 3 SimpleBlocks da 191 bytes.
- El test **redemuxea el contenedor generado**: lee pagina por pagina (magic
  `OggS`, `headerType`, granule, serial, secuencia, tabla de segmentos,
  payload), recalcula el checksum con el campo CRC en cero y lo compara con el
  almacenado. El CRC del test se calcula **bit a bit, con otro algoritmo**
  que la tabla del modulo, para que un error no se repita dos veces. Afirma:
  3 paginas, BOS `0x02` en la de OpusHead, `0x00` en la de OpusTags, EOS
  `0x04` en la del audio, granule 960, serial unico y el payload del audio
  igual al frame extraido del WebM.
- `scripts/parity.py`: **153/153 (100,0 %)**, sin casos "sobra".
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: limpio.

**Rarezas que hubo que portar tal cual**

- Todos los campos de la pagina Ogg son **little endian**, la granule
  position incluida: el original usa `setBigInt64(6, granulePos, true)` y
  `true` es little endian.
- El conteo de segmentos se trunca a 8 bits (`page[26] = segmentTable.length`
  sobre un `Uint8Array`).
- El scanner EBML del original es ingenuo y asi queda: en el WebM sintetico
  encuentra el `0xA3` que cierra la magic EBML, lee los 8 bytes siguientes
  como tamano de bloque (el `0x01` del tamano EBML es un varint con el
  marcador en su bit mas bajo) y se queda con 4 bytes, no con los 2 del
  payload. Documentado en el test: cambiarlo romperia la paridad.
- La tabla CRC de 256 entradas se construye en `const fn` en tiempo de
  compilacion, y **no** es el CRC-32 de zlib: es el de Ogg (polinomio
  0x04C11DB7).

**Trampa (la misma de disco, segunda aparicion)**

- C: quedo **con 0 GB libres** y la compilacion del workspace fallo con
  "Espacio en disco insuficiente". Los worktrees de los agentes estan en C: y
  cada uno compila su propio `target/`; con el stack de GPUI son 3 GB por
  worktree. Ya esta corregido: `scripts/test.cmd` y `scripts/lint.cmd` fijan
  `CARGO_TARGET_DIR` a un target compartido en G:.

**Pendiente**

- B-U1 (backend fake) en curso: sin el, la UI no tiene datos con los que
  trabajar.
- Faltan los lotes de UI (B-U2 shell + sidebar como corte vertical, B-U3
  tokens, B-U4 a B-U8).
- Deuda anotada: el parser de URLs de `whatsapp_formatting.rs` son ~1.100
  lineas de WHATWG dentro de un solo archivo; deberia pasar a su modulo.

## 2026-10-20 ? El disco C: limpio y las compilaciones en un solo lugar en G:

**Que se cambio**

- **`G:\_build` es la unica casa de las compilaciones y las caches.**
  Todo lo regenerable esta ahi, y `G:\_build\borrar-cache.cmd` lo borra
  entero de un comando.
- C: paso de **8,6 GB libres a 40,8 GB libres** (168 GB usados -> 136 GB).
- Movidos a `G:\_build`: el target de cargo (10,85 GB), Gradle (14,78 GB),
  el AVD de Android (8,63 GB), la distro WSL `archlinux` (8,11 GB), el cache
  de Pub (1,44 GB) y los navegadores de Playwright (1,92 GB).
- Borrados de C: el cache de shaders de NVIDIA (12,58 GB), una extraccion de
  PyInstaller (1,66 GB), los caches de Go/uv/pnpm (3,84 GB) y ~2.100
  archivos de scratch de sesiones anteriores (3,34 GB).
- **Los worktrees de los agentes que ya terminaron, eliminados** (a1-a7, p1,
  u0, u1). Se conservaron solo los que estan mergeados: verificado con
  `git cherry main <rama>`, que compara por patch-id y no por hash.
- `scripts/test.cmd` y `scripts/lint.cmd` ahora leen `WHATFAST_TARGET_DIR`
  del entorno en vez de tener una ruta fija de esta maquina.

**Por que**

C: se llenaba con cada compilacion: los worktrees de los agentes viven en C:
y cada uno compilaba su propio `target/`, y con el stack de GPUI son 3 GB por
worktree. Dos veces se mato una compilacion por falta de espacio. La
solucion no es solo limpiar: es que **las compilaciones no vivan en C:**.

**Evidencia**

- `G:\_build` queda asi: `whatfast-target` 10,85 GB, `gradle` 14,78 GB,
  `android-avd` 8,63 GB, `wsl` 8,11 GB, `playwright` 1,92 GB, `pub` 1,44 GB.
- El emulador de Android encuentra el AVD en G::
  `emulator -list-avds` -> `leonsys` (con `ANDROID_AVD_HOME`).
- El crate `whatfast-ui` **compila y enlaza** con el target nuevo: se toco
  `main.rs` a proposito para forzar el relink, y produjo el `.exe` de 45 MB.
- 267 tests en verde en el workspace.

**Trampas (dos, y una me costo trabajo)**

1. **El scratch de `Temp\opencode` contenia `msvc.ps1`**, el script que
   deja el linker de MSVC en el PATH. Al limpiar la carpeta, borre el script.
   **Recreado** en el mismo lugar, ahora con una version que llama a
   `vcvars64.bat` y importa el entorno, en vez de tener las rutas fijas.
2. **Mover `.gradle` y el AVD no es un rename**: son volumenes distintos, y
   hay que reescribir las rutas absolutas. El `.ini` del AVD
   (`leonsys.ini`) tiene un `path=C:\Users\perca\.android\avd\leonsys.avd`
   que hay que cambiar a mano.

**Pendiente**

- La distro de WSL y el AVD de Android **no son caches**: se pueden borrar
  (libran 16 GB), pero se pierden el sistema y el emulador.
- El `opencode.db` (5,79 GB en `C:\Users\perca\.local\share\opencode`) es
  el historial de sesiones: no se toco.

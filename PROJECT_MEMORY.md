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
   (`leonsys.ini`) tiene un `path=<perfil>\.android\avd\leonsys.avd`
   que hay que cambiar a mano.

**Pendiente**

- La distro de WSL y el AVD de Android **no son caches**: se pueden borrar
  (libran 16 GB), pero se pierden el sistema y el emulador.
- El `opencode.db` (5,79 GB en `<perfil>\.local\share\opencode`) es
  el historial de sesiones: no se toco.

## 2026-10-20 - El target compartido se resuelve solo (sin variables de entorno)

**Que se cambio**

- `scripts/test.cmd` y `scripts/lint.cmd` resuelven el target compartido en
  este orden: `CARGO_TARGET_DIR`, despues `WHATFAST_TARGET_DIR`, y si no hay
  ninguno, **el `.cache\target` del worktree principal**, que se localiza
  con `git rev-parse --git-common-dir`.
- `G:\Proyectos\17-whatfast-rs\.cache\target` es un **enlace de directorio**
  (junction) que apunta a `G:\_build\whatfast-target`. Los bytes viven en
  `G:\_build`; el repo solo tiene el puntero. `.cache/` esta en el
  `.gitignore`.
- Verificado simulando a un agente: desde un worktree en `C:`, **sin ninguna
  variable de entorno**, `scripts\test.cmd` compila en el target de `G:` y
  **no crea** `target\` en el worktree.

**Por que**

Los procesos que ya estaban abiertos (el opencode de esta sesion, y por lo
tanto los agentes que lanza) no ven las variables de entorno nuevas: el
proceso tiene su propia copia del entorno desde que arranco. Con el enlace,
el path se deriva del repositorio y no hace falta configurar nada.

**Trampas (dos, y las dos costaron tiempo)**

1. **Dentro de `for /f`, el `=` se come**: `for /f "delims=" %%i in ('git
   rev-parse --path-format=absolute --git-common-dir')` se ejecuta como
   `--path-format absolute` y git falla en silencio. **La forma que si
   funciona** es `git rev-parse --git-common-dir` a secas: en un worktree
   devuelve igual la ruta absoluta del `.git` del repo principal.
2. **Un worktree tiene su propia copia de los scripts**. El worktree `u2`
   se creo antes del arreglo y seguia con la version vieja, que apuntaba a
   una carpeta que ya no existia: por eso compilaba en el disco local. Los
   worktrees de los agentes se crean desde `main`, asi que hay que
   **commitear el arreglo antes de crear el worktree**.

**Pendiente**

- `whatfast-ui` tiene un `[profile]` en su `Cargo.toml` y cargo avisa:
  "profiles for the non root package will be ignored". Anotado para el
  lote de UI: el perfil tiene que estar en la raiz del workspace.

## 2026-10-20 - El uso de disco de Rust: de 39,8 GB a 12,9 GB

**Que se cambio**

- **El perfil de desarrollo**. El `[profile.dev]` estaba en
  `crates/whatfast-ui/Cargo.toml`, y cargo **ignora los perfiles de los
  miembros del workspace**: no se aplicaba ninguno. Ahora esta en la raiz
  del workspace, con `debug = "line-tables-only"`.
  **Medido por crate**: el rlib de `image` pesa 62,6 MB con `limited` y
  24,4 MB con `line-tables-only` (igual que sin debug, pero conserva los
  numeros de linea).
  **Medido sobre el stack completo: NO ahorra nada** (3,40 GB con el perfil
  viejo y 3,41 GB con el nuevo, dos builds limpios en directorios
  separados). El tama?o lo domina el stack de Zed, no la info de debug.
  Se deja `line-tables-only` porque es el ajuste correcto y no cuesta nada.
  **La correccion importante**: los 10,95 GB que pesaba el target NO eran
  el tama?o de una compilacion: eran el accumulo de varias configuraciones
  de perfil, porque **cargo nunca borra los artefactos que quedaron
  viejos**. Un `cargo clean` y a compilar de nuevo: **2,64 GB**.
- **sccache 0.18.0** instalado (binario precompilado, 9,2 MB) en
  `G:\Dev\cargo\bin`, con la cache en `G:\_build\sccache` y un tope de
  8 GiB. `RUSTC_WRAPPER=sccache` a nivel de maquina: los nueve proyectos
  Rust de G: comparten la cache. **Medido**: el stack completo de Zed
  ocupa **506 MB en la cache** frente a los 2,64 GB del target, y el rebuild
  paso de 225 s a 100 s (427 aciertos).
- **Toolchain GNU desinstalado** (1,54 GB): no lo usa ningun proyecto.
  `rustup` paso de 3,51 a 1,96 GB.
- **Tarballs del registro de cargo borrados** (0,38 GB): los `.crate` son
  redundantes con las fuentes ya extraidas en `registry\src`.
- **Targets de proyectos inactivos borrados** (12,39 GB): el repo viejo
  congelado (10,46 GB), `Descargas\voicebox` (4,98 GB),
  `opencode-remote-android` (2,72 GB), `20-vistamd` (1,61 GB),
  `10-widget-tablero` (2,40 GB) y `Bibliosflutter` (0,30 GB).
  Los `.exe` de `rust-meow` quedaron porque **la app estaba corriendo**.

**Por que**

El plan de la interfaz necesita GPUI, y el stack de Zed (gpui, wgpu,
font-kit) es un orden de magnitud mas caro que el resto del proyecto. Con
un solo target compartido y la info de debug recortada, la iteracion de los
agentes deja de costar 3 GB por worktree.

**Evidencia**

- Target del repo: 10,95 GB -> **2,64 GB** (-76 %).
- Cache de sccache: 506 MB para el stack completo.
- 267 tests en verde, clippy limpio, desde el worktree y sin target local.

**Trampas (cuatro, y dos hicieron perder horas)**

1. **Un worktree tiene su propia copia de los scripts**. El worktree se
   creo antes del arreglo y seguia con la version vieja: por eso
   compilaba en el disco local. **Hay que commitear el arreglo antes de
   crear el worktree**.
2. **`git rev-parse --git-common-dir` devuelve ruta RELATIVA (`.git`) en el
   repo principal**, y absoluta en un worktree. La unica forma que sirve
   para los dos: `git worktree list --porcelain`, que siempre devuelve la
   ruta absoluta del worktree principal.
3. **Dentro de `for /f`, el `=` se come**: `--path-format=absolute` se
   ejecuta como `--path-format absolute` y git falla en silencio.
4. **Un enlace de directorio colgante rompe a sccache**: la cache de
   compilaciones daba `Cache write errors: 936` en cada escritura, porque
   el directorio real habia desaparecido y el enlace apuntaba al vacio.
   `sccache` **no** es capa de tolerar eso. Los scripts ahora crean los
   directorios si no existen.

**Pendiente**

- Los targets de los proyectos **activos** (`computer-use2-rs` 5,49 GB,
  `qwen-image-rs` 0,71 GB, `noblex` 0,18 GB) se dejaron intactos. Con
  sccache activo se pueden borrar y restaurar en la mitad de tiempo.
- Los proyectos MCP pinean `1.98.0` en su `rust-toolchain.toml`, que no
  esta instalado: cuando se compilen, rustup baja ese toolchain aparte
  (0,56 GB). El `stable` de esta maquina **es** 1.98.0.

## 2026-10-20 - Tanda 1: el sidecar en Go y los tokens del tema

**Que se cambio**

- **`backend/`**: el sidecar en Go del repositorio de origen, copiado al
  repo nuevo. 32 archivos, **identicos byte a byte** (verificado por
  SHA256 contra el oraculo), mas tres archivos nuevos: `.gitignore`,
  `README.md` y `tools/smoke/main.go`.
- **`crates/whatfast-ui/src/tokens.rs`**: los tokens del tema (1.133 l., 21
  tests). La escala de la interfaz, las medidas del CSS, los 30 colores del
  tema convertidos a `Rgba` y la aplicacion del tema sobre `gpui_component`.
- **`crates/whatfast-ui/src/main.rs`**: la entrada de la app (la ventana, las
  acciones globales, los modulos).
- **CI y release**: `.github/workflows/ci.yml` (tests, lint y juez de
  paridad) y `release.yml` (el paquete de Windows y de Linux).
- **`scripts/build.cmd`**, **`scripts/medir.ps1`** y **`scripts/capturas.ps1`**.
- **`docs/api-mapping.md`** cerrado: los 50 RPC del sidecar y los 56 comandos
  de la app web, con lo que falta (22 cubiertos, 28 pendientes).

**Evidencia**

- `go build ./...` y `go test ./...`: exit 0. 226 casos, 220 pasan, 6 se
  saltan (tests de permisos POSIX que hacen `Skip` en Windows, tal como en
  el oraculo).
- **El binario real responde un `Hello`**: `hello ok: protocol_version=21
  backend_version="0.1.0"` y `shutdown ok: the sidecar exited with code 0`.
  El smoke test levanta el binario de produccion con un `--data-dir`
  temporal, manda el `Hello` con el codec de produccion y lee la respuesta
  con el mismo framing que el lado Rust.
- El modelo de dominio del tema **coincide exacto con el oraculo en los 7
  temas built-in**: 0 diferencias en los 4 temas con objeto plano y 0 en los
  32 overrides de los 3 temas derivados.

**El hallazgo del tema**

- **El CSS no declara ningun color de tema.** Las unicas custom properties
  del `:root` son la escala, cuatro radios, tres alturas y seis colores del
  *swatch* de los ajustes (hardcodeados, no son el tema). Los 30 colores los
  escribe en runtime `applyTheme` de `lib/theme.ts` sobre el `<html>`. Por
  eso la comparacion real es contra el TypeScript, no contra el CSS.
- Hay **dos juegos de radios distintos**: el `:root` usa 6/8/12 px, y el
  estilo skeuomorfico (`html[data-theme-style="skeuomorphic"]`, lineas
  4345-4351 del CSS) los reemplaza por 5/7/9.
- `--radius-full` (999px) es un literal en los dos bloques: la escala no lo
  toca.

**Trampas (dos, una de entorno)**

1. **El GOROOT de esta maquina estaba incompleto**: le faltaban 70 archivos
   de la distribucion oficial de Go 1.26.7 (`src/go/build/` completo,
   `src/cmd/dist/` y `src/cmd/vendor/golang.org/x/build/`). Por eso
   `go build ./...` pasaba pero **`go test ./...` fallaba en los 8 paquetes
   con tests**: `go/build/constraint` importa `go/parser`, y el grafo de
   tests lo necesita. **Se arreglo el toolchain, no el codigo**: se bajo
   `go1.26.7.src.tar.gz` de go.dev, se verifico el SHA256 contra el
   publicado y se restauraron los 70 archivos.
2. El `bridge.proto` del oraculo usa **LF** y el del repo nuevo usa
   **CRLF**: el contenido es identico (comparado normalizado), pero los
   SHA256 crudos no coinciden. Los 13 archivos del oraculo que usan CRLF
   quedaron con LF por la normalizacion estandar de git.

**Pendiente**

- Faltan los modulos `app.rs`, `rpc.rs`, `text.rs`, `render.rs` y
  `helpers.rs` de la interfaz (dos agentes en curso).
- En la integracion: reconciliar `apply_theme_scale` (el del tema y el que
  trae el oraculo), llamar `tokens::apply_theme` al arrancar y sacar el
  `#![allow(dead_code)]` del modulo de tokens.
- Los 28 RPC que faltan y los 11 componentes que el cliente GPUI del
  oraculo no tiene (los que estan en `gpui_component`).

## 2026-10-20 - Tanda 1 completa: la app nativa anda de punta a punta

**Que se cambio**

- **`crates/whatfast-ui/`**: los 6 modulos de la interfaz.
  `app.rs` (3.039 l., el estado y las acciones), `rpc.rs` (434, el ciclo de
  vida de los pedidos), `text.rs` (234, los helpers de texto),
  `render.rs` (3.034, el dibujo), `helpers.rs` (236) y `tokens.rs` (1.133, el
  tema). `main.rs` (49 l.) es el entrypoint.
- **La app corre**: abre la ventana, se conecta al backend, lista 10.000
  chats del backend de demostracion, abre conversaciones, dibuja las
  burbujas con reacciones, respuestas y previews de links, y pinta las fotos
  que genera el backend.
- **README nuevo** con la tabla comparativa y los numeros medidos.
- **Cuatro capturas** propias en `docs/screenshots/`.

**Evidencia**

- `scripts\test.cmd`: **288 tests en verde**. `scripts\lint.cmd`: limpio.
- `scripts/parity.py`: **153/153 (100 %)** del oraculo.
- **RAM: 91,9 MB** (mediana de 3 corridas del release con el sidecar real):
  60,5 MB la interfaz + 31,4 MB el sidecar. La meta era < 120 MB.
  El cliente anterior con webview usaba **432 MB**: el 85 % era el WebView2.
- **Ventana visible: 0,70 s** (mediana de 3). La meta era < 0,5 s: **no se
  llega**. El costo es la inicializacion de la plataforma de GPUI (la base
  de datos de fuentes del sistema), no el codigo: con y sin sidecar la
  diferencia es de 0,03 s, y con y sin aplicar el tema, de 0,1 s.
- **UI lista (lista de chats)**: ~1,1 s (el cliente anterior: 1,16 s).
- El paquete de descarga: **33,2 MB** (10,6 la app + 22,6 el sidecar).

**Tres defectos del cliente GPUI del origen que se corrigieron**

1. **La fila de la lista de chats no tenia ancho.** En GPUI el
   `uniform_list` mide cada item, asi que la fila no heredaba el ancho de la
   lista: el preview se desbordaba hacia el panel de conversacion y
   **empujaba los contadores de no leidos fuera del panel**. Con `.w_full()`
   en la fila, el preview se recorta con puntos suspensivos y los
   contadores vuelven a verse.
2. **El titulo de la cabecera se partia en dos lineas** cuando los botones
   no entraban. Con `.flex_1().min_w_0().truncate()` y los tres botones
   como iconos (sol, carpeta, engranaje) el header entra en los 340 px del
   panel lateral.
3. La escala de la interfaz estaba **definida tres veces** (las constantes
   en `app.rs`, `normalized_ui_scale` en `text.rs` y el modulo `tokens.rs`).
  Ahora `tokens.rs` es el unico due?o y los otros dos re-exportan, para no
  romper los imports que ya estaban escritos.

**Pendiente (la tanda 2)**

- **28 de los 50 RPC** sin interfaz: sincronizacion e historia, medios que
  no son imagen, reenvio, edicion y borrado, encuestas, newsletters,
  estados y las acciones del chat (fijar, archivar, silenciar, bloquear).
- **11 componentes** que tiene el cliente web: barra de titulo, strip de
  tabs, panel de miembros, lista de participantes, panel de newsletters,
  tira de estados, bandeja de stickers y el editor de imagen.
  De estos, la barra de titulo, las tabs, el sidebar, el avatar, el dialogo,
  el menu y los ajustes **ya existen en `gpui_component`**: la tanda 2 es
  mas cableado que dibujo.

## 2026-10-20 ? Tanda 2 cerrada: los 50 RPC y los 21 componentes

- **Los 28 RPC que faltaban**, de punta a punta (la peticion, el estado del
  store y la accion que la dispara), en el lote `r1`: 25 tests nuevos, el
  store con 11 campos y 30 metodos nuevos (`apply_sync_status`,
  `set_history_coverage`, `set_pinned_messages`, `set_sticker_packs`,
  `set_statuses`, `set_newsletters`, `apply_poll_vote`, `revoke_message`...).
  La interfaz ahora pide **50 de 50** RPC (lo cuenta `scripts/funcionalidad.py`).
- **Los componentes que faltaban**, en cuatro lotes: `r2` (los 34 iconos del
  cliente web y el editor de imagen con recorte libre y rotacion),
  `r3` (la columna de iconos de 56 px y el strip de pesta?as con arrastre y
  teclado), `r4` (los cuatro paneles: newsletters, estados, miembros y
  bandeja de stickers) y `r5` (el menu del mensaje, el reenvio, el
  formulario de encuesta y el clip de adjuntos). La app cubre **21 de 21**
  componentes del cliente web.
- **El editor de imagen esta completo**: el agente entrego el recorte y la
  rotacion, pero el plan no se aplicaba a ningun archivo. Se escribio
  `apply_edit` (el crate `image` rota, recorta y guarda) y el app manda el
  archivo editado.
- **El cerrojo de instancia unica** (`instance.rs`): dos instancias serian
  dos sidecars con la misma sesion del dispositivo, y el servidor podria
  invalidarla. Un archivo de bloqueo con el PID, liberado al salir, con la
  comprobacion de que el proceso sigue vivo.
- **Defecto encontrado y corregido del script de capturas**: las capturas
  salian recortadas porque el rectangulo de la ventana esta en pixeles
  logicos y la pantalla del 125 % pide los fisicos. Con `GetDpiForWindow` la
  captura sale entera: se veia que el compositor de la conversccion nunca
  habia estado en la foto (no era un defecto del layout).
- **El backend de demostracion** devolvia un chat vacio al fijar, archivar,
  silenciar o bloquear: el titulo y el avatar se perdian. Ahora devuelve el
  chat entero del fixture con la bandera cambiada.
- **El workflow de release** de Linux no tenia las dependencias del sistema
  que pide el stack de GPUI, y el instalador de Windows (NSIS) no existia.
- **Estado**: 438 tests en verde, clippy limpio, paridad 153/153,
  50/50 RPC y 21/21 componentes.
- **Publicado**: la etiqueta `v0.1.0-beta.1` disparo el workflow de release y
  salio con tres artefactos: `whatfast-setup.exe` (el instalador NSIS de
  Windows), `whatfast-windows.zip` y `whatfast-linux.tar.gz`. Los dos jobs
  (Windows y Linux) compilaron el stack de GPUI en el runner: el de Linux
  tambien, con las dependencias del sistema que pidio el workflow.
- **El CI de GitHub** corre el formateo, el clippy, los tests y el juez de
  paridad en los dos sistemas. La primera corrida fallo por el formateo (los
  agentes formatearon archivo por archivo con `rustfmt` y no con
  `cargo fmt`): se corrigio con `cargo fmt --all` y quedo limpio.

## 2026-10-21 ? El logo de la app y el acceso directo en el escritorio

- **El logo ya estaba**: `assets/icons/icon.png` (512x512, la burbuja verde
  con las lineas de velocidad del cliente web). Lo que faltaba era??
  llevar a donde Windows lo mira.
- **El `.ico` ahora tiene los diez tamanos** (de 16 a 256) y lo regenera
  `scripts/iconos.py` desde el logo maestro. El que habia venia del cliente
  web con seis tamanos y el de 32 era el que se veia en la barra.
- **El ejecutable lleva el icono incrustado**: un `build.rs` en
  `crates/whatfast-ui` compila el recurso con `embed-resource` (que ya
  estaba en el lock como dependencia de transitive) y `rc.exe` del SDK de
  Windows. Sin esto, el Explorador, la barra de tareas y el alt-tab
  mostraban el icono generico.
- **El instalador de Windows** (NSIS) deja el logo en los accesos directos
  del escritorio y del menu Inicio, y en el archivo `whatfast-setup.exe`.
- **`scripts/instalar.ps1`** instala la app en
  `%LOCALAPPDATA%\Programs\Whatfast` (por usuario, sin permisos de
  administrador) y crea los dos accesos directos con el logo. Es la forma
  de probar la app en esta maquina sin depender del instalador de NSIS,
  que solo se compila en el runner de GitHub.

### Lo que se agrego despues de esa entrada

- **Los estados, los canales y los archivados son destinos del riel**: cada
  uno con su columna y su panel a la derecha, como en la referencia. El boton
  de los estados abre la vista con "My status" y la seccion "RECENT"; el de
  los canales, la lista con buscador, la hora a la derecha y los botones
  "Follow" y "New local channel"; el de los archivados, la explicacion de
  arriba y la lista.
- **El compositor de estado de texto** quedo en el panel de la vista de los
  estados, con los botones "Text status" y "Photo and video" de la
  referencia: el primero abre el compositor y el segundo elige la foto.
- **La fila de chat** quedo con la hora a la derecha del titulo, el
  prefijo "You:" y los indicadores de silencio, favorito y pin.
- **La captura de las pantallas** la hace `scripts/captura-frontal.ps1`:
  trae la ventana al frente (con el truco de ALT, la unica forma de que
  Windows deje robar el foco), la deja en la esquina y mide el boton del
  riel sobre una foto de prueba (`scripts/medir_riel.py`).

**Evidencia**: `cargo test --workspace` en verde, `cargo clippy
--all-targets --locked -- -D warnings` limpio, `python
scripts/funcionalidad.py` con 50/50 RPC y 21/21 componentes, `python
scripts/parity.py` con 153/153 casos del oraculo.

**Pendiente**: la foto de la vista de estados y la de la de canales: el clic
del guion no llega a los botones del riel (el de las filas de chat si), asi
que hay que hacerlas a mano.

### La paleta, los iconos y las fotos, como en la referencia

- **La app se pinta con la paleta de WhatsApp**: los dos temas nuevos del
  catalogo del nucleo (`whatsapp-dark` y `whatsapp-light`) con los colores
  medidos sobre `docs/referencia/01-whatsapp-chats.png` (la columna de
  chats `#161717`, la de los iconos `#1d1f1f`, el buscador `#2e2f2f`, el
  verde `#21c063`). El catalogo del cliente web sigue intacto: el tema por
  defecto del nucleo sigue siendo `vercel-dark` y la app elige el suyo.
- **Los iconos del riel** son los de la referencia: la burbuja del chat, la
  lupa, el circulo punteado de los estados, la burbuja de los canales, la
  caja de los archivados, el escudo y el engranaje. El boton activo es un
  cuadro gris claro, no del color de acento.
- **La cabecera de los chats** lleva el boton verde redondo de nuevo chat,
  el buscador en pastilla con la lupa dentro y el estado de la
  sincronizacion al pie de la columna (como la franja del cliente web).
- **Cada conversacion tiene su foto**: el backend de demostracion
  genera un avatar cuadrado por chat (`ensure_fake_avatar`) y la app lo
  pide para todas las filas visibles, como hace el origen.
- **El avatar no se borra** cuando el backend vuelve a mandar el chat sin
  el camino de la foto: el modelo lo conserva, como ya hacia con las
  imagenes de los mensajes.
- **El boton del tema y el de la salida** se fueron al panel de ajustes, y
  la cabecera de la conversacion lleva el buscador, como la referencia.

### Los atajos de teclado de las vistas nuevas

- **Los estados, los canales y los archivados** responden al teclado:
  `Ctrl+Shift+S`, `Ctrl+Shift+N` y `Ctrl+Shift+H`. Son vistas propias y la
  columna de iconos no es la unica puerta de entrada.
- **`scripts/captura-estados.ps1`** abre una de esas vistas con el atajo y
  saca la foto: el riel no toma los clics sinteticos (los de las filas de la
  lista si), asi que el guion usa el teclado.

### El menu de la fila y la cabecera de la conversacion

- **El clic derecho sobre una fila abre su menu**: fijar, silenciar,
  archivar, favoritos, marcar como leido o no leido y bloquear el contacto.
  Son los items del `chat-context-menu` del cliente web, en el mismo orden.
  El menu abre donde esta el puntero, se cierra con Escape o con un clic
  fuera, y el separador antes de bloquear solo aparece en los contactos que
  no son grupos.
- **Marcar como no leido** suma uno al contador de mensajes sin leer (el
  cliente web hace lo mismo) y **marcar como leido** lo deja en cero; si la
  conversacion es la que esta abierta, el marcado va con el RPC.
- **La cabecera de la conversacion** quedo como la de la referencia: la foto
  de 41 px, el nombre, la linea `Tap for contact info` (o `Tap for group
  info`) y a la derecha la videollamada, la llamada, el buscador y el menu.
- **Los glifos de la llamada** (la camara y el auricular) son los unicos que
  no estan en el cliente web: el cliente no tiene llamadas y avisa que el
  backend todavia no las tiene.
- **El panel de los miembros** de un grupo se abre con el boton de la
  cabecera, como en el cliente web.
- **Bug del lock de instancia unica**: el archivo de bloqueo solo guardaba el
  numero del proceso, y Windows vuelve a dar esos numeros a otros
  programas: un lock de una app matada podia bloquear la app para siempre.
  Ahora el lock tambien escribe el nombre del programa y compara los dos.

### Las burbujas y las acciones de los mensajes

- **Las acciones de un mensaje van fuera de la burbuja**, abajo: el estado
  de lectura, las respuestas, el corazon y el menu. La referencia de
  WhatsApp Desktop las pinta debajo de la burbuja y no dentro.
- **Los mensajes recibidos no tienen burbuja**: el texto va sobre el fondo de
  la conversacion. Los enviados usan el verde de la referencia (`#144d37`).
- Los verdes que estaban escritos a mano (la barra de la cita, el borde del
  mensaje buscado y el de las reacciones) salen ahora del acento del tema.

### La paridad visual, con agentes en paralelo

Ocho agentes trabajaron en worktrees separados (`agente/<nombre>`) y sus
ramas se integraron con `scripts/integrar-agentes.ps1`. Cada uno tenía una
zona de archivos y nadie toco `render.rs` fuera de su region.

- **`conversacion`**: el fondo con dibujitos (una capa fija de glifos a una
  opacidad de 0.055), la cola de la burbuja (un triangulo del color de la
  burbuja), la hora dentro de la burbuja, la pastillita de "Load older
  messages" arriba y el boton redondo de "ir a los ultimos" abajo a la
  derecha. La burbuja ahora hugge su contenido (`align-self`).
- **`emojis`**: el panel es una caja flotante con borde y sombra, la
  categoria activa lleva la linea verde de 2 px debajo, hay seccion
  "Recent" (los emojis usados, en `emoji_recent.rs`) y la rejilla es densa
  (44 px, diez columnas).
- **`stickers`**: la bandeja flotante con el buscador de la tienda de
  stickers, el boton de "Crear" y la fila de packs.
- **`lista`**: el boton de la flecha al final de los filtros, que abre el
  menu con los filtros de la app, y la columna de la derecha de la fila
  con la hora arriba y los dibujitos (silencio, favorito, pin) abajo.
- **`paneles`**: el panel de ajustes con la forma del perfil de la
  referencia (nombre, buscador con borde verde, foto en un circulo y las
  secciones con dibujo) y el de info del contacto con titulo en negrita y
  valor apagado.
- **`adjuntos`**: la tarjeta de un archivo que viene con un mensaje (el
  dibujo del tipo, el nombre, el tamano, la duracion y el mime), que baja
  el archivo con el RPC y lo abre con el programa del sistema.
- **`fake-media`**: el backend de demostracion genera un documento, un
  video, un audio y una nota de voz, con los archivos escritos en disco.
- **`ubicacion`**: la tarjeta de la ubicacion (que abre el mapa) y la de
  los contactos (que copia la vcard).

- **Las horas del backend de demostracion son reales**: antes cada mensaje
  tenia `timestamp_ms: id` (las 00:00 de 1970). Ahora la historia va para
  atras desde el ahora, un mensaje cada noventa segundos, asi que se ven
  la hora, el "Today"/"Yesterday" y el separador de dia.
- **Los tres enganches de las tarjetas** (el adjunto, la ubicacion y los
  contactos) los hice yo al final, en `render.rs`: cuelgan la tarjeta
  antes del texto, dejan de repetir el texto debajo y suman el alto de la
  tarjeta a la medida de los mensajes (si no, la lista virtual la corta).
- **Trampa**: un dialogo de "Open with" del sistema, abierto por una
  prueba de los adjuntos, se queda con el foco y hace fallar todas las
  capturas ("la ventana de la app no queda en primer plano"). Se cierra
  matando el proceso `OpenWith`.

## 2026-10-22 - El puerto a Linux y macOS de la interfaz

- **Que**: `whatfast-ui` compila para Linux (check, clippy, build con enlace
  y 185 tests de la interfaz) y para macOS (`cargo check --target
  x86_64-apple-darwin`), sin cambios en Windows.
- **Por que**: la meta es que la interfaz no sea solo de Windows.
- **Como**: `is_alive` de unix usa `libc::kill(pid, 0)` en vez de `/proc`
  (macOS no tiene `/proc`: el lock se veia vencido siempre y el arranque no
  era de una sola instancia); `build.rs` decide por `CARGO_CFG_TARGET_OS` y
  no por `#[cfg(windows)]` (en compilacion cruzada el host no es el destino);
  `libc` como dependencia solo de unix.
- **Evidencia**: WSL archlinux nativo: `cargo check`, `clippy --all-targets
  -D warnings`, `cargo build` (ELF x86-64) y `cargo test --workspace` (20
  suites ok, `-p whatfast-ui` 185 ok). Windows: build, clippy -D warnings y
  34 suites ok; el icono sigue incrustado (`rc.exe` -> `icon.lib`).
- **Trampa**: cruzar desde Windows no sirve: `psm` pide
  `x86_64-linux-gnu-gcc`; y `media` y los shaders de `gpui_macos` solo se
  generan con host macOS (`xcrun`). Hace falta un runner nativo.
- **Trampa**: `yeslogic-fontconfig-sys` exige `fontconfig.pc`: sin
  `libfontconfig-dev` (`fontconfig` en Arch) el build de Linux muere antes
  de compilar la interfaz.
- **Pendiente**: el enlace de macOS no se pudo verificar (esta maquina no tiene
  SDK de Apple); lo cubre el CI en un runner macOS.

### La app en Linux y macOS

- **La interfaz compila para Linux y para macOS.** Verificado con los targets
  `x86_64-unknown-linux-gnu` y `x86_64-apple-darwin`: en Linux ademas enlaza
  (un ELF x86-64) y pasan los tests nativos en WSL Arch Linux; en macOS el
  `cargo check` da cero errores y cero avisos, pero el enlace necesita el SDK
  de Apple y por eso lo cierra el job del CI en un runner macOS.
- **El lock de instancia unica estaba roto en macOS**: usaba `/proc/<pid>`,
  que en macOS no existe, asi que el lock se veia vencido siempre. Ahora usa
  `libc::kill(pid, 0)` en unix (EPERM cuenta como vivo).
- **El icono del ejecutable** se decide por el sistema de destino
  (`CARGO_CFG_TARGET_OS`) y no por el de la maquina: al cruzar, el recurso iba
  al sistema equivocado.
- **La compilacion cruzada de Linux desde Windows no sirve** para verificar:
  `psm` pide `x86_64-linux-gnu-gcc`. Se verifica con WSL nativo o con el CI.
  El `cargo check` de macOS desde Windows tampoco alcanza: los build scripts
  de `media` (los bindings con `xcrun`) y de `gpui_macos` (los shaders) se
  compilan para el host y quedan vacios.
- **Los paquetes de las tres plataformas** salen del workflow de release: el
  `.zip` y el instalador de Windows, el `.tar.gz` de Linux con el `.desktop` y
  los iconos en `share/`, y el `Whatfast.app` de macOS (arm64 nativo y x64
  cruzado, con firma ad hoc). La guia de instalacion esta en
  `docs/plataformas.md`.

## 2026-10-22 - El riel que no respondia

- **El riel no cambiaba de vista** (estados, canales, archivados): los botones
  quedaban deshabilitados porque la columna se sincronizaba con la pantalla
  solo en `sync_shell`, y ese metodo no se llamaba cuando el backend respondia
  la lista de chats ni el progreso del sync. La columna se quedaba en
  `LinkState::Pairing` para siempre y solo respondia el engranaje, que tiene
  `enabled: true` fijo. Ahora `handle_bridge` sincroniza despues de cada
  mensaje del backend (commit `10b6aca`).
- **Como se encontro**: un `eprintln!` temporal en el `on_click` del boton y en
  `on_nav_action` mostro `CLICK States enabled=false` y ningun `NAV States`,
  mientras que el engranaje si llegaba a `NAV Settings`.
- **Evidencia**: `cargo fmt --check` limpio, `clippy --all-targets -D warnings`
  limpio, 34 grupos de tests en verde, `funcionalidad.py` con 50/50 RPC y 21/21
  componentes, `parity.py` con 153/153.
- **Trampa para las capturas**: con el navegador de OpenCode en primer plano,
  `AttachThreadInput` y `SetForegroundWindow` fallan (devuelven `False`) y la
  primera pulsacion sobre la app solo la activa. Hay que poner la ventana
  arriba con `SetWindowPos` y `HWND_TOPMOST` e insistir hasta que el foco quede
  en la app, o el primer clic de la secuencia se pierde.

## 2026-10-22 - El panel de ajustes dejaba un bloque gris

- **El tema claro mostraba el resto de la ventana en gris** al abrir los
  ajustes: el panel se dibujaba sobre un velo `theme.overlay`, que en el tema
  claro es negro al 40 % sobre blanco. La referencia no oscurece nada: se
  quitó el velo y la conversación sigue visible al lado (commit `44faa41`).
- **El primer build de release salió corrupto** (`E0786: corrupt metadata`
  en `libwhatfast_protocol` y después en `gpui`): el corte del servidor mató
  `rustc` a mitad de escritura. Se arregla con `cargo clean --release` y
  6m55s de compilación; limpiar un solo crate no alcanza.
- **Evidencia**: las cinco puertas en verde el 22/10 (fmt, clippy, 34 grupos
  de tests, `funcionalidad.py` 50/50 y 21/21, `parity.py` 153/153), las doce
  vistas capturadas y revisadas, `main` pusheado y la etiqueta
  `v0.1.0-beta.3` movida a `44faa41`.

## 2026-10-22 - El README reconstruido y las fotos de la pagina

- **El README** paso a una galeria de dos columnas con las 15 capturas
  agrupadas por tema (lista y vistas propias, conversacion y compositor,
  paneles de la derecha, ajustes) y se corrigio el titulo `Quéfast` por
  `Whatfast`. Los 20 enlaces e imagenes se verificaron con un comando.
- **La pagina** (`web/index.html`) tenia 6 vistas y las fotos viejas: ahora
  tiene 15 botones, cada uno con su captura nueva en `web/assets/`, y la
  columna del banco de vistas se desplaza adentro de un marco de 520 px
  (media query de 993 px) para que la captura no cambie de tamano al cambiar
  de vista.
- **Verificado en el navegador**: las 15 imagenes cargan con 1475 px de ancho
  y el clic en un boton cambia la captura y el rotulo. Medido con
  `browser.evaluate`: 15 botones y columna de 520 px con 799 px de contenido.
- **Trampa**: `web/` sigue fuera del indice de git. No se commiteo porque no
  hay un pedido explicito; si la pagina se publica, hay que agregarla.

## 2026-10-22 - La pagina se recorre haciendo clic sobre la captura

- **El visor**: `web/index.html` ya no tiene la columna de 15 botones ni la
  barra de RAM/FPS/JS. Queda la captura sola dentro del marco y, encima,
  zonas transparentes (`button.hotspot`) en porcentajes del lienzo
  1475x950: riel, filas de chats, chevron de filtros, cabecera, clip, emoji,
  boton de tema y cerrar. El clic cambia la imagen (`render(id)`), con
  `VIEWS` de 13 vistas y `RAIL` compartido.
- **Por que**: el pedido fue que se vea solo la captura y que se navegue como
  en la app, no con una lista tecnica al costado.
- **Lo que no se alcanza**: `03-mensaje-con-foto.png` no tiene entrada
  realista en el recorrido y queda solo en el README. `07-menu-contextual.png`
  se abre con el boton derecho sobre una fila.
- **Para el usuario no programador**: hero en lenguaje llano, nav de tres
  enlaces (Recorrido, Ventajas, Descargar), comparacion de rendimiento,
  comandos y arquitectura plegados en `<details id="detalles-tecnicos">`
  (con JS para que el enlace del pie lo abra), tildes del riel y pie
  reescritos. El chip "Recorrido interactivo" del marco se saco; los
  `flow-tag` de los diagramas pasaron de globito a texto de color.

## 2026-10-22 - Blindaje de seguridad y publicacion del sitio

- **Secretos**: `.gitignore` ahora cubre `.env*`, claves y material de firma.
  El escaneo de los 220 archivos versionados y de los 141 commits no encontro
  nada. El repositorio es publico y GitHub quedo con escaneo de secretos,
  proteccion de push, alertas de dependencias y reporte privado de
  vulnerabilidades activados; los patrones propios y la validacion de vigencia
  no los habilita el plan.
- **Antes de commitear**: `scripts/chequear-secretos.py` corre en
  `.githooks/pre-commit` (se activa con `git config core.hooksPath .githooks`).
  Probado con una clave AWS falsa: la detecta y corta el commit.
- **CI**: las cinco acciones quedaron fijadas a un commit en vez de una
  etiqueta, y `checkout` ya no deja el token en el runner
  (`persist-credentials: false`). Windows publica `whatfast-windows.zip.sha256`
  y `whatfast-setup.exe.sha256`. El release acepta `omitir_macos`.
- **SSRF**: el link preview bloquea loopback, privadas, link-local y CGNAT con
  `BlockPrivateNetworks`, activa en `client.go`. Los tests de `httptest`
  (127.0.0.1) inyectan un resolvedor sin bloqueo.
- **Sitio**: `web/` se publica con `firebase deploy --only hosting`
  (`https://what-fast.web.app`). La CSP usa el hash del `<script>` inline: si
  cambia el script, hay que recalcularlo en `firebase.json`. Verificado en el
  navegador: cero errores de consola y el visor cambia de vista al hacer clic.
- **Trampa de coordenadas**: la cabecera de la conversacion arranca en 38.5%
  del ancho (no en 27%) y el panel de info en 67%; medido sobre captura a
  1440 px con los rectangulos pintados. El boton Volver va en la linea del
  pie, no arriba de la imagen, porque tapaba el icono de chats del riel.
- **Verificado**: tour completo (estados, canales, archivados, ajustes, tema
  claro, filtros, grupos, conversacion, adjuntos, emojis, info de grupo,
  info de contacto, volver, boton derecho) con `browser.evaluate`; consola
  sin mensajes; sin desborde horizontal a 1380 px y a 390 px.

## 2026-10-22 - Menu del mensaje flotante y botones de descarga directa

- **Sintoma**: el clic derecho sobre un mensaje abria el menu con el ancho de
  la ventana y al final de la conversacion, no donde estaba el puntero.
- **Causa**: `MessageMenu` se sumaba como un hijo mas de la columna; sin
  `position: absolute` el `align-items: stretch` lo estiraba y caia al pie.
- **Arreglo**: el panel es `absolute` con ancho fijo 230 y `floating_origin`
  lo mete dentro de la ventana; el clic derecho de la burbuja y el boton "..."
  abren en la posicion del puntero; un fondo transparente lo cierra con clic
  afuera y con escape. `cargo test -p whatfast-ui` en verde (194, tres tests
  nuevos de geometria), clippy y fmt limpios.
- **Pagina**: el hero y `#descargar` tienen "Descargar para Windows"
  (`whatfast-setup.exe`) y "Descargar para Linux" (`whatfast-linux.tar.gz`)
  apuntando a `releases/latest/download/`; las tres URLs responden 200.
- **Trampa**: el comando de `#desarrolladores` bajaba `instalar.ps1`, que no
  se publica y espera `dist\` local; ahora baja `whatfast-windows.zip` y lo
  descomprime con `Expand-Archive`.
- **CSP**: al tocar el `<script>` inline el hash cambio a
  `sha256-kwF4pkFpYP7GotkhrYh3MShOasvsXVWkJnD3lkqshr8=`; verificado en el
  navegador (las pestanas de codigo cambian, consola sin mensajes).
- **Cache**: el HTML sale con `Cache-Control: no-cache` (antes 3600 s) y las
  imagenes con un dia. En pantalla chica los botones usan `flex: 0 0 auto`
  para no partir el texto, y `.btn-secondary` lleva un borde interno para
  distinguirse del fondo de la tarjeta de descarga.

## 2026-10-22 - La nota de voz suena en el chat y la columna se acomoda

- **Nota de voz**: se reproduce dentro de la burbuja (chip de 1x/1.5x/2x,
  play y pausa, onda y reloj). El hilo `whatfast-audio` decodifica una vez
  (Ogg Opus con `opus-decoder`; el resto con rodio) y empuja el PCM al
  dispositivo con `rodio::Player`. La posicion la lleva un reloj propio, asi
  la onda avanza pareja aunque el decodificador tarde. Sin dispositivo sale
  un aviso y el chat sigue.
- **Columna de chats**: se esconde con el boton del icono de panel y vuelve
  con el mismo boton en la cabecera de la conversacion; el ancho se arrastra
  desde el borde (260..560). El arrastre se sigue con `window.on_mouse_event`
  porque el puntero sale del asa de seis pixeles enseguida.
- **Cabecera**: se fueron los botones de llamada y videollamada; quedan el
  buscador y el menu (y el de miembros en los grupos).
- **Pegado**: Ctrl+V con una imagen en el portapapeles la manda como foto.
  Windows entrega el mapa de bits como BMP y `send_image` solo acepta
  JPEG, PNG, GIF o WebP: la imagen se pasa a PNG con el crate `image`.
- **Trampa**: el campo de escritura se queda con Ctrl+V; un
  `capture_key_down` no llega a verlo. Hay que interceptar la accion `Paste`
  con `capture_action` sobre la conversacion.
- **Version**: 0.2.1, para publicar el release con estas tres cosas.
- **Release**: `v0.2.1` salio en verde (windows y linux; macOS salteado) y
  quedo como `latest`: los tres botones de la pagina responden 200.

## 2026-10-22 - El chip de la reaccion quita la propia

- **Sintoma**: tocar el chip de una reaccion abria la lista de quienes
  reaccionaron, tambien en las propias, y no habia forma de sacarla.
- **Arreglo**: el clic en el chip propio manda la reaccion vacia (la quita) y
  el clic en el chip de otro suma la propia, como el cliente de WhatsApp. La
  lista de quienes reaccionaron queda en el boton derecho del mouse.
- **Release**: `v0.2.2` en verde (windows y linux) e instalada en la maquina.

## 2026-10-22 - Las fotos pegadas esperan el boton de mandar

- **Sintoma**: pegar una foto con Ctrl+V la mandaba sola. La referencia de
  WhatsApp Desktop la deja arriba del campo de escribir, con el borde verde
  de la que esta elegida, la cruz para sacarla, el lapiz del recorte y el
  cuadradito del `+` para sumar mas, y recien sale con el boton de mandar.
- **Bandeja**: `crates/whatfast-ui/src/composer_attachments.rs` guarda las
  fotos que esperan (`PendingImages`), con la miniatura ya cuadrada y
  reducida a 124 px, y dibuja la tira. Entra entre la lista y el campo, asi
  el campo no la tapa y la lista le hace lugar (79 px de alto con escala 1).
- **Pegado y selector**: `Ctrl+V` guarda el mapa de bits en la carpeta
  temporal y lo deja en la bandeja; el menu del clip abre el selector con
  `multiple: true` y tambien las deja ahi. El lapiz abre el editor de siempre
  y la foto recortada vuelve al mismo lugar de la tira.
- **Envio**: el boton y el return llaman a `submit_composer`: si hay fotos
  manda las fotos (la primera con el texto como epigrafe, como la
  referencia) y si no, el texto. El campo dice "Add a caption…" mientras
  hay fotos y vuelve a "Type a message" cuando la tira queda vacia.
- **Bordes flotantes**: la barra de respuesta, la linea de "escribiendo", el
  panel de emojis y la tira de stickers se corren lo que mide la bandeja
  para no quedar tapados.
- **Prueba**: 7 tests de la bandeja (alta, tope de 8, seleccion al sacar una,
  reemplazo por la foto recortada, vaciado, alto y rotulo del boton). El
  resto del workspace queda en 209 en verde y clippy limpio.
- **Verificado en la app**: con `--fake-backend` y una foto en el
  portapapeles, `capturas/bandeja-despues.png` muestra el borde verde en el
  pie y `capturas/bandeja-enviada.png` ya no lo tiene despues del return.

## 2026-10-22 - Las tildes del mensaje enviado

- **Sintoma**: los mensajes propios no mostraban la tilde de WhatsApp: debajo
  de la burbuja habia una palabra (`sent`, `delivered`, `read`) y con una
  foto recien mandada no habia nada.
- **Arreglo**: `helpers::message_status_glyph` traduce el estado del mensaje
  al dibujo de la referencia (reloj mientras sale, una tilde para el enviado,
  dos para el entregado, dos azules para el leido y el triangulo de aviso
  para el fallido). La hora y la tilde van juntas al final del texto, dentro
  de la burbuja; la foto o el sticker sin epigrafe llevan su propia linea
  abajo a la derecha. La palabra del estado ya no se escribe.
- **Azul**: `#53bdeb` en el tema oscuro y `#34b7f1` en el claro, los dos de
  la referencia.
- **Medida**: el ancho del texto descuenta los 18 px de la tilde, si no el
  texto se le va encima.
- **Prueba**: 2 tests del mapeo estado → dibujo. 211 tests en `whatfast-ui`
  (503 en el workspace), clippy limpio. En la app con `--fake-backend` (los mensajes propios del demo estan
  en `READ`) la captura `capturas/ticks.png` tiene 4 tildes azules de 15x9
  px en el borde derecho de las burbujas.
- **Pendiente**: la lista de chats no tiene la tilde porque el `Chat` del
  protocolo no lleva el estado del ultimo mensaje, solo si es propio.

## 2026-10-23 - La hora de la maquina, la foto en su tarjeta y los fijados

- **Sintoma de la hora**: la lista de chats y las burbujas escribian la hora de
  UTC. En Buenos Aires (UTC-3) un mensaje de las 23:38 salia a las 02:38, y un
  mensaje de las 23:30 cambiaba de dia.
- **Arreglo**: nuevo modulo `crates/whatfast-ui/src/local_time.rs`, que le
  pregunta el desfasaje al sistema con el instante exacto (`GetTimeZoneInformationForYear`
  + `SystemTimeToTzSpecificLocalTime` en Windows, `localtime_r` y `tm_gmtoff`
  en unix) y lo guarda por hora de UTC. `whatfast-core` no puede saberlo (no
  tiene base de zonas), asi que el desfasaje entra por parametro, como pide el
  rulebook. Lo usan la burbuja, la lista, las pildoras de dia, los estados y
  los canales.
- **Reloj de la lista**: `chat_time_label` escribe el mensaje de hoy con el
  mismo reloj de veinticuatro horas que la burbuja ("01:02" y no "01:02 AM"),
  como la referencia. Los dias anteriores siguen con la palabra o la fecha.
- **Sintoma de la foto**: el visor se pintaba a pantalla completa
  (`absolute().inset_0()` de borde a borde) y tapaba toda la interfaz.
- **Arreglo**: la foto se mira en una tarjeta centrada del 70% de la ventana
  con tope de 1080x760 (`VIEWER_MAX_WIDTH`, `VIEWER_MAX_HEIGHT`), sobre un
  fondo oscurecido que sigue mostrando la app; el clic en el fondo y `Escape`
  la cierran, el clic dentro no.
- **Barra de fijados**: los fijados ya se pedian al backend y se guardaban en
  el modelo, pero no habia nada que los mostrara. Ahora hay una barra arriba
  de la lista con el pin, quien fijo y el preview; el clic vuelve al mensaje.
- **Prueba**: 4 tests del desfasaje (dentro del mundo, estable por hora, sin
  panico antes de la epoca ni lejos en el futuro), 3 de la hora de la burbuja
  con desfasajes fijos, 1 mas del reloj de 24 horas de los estados. 220 tests
  en `whatfast-ui`, `cargo fmt --all --check` limpio, clippy de `whatfast-ui`
  limpio. En la app con `--fake-backend`: la lista dice `01:02` y las burbujas
  `23:38` con la hora local del equipo (`date` daba 00:53 -03), la tarjeta del
  visor mide 985x630 en una ventana de 1475x950 y la barra de fijados aparece
  al fijar "El viaje" (capturas en `capturas/verif/`).
- **Ajeno**: `whatfast-model` no pasa el clippy del workspace por
  `Density::from_str` (`clippy::should_implement_trait`), y `attachments.rs` /
  `navrail.rs` tenian un `use` antes de `#![allow(dead_code)]` que no compila:
  el `use` se movio debajo del atributo. Los dos son del trabajo de i18n que
  corre en paralelo.

## 2026-10-23 - Ajustes, notificaciones e interfaz en tres idiomas

- **Ajustes completos**: `whatfast-model/src/settings.rs` guarda un JSON con
  escala, idioma, tema, densidad (comoda/compacta), Enter envia, avisos y
  previews, carpeta de descargas, vista partida, ahorro de bateria y los temas
  propios. `serde(default)` completa un archivo viejo y el `settings` de una
  sola linea se migra sin borrarlo.
- **Idiomas**: crate `whatfast-i18n` con un JSON por idioma en `locales/`
  (`en`, `es`, `zh`); el `build.rs` los descubre y los incrusta. La clave es la
  frase en ingles, asi que una traduccion faltante cae al ingles. El idioma se
  aplica antes de abrir la ventana y el panel de ajustes lo cambia en el acto.
  `scripts/idiomas.py --strict` informa la cobertura (hoy 214 claves al 100%).
- **Notificaciones**: `notify-rust` en un hilo propio (`notifications.rs`). Solo
  suenan con la ventana inactiva, el chat sin silenciar y un mensaje de menos de
  60 s; el ajuste de previews decide si el cuerpo lleva el texto.
- **Temas propios**: el catalogo del core, duplicar el tema actual y editar el
  color de acento con una paleta fija; se guardan como JSON dentro de los
  ajustes, sin tocar el perfil de la cuenta.
- **Guardar en carpeta**: la tarjeta de un adjunto ya descargado lleva el boton
  que copia el archivo a la carpeta elegida (o pregunta una con el selector del
  sistema). Nunca sobrescribe: agrega " (1)", " (2)". Un aviso verde de 4 s
  confirma la ruta.
- **Prueba**: 527 tests del workspace en verde, `cargo fmt --all` y
  `scripts\lint.cmd -D warnings` limpios. Con `--fake-backend` y un perfil
  aparte: ajustes en espanol y chino, densidad compacta, ahorro de bateria sin
  wallpaper y la copia de `vacaciones.mp4` a la carpeta configurada.
- **Trampa**: `StatefulInteractiveElement::tooltip` pide un closure; el tooltip
  de texto es el de `Button`. `Density::from_str` se renombro a `from_value`
  porque clippy lo confundia con `std::str::FromStr`.
- **Pendiente**: los errores de validacion de `whatfast-core` y los textos
  armados con `format!` siguen en ingles; el arrastre del ancho de la columna
  no se probo con mouse real (los clics sinteticos mandan un solo movimiento).

## 2026-10-23 - Botones al pasar el cursor, stickers animados, estrellas, descargas y notas de voz

- **Botones del mensaje**: la fila de la burbuja es un `group` y la barra de
  responder / reaccionar / menu arranca en `opacity(0.)` y sube a 1 con
  `group_hover`; asi el puntero puede bajar hasta los botones sin que se
  vayan.
- **Sticker animado**: `image_source_candidates` prefiere el archivo entero
  (`local_path`) sobre el thumbnail cuando `ImageContent.animated`; GPUI solo
  reproduce los cuadros si lee el WebP animado completo, y el thumbnail es un
  solo cuadro. El visor abre el archivo completo cuando esta en disco.
- **Estrella**: `whatfast-model/src/starred.rs` guarda el par chat + tabulador
  + mensaje en `config/starred-messages`, con reemplazo atomico
  (`NamedTempFile` + `persist`). El modelo expone `set_starred_messages`,
  `message_is_starred` y `toggle_starred_message`. El menu suma "Star
  message"/"Unstar message" y la burbuja pinta una estrella de acento al lado
  de la hora. El dispositivo vinculado no tiene RPC de las estrellas de
  WhatsApp (`bridge.proto`): la estrella es de este perfil, como los favoritos.
- **Descargas**: la flecha de la tarjeta de un adjunto copia el archivo a la
  carpeta de descargas (o pregunta una con el selector); el visor suma
  "Download" para la foto o el sticker. `save_file_to_downloads` nunca
  sobrescribe (" (1)", " (2)") y, si el archivo todavia no llego, el pedido
  queda en `attachment_download_attempted` y la copia se hace cuando aparece.
- **Nota de voz**: `recording.rs` abre el microfono con `cpal` en su propio
  hilo (el handle del stream no es `Send`), codifica cuadros de 20 ms a
  48 kHz mono con `opus-pure` (32 kbps) y escribe Ogg. El boton del microfono
  del compositor pasa a "Recording 0:0N" con enviar y cancelar; menos de 0.4 s
  no se manda. Al enviar se sube el archivo y se manda
  `SendAttachmentRequest.voice_note`.
- **Textos**: 10 claves nuevas (Download, Recording, Record/Send/Cancel the
  voice note, Sending voice note…, Star/Unstar message, la nota corta, las
  estrellas que no se pudieron guardar) en `en`, `es` y `zh`;
  `scripts/idiomas.py --strict` da 100% (195 claves, 224 textos por idioma).
- **Prueba**: 539 tests del workspace en verde (227 de `whatfast-ui`, 65 de
  `whatfast-model`), `cargo fmt --all --check` limpio. El clippy del workspace
  falla antes de llegar a la interfaz por tres lints nuevos de Rust 1.98 en
  archivos que no toque (`collapsible_if` en `whatfast-core`,
  `manual_is_multiple_of` en `whatfast-protocol` y
  `chunks_exact_to_as_chunks` en `media_editor.rs`); permitiendo esos tres,
  `cargo clippy --workspace --all-targets -- -D warnings` termina en 0.
- **En la app** (`--fake-backend`, perfil en `/tmp/whatfast-verif`): capturas
  en `capturas/nuevo/` (hover, menu, estrella, visor, descargas, grabacion);
  los tres adjuntos quedaron en `downloads/` (`Photo 962.png`,
  `vacaciones.mp4`, `informe.pdf`) y `config/starred-messages` conserva
  `chat-1` + tab + `message-948`. Dos notas grabadas con el microfono real son
  Ogg con `OpusHead` y 1,75 s (granulo 83832 a 48 kHz).
- **Pendiente**: la visibilidad por `group_hover` y el envio de la nota no
  tienen test automatico (se probaron en la app); la animacion del sticker
  depende de que el backend guarde el WebP animado completo y no solo el
  thumbnail, y no se probo contra una cuenta real de WhatsApp.

## 2026-10-23 - Repaso de rendimiento y revision de los cambios

- **Nota de voz al chat equivocado**: `RecordingState` ahora guarda el
  `chat_id` donde empezo la nota y una marca `discarded`. `send_voice_note`
  manda a ese chat, no al que este abierto; cambiar de conversacion descarta
  la nota que todavia se esta grabando, pero deja terminar la que ya se esta
  codificando. El archivo de una nota descartada se borra cuando el hilo
  termina, aunque el `Stop` haya salido antes que el `Cancel`. Test
  `a_note_goes_to_the_conversation_where_it_started`.
- **Descarga sin congelar la ventana**: `copy_attachment_to` corre en
  `cx.background_spawn` y copia a un archivo `.part` que despues renombra: una
  copia que muere a la mitad no deja un archivo truncado con el nombre final.
  Tres intentos cubren el doble clic que pelea por el mismo nombre. Medido en
  esta maquina: 192 MB tardan 86 ms, cinco cuadros a 60 fps; un disco lento o
  de red lo multiplica.
- **Estrella sin allocations**: `starred_messages` paso de
  `HashSet<(String, String)>` a `HashMap<String, HashSet<String>>`;
  `message_is_starred` corre por mensaje y por cuadro y ya no arma dos
  `String` para consultar. Microbenchmark de 2.000.000 de consultas: 259 ms
  con el par clonado contra 111 ms con el mapa anidado (2,3x).
- **Sticker animado que no se bajaba**: la lista de imagenes visibles solo
  pedia la descarga cuando no habia thumbnail, asi que un sticker animado con
  thumbnail y sin el WebP completo se quedaba en un cuadro. El helper
  `image_needs_download` pide el archivo entero cuando `animated` y falta
  `local_path`.
- **Descarga fallida que guardaba despues**: el timeout y el error de
  `MessageAttachment` limpiaban `attachment_open_requested` pero no
  `attachment_save_requested`; una descarga posterior del mismo archivo lo
  guardaba sin que el usuario lo hubiera pedido. `forget_attachment_request`
  limpia el pedido y tiene test.
- **Prueba**: 542 tests del workspace en verde (230 de `whatfast-ui`, 65 de
  `whatfast-model`), `cargo fmt --all --check` limpio y clippy limpio con los
  tres lints ajenos de Rust 1.98 permitidos. En la app, la descarga
  asincrona de `vacaciones.mp4` creo `vacaciones (1).mp4` y mostro el aviso
  "Saved to ..." (captura `capturas/review/03-download.png`).
- **Pendiente**: la nota de cinco minutos guarda hasta 57 MB de muestras en
  mono y 115 MB en estereo antes de codificar; las notas enviadas no se
  borran de `data/voice-notes` (solo las descartadas); y `sanitize_file_name`
  no cubre los nombres reservados de Windows (`CON`, `NUL`, `COM1`...).

## 2026-10-23 - La app se cerraba al tocar una pestaña

- **Sintoma**: un clic en una pestaña del riel cerraba la app; lo mismo pasaba
  con la X de cerrar y con el boton de abrir una pestaña nueva.
- **Causa**: el manejador del riel corria dentro de `TabStrip` (un listener de
  la entidad). `on_tab_action` termina en `sync_shell`, que hace
  `self.tab_strip.update(...)`: GPUI no deja actualizar una entidad dentro de
  su propio manejador y paniquea con "cannot update TabStrip while it is
  already being updated". El stderr del `.exe` lo mostro en la linea 142 de
  `entity_map.rs`.
- **Arreglo**: el manejador del riel ahora programa `on_tab_action` con
  `cx.defer_in`, al final del ciclo de efectos, cuando la entidad ya salio de
  la pila. La seleccion, el cierre y el alta de pestaña siguen el mismo camino.
- **Prueba**: con `--fake-backend` y perfil aparte, tres clics que antes
  cerraban la app: abrir un chat, cambiar de pestaña y cerrarla con la X.
  Ninguno deja panic en el stderr y la conversacion cambia. El mas abre la
  busqueda y el foco llega al campo.
- **Release**: v0.2.4 (Windows y Linux; macOS sigue fuera).


## 2026-10-23 - El texto de los mensajes se selecciona y se copia

- **Pedido**: el texto de la app no se podia seleccionar; hacia falta poder
  arrastrar sobre un mensaje y copiarlo con Ctrl+C.
- **Cambio**: `render_linkified_text` deja de armar una fila de divs por palabra
  y usa `TextView` (gpui-component, HTML simple) con `selectable(true)`. La
  seleccion del componente es de ventana: un arrastre puede cruzar varios
  mensajes y el Copy del `Root` copia el texto elegido.
- **HTML**: el texto se escapa (`&`, `<`, `>`, `"`, `'`) y cada URL del mensaje
  queda en un `<a href>`; los enlaces siguen abriendo con el clic. Un parrafo
  por linea porque el `<br>` de esa vista se pinta como un div vacio.
- **Prueba**: 545 tests en verde (tres nuevos del armado del HTML). Con
  `--fake-backend`, un arrastre sobre "Fast native message 947" y Ctrl+C dejo
  ese texto en el portapapeles; un arrastre entre mensajes pinta la seleccion
  en los tres.
- **Release**: v0.2.5.

## 2026-10-23 - La demo del backend falso es gente de verdad

- **Pedido**: los contactos del ejemplo eran "Fast friend N" con mensajes
  "Fast native message N", avatares degradados y un tono de 440 Hz: la
  presentación no se parecia a un WhatsApp real. El dueño pidió nombres de
  personas reales, fotos de perfil, fotos en el chat y un audio de prueba
  reproducible.
- **Cambio**: el dataset de `fake.rs` es ahora una demo del noroeste
  argentino, sigue determinista (el indice decide todo) y sigue siendo el
  oraculo:
  - `PEOPLE`: nomina de 24 personas con nombre y apellido, push name con
    emoji y telefono con formato real (+54 9 381 550-xxx). Los chats de
    mas alla de la nomina combinan dos bancos de nombres (FIRSTS/LASTS).
  - `GROUPS`: 10 grupos con nombre y descripcion ("Familia Bazan", "Futbol
    de los martes"). Los participantes de un grupo salen de la nomina y el
    "yo" de la demo (Octavio Gonzalez) es miembro de todos.
  - Los mensajes alternan "yo" (banco MY_TEXTS) con la persona del chat
    (THEIR_TEXTS), todos en espanol con emojis. El mensaje largo del zancado
    de 9 invita a un cumpleanos en 4 lineas. La preview de la previa de un
    link es La Gaceta (diario de Tucuman).
  - `art`: modulo nuevo que pinta los PNG a mano (sin `image`). Los
    avatares son retratos (rostro con ojos, cejas, sonrisa, rubor, cuatro
    cortes de pelo, 5 tonos de piel) y los de grupo son tres personas. Las
    fotos del zancado de 37 son 8 escenas (atardecer, cafe, gato, cerro,
    ciudad de noche, pizza, flores, playa) y la caption de cada foto nombra
    su escena (PHOTO_CAPTIONS).
  - `wav`: el tono de 440 Hz es ahora una cancion (arpegios C-G-Am-F con
    bajo, 32 notas, ~10 s) y la nota de voz es un tarareo con vibrato
    (~9 s), ambas PCM WAV mono 16 bits a 22050 Hz. El reproductor en linea
    de la UI las decodifica con rodio.
  - Los eventos en vivo llegan de miembros del grupo familiar con textos de
    LIVE_TEXTS; los estados son de Paulina Teran y Renata Mendoza; los
    canales son "Whatfast Novedades" y "Futbol de Primera" con posts
    propios.
- **Decision**: D5 del RULEBOOK queda "no": el nombre "Fast friend N" ya no
  es el oraculo; `tests/fake.rs` afirma la nomina nueva. El espejo del
  dataset de `tests/transport.rs` no se toca (es contracto de transporte,
  no app data).
- **Prueba**: cargo test --workspace en verde; la demo con --fake-backend
  muestra la agenda con retratos, fotos de escenas en el chat y el audio
  se reproduce.

## 2026-10-23 — Sidecar Rust whatfast-wa (ola 0 de la migración Go→Rust)

- Decisión: migrar el sidecar de Go/whatsmeow a Rust/whatsapp-rust para traer llamadas (el stack `voip` exige que la sesión viva ahí). Sin doble vinculación; el usuario re-vincula una vez el día del swap. Del fork crmne solo se consume la librería, sin copiar código de zapfast.
- Qué cambió: crate `crates/whatfast-wa` (main/bridge/server, flags `--stdio --data-dir --log-file` idénticos al Go, lock de perfil con fs4, `not_implemented` honesto por RPC), miembro nuevo del workspace, `MIGRATION.md` con las olas 1-5, el AGENTS.md de opencode pegado como global en `~/.zcode/AGENTS.md` (EQUIPO + memoria de proyecto), línea nueva en `PROJECT_MAP.md`.
- Evidencia: `cargo check --workspace` en 0; `cargo test -p whatfast-wa` 3/3 (handshake+estado+shutdown, pre-hello rechazado, segundo backend rechazado). El árbol completo de whatsapp-rust con `voip` compila en Windows con rustc 1.98.
- Trampa: `fs4::try_lock_exclusive` devuelve `Ok(false)` cuando el lock está tomado, no `Err`. `prost` expone el oneof como campo `body`, no como campo por mensaje.
- Coordinación: el `fake.rs` de whatfast-protocol estaba a medio editar por otra sesión (dataset del norte argentino) y no compilaba; le corregí solo la anotación `[(f32, [(f32,f32,f32); 3]); 3]` en la línea del array `ranges` de la escena de montañas. Nada más de ese WIP se tocó.
- Pendiente: ola 1 (store SQLite + `StartPairing` con QR + eventos `PairingQr`/`ConnectionChanged` + `GetAuthState` real). Orden completo en `crates/whatfast-wa/MIGRATION.md`.

## 2026-10-23 — Plan operativo completo de la migración del sidecar

- Corrección del dueño confirmada con evidencia: whatsapp-rust tiene paridad con whatsmeow para todo lo que el Go usa. Inventario verificado en el fuente del fork (cache cargo, rev 94f7c13): `features/` con newsletter, reaction, stickers, status, polls, pictures, presence, chat_actions, blocking, contacts, groups, call_log; `EventKind` con los ~15 eventos que el Go consume más `IncomingCall`/`CallLog*`.
- Qué cambió: `crates/whatfast-wa/MIGRATION.md` dejó de ser un esquema y pasó a ser el plan operativo completo (reglas, mapa de recursos, tabla de correspondencia whatsmeow→whatsapp-rust, arquitectura de archivos, olas 1-5 con RPCs/eventos/gates, día del swap, trampas).
- Evidencia: superficie whatsmeow extraída de `client.go` (`.wa.*` + `events.*`); API del fork verificada función por función (`send_message`, `pair_with_qr_code`, `upload`/`download`, `send_reaction`, `edit_message`, `voip`); store Go: 20 tablas en `internal/store/store.go`.
- Pendiente: ejecutar ola 1 (sesión + QR) siguiendo el plan.

## 2026-10-23 — Ejecución de la migración: olas 1 y 2 (base) del sidecar Rust

- Ola 1 completa en código: `session.rs` (Bot del fork + estado de conexión por eventos), QR vía `Event::PairingQrCode`, StartPairing/Reconnect/Logout reales, eventos `ConnectionChanged`/`PairingQr`/`BackendProblem` por el pipe. Gate pendiente: escanear el QR con un teléfono real.
- Ola 2 base: `db.rs` (las 20 tablas del Go, idioma SQL idéntico; puntero de último mensaje recalculado como el Go), `ingest.rs` (clasificación de contenido fiel a `domainMessage`: 20+ tipos, edits/revokes por ProtocolMessage, history sync parseado del `LazyHistorySync`), `wire.rs` (filas→protos del bridge). RPCs vivos: ListChats (cursor opaco), ListMessages/After/Around, OpenMessageWindow (con first_unread), MarkRead, SetTyping.
- Evidencia: `cargo test -p whatfast-wa` 9/9 (4 smoke de pipe + 5 de store); `cargo build --workspace` 0; 0 warnings. El crate pasó a lib+bin para que los tests ejerciten la librería.
- Trampas: sqlite del fork necesita la feature `sqlite-storage-bundled` explícita al depender de `wacore` directo (el link pide sqlite3.lib del sistema si no); los protos del fork usan `MessageField` con `as_option()` solo en algunos niveles (key.id es Option plano); `Event::Reaction` NO existe: las reacciones llegan dentro de los envelopes de mensaje (`reaction_message`/`enc_reaction_message`).
- Pendiente: cerrar ola 2 (SearchLocal, sync status, coverage/integrity, journal de outbound, history on-demand, reacciones encriptadas vía `decrypt_reaction_with_secret` + `backend().get_msg_secret`), luego olas 3-5 según `MIGRATION.md`.

## 2026-10-23 - La pagina publica muestra la demo nueva (y tres bugs de la UI abajo)

- **Pedido**: actualizar `https://what-fast.web.app` con la demo realista y
  publicarla.
- **Capturas**: las 15 de `web/assets/` se regeneraron con
  `scripts/captura-frontal.ps1`, `capturas.ps1` y `captura-estados.ps1` sobre
  el binario nuevo. Coordenadas de esta vuelta: fila del grupo (150,230),
  clip (589,916), emoji (630,916), cabecera (900,86), chevron de filtros
  (469,136), item Grupos (600,289), fila de tema con panel scrolleado
  (355,484) y (355,420) para volver a oscuro. La ventana va forzada a cliente
  1475x950 en los tres guiones: la app recuerda el ultimo tamano y sin eso
  las capturas salen de medidas distintas.
- **Bugs de la UI encontrados y arreglados** (todos bloqueaban capturas y
  afectan al usuario real):
  1. El clip del compositor no abria su menu con el clic sintetico: quedo
     como div crudo y la seleccion de texto de ventana (commit 16af95c) se
     come el press que ningun componente reclama. Paso a `Button` ghost como
     el de los emojis (`message_actions.rs`).
  2. El panel de Ajustes no scrolleaba con la rueda: sin `track_scroll` ni
     listener, el catalogo de temas quedaba inalcanzable bajo el doblez.
     Copiado el patron de ChatInfo (`ScrollSurface::Settings` nuevo en
     `app.rs`).
  3. `web/index.html` no llamaba nunca a `render()` al cargar: las zonas del
     recorrido no existian hasta el primer clic, y el primer clic no tenia
     donde caer. La verificacion del 2026-10-22 uso `browser.evaluate` y la
     salteo. Agregado `render(currentView)` al final del script.
- **CSP**: el hash del script inline rompia con cada cambio (el valor
  publicado ya no coincidia ni con el HEAD). El script paso a `web/app.js`
  externo y la CSP a `script-src 'self'`: no hay hashes que mantener.
- **i18n**: quedaban cadenas duras en ingles; hoy la cobertura da 100% en
  en/es/zh (`idiomas.py`): panel de info de grupo y contacto, menu contextual
  del chat, acciones del menu de adjuntos, temporizador efimero, titulo del
  panel de emojis y favoritos. `web/app.js` no toca el gate de idiomas.
- **Publicacion**: `firebase deploy --only hosting`; verificado en el
  navegador: carga sin bloqueos, 9 zonas del recorrido activas al cargar y
  el tour completo cambia de vista con las capturas nuevas.

## 2026-10-23 — Migración: olas 3 y 5-sidecar completadas, proto v22

- Ola 3: `send.rs` (SendText idempotente vía outgoing_requests, reacciones/ediciones/borrados con `MessageRef`, receipts con `mark_as_read`, tipeo con `chatstate()`), `media.rs` (SendImage con upload del fork + sniff PNG/JPEG). RPCs vivos: 9 de envío.
- Ola 5 sidecar: `bridge.proto` v22 (CallOffer/CallUpdate/CallEnded/CallAudio eventos; CallPlace/Accept/Reject/Hangup/Mute/CallAudio RPCs), `calls.rs` (una llamada viva, offers pendientes, mute con ceros en el borde, audio 16 kHz mono i16 por el bridge como bytes), watcher de fases (RelayAllocated→connecting, MediaConnected→active, wait_ended→ended). PROTOCOL_VERSION 21→22 (Go congelado falla con unsupported_protocol, esperado).
- Ola 2-resto: SearchLocal (grupos rankeados + mensajes ≥3 runes), GetSyncStatus real (fase/contadores por chunk), reacciones encriptadas resueltas async con `decrypt_reaction_with_secret` + secrets del backend, SetChatPin/Archive/Mute/Block (store local, eventos ChatUpserted).
- Evidencia: `cargo check --workspace` 0 errores; `cargo test -p whatfast-wa` 9/9; 0 warnings.
- Trampas: el match de eventos de `whatfast-ui/src/app.rs` es exhaustivo — los eventos nuevos del proto rompen el build del UI hasta darles brazo (hoy ignorados, overlay pendiente); `fake.rs` necesitó los 6 brazos nuevos para el demo.
- Pendiente: UI de llamadas (overlay GPUI + botones Phone/Video del header + cpal/rodio lado escritorio + AEC sonora), stickers/adjuntos/encuestas/forward, coverage/integrity, history on-demand, journals, newsletters/avatares/ChatInfo.

## 2026-10-24 — Migración completa: olas 3b, 4 y 5b (UI de llamadas)

- Ola 4: tabla `contacts` (push names aprendidos de cada mensaje y del history sync), títulos con la regla del Go (contacto>negocio>push>nombre>dígitos), backfill de nombres de grupo por `fetch_metadata` con pausa de 250 ms, GetChatInfo con participantes/admins, avatares con caché determinista (`avatar-<fnv>.jpg`) y marcador `not-set` para privacidad, ListNewsletters desde la tabla local.
- Ola 3b: columna `payload` en messages (bytes wire del proto), SendAttachment (video/audio/documento con voice_note), SendSticker (WebP con sniff VP8X), CreatePoll (`client.polls().create`), ForwardMessage (replay del payload con `forward_message`), SendStickerFromLibrary por hash. VotePoll sigue not_implemented (PollRef::new es pub(crate)).
- Ola 5b: `whatfast_model::CallState` en el Store, `call_audio.rs` (captura cpal con remuestreo lineal a 16 kHz mono, mute por ceros, playout rodio a f32), App con acciones call_place/accept/reject/hangup/mute, brazos de eventos (CallOffer muestra barra, CallUpdate activa audio, CallEnded corta, CallAudioOut toca), pump GPUI de 20 ms que manda CallAudio, barra overlay con contestar/rechazar/mutear/cortar, botón Phone en el header de chats directos.
- Evidencia: `cargo build --workspace` ok, `cargo test -p whatfast-wa` 9/9, 0 errores. handle_event ahora recibe cx (necesario para spawn del pump).
- Trampas: GPUI Div no tiene z_index (usar .relative().occlude()); rodio 0.23 SamplesBuffer pide NonZeroU16/NonZeroU32; `let view = cx.entity()` aparece en varios loops del app.rs — no renombrarlos a ciegas; los heredocs con backslashes corrompen Rust — usar Edit.
- Pendiente conocido: VotePoll, coverage/integrity, history on-demand, journals de outbound, video de llamadas, AEC (sonora), keys de i18n nuevas (caen al inglés por diseño). La validación real (QR + llamada con un teléfono) es el gate del usuario.

## 2026-10-24 — Medios entrantes, release 0.2.6 y swap de los builds

- Medios entrantes: `media.rs::fetch_message_media` descarga y desencripta el adjunto o la imagen de un mensaje con `DownloadParams::encrypted` + `client.download_to_writer` (streaming a archivo, no RAM), cache `media-<fnv><ext>` en `data_dir\media`, setters `set_media_local_path`/`set_image_local_path` en db.rs. RPCs `GetMessageAttachment` y `GetMessageImage` ya no responden `not_implemented`: el video, el audio y el documento se abren con el reproductor del sistema y la foto baja a resolución completa en el visor. Falta el recovery de referencias vencidas por app-state que tenía el Go (responde `not_found` honesto).
- Verificación del puente: probe por pipe (Hello→HelloResponse 0.2.5/v22, pre-Hello→`handshake_required`, el proceso vive) contra debug y release; el test de humo de bridge_smoke ya cubría el resto.
- Release: `scripts/build.cmd` y `build.sh` ya no compilan Go — un solo `cargo build --release -p whatfast-ui -p whatfast-wa` y copian `whatfast-wa(.exe)` como `whatfast-backend(.exe)` a `dist\`. `release.yml` ídem en los tres OS (sin setup-go); `ci.yml` suma `whatfast-wa` al job de lógica.
- CI preexistente arreglado: 15 lints de clippy 1.98 en `whatfast-core` (collapsible_if con let-chains, `--fix` mecánico) y el gate de `parity.py` que contradecía su docstring (fallaba por tests de spoiler que el oráculo no reclama; ahora solo falla si falta un caso del oráculo).
- Evidencia: 557 tests del workspace en verde, clippy limpio, `parity.py` exit 0, `funcionalidad.py` exit 0, build release local 15,1 MB (ui) + 12,6 MB (wa).
- Trampa: el heredoc de bash vuelve a cortar código Rust a mitad — escribirlo siempre con Edit/Write.
- Pendiente: la app instalada pide re-vincular por QR al pasar del sidecar Go (las claves viven en bases distintas); remeasure de RAM con el sidecar Rust.

## 2026-10-24 — Identidad: gemelos LID/PN y nombres de agenda (0.2.7)

- Diagnóstico con el perfil real: 474 chats @lid + 248 @pn (WhatsApp migra a identidades ocultas), chat_addresses sin mapeos útiles y contactos solo con push names. Los mensajes "desordenados y tarde" eran la conversación partida en dos filas (history bajo @pn, mensajes vivos bajo @lid).
- El fork ya tiene todo en session.db: `whatsmeow_contacts` (full_name = agenda del teléfono, 1344 filas), `whatsmeow_lid_map` y `lid_pn_mapping` (lid↔teléfono). Lo que el Go leía por Store.LIDs/Contacts.
- Nuevo `identity.rs`: conexión read-only a session.db (fallback en memoria vacía), aliases(pn↔lid), phone_contacts() con la agenda. `reconcile.rs`: canonical_chat (el @pn gana; fusión con merge_chats que mueve mensajes/reacciones/encuestas/pins/envíos, flags max, redirect old→new, puntero recalculado), learn_names (agenda > first_name > push; learn_contact ahora reporta si cambió) y reconcile() que emite ChatMerged por cada fusión y ChatUpserted por nombre cambiado.
- Cableado: apply_event/store_conversation/reactions resuelven el chat canónico al insertar (transport_jid original se conserva); receipts y presencia por resolve_chat_id; el dispatch del server normaliza el chat_id de todos los RPCs (ForwardMessage sus dos); reconciliación al Connected, al fin del history sync y en cada ContactUpdate.
- Validación: test nuevo de integración (gemelos → un chat, agenda gana, history aterriza canónico) + corrida sobre COPIA del perfil real: 471 lid chats → 42, 251 chats con nombre de agenda. La UI ya manejaba ChatMerged (retira la fila vieja y remapea selección/recientes).
- CI arreglado de paso: libasound2-dev faltante en ubuntu (cpal del audio de llamadas), 6 lints de clippy 1.98 en whatfast-ui (2 míos en call_audio, 4 del otro agente: chunks_exact→as_chunks ×2, is_multiple_of, items_after_test_module moviendo render_call_bar antes de los tests).
- Evidencia: 560 tests del workspace en verde; clippy -D warnings limpio en whatfast-ui y whatfast-wa.
- Pendiente: los 42 lid chats que quedan son personas sin mapeo aún (o grupos @lid); el recovery de referencias vencidas de medios sigue not_found honesto.

## 2026-10-24 — Auditoría de paridad Go→Rust del proceso y el bridge (ola 0)

- Auditados main.rs/bridge.rs/bridge_smoke.rs contra el oráculo Go (cmd/whatfast-backend, internal/bridge, app/server.go): framing, lock, handshake, shutdown, exit codes, logs. Evidencia empírica con probe python (target/probe_wa.py, varint a mano) pre y post fix.
- Divergencias corregidas: (1) fallo de framing (frame gigante/zero/truncado/no-request) salía con exit 0, el Go sale 1 → ahora FAILURE vía flag del hilo lector; (2) EOF dentro del payload tras header completo pasaba por cierre limpio → ahora es frame truncado (bridge.rs); (3) mensaje del lock "another backend owns" no matcheaba el needle FATAL "profile is already in use" de classify_exit del transport (la UI habría reintentado para siempre) → texto del Go restaurado; (4) con --log-file los logs iban solo al archivo → tee stderr+archivo como el MultiWriter del Go; (5) eventos pre-handshake sin gate en el loop de serve → gate handshaken tras escribir la respuesta Hello; (6) permisos unix 0700/0600 del securefs del Go ahora espejados (Windows: ACL del perfil, como el Go).
- Fix CI macOS: a_second_backend_cannot_own_the_same_directory tenía una carrera de arranque entre dos procesos (el intruso podía tomar el lock y salir limpio por EOF de stdin); ahora el holder completa handshake antes del intruso y se verifica que sigue vivo tras la negativa. fs4 0.13.1 usa flock vía rustix: exclusión entre procesos correcta en macOS.
- Tests bridge_smoke: 4 → 10 (mismatch post-handshake responde y sale limpio; hello sin solape; frames gigante/zero; truncado; no-request; flags requeridos). clippy --all-targets 0 warnings, cargo test -p whatfast-wa verde.
- Hallazgos ajenos reportados: Go congelado en v21 y la UI negocia min=max=22 → el exe Go ya no sirve de rollback (hello lo rechaza); lock file Go=.profile.lock vs Rust=backend.lock (sin exclusión mutua entre binarios, sancionado por MIGRATION.md); pool de medios/"busy" del server.go sin equivalente en el loop Rust (todo serializado); log en texto tracing vs JSON slog del Go (requiere feature json fuera de partición).

## 2026-10-24 — Auditoría de paridad del store y el esquema (partición B)

- Auditados `crates/whatfast-wa/src/db.rs` y `tests/store.rs` contra `backend/internal/store/store.go` (~100 métodos exportados, 20 tablas, índices, defaults y claves foráneas).
- Corregido en db.rs: `insert_message` era INSERT OR IGNORE y ahora aplica el reducer del Go (merge columna por columna, stamp de timestamp 0, `storage_health`, unread una sola vez, puntero solo hacia adelante); `set_message_status` con la tabla de FAILED; reacciones con tombstone y colapso PN/LID; `messages_around` con la ventana centrada y los clamps del Go; `messages_after` con limit+1; `merge_chats` fusiona las filas en colisión (antes las descartaba) y recalcula el unread; búsquedas con el matcher difuso del Go (trigramas LIKE, ranking, límites 20/6); newsletters DESC + `subscribed_at`; `upsert_poll` refresca si `snapshot_at=0`; `resolve_chat_id` en el orden del Go; flags/mute/nombre crean el chat; paths de medios con guard por kind.
- Schema: tablas, índices, defaults y FKs iguales; extras del puerto documentados (`media_*`, `payload`, tabla `contacts`); falta el FTS5 `message_search` + triggers (la búsqueda usa trigramas LIKE + scoring en Rust); `schema_version` inicial 2 vs 18.
- Tests y evidencia: `tests/store.rs` 5→27 (23 de esta pasada + 4 de la verificación final: clamp 50/200, revoke con preview, pseudo-mensajes reaction, desempate de cursor por jid); `cargo test -p whatfast-wa` 46 passed / 0 failed / 1 ignored (16 bridge + 2 identity + 27 store + 1 lib), clippy --all-targets -D warnings en 0, fmt limpio.
- Pendiente: APIs del Go sin puerto (journals inbound, integrity/coverage, MarkReadThrough/UnreadThrough, pins, sticker favorites, poll read/vote, outgoing reactions, StatusMessagesSince, Groups) — las tablas están, los RPC siguen `not_implemented`; `mark_chat_read` limpia `messages.unread` además del contador (decisión del puerto, documentada).
- Trampa: `refresh_chat_pointer` escribía `last_message_from_me`, columna que no existe en el schema propio → toda fusión de chats fallaba con "no such column".

## 2026-10-24 - Auditoria de paridad Go->Rust: dispatch, RPC faltantes e identidad

- E1 (`server.rs`, `session.rs`, `wire.rs` y `db.rs` aditivo): los 8 RPC que respondian `not_implemented` sin estar en el plan ya tienen brazo real (`OpenContact`, `ListStatuses`, newsletters x3, `ListStickers`, `SetMessagePin`, `ListPinnedMessages`). Ademas: ventana 25/50 en `OpenMessageWindow`, `has_more` con `limit+1`, `MarkRead` con `through_message_id` y evento, `SendReaction`/`ForwardMessage` idempotentes, avatares revalidados a 24 h, handshake despues de escribir la respuesta, `wire` lee `image_*` para adjuntos heredados y encuestas desde `polls`, sync inicial con la regla del Go.
- E2 (`identity.rs`, `reconcile.rs`): apertura perezosa de `session.db`, `user_part` tolera el sufijo `:59`, la fila de chat huerfana de direccion converge, segunda pasada idempotente (0 `ChatMerged`), `learn_names` salta contactos iguales (antes ~4000 escrituras por pasada). Perfil real en copia: 35415 mensajes antes y despues, 470 redirects coherentes con el mapa, 0 folds pendientes.
- El juez de paridad tenia los 8 RPC declarados pendientes (el gate pasaba por declaracion): ahora son brazos reales y `PENDIENTES_DECLARADOS` quedo con los 7 del plan. `docs/paridad-sidecar.md` se reescribio con 43/50 reales y la seccion "Como verificar".
- Evidencia: `cargo fmt --all --check` 0; clippy de los 5 crates + UI 0 warnings; `cargo test --workspace` 674 passed / 0 failed (whatfast-wa 126: 44 lib, 16 bridge, 27 store, 12 ingest, 19 server, 8 identity + 1 ignorado); `parity.py` 123/123 exit 0; `funcionalidad.py` exit 0; `paridad_sidecar.py` 43/50, 7 pendientes, 0 sin declarar, exit 0.
- Trampas: `has_more = len == limit` anunciaba una pagina que no existe; `insert_message` no crea el chat y `messages.chat_jid` tiene FK; `send_image` sigue guardando el path que eligio el usuario (contra `bridge.proto:1005-1007`) y `fetch_message_media` no genera miniatura.
- Pendiente: `SearchLocal` sin contactos ni reemplazo de menciones; `sender_name`/`sender_phone_number`/`sender_avatar_path` y `mention_texts` vacios en `wire`; FTS5 sin portar (trigramas LIKE, no ignora acentos); pool de medios del Go sin equivalente.

## 2026-10-25 - Cero not_implemented: los 7 RPC pendientes ganan brazo real

- Queja: la app mostraba muchos errores `not_implemented` (cada chat abierto
  pedia `GetChatHistoryCoverage`, el panel pedia `GetIntegrityStatus`, etc.).
- Los 7 eran los ultimos del ledger: `VotePoll`, `RequestOlderHistory`,
  `GetChatHistoryCoverage`, `GetHistoryOverview`, `GetIntegrityStatus`,
  `RepairLocalCache`, `RepairRecentReactions`.
- Locales (solo DB): `history_coverage`/`save_history_coverage`/
  `history_overview`/`oldest_message_row`/`integrity_status`/
  `repair_local_cache` en `db.rs`, con la semantica del Go (no_anchor,
  exhausted-con-recibidos pasa a more_available, `PRAGMA quick_check`,
  journal inbound, resamenes derivados; la reparacion no toca FTS porque el
  puerto no tiene tabla `message_search`).
- Encuestas: `polls` suma `creator_jid` + `message_secret` (migracion con
  `ALTER TABLE` tolerante en `Store::open`); `send.rs` guarda la clave al
  crear, `ingest.rs::persist_poll_definition` la guarda al recibir y en el
  historial; `apply_poll_vote` con guarda de timestamp. `VotePoll` valida
  como el Go y vota con `vote_raw` de verdad.
- `RequestOlderHistory`: maquina de estados + `fetch_message_history` real
  (50 mensajes), ventana anti-doble-pedido en el `Server`, evento
  `HistoryCoverageChanged` al pedir, y expiracion a 30 s que marca
  `more_available` si el conteo crecio o `failed` con reintento si no.
- `RepairRecentReactions`: reserva hasta 16 replays pendientes, un resend PDO
  por item (`retry_pdo_placeholder_resend_request`), completa los que el
  telefono responde; sin pendientes responde `not_found` igual que el Go.
- El `match` del dispatch quedo exhaustivo: se elimino el brazo `Some(other)`
  (el compilador lo marco inalcanzable, esa es la prueba) y `request_name`;
  un RPC nuevo no compila hasta tener brazo. `PENDIENTES_DECLARADOS` quedo
  vacio y `docs/paridad-sidecar.md` dice 50/50.
- Trampa: `ContextInfo` no tiene `message_secret`; el secreto esta en el
  `MessageContextInfo` del `wa::Message` (`msg.message_context_info`).
- Evidencia: `cargo fmt --all --check` 0; clippy 5 crates + UI 0 warnings;
  `cargo test --workspace` 685 passed / 0 failed (whatfast-wa 137: 45 lib,
  16 bridge, 33 store, 12 ingest, 23 server, 8 identity + 1 ignorado);
- Release 0.2.8 instalada y verificada: `dist\` con `whatfast.exe`
  (15,1 MB) y `whatfast-backend.exe` (13,0 MB), ambos con `0.2.8` adentro;
  `instalar.ps1` los dejo en `%LOCALAPPDATA%\Programs\Whatfast` y el smoke
  de arranque (perfil temporal, backend hijo vivo) paso.
- Release 0.2.10 instalada y verificada (15,1 + 13,0 MB, `0.2.10` adentro):
  cura de raiz el badge que no se iba (ver entrada 2026-10-26 b): el
  `MarkRead` con cero filas recalcula igual, y abrir el chat marca sin
  esperar al render.
  `parity.py` 123/123 exit 0; `funcionalidad.py` exit 0;
  `paridad_sidecar.py` 50/50 reales, 0 pendientes, 0 sin declarar, exit 0.
- Pendiente: la cobertura no se actualiza al llegar historial bajo demanda
  (la expiracion la normaliza a los 30 s); `SearchLocal` sigue sin contactos;
  `sender_name`/`sender_phone_number` vacios en `wire`.

## 2026-10-25 — Campo del mensaje de varias líneas y texto largo sin desbordar

**Qué cambió**

- El campo del mensaje pasó a `auto_grow(1, 6)`: crece hasta seis renglones,
  scrollea adentro y el pegado conserva los saltos de línea. En una línea el
  componente borraba los `\n` del portapapeles y el texto pegado salía todo
  junto (`InputState::paste` hace `replace('\n', "")` fuera de multi-línea).
- El pie del chat dejó de tener alto fijo: el panel de emojis, el de stickers,
  el de menciones, la línea de "escribiendo" y la barra de respuesta cuelgan de
  un ancla `absolute` con `bottom(relative(1.))` sobre el campo, así lo siguen
  cuando crece. Antes se anclaban con `bottom(56 px + bandeja)` y un campo de
  tres líneas los tapaba.
- La burbuja del mensaje topa el ancho del texto en `TEXT_MAX_WIDTH` (512). La
  vista de texto se medía con el ancho de una burbuja que se encoge al
  contenido, o sea infinito: un mensaje largo se pintaba en una línea y se
  salía de la ventana. Con el tope envuelve, incluso un token sin espacios.
- Visor de fotos: el tamaño de la imagen se calcula en píxeles desde la ventana
  y el tamaño real del archivo (`viewer_picture_size` + lectores PNG/JPEG/WebP)
  y la tarjeta recorta con `overflow_hidden`; una foto vertical entra entera.
- `jpeg_dimensions` tenía un off-by-one (`index + 9 < len`): un SOF pegado al
  final del buffer se descartaba. Lo encontró el test nuevo del header.
- El input del campo no se repintaba al abrirse el panel de menciones: el
  carácter que lo abría no se veía hasta el clic siguiente. Se notifica al
  input cuando el panel se abre o se cierra.

**Por qué**: reporte del usuario — "no permite pegar mensajes, ni visualizar
mensajes largos, y se rompe la visualización"; las fotos del visor se veían
cortadas al medio.

**Evidencia**: 692 tests en verde (`cargo test --workspace`), `fmt` y `clippy
-D warnings` limpios, `parity.py` 123/123, `funcionalidad.py` y
`paridad_sidecar.py` (50/50, gate OK) exit 0. Capturas con el banco rápido
`%TEMP%\opencode\probar.ps1` (una sola corrida de la app para toda la tanda).

**Trampas**

- `whatfast-backend.exe --version` **ignora** el argumento y arranca el backend
  con la sesión real: no se usa para verificar la versión. Se comprueba
  buscando el texto `0.2.11` dentro de los binarios.
- `MinimizeAll` (Shell.Application) deja la app nueva arrancada minimizada y la
  captura sale del escritorio. `IsWindowVisible` sigue en true: no sirve para
  detectarlo.
- `scripts\build.cmd` sale con 1 por el `dir` final, aunque compile.

**Pendiente**: el panel de menciones repite un participante al final de la
lista del backend falso.

## 2026-10-25 — El pegado conserva la forma del texto

**Qué cambió**

- `message_html` envuelve el cuerpo de cada renglón en `<div><pre>…</pre></div>`.
  La vista de texto (`TextView::html`) minifica el HTML antes de leerlo con
  `html5minify`, y ese minificador colapsa todo espacio fuera de `<pre>`:
  la sangría, los espacios repetidos y las líneas en blanco se perdían al
  pintar la burbuja aunque el mensaje guardado estuviera intacto.
- Los renglones en blanco llevan `&#8203;` (espacio de ancho cero) para que
  midan el alto de una línea; sin él, el párrafo vacío se pintaba con alto cero.
- El tabulador se expande a cuatro espacios: la vista de texto no le da ancho.
- Tests: `message_html` con sangría, columnas y renglón en blanco.

**Por qué**: el usuario pidió que el texto pegado se vea con la misma forma,
espaciado y columnas con que se pegó.

**Evidencia**: el texto del modelo y del portapapeles ya era fiel
(code points verificados: 4 espacios de sangría, 8 entre columnas, `\n\n`);
solo fallaba el pintado. Captura G2 con la burbuja igual al texto pegado y
enlaces + wrap intactos. 693 tests, fmt y clippy limpios, los 3 jueces exit 0.
Release 0.2.12 instalada.

**Trampas**

- `TextView::html` no conserva espacios aunque el HTML los traiga: hay que
  pasar por `<pre>` o el minificador los colapsa. Un `<p>` vacío tampoco da
  altura.
- Verificar el texto de un mensaje con el menú "Copy" y `Get-Clipboard` sirve
  para separar "se guardó mal" de "se pinta mal".

## 2026-10-26 — El video se reproduce dentro de la burbuja

**Qué cambió**

- Módulo nuevo `crates/whatfast-ui/src/video.rs`: el video de un mensaje se
  decodifica en un hilo propio con Media Foundation (viene con Windows, sin
  decodificador externo) y se pinta como una imagen más de la burbuja. El
  sonido del archivo va por `rodio` (la feature `mp4` ya traía el AAC).
- La burbuja de video tiene caja con la forma del archivo, botón de
  reproducir/pausar, barra de progreso, tiempo transcurrido y de duración, y
  la flecha de descarga al costado. El archivo ya no sale al reproductor del
  sistema.
- Un video que todavía no está en el disco muestra la misma caja: el primer
  clic pide la descarga y arranca solo cuando el archivo llega
  (`attachment_play_requested`). Si la descarga no lo consigue, el error se ve
  en la burbuja y no queda esperando para siempre.
- El video se corta al cambiar de conversación y el reloj se detiene al
  pausar. La cola de cuadros es corta: si la interfaz va atrasada se tira el
  cuadro, nunca se acumula atraso.

**Por qué**: el usuario pidió ver el video con su reproductor dentro del chat,
no una tarjeta de descarga.

**Evidencia**

- `cargo test -p whatfast-ui`: 245 en verde, 1 ignorado (el del video real).
- El decodificador se probó contra un MP4 4K de la máquina
  (`WHATFAST_VIDEO_SAMPLE`): abre en ~40 ms, entrega cuadros en BGRA de
  `ancho*alto*4` con el alfa en 255, topados a 720 de ancho.
- Ritmo en release: 10 cuadros de un 4K en 0,5 s (~33 ms por cuadro) después
  de cambiar la reducción a `imageops::thumbnail` (17 ms contra 65 ms de
  `resize` con Triangle).
- `cargo clippy --workspace --all-targets -- -D warnings` limpio y `fmt` OK.

**Trampas**

- `MFCreateSourceReaderFromURL` con la ruta cruda devuelve `E_INVALIDARG`:
  hay que pasar la url `file:///C:/...`.
- `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING` **rompe** la creación
  del lector en esta versión de Windows (E_INVALIDARG); sin él, `SetCurrentMediaType`
  con `MF_MT_FRAME_SIZE` reducido no escala nada, por eso la reducción se hace
  en software sobre el cuadro decodificado.
- El atlas de gpui en Windows es `B8G8R8A8`: los cuadros van en BGRA, que es
  justo lo que entrega `MFVideoFormat_RGB32`.
- `RenderImage::new` necesita `Vec<Frame>`: `SmallVec` no acepta un `Frame`
  suelto.

**Pendiente**

- `cargo test --workspace` tiene un rojo ajeno en
  `whatfast-wa/tests/server.rs::list_statuses_groups_by_sender_newest_first`
  (espera 3 grupos y encuentra 2) por el trabajo en curso de otro agente sobre
  `crates/whatfast-wa/src/server.rs`; no es de estos cambios.
- El reproductor no tiene salto en la barra (seek): reinicia desde el
  principio con el botón.

## 2026-10-26 — La pantalla del QR ya no aparece al abrir

**Qué cambió**

- La ventana pregunta el estado de la cuenta al conectarse al sidecar, y el
  sidecar abre la sesion despues del saludo: mientras no la tiene, contesta
  `paired: false`. Antes la ventana le creia a esa primera respuesta y mostraba
  la pantalla de vinculacion con "Esperando el QR..." por un segundo, hasta
  que llegaba `Connected`.
- Ahora esa respuesta se vuelve a preguntar hasta 8 veces cada 200 ms
  (`AUTH_PROBE_ATTEMPTS`), solo mientras la pantalla siga en `Starting`. El
  usuario ve "Starting Whatfast" un instante y entra directo.
- Un `ConnectionChanged(Connected)` con la pantalla todavia en `Starting` o
  `Pairing` dispara la misma consulta, sin duplicarla si ya hay una en vuelo.
- Si de verdad no hay dispositivo vinculado, tras 1,6 s se abre la pantalla
  del QR como antes.

**Por qué**: el usuario reporto que al abrir la app veia la pantalla de
vinculacion por un segundo y despues entraba sola.

**Evidencia**: 245 tests de `whatfast-ui` en verde, `fmt` y `clippy -D warnings`
limpios. Release 0.2.14 instalada.

**Trampa**

- `GetAuthState` con la sesion todavia sin abrir devuelve "sin vincular" y
  `Offline`; no es una respuesta sobre la cuenta, es la ausencia de sesion.

## 2026-10-26 — El arranque pinta la lista al instante y no pasa por el QR

**Qué cambió**

- La ventana pide `ListChats` y `Reconnect` junto con el saludo, en paralelo.
  La lista sale de la base local (`chats.db`), no de la sesión de WhatsApp, así
  que se pinta apenas contesta el sidecar, sin esperar el handshake.
- El sidecar no abría la sesión al arrancar: la creaba recién en `StartPairing`
  o `Reconnect`. Por eso la cuenta vinculada tardaba ~1 s en conectar y la
  ventana mostraba el QR mientras tanto. Ahora la ventana pide `Reconnect` en
  el saludo: con dispositivo vinculado arranca la sesión (`started: true`), sin
  dispositivo responde `false` sin efectos.
- La pantalla de vinculación ya no se abre por un `AuthState.paired == false`:
  se piden los códigos con `request_pairing()` sin cambiar de pantalla, y el QR
  aparece solo cuando llega un código de verdad (`PairingQr`).
- `account_ready` recuerda que la cuenta respondió "vinculado": una lista
  vacía no deja la ventana en `Syncing` ni en `Starting`.
- La rama `AuthState` no vuelve a `Syncing` si la lista ya está en pantalla.

**Por qué**: el usuario reportó que al abrir veía "Starting", después la
pantalla del QR y recién ahí WhatsApp; pidió que el arranque sea inmediato.

**Evidencia**: 245 tests de `whatfast-ui`, `fmt` y `clippy -D warnings` limpios.
Release 0.2.15 instalada (la app estaba abierta; el usuario autorizó cerrarla).

**Trampa**

- `Server::new` deja `session: None` y solo la crea en `StartPairing` o
  `Reconnect`; cualquier cliente que asuma que el sidecar conecta solo se queda
  esperando y cree que no hay dispositivo.

## 2026-10-26 — La barra de "sin conexión" ya no sale al arrancar

**Qué cambió**

- La barra amarilla del chat se dibujaba con cualquier estado distinto de
  `Connected`, incluido `Connecting`. Ahora pide `Offline`, `Reconnecting` o
  `Failed`: es el aviso de una conexión perdida, no el del handshake.
- `AuthState` con `paired: false` ya no escribe el estado de la conexión: esa
  respuesta es la sesión todavía sin abrir y su `Offline` pisaba el
  "conectando" del arranque. Solo una cuenta vinculada informa el estado.
- El sondeo del estado de la cuenta sigue hasta 8 veces aunque la lista local
  ya esté en pantalla (`!account_ready` en vez de `screen == Starting`).
- `Reconnect` en el saludo ya no deja el detalle "Connecting on your request":
  el detalle queda vacío y la cabecera muestra el estado traducido.

**Por qué**: con 0.2.15 el usuario vio "sin conexión" al abrir. La lista se
pinta desde `chats.db` antes del handshake, y el `AuthState` de esa primera
respuesta (`paired:false`, `Offline`) dejaba la barra amarilla a la vista hasta
que llegaba `Connected`, segundos después.

**Evidencia**: 245 tests de `whatfast-ui`, `fmt` y `clippy -D warnings` limpios.
Release 0.2.16 instalada (el usuario autorizó cerrar el backend huérfano).

**Trampa**

- `Reconnect` cuando no hay sesión la crea pero no emite
  `ConnectionChanged(Connecting)` (solo lo hace `StartPairing`).
- Quedó un `whatfast-backend.exe` vivo (pid 17968) con la ventana cerrada desde
  las 13:32; el backend no muere cuando se va la interfaz. Pendiente: que salga
  al cerrarse stdin, y que la interfaz no deje huérfanos.


## 2026-10-26 — Plan de agentes y el catálogo de harnesses

**Qué cambió**

- `docs/plan-agentes.html` (empaquetado en `plan-agentes.packed.html`): el plan
  de conectar harnesses con un click, con las cinco decisiones del usuario
  aplicadas. Manda directo cuando el chat está habilitado, pregunta qué harness
  atiende al activar el modo agente, y el resto queda como se propuso.
- Crate nuevo `whatfast-agents`: las ocho fichas (Claude Code, Codex, OpenCode,
  Cursor, Claude Desktop, VS Code, Pi, Hermes) con detección por PATH y archivo,
  rutas por sistema, token por agente y registro del servidor en JSON, TOML,
  YAML o por la CLI del cliente.
- `Method::JsonArg` cubre VS Code (`code --add-mcp <json>`).
- `RunSpec` es opcional: Cursor, Claude Desktop, VS Code y Hermes no tienen modo
  headless confirmado, así que no atienden pedidos del modo rápido.

**Por qué**: el usuario eligió el plan y pidió empezar por la conexión.

**Evidencia**: `cargo check --workspace --all-targets` limpio; 17 tests de
`whatfast-agents` en verde; `clippy -D warnings` limpio.

**Trampa**

- `opencode mcp add` necesita `--global` para escribir la config del usuario;
  sin la bandera escribe la del proyecto.
- El merge de TOML y YAML es por bloque y conserva comentarios, pero no es un
  parser: si el archivo tiene el bloque indentado de forma rara, falla el
  reemplazo. El backup queda en `<archivo>.whatfast-backup`.

## 2026-10-26 — M1: el socket local y el servidor `whatfast mcp`

**Qué cambió**

- `crates/whatfast-protocol/src/local.rs`: `endpoint(data_dir)` deriva el nombre
  del perfil (fnv64). Windows: `\\.\pipe\whatfast-<huella>`; Unix:
  `<perfil>/agent.sock` con permisos de dueño.
- `crates/whatfast-wa/src/socket.rs`: segundo transporte del backend, siempre
  encendido. Contrato de sobres v22 igual que stdin, una conexión, una
  operación por vez, `Shutdown` de un cliente local cierra el proceso.
- `crates/whatfast-wa/src/main.rs`: `serve()` con dos transportes, `--serve`
  (sin stdin, sin ventana) y `--idle-exit N`.
- `crates/whatfast-mcp/`: el servidor MCP completo (siete herramientas), el
  cliente del socket, el arranque del sidecar y `whatfast.exe mcp --agent <id>`
  despachado en `whatfast-ui/src/main.rs` antes del lock de instancia.
- `scripts/probar-mcp.py` y `docs/agentes.md`.

**Por qué**: el usuario pidió conectar cualquier harness con un click; el MCP es
el camino agente → Whatfast, y el socket es lo que evita abrir un segundo
proceso con la app ya abierta.

**Evidencia**: `probar-mcp.py` contra una copia del perfil real: 786 chats,
mensajes, búsqueda y espera; salida 0 y sin backend huérfano. 16 tests de
`bridge_smoke`, 14 de `whatfast-mcp`, 24 de `whatfast-wa --test server`.
`cargo test --workspace` y `clippy --workspace --all-targets -D warnings`
limpios.

**Trampa**

- `async-channel 2.5` panica con `bounded(0)` (`capacity cannot be zero`): el
  rendezvous que usaba la cola de entrada tiraba el backend al arrancar. Queda
  en 256 y el fin de stdin es una señal aparte (`stdin_closed`), con drenaje
  `try_recv` antes de salir para no perder ninguna respuesta.
- Una tubería de Windows abierta en modo síncrono serializa lectura y escritura:
  una lectura pendiente bloquea la escritura siguiente. Los dos lados son tokio.
- El backend instalado (0.2.16) no tiene el socket, así que con la app abierta el
  MCP no lo encuentra: hay que probar con `--data-dir` de una copia, o instalar
  una release con M1.
- `tests/server.rs::list_statuses_groups_by_sender_newest_first` esperaba tres
  grupos (a, b, a) y el servidor agrupa uno por autor: quedó en dos, con
  a = [s3, s1].
- `status_identity` (de WhatFast-Fixes) tenía dos `if` anidados que rompían
  clippy; pasó a let-chains y su comentario de `newsletter_invite_key` volvió a
  su función.

## 2026-10-26 — M2: la base de los agentes, los permisos y los borradores

**Qué cambió**

- Crate nuevo `whatfast-auto`: `agents.db` en el perfil con las cuatro tablas
  (`agents`, `drafts`, `schedules`, `agent_calls`), el permiso (`read`/`draft`/
  `send` por agente y por chat, con tope por hora y ventana horaria), el ritmo
  humano (`humanize`: partes y esperas sembradas) y el cron de cinco campos con
  la cola fuera de la ventana (`cron`, `defer_to_window`) más el desfasaje local
  del sistema (`local`).
- `crates/whatfast-wa/src/guard.rs`: el guardián del endpoint local. El saludo
  de un agente lleva `agent_id` y `agent_token` (campos nuevos del contrato), el
  token se compara contra su SHA-256 y cada pedido se filtra por escala y chats.
  Las listas se recortan antes de salir; cada llamada queda en `agent_calls`.
- `crates/whatfast-wa/src/drafts.rs`: los tres RPC de borradores
  (`DraftPropose`, `DraftList`, `DraftDecide`), contestados por el bucle de
  servicio antes de llegar al `Server`: la tabla vive al lado de la sesión, no
  adentro.
- `whatfast-mcp`: 10 herramientas (las tres de borradores), el ritmo aplicado en
  `message_send` (partes, `SetTyping`, esperas) y el borrador automático cuando
  el chat no está habilitado para enviar.

**Por qué**: el usuario eligió mandar directo donde el chat está habilitado y
dejar borrador donde no. Sin permisos, cualquier proceso del mismo usuario podía
leer y mandar por el socket.

**Evidencia**: 55 tests de `whatfast-auto`, 23 de `whatfast-mcp` (9 de punta a
punta contra el backend real: token ajeno, recorte de listas, `forbidden`,
`rate_limited`, borradores), 18 de `whatfast-wa --test guard`; `probar-mcp.py`
lista las 10 herramientas y sigue leyendo los 786 chats de una copia del perfil.
clippy `-D warnings` limpio en los cuatro crates.

**Trampa**

- Un backend viejo descarta los pedidos que no conoce y contesta
  `invalid_argument: empty request`; el MCP lo traduce a "actualizá la
  aplicación". Antes de correr los tests de punta a punta hay que `cargo build -p
  whatfast-wa`: el test arranca el binario del `target`, no el del workspace.
- Los e2e dejan `whatfast-wa.exe` vivos 10 minutos si el test paniquea antes de
  `client.shutdown()`; hay que matar los del `target\debug` antes de un build.
- Un perfil sin ninguna fila en `agents.db` trata al harness sin token como el
  usuario; con una fila, el token es obligatorio. Es la regla que deja probar
  antes de que exista la pantalla Agentes.
- `ChatFilter::only` incluye grupos; `ChatFilter::default` no cubre ningún chat.
- El borrador aprobado **no** se envía solo: queda `approved` y lo manda la
  ventana (o el runner), que es donde está la sesión.

## 2026-10-26 — M3: el runner headless, el cronómetro y la guía

**Qué cambió**

- `whatfast-auto/src/runner.rs`: lanza un harness sin ventana (prompt por stdin,
  stdout leído en hilos para no bloquearse, plazo que mata al hijo, `RunError`
  con `NoHeadless`/`Start`/`Timeout`/`Failed`/`Empty`). Siete tests con comandos
  del sistema (`cmd /c more`, `ping`, `cat`, `sleep`).
- `whatfast-wa/src/scheduler.rs`: cada 30 s (60 s en `--serve`) mira las tareas
  vencidas, escribe el próximo turno **antes** de correr y lanza el harness en el
  pool bloqueante. La respuesta queda siempre como borrador en `agents.db`; el
  resultado va a `agent_calls` con la herramienta `schedule`.
- `whatfast-agents/src/guide.rs`: la guía del agente (14 líneas) y su instalación
  por harness dentro de un bloque `<!-- whatfast:inicio -->`/`fin` en el archivo
  de reglas, con copia de respaldo y sin duplicar. Cubre claude-code, codex,
  opencode y cursor; los otros cuatro no declaran archivo.

**Por qué**: el modo rápido del chat y las automatizaciones necesitan las mismas
dos piezas: preguntarle a un harness y leer su respuesta, con plazo.

**Evidencia**: 62 tests de `whatfast-auto`, 27 de `whatfast-agents`, 54 del
sidecar (5 nuevos del scheduler) y el resto del workspace igual; clippy y fmt
limpios en los tres crates.

**Trampa**

- `ClientSpec` **no** tiene el campo `RulesTarget` que dibuja el plan: `guide.rs`
  lleva su propia tabla de destinos por id. Si se agrega al spec, la tabla se
  muda.
- El prompt va por stdin, no por argumento: en la lista de procesos de la
  máquina no debe aparecer el texto del usuario.
- `cmd /c more` agrega saltos de línea: el test del prompt largo compara por
  longitud mínima y sufijo, no por igualdad.
- Un harness sin `RunSpec` (Cursor, Claude Desktop, VS Code, Hermes) no atiende
  ni el modo rápido ni una automatización: la tarea queda registrada como fallo.

## 2026-10-26 — M3 bis: el modo rápido y las automatizaciones del bridge

**Qué cambió**

- `whatfast-auto/src/ask.rs`: arma el pedido del modo rápido (pedido del usuario
  + los últimos 6 000 caracteres del chat, los más nuevos) y lanza el harness
  con plazo de 90 s. Seis tests.
- `whatfast-wa/src/schedules.rs`: los cuatro pedidos de las automatizaciones
  (`ScheduleList`, `SchedulePut`, `SchedulePause`, `ScheduleRemove`). Valida el
  harness (tiene que tener modo headless), el cron de cinco campos, la ventana y
  el instante; calcula el primer turno con la zona del sistema. Diez tests.
- `whatfast-auto/src/db.rs`: `schedule(id)` y `update_schedule` (reescribe sin
  cambiar de fila).
- `proto/bridge.proto` v24: `AgentAsk` (69), `ScheduleList` (70), `SchedulePut`
  (71), `SchedulePause` (72) y `ScheduleRemove` (73), más el mensaje `Schedule`.
  `main.rs::route` los contesta; `server.rs` los rechaza si llegan a su match y
  `fake.rs` los simula.

**Por qué**: el runner y la tabla estaban, pero ni la ventana ni nadie podía
pedirle algo a un harness ni editar una tarea. El modo rápido y la pantalla de
automatizaciones necesitan esta puerta.

**Evidencia**: 68 tests de `whatfast-auto`, 64 del sidecar (10 nuevos de
schedules), 9 de `whatfast-protocol`; clippy `-D warnings` y `cargo fmt --check`
limpios en los tres crates.

**Trampa**

- Solo el usuario edita automatizaciones: un agente del socket recibe
  `forbidden`. No es un permiso configurable.
- El modo rápido corre en el pool bloqueante y espera al harness: la conexión
  que lo pidió queda tomada hasta 90 s, el resto del backend sigue.
- `AgentAsk` no manda nada: el texto vuelve al cuadro de composición de la
  ventana, y mandarlo sigue siendo una decisión del usuario.

## 2026-10-26 — La ventana de los agentes: pantalla, bandeja, modo rápido y automatizaciones

**Qué cambió**

- `crates/whatfast-ui/src/agents_panel.rs` (1539 líneas): la pantalla Agentes en el
  riel (`ctrl-shift-a`), conectar y desconectar un harness con un clic, los
  permisos completos (escala, chats, tope por hora, ventana, ritmo) y la bandeja
  de borradores con Enviar y Descartar.
- `crates/whatfast-ui/src/quick_ask.rs` (489 líneas): el modo rápido del chat,
  `ctrl-shift-q` y el botón "Ask an agent"; la respuesta va al compositor, no se
  manda.
- `crates/whatfast-ui/src/automations_panel.rs` (1156 líneas): la sección
  Automations dentro de la pantalla Agentes, con alta, pausa y borrado.
- `crates/whatfast-wa/src/guides.rs` y los RPC `GuideInstall/Remove/Status`;
  `DraftFinish` para que la ventana cierre un borrador aprobado.
- `rpc.rs` ganó cinco variantes de `PendingRequest`; `AgentAsk` va por la clase
  `Attachment` porque el harness tiene 90 s y `Write` corta a los 60.
- Los RPC nuevos del bridge (v24): `AgentAsk` (69), `ScheduleList/Put/Pause/Remove`
  (70-73), `DraftFinish` (74) y `GuideInstall/Remove/Status` (75-77).

**Por qué**: el backend ya sabía pedirle algo a un harness y guardar tareas; sin
estas pantallas el usuario no podía conectar nada ni ver un borrador.

**Evidencia**: 267 tests de `whatfast-ui` (11 nuevos), 67 del sidecar, 68 de
`whatfast-auto`, 27 de `whatfast-agents`; `cargo test --workspace` en verde,
clippy `-D warnings` y `fmt --check` limpios. Dos pruebas nuevas por el pipe real
en `bridge_smoke.rs`: la lista de automatizaciones de un perfil nuevo y el
`AgentAsk` a un harness sin modo headless.

**Trampa**

- La pantalla Agentes y la bandeja leen `agents.db` directo, no por RPC: viven en
  la misma máquina y el archivo está al lado del perfil.
- El registro del servidor MCP usa `std::env::current_exe()` de la ventana: es
  la ruta correcta, el sidecar no sirve para eso.
- Desconectar revoca el token y la fila, pero no borra la entrada del servidor en
  la config del harness (`write::unregister` queda para el tramo siguiente).
- Un borrador que falla sale de la bandeja: se guarda `failed` y la bandeja lista
  `proposed` y `approved`.
- La pantalla solo crea tareas: no edita una existente, aunque el backend acepta
  `id != 0` en `SchedulePut`.

## 2026-10-26 — Release 0.2.17 instalada

**Qué cambió**: se subió la versión del workspace a 0.2.17, se compiló el release
con `scripts\build.cmd` y se instaló con `scripts\instalar.ps1` en
`%LOCALAPPDATA%\Programs\Whatfast` (la app estaba cerrada, no hubo que matar
nada). `dist\whatfast.exe` 16 802 304 b y `dist\whatfast-backend.exe`
13 299 200 b.

**Evidencia**

- `cargo test --workspace` en verde antes del build (UI 267, sidecar 67,
  `whatfast-auto` 68, `whatfast-agents` 27).
- El sidecar instalado, con `--serve --data-dir %TEMP%\wf-017 --idle-exit 8`:
  crea `agents.db`, `chats.db` y el lock del perfil, y sale solo con "the local
  endpoint stayed idle: exiting".
- `probar-mcp.py` contra los binarios instalados: servidor `whatfast 0.2.17`,
  diez herramientas, 786 chats del perfil copiado, salida limpia del servidor.
  `chat_info` falla con `chat_info_unavailable: client is not connected` porque
  la copia del perfil no tiene la sesión de WhatsApp conectada; no es del MCP.

**Trampa**

- `scripts\build.cmd` sale con código 1 aunque compile: hay que mirar `dist\`.
- El bump de la versión del workspace invalida todos los crates del workspace,
  no las dependencias: el release tardó 5 min 40 s con la caché de `G:\_build`.
- La copia de un perfil sirve para leer, no para `chat_info` ni para enviar:
  esas rutas van a la red y necesitan el dispositivo vinculado.

## 2026-10-26 — 0.2.17 reinstalada: la ventana horaria también rige las tareas de una vez

**Qué cambió**: `schedules.rs::turn` aplica `defer_to_window` también a `Kind::Once`.
Una tarea fijada a las 03:00 con ventana `08:00-23:00` se mueve a la apertura en
vez de correr de madrugada. Test nuevo: `a_once_task_outside_the_window_waits_for_it_to_open`.

**Por qué**: la decisión del usuario es que toda automatización respete la ventana;
antes solo la respetaba el cron. Lo detectó el subagente que escribió el manual.

**Evidencia**: 11 tests de `schedules`, clippy y fmt limpios; release recompilada
en 3 min 41 s, `scripts\instalar.ps1` corrido y verificado sobre el binario
instalado: `--serve --idle-exit 6` sale solo con "the local endpoint stayed idle:
exiting", y `probar-mcp.py` responde `whatfast 0.2.17` con diez herramientas.

**Trampa**: `Start-Process -PassThru` no devuelve `ExitCode` de forma fiable en
PowerShell; para saber si el sidecar salió bien hay que mirar el log de stderr.

## 2026-10-27 — OpenCode se registra por archivo y el usuario puede elegir el suyo (0.2.18)

**Qué cambió**

- `whatfast-agents`: `Method::OpenCode` escribe `~/.config/opencode/opencode.jsonc`
  en `mcp.servers` con `{"type":"local","command":[exe,"mcp","--agent","opencode"]}`.
  Antes se ejecutaba `opencode mcp add`, y en Windows el `opencode` del PATH es un
  shim de Bun que no arranca desde otro proceso ("bin executable does not exist on
  disk").
- Lector de JSONC propio: `strip_jsonc` quita comentarios y comas finales; los
  comentarios del encabezado se conservan y los del medio se pierden (queda el
  respaldo `.whatfast-backup`).
- `write::register_at` / `unregister_at`: escriben en un archivo elegido a mano,
  con el formato que dicta la extensión (`.json`, `.jsonc`, `.toml`, `.yaml`).
  La ruta se guarda en `agents/<id>.config` y `register` la usa sin preguntar.
- Pantalla Agentes: botones "Choose config file…" y "Write server again" en la
  tarjeta del harness.

**Por qué**: la ruta de la ficha es una suposición sobre el sistema; cuando el
cliente se movió, el usuario tiene que poder apuntar al archivo real.

**Evidencia**: 33 tests de `whatfast-agents` (6 nuevos, uno de ellos corre contra
el `opencode.jsonc` real de la máquina: conserva `disabled_providers`, agrega
`whatfast` y lo quita sin tocar el resto); 267 de la UI; clippy `-D warnings` y
`fmt --check` limpios.

**Trampa**

- `unregister` sin ruta guardada escribe en la config real del cliente: los
  tests usan `unregister_at` con una copia en `%TEMP%`.
- El merge de JSONC reescribe el documento entero: el orden de las claves y la
  indentación cambian, y los comentarios internos se pierden.
- La fila del agente puede quedar `Conectado` con la configuración sin escribir:
  "Write server again" la repara sin volver a desconectar.

## 2026-10-27 — El borrador se ve en la conversación (0.2.19)

**Qué cambió**: `crates/whatfast-ui/src/chat_drafts.rs` dibuja una banda sobre el
compositor con los borradores `proposed` del chat abierto: texto, agente, motivo y
tres botones. Enviar usa `send_agent_draft` (el permiso y el estado siguen en la
bandeja); Editar marca `discarded` y pone el texto en el compositor; Cancelar
marca `discarded`. Hasta tres filas, la más nueva primero, y `+N more in the
Agents tray` para el resto.

**Por qué**: el borrador tiene que estar donde va a salir. La bandeja de la
pantalla Agentes se queda como vista de conjunto.

**Evidencia**: 271 tests de `whatfast-ui` (4 nuevos de `chat_drafts`); recarga al
elegir chat, al cerrarlo y cada 3 s sobre el barrido de 1 s que ya existía.

**Trampa**

- La banda va fuera del `uniform_list` del hilo: meterla adentro rompe el scroll.
- Si `agents.db` no abre, la banda no se dibuja y la conversación sigue igual.
- El `+N` es texto, no un botón que salta a la bandeja.

## 2026-10-27 — 0.2.19 instalada y el parser de bloques de markdown

**Qué cambió**

- Release 0.2.19 instalada (el usuario autorizó cerrar los 13 procesos `whatfast`
  y el backend). Lleva la banda de borradores en la conversación.
- `crates/whatfast-core/src/markdown.rs`: parser de bloques de un mensaje
  (`Paragraph`, `Heading`, `Code`, `List`, `Quote`, `Table`, `Rule`) con
  `has_blocks()` como puerta. 13 tests.

**Por qué**: el markdown en las burbujas no necesita un parser nuevo de formato:
`whatsapp_formatting.rs` ya lee las marcas de WhatsApp con estilos por segmento.
Lo que faltaba era la capa de bloques alrededor.

**Evidencia**: 13 tests de markdown, clippy `-D warnings` limpio; versión
reportada por el MCP instalado: `whatfast 0.2.19` con las diez herramientas.

**Trampa**

- Con la app abierta, `scripts\instalar.ps1` falla con "el proceso no puede
  obtener acceso al archivo": hay que cerrar la ventana, y eso necesita el OK
  del usuario.
- `has_blocks` existe para que un mensaje sin bloques se dibuje exactamente como
  antes: cualquier cambio de dibujo nuevo tiene que pasar por esa puerta.

## 2026-10-27 — Los estados no se podian abrir: el visor no se dibujaba

**Que cambio**

- `whatfast-ui/src/render.rs`: el visor de un estado se dibuja sobre la
  ventana cuando `status_strip.viewer_open()`. Antes `render_viewer` existia en
  `statuses.rs` y **nadie lo llamaba**: el clic abria el visor en memoria y no
  se pintaba nada.
- `statuses.rs`: `StatusStripState.requested` (bool eterno) pasa a
  `requested_at`; `should_request` reintenta cada `RETRY_AFTER` (10 s) mientras
  la lista siga vacia. Antes, un primer pedido fallido dejaba la vista vacia
  toda la sesion.

**Evidencia**

- 21 tests de statuses en verde (uno nuevo, `the_strip_asks_again_after_a_failed_request`), clippy `-D warnings` limpio.
- La base real tiene 319 mensajes de estado en las ultimas 24 h de 73 autores,
  y el chat `status@broadcast` existe: el backend tenia datos, el problema era
  de dibujo.

**Trampa**

- Los estados no traen el archivo: 215 de 319 tienen solo la miniatura JPEG
  (~3 KB) y 0 tienen `image_local_path`. El visor se abre con la miniatura o
  el texto; falta pedir la descarga al abrir.

## 2026-10-27 — Logos de los harnesses, Antigravity, y el arranque que se cuelga

**Que cambio**

- `whatfast-ui/src/harness_logos.rs` (nuevo): los SVG oficiales de los ocho
  harnesses con marca (Claude Code, Claude Desktop, Codex, Cursor, OpenCode,
  Pi, Antigravity, VS Code), embebidos en el binario y servidos por
  `WhatfastAssets` bajo el path `harness/<id>`. La tarjeta de Agentes ya no
  dibuja el mismo robot para todos; Hermes, sin marca publicada, cae al robot.
- `whatfast-agents/src/spec.rs`: Antigravity entra al catalogo con su ruta real,
  `~/.gemini/config/mcp_config.json` (clave `mcpServers`), sin modo headless.
- `whatfast-ui/src/render.rs` y `statuses.rs`: el visor de los estados se dibuja
  y la lista reintenta la carga (version 0.2.20).
- Version 0.2.21 instalada.

**Evidencia**

- `whatfast 0.2.21` con las diez herramientas; 3 tests de logos (uno recorre el
  catalogo entero), 33 de agentes y 38 del core en verde.
- Lanzada desde `dist` en consola, la 0.2.21 abre ventana (handle 9046544) y se
  conecta a WhatsApp.

**Trampa**

- Los logos salen de `@lobehub/icons-static-svg` y `simple-icons`; el de
  VS Code ya no esta en la rama `develop` de simple-icons, hay que pedirlo a la
  version 11.
- Al 27/10 15:25 quedaron nueve procesos `whatfast` de la 0.2.20 sin ventana, sin
  CPU y con 10-18 MB: no se sabe por que se cuelgan. Bloquean el `.exe`, que se
  puede reemplazar igual moviendolo a `whatfast.exe.old` (Windows permite
  renombrar un ejecutable en uso).
- En el arbol hay codigo de otra sesion a medio hacer: `whatfast-ui/src/document.rs`
  (15 KB, sin un solo llamador) y `whatfast-core/src/diagrams.rs` ya conectado al
  parser. `clippy -D warnings` falla por las 18 constantes y funciones muertas de
  `document.rs`.

## 2026-10-27 — Un mensaje eliminado ya no se ve en el chat

**Que cambio**

- `whatfast-model/src/model.rs`: `revoke_message` saca el mensaje del hilo en
  vez de marcarlo; `upsert_message` descarta lo que llega revocado, y
  `prepend_messages` y `replace_message_window` filtran lo ya eliminado. Antes
  la fila quedaba con "This message was deleted".
- `whatfast-wa/src/db.rs`: `mark_message_revoked` refresca el puntero del chat,
  y `refresh_chat_pointer` salta los revocados en las cuatro subconsultas. La
  vista previa de la lista de chats vuelve al ultimo mensaje visible.

**Por que**: el usuario pidio que al eliminar un mensaje —propio o ajeno— no
quede nada visible en el chat.

**Evidencia**

- 3 tests nuevos en `model.rs` (sale del hilo, lo revocado por el otro nunca se
  muestra, la ventana de la conversacion lo descarta) y 1 en
  `tests/store.rs` (la vista previa vuelve al mensaje anterior). 69 del modelo y
  36 del store en verde.
- En la base real habia 1 chat de 786 con el ultimo mensaje revocado: su vista
  previa mostraba el texto de un mensaje eliminado.

**Trampa**

- La fila conserva su `text` en la base: el test
  `edit_updates_the_chat_preview_but_a_revoked_row_ignores_it` fija que una fila
  revocada nunca acepta texto nuevo, y borrarlo rompia ese contrato. Lo que
  cambia es la visibilidad, no el almacenamiento.
- `message_text` sigue devolviendo "This message was deleted" para las citas y
  las notificaciones; no se toco.

## 2026-10-27 — Los videos de los estados se miran en el visor

**Que cambio**

- `statuses.rs`: `render_message` ya no cae al texto "Video" cuando el estado
  trae un video; dibuja el reproductor de la burbuja en el visor. El visor le da
  una caja de 800x640 (`video::status_box_size`) y colores claros sobre el fondo
  oscuro. La leyenda del video va debajo, como en las fotos.
- `app.rs`: `message_attachment` busca el adjunto en el hilo y en los estados,
  asi el reproductor y la descarga encuentran el archivo; `tick_video` no corta
  el video de un estado al ver que el chat seleccionado es otro.
- El avance automatico de tres segundos no mueve el visor cuando el estado es un
  video: queda hasta que el usuario toca la flecha o cierra.

**Por que**: el usuario reporto que en los estados solo se veian las fotos.

**Evidencia**

- 2 tests nuevos (la caja del visor es mas grande que la burbuja y conserva la
  proporcion; un estado de video no deja avanzar el visor). 280 tests de la
  interfaz en verde.

**Trampa**

- Los videos de estado no tienen `media_local_path` hasta que se piden: el
  reproductor se dibuja igual y la descarga arranca al tocar play. En la base
  real habia 134 videos de estado y 0 con archivo.
- `Store::message` de la interfaz no indexa los estados: hay que buscarlos en
  `store.statuses()`.

## 2026-10-27 — Los audios se bajan solos y no repiten "Voice message"

**Que cambio**

- `whatfast-wa/src/wire.rs`: la leyenda de un audio o un video sale solo de
  `media_caption`. Antes, si estaba vacia, se usaba el texto de la fila, que es
  el marcador del ingest ("🎤 Voice message", "🎬 Video"); la burbuja lo repetia
  debajo de la tarjeta.
- `whatfast-ui/src/render.rs`: la lista virtual pide solos los audios visibles
  que no estan en el disco, con el mismo tope de 4 descargas a la vez que las
  fotos. Cuando llega el archivo, la tarjeta se convierte en el reproductor.
- `attachments.rs`: `audio_needs_download` decide si falta el archivo (ruta
  vacia o inexistente).

**Por que**: el usuario reporto que cada audio mostraba "🎤 Voice message" y
que los audios tienen que descargarse automaticamente.

**Evidencia**

- 1 test nuevo en `wire.rs` (el marcador no es leyenda; la leyenda real de un
  video sigue viajando) y 1 en `attachments.rs` (un audio sin archivo se pide,
  uno de otro tipo no). 281 tests de la interfaz y 4 de wire en verde.
- En la base real: 1985 notas de voz y 22 audios; solo 15 tenian archivo.

**Trampa**

- La vista previa de la lista de chats sigue mostrando el texto guardado
  "🎤 Voice message": es el preview del chat, no la leyenda de la burbuja.
- El tope de descargas simultaneas es 4 y la lista pide las visibles: al
  desplazarse se piden las demas.

## 2026-10-27 - Llamadas de voz arregladas y videollamadas con ffmpeg

**Que cambio**

- `whatfast-ui/src/app.rs`: el `CallPlace` guarda el `call_id` que devuelve el
  sidecar (`CallState::placed`); antes la llamada saliente se quedaba en
  `placing`, los `CallUpdate`/`CallEnded` no coincidian y colgar mandaba un id
  vacio. Contestar cambia la fase en `store.call`, y un fallo de `CallPlace` o
  `CallAccept` saca la barra.
- `call_audio.rs`: el microfono acumula lo del dispositivo y entrega frames de
  exactamente 960 muestras (60 ms a 16 kHz), que es lo unico que acepta el
  motor; el `MUTE` global se resetea al terminar la llamada.
- `whatfast-wa/src/session.rs`: solo `CallAction::Offer` timbra. Antes cada
  `Event::IncomingCall` (accept, reject, terminate y cada candidato ICE)
  creaba una oferta falsa y podia desalojar la real. Un terminate o reject de
  una oferta pendiente la saca y emite `CallEnded`.
- `calls.rs`: las ofertas se indexan por el id de llamada del action, no por
  el stanza id, para que accept, reject y terminate encuentren la misma
  entrada; la oferta lleva el chat resuelto y la barra muestra el contacto.
- Video: `bridge.proto` suma `CallVideo`/`CallVideoOut` y `CallAccept.video`.
  El sidecar expone `BridgeVideo`/`BridgeVideoSink` y corta los trozos del
  desktop con `AnnexBAuSplitter`. El desktop (`call_video.rs`) captura con
  ffmpeg (dshow, libx264 baseline 640x360@15, 800 kbps, AUD y headers
  repetidos), decodifica al otro lado y decodifica la vista propia.

**Por que**: el usuario reporto que las llamadas y videollamadas no
funcionaban, y eligio ffmpeg para el video.

**Evidencia**

- 284 tests de UI, 70 del modelo, 8 de i18n y 69+ del sidecar en verde; 1 test
  nuevo del splitter (trozos arbitrarios -> access units).
- Prueba manual del codec: camara "Redmi Note 13 Pro (Windows Virtual
  Camera)" -> 30 frames en 2 s -> decodificados 30 cuadros de 640x360 BGRA.

**Trampa**

- `-fflags nobuffer` en el decodificador deja la salida vacia; con
  `-probesize 32 -analyzeduration 0` anda.
- ffmpeg se busca en `WHATFAST_FFMPEG`, junto al ejecutable (copia en la
  carpeta de instalacion) y en el PATH. Sin ffmpeg la llamada sale de voz.
- La app tiene que cerrarse para instalar el .exe nuevo: lo esta usando.

## 2026-10-28 - Videos de estado derechos, texto con CR en líneas y errores en el idioma del usuario

**Qué se cambió**

- `whatfast-ui/src/video.rs`: `unpack_size` lee el ancho de los 32 bits altos
  del `MF_MT_FRAME_SIZE` y `pack_size` lo escribe igual; `shrink` devuelve las
  dimensiones reales del cuadro, no las pedidas. Sin eso los cuadros salían
  con franjas y estirados.
- `whatfast-wa/src/wire.rs`: `normalize_newlines` pasa `\r\n` y `\r` a `\n`
  en el texto, los epígrafes y el preview del chat. `render.rs` lo repite como
  defensa en `message_html_with_mentions` y `measure_message_heights`.
- Errores: `helpers::user_error` manda el detalle técnico a `eprintln!` y
  devuelve una frase corta. Los toasts de `app.rs` usan un texto por acción
  (timeouts, envíos, descargas, agente, video) y los de agentes,
  automatizaciones y borradores usan `user_error`. También se tradujeron las
  etiquetas del visor, la burbuja de imágenes, `recording.rs` y
  `media_editor.rs`.
- Locales: 57 claves nuevas en `en`/`es`/`zh` y los valores con `{error}`
  pasaron de `: {error}` a `. {error}` para que la frase cierre bien.

**Por qué**

El usuario reportó videos de estado deformados, una fecha suelta y filas
encimadas al borrar mensajes, y mensajes de error en inglés poco claros.

**Evidencia**

- Tests nuevos: `the_size_of_a_frame_keeps_the_width_in_the_high_bits`,
  `the_line_breaks_of_a_text_arrive_as_lf` (sidecar, 5 verdes) y
  `the_html_of_a_message_splits_a_carriage_return_in_lines` (UI, 10 verdes).
- Decoding real: `media-b53cf572f8cbe4fc.mp4` (576x1024) y
  `media-44a4bbfbd482099f.mp4` (960x1280) dan cuadros derechos.
- 286 tests de UI y 8 de i18n en verde; `scripts\idiomas.py --strict` dice
  100 % en los tres idiomas (493/526/496 textos).
- `dist\whatfast.exe` 16942080 bytes y `dist\whatfast-backend.exe` 13335040
  bytes; los dos instalados como 0.2.28.

**Trampa**

- Media Foundation guarda el ancho en los bits altos: leerlo al derecho
  deforma el cuadro y deja franjas.
- El backend y la UI tienen que ser la misma versión; 0.2.28 con 0.2.27
  muestra "versiones distintas" y no deja usar la app.
- `scripts\build.cmd` sale con código 1 aunque compile: hay que mirar `dist\`.
- El proceso `whatfast.exe mcp --agent opencode` mantiene el .exe en uso; se
  puede renombrar e instalar al lado, pero el `.old` queda hasta que salga.

**Pendiente**

Probar en la app los videos de estado, borrar mensajes y los textos de error.

## 2026-10-28 - Tarjetas de link al compartir, armadas antes de enviar

**Qué se cambió**

- `whatfast-wa/src/link_preview.rs` (nuevo): busca el primer link del texto
  con `whatfast_core::whatsapp_formatting::parse_whatsapp_text`, baja la
  página (3 s), lee Open Graph (`og:title`, `og:description`, `og:image`),
  resuelve URLs relativas y decodifica entidades. El thumbnail va a JPEG de
  320 px de lado, calidad 70, porque el campo del protocolo es
  `jpeg_thumbnail`.
- `send.rs`: `send_text` calcula el preview en `spawn_blocking` y
  `text_message` arma el `extendedTextMessage` con `matchedText`, título,
  descripción y thumbnail. Sin thumbnail manda `PreviewType::NONE`. Las
  columnas `link_preview_*` guardan lo mismo para la base local.
- Dependencias nuevas de `whatfast-wa`: `whatfast-core`, `ureq 3.3`
  (rustls) e `image 0.25` (jpeg/png/webp).

**Por qué**

WhatsApp arma la tarjeta desde el cliente que envía, no solo al recibir.
Sin esto el mensaje llegaba como texto pelado a los demás clientes.

**Evidencia**

- `cargo test -p whatfast-wa`: 199 verdes y 2 ignorados en 7 suites.
- Prueba real de red: `https://www.rust-lang.org/` devolvió título,
  descripción y un thumbnail JPEG de 12096 bytes en 320x320.

**Trampa**

- La interfaz subraya el link con el mismo parser que arma la tarjeta: si
  se cambia uno hay que cambiar el otro.
- Un texto sin link no abre hilo ni toca la red.
- Si la página no responde, el mensaje sale sin tarjeta: el envío no se
  bloquea. El tope es 3 s de página más 2 s de imagen.

**Pendiente**

Compilar el release e instalar; la app instalada (0.2.28) no tiene el
preview todavía.

## 2026-10-28 - SEO del sitio what-fast.web.app

**Qué se cambió**

- `web/index.html`: título "Whatfast — Cliente de WhatsApp para Windows y
  Linux" (51 caracteres) y descripción con la keyword y el dato de RAM (157),
  canonical, `hreflang`, robots, Open Graph y Twitter card con
  `assets/og-image.png` (1200x630), y JSON-LD con `WebSite`, `Organization`,
  `SoftwareApplication` y `FAQPage`.
- Sección "Preguntas frecuentes" con 8 respuestas; el JSON-LD repite el
  mismo texto palabra por palabra. El nav y el pie llevan el ancla
  `#preguntas`.
- H1 con "Windows" y los H2 de `#caracteristicas` y `#descargar` con
  "cliente de WhatsApp" y "Windows y Linux". La captura del hero lleva
  `width`/`height` y `fetchpriority="high"`.
- `web/app.js`: las capturas del recorrido se precargan en
  `requestIdleCallback`, no antes del primer pintado.
- `web/robots.txt` y `web/sitemap.xml` nuevos.

**Por qué**

El título no decía qué es el producto y el sitio no aparecía buscando
"cliente de WhatsApp para Windows".

**Evidencia**

- Validación en el navegador con la CSP de `firebase.json`: 4 tipos en el
  JSON-LD, 8 preguntas, 1 H1, 6 H2 y cero errores de consola.
- `html.parser`: 361 etiquetas abiertas, 0 sin cerrar, 0 ids duplicados y
  0 imágenes sin `alt`.
- `web/assets/og-image.png`: 1200x630, 179 KB.

**Trampa**

- Google pide que la respuesta del `FAQPage` sea idéntica al texto visible:
  si se toca una, se toca la otra.
- 432 MB es el cliente anterior con WebView2, no WhatsApp Web. El
  comparador del sitio usa 690 MB para la pestaña de WhatsApp Web.
- `cleanUrls: true` redirige `/index.html` a `/`; el canonical ya apunta a
  `/`.

**Pendiente**

Publicar con `firebase deploy --only hosting` en el proyecto `what-fast`.


## 2026-10-28 - Iconos de WhatsApp en el rail y la foto de quien sube el estado

**Que cambio**

- `icons.rs`: `BOT`, `CHANNELS` y `STATUS` nuevos; `ALL` pasa a 41
  glifos. Chats usa `CHAT`, Agentes `BOT`, Estados un anillo (disco con un
  agujero) y Canales una burbuja con `) · (` adentro.
- `navrail.rs`: boton circular de 40 px y el circulo verde con el numero de
  no leidos; el punto verde del mismo color marca los estados sin ver.
- `statuses.rs`: la tira y el visor pintan la foto real de quien sube el
  estado (`participant_avatar_path`) dentro del anillo verde. Los avatares
  se separaron en `render_ring` y `render_status_row`; el visor usa
  `VIEWER_AVATAR_SIZE = 44`.
- `app.rs`: `sync_shell` suma los no leidos de la lista y consulta los
  estados sin ver para alimentar los dos indicadores.
- Version 0.2.29.

**Por que**

Los botones usaban glifos genericos de la libreria y el estado mostraba la
inicial en vez de la foto de quien lo subio.

**Evidencia**

- `cargo test -p whatfast-ui`: 286 verdes, 1 ignorado.
- Rail dibujado a 20 y 96 px y comparado con la referencia antes y despues
  de cambiar el anillo de Estados.

**Trampa**

- `the_path_of_the_icon_is_the_join_of_its_shapes` exige que `path` sea
  exactamente `main` + `cutout` + `overlay`: al tocar una forma se tocan
  las dos.
- `the_measures_of_the_column_come_from_the_stylesheet` fija
  `BUTTON_SIZE = 40`: si se cambia el tamano del boton, se cambia el test.


## 2026-10-28 - El numero de quien no esta agendado, con el formato de WhatsApp

**Que cambio**

- `whatfast-core/src/phone.rs` nuevo: `phone::format` escribe `+54 9 381
  604-1868` con el codigo de pais y los grupos de cada pais (54, 55, 56,
  57, 58, 51, 52, 53, 591, 593, 595, 598, 1, 33, 34, 39, 44, 49, 351,
  353). `phone::digits` saca el mas, los espacios y los guiones.
- `wire.rs::chat_title`: el titulo de un chat directo es el nombre agendado,
  despues el nombre del negocio, despues el nombre guardado y al final el
  numero con formato. El nombre que el contacto eligio (`push_name`) ya no
  es titulo.
- La interfaz formatea el numero en la tira de chats, el subtitulo, el
  panel del contacto, los miembros del grupo, el remitente de un mensaje,
  el autor de un estado, el reenvio y el aviso de escritorio.
- `message_actions::chat_matches` encuentra el numero escrito con el mas o
  con los digitos pelados.

**Por que**

469 de los 722 chats directos no tienen nombre agendado. 427 mostraban los
digitos pegados del jid (`5493816041868`) y 42 mostraban el nombre que el
contacto se puso a si mismo, que puede ser un punto.

**Evidencia**

- `cargo test -p whatfast-core phone::`: 7 verdes.
- `cargo test -p whatfast-wa`: 199 verdes, 2 ignorados.
- `cargo test -p whatfast-ui`: 286 verdes, 1 ignorado.

**Trampa**

- `format` devuelve el texto tal cual si ya tiene el mas y un espacio, asi
  que se puede llamar sobre un valor de origen desconocido sin formatearlo
  dos veces.
- Un numero de un pais que no esta en la tabla queda como vino, y un numero
  con mas digitos que el pais admite (`265867099152617@lid`) tambien: un
  agrupado inventado es peor que ninguno.
- El telefono del protocolo sigue crudo a proposito (lo usan la busqueda y
  los enlaces); el formato es cosa de la interfaz.

**Pendiente**

Instalar 0.2.29 y probar con un chat sin agendar.

## 2026-10-28 - Visualizacion unica, visor de fotos y menu de tres puntos

**Que cambio**

- `whatfast-wa/src/ingest.rs`: un sobre `view_once` ya no se descarta. Se
  guarda una fila `kind = "view_once"` con el texto "One-time message" y la
  interfaz dibuja el aviso. La rama de `Event::UndecryptableMessage` con
  `UnavailableType::ViewOnce` tambien lo guarda.
- `render.rs::render_view_once`: el circulo "1" punteado, el texto en cursiva
  y el enlace "Mas informacion", como WhatsApp. `chat_preview` traduce la
  clave al idioma activo.
- El visor de fotos (`render_image_viewer`) es una capa a pantalla completa:
  encabezado con avatar, nombre y fecha, barra de iconos sin fondo, flechas,
  contador, tira de miniaturas con borde verde y reacciones rapidas.
  `ImageViewer` guarda la galeria (`Vec<ViewerItem>`), el indice y el zoom.
- El menu de tres puntos (`render_image_menu`) tiene Copiar, Compartir, Abrir
  con, Info. del mensaje, Ir al mensaje, Guardar como, Buscar en la web.
  "Guardar como" abre el dialogo del sistema (`cx.prompt_for_new_path`) y la
  copia muere en un `.part`: el archivo aparece entero o no aparece.
- "Abrir con" usa `cx.open_with_system`, sin `explorer.exe` ni consola.
  El clic derecho sobre la foto sigue con Copiar y Descargar.
- Claves nuevas en `en`/`es`/`zh` y tests de la galeria del visor, del aviso
  y de los bytes magicos de la foto.

**Por que**

El telefono retiene el contenido de una visualizacion unica: el dispositivo
vinculado solo recibe `<unavailable type="view_once">`, asi que no hay archivo
que descargar ni abrir. El aviso es lo unico honesto que se puede mostrar.

**Evidencia**

- `cargo test -p whatfast-ui`: 289 verdes, 1 ignorado.
- `cargo test -p whatfast-wa`: 84 verdes, 1 ignorado.
- `cargo test -p whatfast-i18n`: 8 verdes.
- `scripts\build.cmd`: `dist\whatfast.exe` del 28/10 10:29:52, 17.014.784
  bytes.

**Trampa**

- El dialogo de "Guardar como" necesita una carpeta inicial que exista: si la
  configurada no esta, cae en Descargas y despues en la carpeta del usuario.
- `scripts\build.cmd` sale con codigo 1 aunque compile; hay que mirar `dist\`.
- El visor todavia no reproduce videos: solo fotos y stickers.

**Pendiente**

Instalar 0.2.29: hay que cerrar `whatfast.exe` y el proceso
`whatfast.exe mcp --agent opencode`.
## 2026-10-28 - Burbujas sin solaparse, pildora de grabacion y menus de la columna

**Que cambio**

- `render.rs::measure_message_heights`: la burbuja apila sus hijos con un
  hueco de 4 px y la cuenta no lo tenia. La tarjeta del link se mide segun su
  contenido (62 px sin descripcion, 82 con ella, mas 144 con miniatura) y el
  tabulador se expande a cuatro espacios igual que en `message_html`.
- La grabacion de la nota de voz es una pildora centrada y redondeada: tacho
  outline a la izquierda, punto, reloj blanco, onda en vivo, pausa coral y
  enviar redondo. Ya no dice "Grabando" ni ocupa el ancho del campo.
- `recording.rs`: el callback del microfono deja un pico de nivel por bloque;
  los comandos `Pause` y `Resume` llaman a `cpal::StreamTrait`. `elapsed_seconds`
  descuenta el tiempo pausado.
- `render_chat_list_menu`: Nuevo grupo, Mensajes destacados, Seleccionar chats,
  Marcar todos como leidos, Bloqueo de aplicacion y Cerrar sesion. Las cuatro
  acciones sin backend o sin pantalla se ven apagadas y avisan "Todavia no
  esta disponible".
- `mark_all_chats_read` manda `MarkRead` sin `through_message_id` por cada chat
  con no leidos. `render_logout_confirm` confirma antes de `request_logout`.
- "Mas informacion" del aviso de visualizacion unica abre una tarjeta interna
  en vez de la pagina de WhatsApp, y el menu del mensaje solo ofrece eliminar.
- Icono nuevo `TrashOutline` (el lleno queda para los menus).

**Por que**

La medicion corta hacia que la fila siguiente se dibujara encima de la ultima
linea del texto; el grabador no se parecia al de WhatsApp; la columna de los
chats no tenia su menu, y el aviso de visualizacion unica mandaba al navegador.

**Evidencia**

- `cargo test -p whatfast-ui`: 291 verdes, 1 ignorado.
- `cargo test -p whatfast-wa`: 84 verdes, 1 ignorado.
- `cargo test -p whatfast-i18n`: 8 verdes.
- `scripts\build.cmd`: `dist\whatfast.exe` del 28/10 10:59:50, 17.050.624
  bytes.

**Trampa**

- El backend no tiene creacion de grupos ni bloqueo de aplicacion: esos dos
  items quedan apagados a proposito, no rotos.
- Pausar el stream de `cpal` detiene el callback pero no cierra el microfono:
  las muestras ya guardadas se codifican igual al mandar.
- El ancho de la pildora es 420 px sin escalar; con una ventana angosta manda
  `max_w_full`.
- `scripts\build.cmd` sale con codigo 1 aunque compile; hay que mirar `dist\`.

**Pendiente**

Instalar 0.2.29 (hay que cerrar `whatfast.exe` y el MCP) y probar con una foto
de una sola vez y con una nota de voz pausada.
## 2026-10-28 - 0.2.29 instalada y commit 7364b24

**Que**

- `dist\whatfast.exe` (17.107.968 bytes) y `dist\whatfast-backend.exe`
  (14.856.704) copiados a `%LOCALAPPDATA%\Programs\Whatfast`.
- El `.exe` en uso se renombro a `whatfast.exe.old-20261028-1148`: no se cerro
  ningun proceso.
- Commit `7364b24` con 60 archivos; el hook de secretos dio "nada en lo
  preparado".

**Por que**

La app instalada era 0.2.28 y no tenia el visor, los menus, los grupos, el PIN
ni la seleccion multiple.

**Evidencia**

- sha256 del `whatfast.exe` instalado igual al de `dist\`:
  `36884F309D92FB8F598BA36F5F9EDF3562A35853399B97247366D2FA38BAAB51`.
- Cadenas `App lock`, `Create group`, `Select chats` y `Enter your PIN`
  presentes en el binario.

**Trampa**

- Los procesos que ya estaban corriendo siguen con el binario viejo hasta que
  se reinicien, incluidos los `whatfast.exe mcp --agent ...`.
- El proximo arranque de la app usa 0.2.29; no hace falta cerrar nada ahora.

**Pendiente**

Probar en la app nueva: una foto de visualizacion unica, una nota de voz
pausada, crear un grupo y activar el PIN.


## 2026-10-28 - Tarjeta de enlace, filas vacias y boton del agente

**Que**

- `render_link_preview` toma el fondo y el texto de la burbuja que lo
  contiene, oscurece el fondo un 18 %, lleva la imagen a todo el ancho y
  el titulo a dos lineas.
- El hilo esconde las filas sin contenido visible: `Unsupported` vacio o
  con `type_name` "revoked" y sin reacciones, ademas de las revocadas.
- La cabecera de la conversacion ya no tiene el boton "Preguntar a un
  agente"; el modo rapido sigue en `ctrl-shift-q`.
- El backend falso trae una miniatura JPEG para el link preview de la
  demo (`link-preview-demo.jpg`).

**Por que**

- La tarjeta pintaba con `cx.theme()` (el tema del componente, claro)
  mientras la burbuja usa el tema de Whatfast (oscuro): fondo blanco y
  titulo blanco encima, invisible.
- Un sidecar viejo guardo sobres de protocolo como filas `unsupported`
  vacias, y se pintaban como burbujas sin texto.

**Evidencia**

- `cargo test -p whatfast-model -p whatfast-ui`: 71 y 294 pasan (1
  ignorado).
- Captura del chat falso: tarjeta verde derivada de la burbuja, titulo
  blanco visible, imagen a todo el ancho y el boton del agente ausente.

**Trampa**

- El test de `settings.rs` no compilaba desde el PIN: le faltaban
  `app_lock_salt` y `app_lock_hash`.
- `whatfast.exe` en uso se puede renombrar y reemplazar sin cerrar la
  app; el proceso vivo sigue con el binario viejo hasta reiniciar.

**Pendiente**

- Probar la tarjeta con un mensaje real y confirmar que las filas vacias
  de la base del usuario desaparecen del hilo.


## 2026-10-28 - El buscador del sidebar, como la referencia

**Que**

- El panel del buscador pinta tres secciones con avatar, fecha y nombre:
  Contactos, Chats y Mensajes. La consulta se resalta en verde, y cuando el
  store la encontro por errata se marca el trozo que el texto y la consulta
  comparten.
- El backend busca conversaciones directas ademas de grupos
  (`search_chats`, sobre `ChatRow::display_title`) y contactos de la agenda
  (`search_contacts`, una fila por persona entre `@lid` y
  `@s.whatsapp.net`). Antes `SearchLocal` devolvia `contacts` vacio.
- El buscador tiene la X para borrar y el borde verde al enfocar; el pie
  con el contador de chats se esconde mientras busca.

**Por que**

- La captura del usuario mostraba una lista plana de mensajes de una sola
  conversacion, sin avatares ni fechas, el encabezado "Messages20" pegado y
  el pie "787 chats" sobre los resultados.

**Evidencia**

- `cargo test -p whatfast-ui`: 295 pasan (1 ignorado).
- `cargo test -p whatfast-wa --test store`: 38 pasan, con los tests nuevos
  de chats directos y de contactos duplicados.
- Captura con el backend falso (`21-resultados.png`): las tres secciones,
  avatares, fechas y "bruno" en verde. `dist` y
  `Programs\Whatfast` quedaron con los binarios nuevos.

**Trampa**

- Las filas del `v_virtual_list` se miden con ancho de 1 px: sin `w_full`
  cada fila se encoge al texto (era el "Messages20" pegado).
- El `border_color` del `Input` se aplica en `refine_style` y pisaba el
  borde de foco verde: hay que dejar que el componente pinte el ring.

**Pendiente**

- Reiniciar la app para ver el cambio: los procesos viejos siguen con el
  binario anterior.
- Probar con la base real ("EL MERCADO DEL CERDO" en Chats y Contactos).
- El recorrido del sitio (`VIEWS`/`RAIL`) sigue sin la vista de agentes.

## 2026-10-28 - La cache de medios: cuanto ocupa y como vaciarla

**Que**

- `whatfast-wa/src/storage.rs`: `scan` mide `<data-dir>/media` y agrupa por
  tipo (foto, video, audio, documento, avatar, otro) y `clear` borra solo esos
  archivos. Nada del lado del telefono ni de la cuenta.
- `proto/bridge.proto`: `StorageUsage` (80) y `ClearMediaCache` (81), con los
  bytes, los archivos y la carpeta de la cache.
- `db.rs::clear_media_paths`: despues de vaciar, las filas vuelven a "sin
  descargar" (`image_local_path` y `media_local_path` en vacio) y conservan
  `image_direct_path`/`media_direct_path`, que es lo que la proxima descarga
  necesita.
- Ajustes > "Cache de medios": total, una fila por tipo, la carpeta,
  "Actualizar" y "Vaciar la cache de medios". El panel lee la carpeta al
  abrirse, no en el arranque.
- `Store::forget_avatars`: al vaciar, los avatares de la sesion quedan sin
  ruta y la proxima pintada los pide de nuevo.

**Por que**: el usuario pidio ver cuanto ocupan las fotos, los videos y demas
en el sistema local, y poder limpiarlo.

**Evidencia**

- `cargo test -p whatfast-wa --lib storage`: 3 pasan. `--test store`: 39
  pasan (el nuevo comprueba que se olvidan las rutas y quedan los
  descriptores). `-p whatfast-protocol --test fake`: 15 pasan (con
  `the_storage_panel_measures_and_clears_only_the_media_tree`, que ademas
  verifica que `chats.db` no se toca). `-p whatfast-ui`: 299 pasan.
- Linux (WSL archlinux): `cargo test -p whatfast-wa --lib storage` (3),
  `--test store` (39) y `-p whatfast-model` (72) en verde, y
  `cargo check -p whatfast-ui` compila.
- Capturas con el backend falso: `31-ajustes-cache.png` (4.0 MB en 52
  archivos), `32-ajustes-vaciado.png` ("No hay archivos descargados", boton
  apagado, aviso "Se vacio la cache de medios. 2.2 MB liberados.").

**Trampa**

- Los avatares que escribe el sidecar actual viven en `<data-dir>/media`
  (`avatar-<hash>-<variante>.jpg`); `<data-dir>/avatars` es cache muerta del
  sidecar Go y ningun codigo la lee.
- La extension sale de `media.rs::cache_extension`: `.oga` es audio y `.3gpp`
  es video; sin esos dos el panel los contaba como "otros".
- Al vaciar, la interfaz vuelve a pedir los avatares visibles en el acto: la
  carpeta se repuebla sola con unos pocos archivos.

**Pendiente**

- Instalar el binario nuevo (`scripts\instalar.ps1`) y reiniciar la app.
- El aviso de `StorageUsage` no se refresca solo mientras el panel esta
  abierto: se actualiza con "Actualizar" o al reabrirlo.

## 2026-10-28 - La nota de voz acelerada ya no suena a ratita

**Que**

- `whatfast-ui/src/audio.rs`: la nota se reproduce a velocidad 1x y el
  estiramiento del tiempo lo hace `stretch` (WSOLA: ventanas de 40 ms con
  Hann al 50 %, cada una alineada por correlacion cruzada dentro de +/-6 ms).
  El tono no cambia.
- `Library` guarda el archivo decodificado y una copia estirada de la ultima
  velocidad. `Command::Speed` lleva la posicion del reloj y reconstruye el
  reproductor; al reanudar despues de una pausa tambien se reconstruye, porque
  la velocidad pudo cambiar mientras estaba en pausa.

**Por que**: el usuario reporto que al acelerar el audio la voz se escuchaba
como una ratita: `rodio` cambia la velocidad remuestreando, y eso sube el
tono (a 1.5x un 50 %).

**Evidencia**

- 4 tests nuevos: un tono de 440 Hz sigue en 440 Hz a 1.5x y a 2x (antes
  subia a 660 y 880), la duracion queda en 1/velocidad, 1x devuelve las
  muestras intactas y el estiramiento conserva los dos canales.
  `cargo test -p whatfast-ui audio::` 9 en verde.

**Trampa**

- El navegador estiraba solo con `playbackRate`; el reproductor nativo no
  tiene ese modo, asi que el estiramiento es codigo propio.
- `Player::set_speed` ya no se usa en `audio.rs`: si vuelve, la voz vuelve a
  sonar como ratita.

## 2026-10-28 - Instalado el binario de la cache y de la voz

**Que**: `scripts\instalar.ps1` dejo en `%LOCALAPPDATA%\Programs\Whatfast` el
`whatfast.exe` y el `whatfast-backend.exe` de `dist\`. El SHA-256 de los dos
coincide con `dist`. La app se cerro con permiso del usuario y se volvio a
abrir con el binario nuevo.

**Evidencia**: captura del panel con el perfil real: "99 MB en 669 archivos"
(Fotos 65 MB, Videos 31 MB, Audio 2.2 MB, Documentos 90 KB, Avatares 262 KB) y
la carpeta `C:\Users\perca\AppData\Local\whatfast\media`.

**Pendiente**: probar una nota de voz a 1.5x y a 2x en la app real.

## 2026-10-28 - Faltaban mensajes: los sobres y el avatar que trababa el bucle

**Que**: los mensajes que no se veian (fotos y videos de albumes, fotos de
estado, stickers Lottie) llegaban envueltos y `classify` los guardaba como
`unsupported`. Ahora `classify` pela los sobres (`envelope_message`, hasta 4
niveles) antes de clasificar; al arrancar el sidecar,
`ingest::recover_legacy_messages` vuelve a clasificar las filas viejas y les
escribe la burbuja y el descriptor de descarga; las filas que solo son senal
(revoke, voto, placeholder) pasan a `kind='signal'` y `message_is_hidden` las
oculta. Aparte, `Server::resolve_avatar` ya no espera la foto de perfil dentro
del bucle de pedidos: la busca en una tarea aparte y contesta en 2 s como
maximo.

**Por que**: el reporte del usuario fue "faltan mensajes". La base real tenia
568 filas opacas: 398 hijos de album (261 fotos y 137 videos), 119 cabeceras
de album, 45 stickers Lottie y 6 fotos de estados. El otro sintoma, el chat
abierto y vacio, era el bucle de pedidos esperando un `GetChatAvatar` que
tardo 75 s (`backend.log`: "GetChatAvatar held the loop for 75004 ms").

**Evidencia**:

- Copia con la recuperacion ya aplicada: `image` 2693 -> 3008, `video` 543 ->
  681, `sticker` 1353 -> 1398, `album` 1 -> 120, `signal` 216.
- Tests de `whatfast-wa`: 91 en la libreria y los 6 binarios de integracion en
  verde (18, 19, 8, 12, 24 y 39), con 4 tests nuevos en `ingest.rs`. El de la
  bitacora (`bridge_smoke`) vuelve a pasar con el `Tee` restaurado.
- En la app instalada, "Alumnos INVECO" abre con el flyer, el PDF y las fotos
  recuperadas del 28/09 y del 30/09 (`136-inveco-arriba2.png`).
- `backend.log` con el binario nuevo: "GetChatAvatar held the loop for 2009 ms"
  cuatro veces, y el MCP `chat_info` contesta en 236 ms mientras tanto.
- Nota de voz de "Guzman Valentin": el boton pasa a 1.5x y a 2x y reproduce sin
  error (`140-voz-15.png`, `141-voz-2.png`).

**Trampa**:

- El `stderr` del sidecar no era la causa del cuelgue: el transporte de la
  interfaz ya lo drena (`whatfast-protocol::transport::from_child`,
  `capture_stderr`). El `Tee` se queda; lo de los 64 KiB era una hipotesis
  falsa.
- Una foto de perfil que no contesta tarda 75 s en el cliente de WhatsApp: la
  gracia de 2 s evita que trabe el resto, y el reintento de la interfaz lee la
  cache cuando la tarea de fondo termina.
- Los 12 `unsupported` que quedan tienen `payload` vacio: no hay nada que
  recuperar y siguen ocultos.
- Una foto recuperada que nunca se bajo antes del vencimiento del enlace del
  CDN (`oe=` en `image_direct_path`) muestra la burbuja pero no el archivo: en
  la base hay 1010 fotos y 220 videos asi. Las recuperadas recientes (27/10)
  si se ven completas.

## 2026-10-29 - Bandeja, volumen del video y duplicados de WhatsApp

**Que**: la ventana ya no se congela al arrancar (`Tray::install` esperaba al
hilo de la bandeja, que contestaba recien al cerrarse; ahora el handle llega
apenas entra el icono, con 2 s de tope, y el de la ventana principal sale de
GPUI por `raw-window-handle`). El video suena (Media Foundation + `rodio`) y
tiene boton de silencio y barra de volumen en la burbuja y en el visor de
pantalla completa. Los videos HD/HEVC que WhatsApp manda como dos halves se
pliegan en el padre (esquema 2 a 3). La app se actualiza desde GitHub
Releases (`update.rs`, `release.yml`, `docs/actualizaciones.md`).

**Por que**: los reportes fueron "la app esta congelada", "los videos no
tienen audio", "los videos llegan duplicados" y "falta el volumen".

**Evidencia**: 309 tests de `whatfast-ui` en verde; en vivo, cerrar esconde
la ventana, relanzar la muestra y Salir termina el proceso; la base real
quedo en version 3 con 39356 filas y 418 pliegues (156 videos, 262 fotos).

**Trampa**: `PrintWindow` sobre una ventana de GPUI devuelve negro a veces:
para las capturas de prueba sirve GDI sobre el rectangulo de la ventana.

## 2026-10-29 - La busqueda y la apertura de un chat

**Que**: `SearchLocal` barria `messages` con hasta 66 `LIKE` por tecla: 1189 ms
para "instrucciones" y 502 ms para "pedido" sobre la base real. Ahora hay un
indice FTS5 trigram (`messages_fts`, esquema 4, columnas externas y
disparadores): la misma busqueda tarda 1,2 a 11,7 ms. Ademas el bucle del
sidecar atiende un pedido por vez y esperaba a la red: `GetChatAvatar` hasta
2 s y `MarkRead` 10,5 s (los dos en `backend.log`). La foto de perfil y el
acuse de leido van en su propia tarea; una foto vieja se devuelve del disco
mientras se refresca.

**Evidencia**: migracion 3 a 4 de la base real en 169 ms y 39553 filas; la
base viva quedo en version 4. `cargo test -p whatfast-wa` en verde, con un test
nuevo que edita y borra y comprueba el indice. Sin avisos de "held the loop"
desde el arranque nuevo.

**Trampa**: FTS5 trigram exige 3 caracteres (`search_trigrams` ya devuelve
vacio con menos), y el indice se mantiene con disparadores: una escritura
directa a `messages` tambien lo actualiza.

## 2026-10-30 - Los emojis de Apple

**Que**: La ventana pintaba los emojis con Segoe UI Emoji, no con el dibujo de
WhatsApp Web. Ahora usa una fuente propia: `<data>/fonts/emoji.ttf` (o
`WHATFAST_EMOJI_FONT`) y, sin archivo, el Noto Color Emoji embebido en
`crates/whatfast-ui/assets/fonts/`. La familia se lee de la tabla `name` en
runtime. En Windows el archivo se copia a las fuentes del usuario y se registra
(HKCU + `WM_FONTCHANGE`); en Linux `add_fonts` alcanza.

**Por que**: GPUI Windows solo pintaba COLRv0. Se vendorizo el crate
(`vendor/gpui_windows` con `[patch]` en `Cargo.toml`) y se agrego el camino PNG
de `direct_write.rs`: `bitmap_glyph` pide la imagen a DirectWrite y
`rasterize_bitmap` la escala al atlas.

**Evidencia**: `scripts/build_apple_emoji_font.py` genera 3.779 emojis (19,7 MB).
Una sonda sin ventana (DirectWrite, `IDWriteFontFace4`) devuelve 3.779 imagenes
y 0 fallos, por archivo y tambien por familia instalada. El ejecutable pasa de
14,7 a 27,8 MB.

**Trampa**: DirectWrite exige el largo del PNG en 4 bytes entre las metricas y
la imagen (formato 17). Sin el, `GetGlyphImageData` devuelve
`DWRITE_E_FILEFORMAT` (0x88985000) aunque `GetGlyphImageFormats` diga PNG. Noto
y nanoemoji escriben ese largo; el generador no lo hacia. El formato 19 no
sirve: DirectWrite lo rechaza.

**Trampa**: el `FontFallbacks` del ejemplo no resuelve familias cargadas con
`add_fonts`, solo instaladas en el sistema. Varias conclusiones previas de la
depuracion eran el fallback del sistema contestando, no la fuente propia.

**Pendiente**: ver la app con la fuente nueva y confirmar el dibujo en Linux.

## 2026-10-30 - Los estados: anillo, respuesta y carga

**Que**: La foto de cada fila de la lista de chats lleva un circulo verde
punteado cuando esa persona subio un estado que no se abrio, y el clic en esa
foto abre el visor de ese estado (corta la propagacion del clic de la fila).
El visor tiene abajo el mismo campo del chat: el texto sale como respuesta al
estado, al chat del autor y con el estado citado. La vista de estados muestra
la foto de perfil del contacto (la del chat, ya descargada) en vez de las
iniciales. El reintento mientras la lista esta vacia paso de 10 s a 1,2 s.

**Por que**: El "visto" vivia solo en memoria de la sesion y no habia forma de
responder un estado desde la ventana. La respuesta ya existia en el contrato
(`SendTextRequest.reply_to_message_id` + `reply_to_chat_id`), el sidecar la
acepta entre chats y no hizo falta tocar el protocolo.

**Evidencia**: 316 tests en verde. Capturas con el backend de demostracion
(`--fake-backend`, perfil aparte en `%TEMP%`): las dos filas de estados salen
con foto y anillo punteado, y el visor sale con la barra "Type a message" +
Send. Un test encontro que el numero del chat y el del estado se comparaban con
el `+` puesto en uno solo y nunca coincidian.

**Trampa**: el lock de instancia unica vive en el perfil. Para una segunda
ventana de prueba hay que darle `WHATFAST_DATA_DIR` y `WHATFAST_CONFIG_DIR`
aparte, o no abre. El guion `captura-frontal.ps1` hace los clics **antes** de
pulsar los destinos del riel: para abrir un estado hay que clickear el icono
del riel dentro de la misma secuencia de clics.

**Pendiente**: probar el envio real de una respuesta a un estado (el visor y la
barra estan, falta un estado real con la sesion conectada) y ver el anillo en
la lista con datos reales.






## 2026-10-30 - El emoji: color, el campo y la rueda del panel

**Que**: Tres arreglos del emoji en Windows. (1) `rasterize_bitmap` cambia el
rojo y el azul del PNG: el atlas del panel es B8G8R8A8 y el PNG llega RGBA, y
por eso todos los emojis salian azules. (2) `font_metrics` arma una caja de un
em cuando la fuente no trae caja de glifos (`bbox 0 0 0 0`): GPUI mide el glifo
con esa caja para decidir si entra en el recorte y, con cero, saltea el emoji
que cae en el borde. Ese era el emoji invisible al principio del campo de
escribir. (3) El panel de los emojis y la bandeja de los stickers cortan la
rueda con `stop_propagation`: la rejilla se mueve y la conversacion de atras
queda quieta.

**Por que**: El usuario reporto los emojis azules, el emoji que no aparecia en
el campo y la rueda que movia el chat. El azul era el orden de los canales; el
emoji invisible era la caja vacia de la fuente de mapa de bits, no el PNG.

**Evidencia**: Capturas con `--fake-backend` y perfil aparte en `%TEMP%`: el
panel sale amarillo, rojo y verde; el campo muestra el emoji despues de
elegirlo; la rueda sobre la rejilla la mueve y deja el mensaje "Gracias por
ayer" en la misma altura. 316 tests en verde.

**Trampa**: La sonda de `layout_line` mostro que el emoji del campo si se
forma con la fuente del telefono y `is_emoji=1`, pero no se rasterizaba: el
problema estaba despues, en el recorte. La fuente de Apple tiene
`head.bbox = 0 0 0 0`, asi que `DWRITE_FONT_METRICS.glyphBox*` viene en cero.
El `eprintln!` de `is_color_glyph` que quedaba del trabajo anterior se borro.

**Pendiente**: Nada de codigo. El commit y el push siguen sin permiso.

## 2026-11-02 - Auditoria P0, rendimiento base e i18n

**Que**: Se aplico el mega plan `docs/mega-plan-2026-11-02.md`. Sidecar: la
reserva del saliente y de `outgoing_requests` va en una transaccion, un fallo
de SQLite se guarda en `inbound_messages`/`inbound_events`, la tarjeta de
contacto parsea su JSON, la busqueda ordena por `bm25` antes del limite y los
errores de media conservan `code` y `retryable`. UI: borradores del compositor
en `<data>/drafts.json`, toast con cierre automatico y por clic, estados
locales que sobreviven al refresco, foto devuelta a la bandeja si falla el
envio y arranque sin la espera de 1,5 s por la fuente de emojis. Rendimiento:
mimalloc en los dos binarios, `opt-level = 3`, pragmas de SQLite y el wallpaper
pintado por un solo `canvas`.

**Evidencia**: `cargo check --workspace --all-targets` termina en `Finished`;
`cargo test --workspace` da 1036 pruebas en verde (la linea base era 1023:
+13 de regresion). Tests nuevos: journal del mensaje fallido, reserva atomica,
contactos, estado local conservado y detalles del remitente.

**Trampas**: `Concurrent` no existe para `Option`; el toast se cierra por el
barrido de un segundo y no por un timer por cada texto. GPUI no pinta un
`Canvas` como `Div`: el wallpaper va dentro de un `div` contenedor.

**Pendiente**: zoom hacia el cursor y arrastre del visor, seek de la nota de
voz, alturas lazy, LRU si algun dia hay cache propio, split de `render.rs` y
`app.rs`, los 29 `unwrap` de produccion y los temporales de los tests. El
commit y el push siguen sin permiso.

## 2026-11-02 - El clic del emoji ya no atraviesa el panel

**Que**: El panel de emojis, la bandeja de stickers, la lista de menciones y
la barra de respuesta declaran `.occlude()`. Antes el clic sobre un emoji
tambien llegaba a la lista de mensajes que queda detras del panel.

**Por que**: GPUI entrega el clic a todos los hitboxes bajo el puntero hasta
uno con `HitboxBehavior::BlockMouse` (`hit_test` en `gpui/src/window.rs`).
Sin `occlude`, el mensaje de atras recibia el mismo clic; con el, su
`is_hovered` da falso y no dispara.

**Evidencia**: `cargo check -p whatfast-ui --all-targets` termina en
`Finished`; `cargo test -p whatfast-ui` da 322 pruebas en verde.

**Pendiente**: nada de codigo. El commit y el push siguen sin permiso.

## 2026-11-02 - Revision de los 11 hallazgos

**Que**: Se cerraron el punto 1 (la nota de voz usa `Store::media_exists` y
`refresh_media_path`; `render.rs`, `attachments.rs` y `statuses.rs` ya no
llaman `is_file` al pintar), el punto 5 (`whatfast-auto` suma dos lectores
WAL, como `whatfast-wa`) y el callback de VoIP (sin Mutex y sin un `Vec` por
frame: el canal lleva arrays fijos de 960 muestras).

**Parciales**: el 2 sigue con el lock y el crecimiento del `Vec` en la
grabacion; el 6 lee el adjunto entero en RAM (hasta 2 GiB) porque la API de
subida pide un `Vec`; el 9 ordena por pagina pero reindexa y recalcula
alturas de toda la ventana.

**Pendiente**: el 10 (manifiesto sin firma Ed25519 y PIN que se desactiva
borrando el hash) espera la decision de clave y de DPAPI. El 4 (ffmpeg) queda
para Linux, como pidio el usuario.

**Evidencia**: `cargo check --workspace --all-targets` termina en `Finished`;
`cargo test -p whatfast-model -p whatfast-auto -p whatfast-ui` da 77, 68 y
323 pruebas en verde.

## 2026-11-02 - El borrado "para mi" del telefono llega al escritorio

**Que**: `whatfast-wa/src/ingest.rs` ahora reduce `Event::DeleteMessageForMeUpdate`:
resuelve el chat canonico, escribe la lapida con `revoked=1, kind='revoked'`,
refresca el puntero del chat y anuncia el mensaje y el chat. Antes el evento
caia en el brazo generico de `session.rs` y la fila seguia visible.

**Por que**: WhatsApp sincroniza "eliminar para mi" como mutacion de app-state (`deleteMessageForMe`), no como `ProtocolMessage::Revoke`. El sidecar solo reducia el revoke, asi que el mensaje borrado desde el telefono seguia en el hilo y en la vista previa.

**Evidencia**: `cargo test -p whatfast-wa` da 96 (lib) + 49 (store) y siguientes en verde, incluido `a_delete_for_me_from_another_device_hides_the_message`; `cargo test -p whatfast-model -p whatfast-ui` da 77 y 323 en verde; `cargo check --workspace --all-targets` termina en `Finished`.

**Trampas**: el tombstone se escribe aunque la fila no exista todavia, para que una pagina de historia posterior no la reviva (`merge_message` conserva `revoked`). `mark_message_revoked` y este brazo devuelven la vista previa al ultimo mensaje visible; el revoke por `ProtocolMessage` sigue dejando "Message deleted" en la vista previa, como el Go.

**Pendiente**: la app instalada sigue en 0.2.31; el fix no llega hasta compilar e instalar. Commit y push sin permiso.

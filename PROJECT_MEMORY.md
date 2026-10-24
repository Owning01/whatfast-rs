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

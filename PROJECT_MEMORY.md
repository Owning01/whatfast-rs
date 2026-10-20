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

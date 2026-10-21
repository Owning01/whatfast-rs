# PROJECT_MAP — whatfast-rs

Estado actual del repo de destino de la migración. Solo datos verificados con
un comando; no lleva fechas. La bitácora vive en `PROJECT_MEMORY.md`.

## Qué es

El destino de la migración de `Owning01/whatfast` (webview SolidJS + Tauri +
sidecar Go) a **Rust puro sin webview**. El repo de origen queda congelado
como oráculo de paridad.

```
GPUI (lo que se va a escribir)     ← sin webview, sin Chromium
        Entity<Store> + cx.notify()
crates/whatfast-protocol            ← protobuf + framing + cliente RPC
crates/whatfast-core                ← lógica pura portada del frontend
        sin GPUI, sin red, sin async
```

## Capas y archivos que importan

| Ruta | Rol |
|---|---|
| `CHARTER.md` | Objetivo medible, qué no se migra, riesgos aceptados. Gate de F0. |
| `RULEBOOK.md` | Las decisiones de traducción. Si dos agentes podrían diferir, está acá. Gate de F3. |
| `PLAN.md` | Mapa de lotes: qué agente es dueño de qué archivo, en qué orden, con qué gate. |
| `docs/dependency-map.{md,json}` | Inventario generado por `scripts/dependency-map.py`. No se edita a mano. |
| `docs/type-map.md` | Los 5 ejes resueltos, toolchain, dependencias, intraducibles. |
| `docs/api-mapping.md` | Los 50 RPC del sidecar y los 56 comandos de la app web, con su destino en Rust. |
| `docs/oracle.md` | Qué es el juez de paridad y cómo se valida. |
| `docs/parity.json` | Resultado del juez, generado por `scripts/parity.py`. |
| `crates/whatfast-core/` | Lógica pura. 25 módulos declarados en `src/lib.rs`. |
| `crates/whatfast-protocol/` | Contrato protobuf + transporte (`transport.rs`) + backend de demostración (`fake.rs`). |
| `crates/whatfast-model/` | El modelo de datos y los módulos sin GPUI: `model`, `paths`, `settings`, `sticker`, `emoji_picker`. Cero GPUI en su árbol de dependencias. |
| `crates/whatfast-ui/` | La interfaz GPUI: `main.rs` (la entrada), `app.rs` (el estado), `rpc.rs` (los pedidos), `render.rs` (el dibujo), `helpers.rs`, `text.rs`, `tokens.rs` (el tema). El commit del stack de Zed lo fija el `Cargo.lock` (ver el rulebook §2). |
| `backend/` | El sidecar en Go (módulo `github.com/Owning01/whatfast/backend`). |
| `proto/bridge.proto` | Copia del contrato v21 del origen. |
| `assets/icons/` | Los iconos de Whatfast (los mismos del cliente de Tauri). |
| `scripts/test.cmd`, `scripts/lint.cmd` | Wrappers de cargo: target compartido, `sccache` y el override de `rustc-wrapper`. |
| `scripts/build.cmd` | El build de distribución: el binario de Rust y el sidecar en `dist\`. |
| `scripts/medir.ps1` | Mide el arranque y la RAM del árbol de procesos. |
| `scripts/capturas.ps1` | Captura la ventana para el README. Mide la escala de la pantalla: sin eso la captura sale recortada. |
| `scripts/funcionalidad.py` | Cuenta los RPC que pide la interfaz y los componentes del cliente web que tiene equivalentes. Sale con 1 si falta algo. |
| `.github/workflows/ci.yml` | Tests, lint y juez de paridad en cada push. |
| `.github/workflows/release.yml` | El paquete de Windows y de Linux. |

## Reglas de trabajo de esta máquina

- El linker del MSVC no está en el PATH: `. %TEMP%\opencode\msvc.ps1` y usá `scripts\test.cmd` / `scripts\lint.cmd` (que resuelven el target compartido y usan `sccache`).
- Las compilaciones **no van al disco C:**: los wrappers resuelven `<repo principal>\.cache\target`, que es un enlace a `G:\_build\whatfast-target`.
- Un lote = una partición de archivos. Cada agente tiene su worktree en
  `%TEMP%\opencode\wf\<lote>`.
- `crates/whatfast-core/src/lib.rs` y `crates/whatfast-ui/src/main.rs` los
  mantiene el orquestador: los agentes escriben su módulo, no el `lib.rs`.
- El repo de ORIGEN (`G:\Proyectos\16-whatsapp2`) es de solo lectura para
  esta migración: no se toca ni un test.

## Comandos

| Comando | Qué hace |
|---|---|
| `scripts\test.cmd` | `cargo test --workspace` |
| `scripts\lint.cmd` | `cargo clippy --all-targets --locked -- -D warnings` |
| `python scripts\dependency-map.py` | Regenera el inventario F1. |
| `python scripts\parity.py` | Corre el juez de paridad F2 (sale con 1 si algo falta). |
| `python scripts\funcionalidad.py` | La puerta de "igual que el cliente web": 50/50 RPC y 21/21 componentes. |

## Baseline y meta

| Métrica | Origen | Meta |
|---|---|---|
| Ventana visible | 0,25 s | < 0,5 s |
| RAM del árbol de procesos | 432 MB | < 120 MB |
| JS de la UI | 411 KB | 0 (no hay webview) |
| Paridad | — | 153/153 casos del oráculo |

## Estado de la migración

| Lote | Alcance | Estado | Paridad |
|---|---|---|---|
| B-A1 | `whatsapp_formatting` (el slice duro) | cerrado | 14/14 |
| B-A2 | 8 módulos puros | cerrado | 36/36 |
| B-A3 | 6 módulos de estado | cerrado | 53/53 |
| B-A4 | consolidación de DTOs en `types.rs` | cerrado | — |
| B-A5 | `send_idempotency`, `emoji_autocomplete`, `performance`, `sync`, `authoritative_resync` | cerrado | 23/23 |
| B-A6 | `theme`, `spotlight`, `notification_routing`, `participant_avatar_queue` | cerrado | 24/24 |
| B-A7 | `voice_note` (remuxer Ogg) | cerrado | 3/3 |
| B-P1 | transporte protobuf (`whatfast-protocol`) | cerrado | 18/18 |
| B-B1 | backend fake (los 50 RPC de demostración) | cerrado | 14/14 |
| B-R1 | spike del protocolo Rust | cerrado (D1: el sidecar Go se queda) | — |
| B-U0 | crate `whatfast-model` (el modelo de datos, sin GPUI) | cerrado | 31/31 |
| B-G1 | el sidecar en Go, en el repo nuevo | cerrado | 220/220 tests de Go |
| B-U2a | el estado y las acciones (`app.rs`, `rpc.rs`, `text.rs`) | cerrado | 3.707 l. |
| B-U2b | el dibujo (`render.rs`, `helpers.rs`) | cerrado | 3.270 l. |
| B-U3 | los tokens del tema (`tokens.rs`) | cerrado | 21/21 |
| B-U4..U8 | los 11 componentes que faltan y los 28 RPC pendientes | pendiente (F4+) | — |

**Paridad del oráculo: 153/153 (100 %) + 45 tests de caracterización.
288 tests en verde en el workspace; clippy limpio.**

## Medido (release, 20/10/2026)

| Métrica | Antes (webview) | Ahora (nativo) | Meta |
|---|---|---|---|
| RAM de la interfaz + el sidecar | 57,4 MB | **91,9 MB** | < 120 MB |
| RAM del motor de render | 366,8 MB (WebView2) | **0** | 0 |
| RAM total | 432 MB | **91,9 MB** | < 120 MB |
| Ventana visible | 0,25 s | 0,70 s | < 0,5 s (**no se llega**) |
| UI lista (lista de chats) | 1,16 s | ~1,1 s | — |
| JS que baja el cliente | 411 KB | **0** | 0 |
| Descarga para instalar | 33,1 MB | 33,2 MB | — |

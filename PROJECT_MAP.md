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
| `docs/api-mapping.md` | Los 64 comandos del bridge y su destino en Rust. |
| `docs/oracle.md` | Qué es el juez de paridad y cómo se valida. |
| `docs/parity.json` | Resultado del juez, generado por `scripts/parity.py`. |
| `crates/whatfast-core/` | Lógica pura. 15 módulos declarados en `src/lib.rs`. |
| `crates/whatfast-protocol/` | Contrato protobuf + transporte. |
| `crates/whatfast-model/` | El modelo de datos y los módulos sin GPUI: `model`, `paths`, `settings`, `sticker`, `emoji_picker`. Cero GPUI en su árbol de dependencias. |
| `crates/whatfast-ui/` | La interfaz GPUI. Tiene su `Cargo.toml` con `gpui` y `gpui-component`; el commit del stack de Zed lo fija el `Cargo.lock` (ver el rulebook §2). |
| `proto/bridge.proto` | Copia del contrato v21 del origen. |
| `scripts/test.cmd`, `scripts/lint.cmd` | Wrappers de cargo con el override de `rustc-wrapper`. |

## Reglas de trabajo de esta máquina

- El volumen `G:` no tiene `sccache` y el linker del MSVC no está en el PATH:
  `. C:\Users\perca\AppData\Local\Temp\opencode\msvc.ps1` y usá
  `scripts\test.cmd` / `scripts\lint.cmd` (ya llevan el `--config
  "build.rustc-wrapper=''"`).
- Un lote = una partición de archivos. Cada agente tiene su worktree en
  `C:\Users\perca\AppData\Local\Temp\opencode\wf\<lote>`.
- `crates/whatfast-core/src/lib.rs` lo mantiene el orquestador: los agentes
  escriben su módulo, no el `lib.rs`.
- El repo de ORIGEN (`G:\Proyectos\16-whatsapp2`) es de solo lectura para
  esta migración: no se toca ni un test.

## Comandos

| Comando | Qué hace |
|---|---|
| `scripts\test.cmd` | `cargo test --workspace` |
| `scripts\lint.cmd` | `cargo clippy --all-targets --locked -- -D warnings` |
| `python scripts\dependency-map.py` | Regenera el inventario F1. |
| `python scripts\parity.py` | Corre el juez de paridad F2 (sale con 1 si algo falta). |

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
| B-R1 | spike del protocolo Rust | cerrado (D1: el sidecar Go se queda) | — |
| B-U0 | crate `whatfast-model` (el modelo de datos, sin GPUI) | cerrado | 31/31 |
| B-U1 | backend fake (los 50 RPC de demostración) | en curso | — |
| B-U2..U8 | interfaz GPUI | pendiente (F4+) | — |

**Paridad del oráculo: 153/153 (100 %) + 45 tests de caracterización.
253 tests en verde en el workspace; clippy limpio.**

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
| `crates/whatfast-ui/` | La interfaz GPUI. Empieza en F4. |
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

| Lote | Alcance | Dueño | Estado |
|---|---|---|---|
| B-A1 | `whatsapp_formatting` | agente A1 | en curso |
| B-A2 | 8 módulos puros | agente A2 | en curso |
| B-A3 | 6 módulos de estado | agente A3 | en curso |
| B-R1 | spike del protocolo Rust | agente R1 | en curso |
| B-P1 | transporte | — | pendiente |
| B-P2 | `docs/api-mapping.md` | — | pendiente |
| B-U1..U8 | interfaz GPUI | — | pendiente (F4+) |

# whatfast-rs

**Whatfast en Rust nativo: sin webview, sin SolidJS, sin Chromium.**

Este repo es el destino de la migración descrita en
[`CHARTER.md`](CHARTER.md) y ejecutada según [`PLAN.md`](PLAN.md).
El origen (la app con webview) vive aparte, en `Owning01/whatfast`, y queda
congelado como **oráculo de paridad**: sus tests son la especificación.

## Qué es este repo ahora

| Crate | Qué es | Estado |
|---|---|---|
| `crates/whatfast-core` | Lógica pura portada del frontend: parser de formato, geometría del editor de imagen, reglas de workspace, orden de chats, políticas | en construcción (lote B-A*) |
| `crates/whatfast-protocol` | Contrato protobuf (`proto/bridge.proto`) + framing y ciclo de vida del transporte | esqueleto |
| `crates/whatfast-ui` | La interfaz GPUI | empieza en F4 |

## Por qué

La app con webview usa **432 MB** de RAM en reposo. 367 de esos MB son el
WebView2: seis procesos de Chromium existen solo para pintar nuestro DOM.
La meta de esta migración es **menos de 120 MB** y una ventana visible en
**menos de 0,5 s**, sin perder una sola función.

## Cómo se trabaja acá

- [`RULEBOOK.md`](RULEBOOK.md) manda. Si dos implementadores podrían
  responder distinto, la respuesta está en el rulebook, no en el código.
- Un lote = una partición de archivos. Nadie toca el archivo de otro.
- Los tests del origen son el oráculo y **no se tocan**. Los tests de Rust
  se escriben nuevos, traducidos caso por caso.
- Nada de stubs: `unwrap()`, `todo!()` y `unimplemented!()` están
  prohibidos fuera de los tests.

## Comandos

```sh
scripts/test.cmd        # cargo test --workspace
scripts/lint.cmd        # cargo clippy --all-targets --locked -- -D warnings
```

Los wrappers existen porque el toolchain está pineado y porque todos los
worktrees de este repo tienen que compilar en el **mismo** sitio: cada
worktree con su propio `target/` llena el disco con una copia de la
compilación, y con el stack de Zed eso son 3 GB por worktree.

El target compartido se resuelve solo:

1. `CARGO_TARGET_DIR`, si ya está definida.
2. `WHATFAST_TARGET_DIR`, si está definida.
3. El `.cache/target` del worktree principal, que se localiza con
   `git worktree list --porcelain`. No hay que configurar nada.

Además, cuando `sccache` está en el `PATH`, los wrappers lo usan: la cache
de compilaciones es de toda la máquina, así que el stack de Zed se compila
**una vez** y el resto de los proyectos la reutilizan. Sin `sccache` el build
es el mismo, solo que repetido tarda más.

Para que las compilaciones no caigan en el disco del repo, exportá
`WHATFAST_TARGET_DIR` a una carpeta de caches:

```sh
export WHATFAST_TARGET_DIR=/ruta/a/caches/whatfast-target
```

## Licencia

MIT o Apache-2.0, a elección del usuario.

Cliente no oficial de WhatsApp. No está afiliado a WhatsApp ni a Meta.

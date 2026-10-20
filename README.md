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

Los wrappers existen porque el volumen `G:` de esta máquina no tiene
`sccache` instalado y el toolchain está pineado.

## Licencia

MIT o Apache-2.0, a elección del usuario.

Cliente no oficial de WhatsApp. No está afiliado a WhatsApp ni a Meta.

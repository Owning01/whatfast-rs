# Whatfast

Cliente de escritorio de WhatsApp, rápido y chico. No es una página web
empaquetada: es una **app nativa** escrita en Rust, con un motor en Go que
habla el protocolo de dispositivos vinculados.

![Lista de chats de Whatfast](docs/screenshots/01-lista-de-chats.png)

> **Estado:** en desarrollo. Es un cliente no oficial. La cuenta de
> WhatsApp nunca entra en el proceso que dibuja la pantalla.

---

## Qué es

- **Interfaz nativa en Rust (GPUI)**: dibuja directamente, sin webview, sin
  DOM y sin motor de navegador.
- **Sidecar en Go** (`whatsmeow`) como único dueño de la conexión, el
  historial, los medios y las bases de datos.

La interfaz nunca ve tus credenciales: el QR, el token del dispositivo y las
bases de datos viven en el sidecar, y la interfaz habla con él por RPC tipados
con protobuf.

Todo el historial es **local** (SQLite: `session.db` y `client.db`).

---

## Qué hay adentro

| Crate | Qué es | Líneas |
|---|---|---|
| `whatfast-core` | El dominio: formato de mensajes, reacciones, sincronización, tema, notas de voz, recortes. Sin dependencias de la interfaz. | 8.955 |
| `whatfast-protocol` | El contrato (protobuf) y el transporte: framing, ciclo de vida de los pedidos, clasificación de la salida del sidecar y el backend de demostración. | 2.260 |
| `whatfast-model` | El modelo de datos: chats, mensajes, borradores, avatares, stickers, rutas. Sin GPUI: se puede testear sin abrir una ventana. | 1.813 |
| `whatfast-ui` | La interfaz: GPUI, sin webview. | (en construcción) |
| `backend/` | El sidecar en Go. | ~20.000 |

El dominio está **completo y verificado**: los 153 casos de prueba del
repositorio de origen están portados y en verde, y el juez de paridad
(`scripts/parity.py`) los compara uno por uno.

---

## Compilar y correr

```sh
scripts/test.cmd        # cargo test --workspace
scripts/lint.cmd        # cargo clippy --all-targets --locked -- -D warnings
scripts/build.cmd       # deja dist\whatfast.exe y dist\whatfast-backend.exe
```

Para correr sin una cuenta de WhatsApp (y sin red), el backend de demostración
trae 10.000 chats simulados:

```sh
dist\whatfast.exe --fake-backend
```

El backend real es el sidecar en Go: el binario tiene que estar al lado del
ejecutable de la interfaz, o en la variable `WHATFAST_BACKEND`.

### El toolchain

- Rust estable (el `rust-toolchain.toml` lo fija).
- Go 1.25 o superior, solo para el sidecar.

Los wrappers de `scripts/` resuelven el directorio de compilaciones
compartido, así que varios worktrees no llenan el disco con una copia de la
compilación cada uno, y usan `sccache` cuando está disponible.

---

## El protocolo

El sidecar habla un protocolo propio: protobuf enmarcado (4 bytes big-endian
de longitud + payload) sobre stdin/stdout.

- El contrato está en [`proto/bridge.proto`](proto/bridge.proto).
- El mapeo de los 50 RPC y de los 56 comandos de la app web está en
  [`docs/api-mapping.md`](docs/api-mapping.md).
- La paridad con el cliente de origen está en [`docs/oracle.md`](docs/oracle.md).

---

## Checks

| Qué | Cómo |
|---|---|
| Los tests | `scripts\test.cmd` |
| El lint | `scripts\lint.cmd` |
| La paridad con el oráculo | `python scripts\parity.py` |
| El mapa de dependencias | `python scripts\dependency-map.py` |

---

## Licencia

MIT o Apache-2.0, a elección del usuario.

Cliente no oficial de WhatsApp. No está afiliado a WhatsApp ni a Meta.

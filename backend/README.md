# `backend/` — el sidecar en Go

Este directorio es el **sidecar en Go** del cliente de Whatfast. Habla con
WhatsApp por `whatsmeow` y le sirve al cliente nativo por protobuf enmarcado.

El sidecar **se queda** (decisión D1 de `CHARTER.md`): la interfaz es 100 %
Rust sin webview, pero el protocolo de WhatsApp sigue en Go porque la
librería Rust del protocolo no puede reusar una sesión ya emparejada
(`docs/protocol-spike.md`). El código de este directorio viene íntegro del
repositorio anterior (`16-whatsapp2/backend`) y no se modifica: cualquier
cambio de comportamiento acá es un incidente de producción.

El contrato de cable es `proto/bridge.proto`, byte a byte idéntico al del
oráculo.

## Requisitos

- Go 1.25 o superior (el toolchain de la máquina es 1.26.7, en
  `G:\Dev\GoLang\bin`).
- La cache de módulos ya está movida a `G:\_build\go-mod`. Si la shell no la
  ve, exportala: `$env:GOMODCACHE='G:\_build\go-mod'`.

## Compilar

```
cd backend
go build ./...
```

El binario del sidecar es:

```
go build -o bin\whatfast-backend.exe .\cmd\whatfast-backend   # Windows
go build -o bin\whatfast-backend ./cmd/whatfast-backend      # Linux, macOS
```

`backend/bin/` está fuera de git (ver `backend/.gitignore`).

## Probar

```
cd backend
go test ./...
```

El smoke test levanta el binario de verdad, le manda un `Hello` por el
transporte real y comprueba la respuesta:

```
cd backend
go run ./tools/smoke .\bin\whatfast-backend.exe
```

Salida esperada:

```
hello ok: protocol_version=21 backend_version="0.1.0"
shutdown ok: the sidecar exited with code 0
```

El smoke test no toca la red: un perfil nuevo no tiene dispositivo emparejado,
así que el sidecar ni intenta conectarse.

## Cómo la app encuentra el binario

La app nativa (el crate `whatfast-ui`) resuelve el ejecutable del sidecar con
esta orden, que es la misma que usaba el cliente anterior
(`backend_executable()` en `desktop/src/bridge.rs`):

1. **La variable de entorno `WHATFAST_BACKEND`**, con la ruta absoluta del
   ejecutable. Es la primera porque es la que usa el desarrollo: el binario
   sale de `backend/bin/` y esa ruta cambia con cada compilación.
2. **Junto al ejecutable de la app**: `<directorio de la app>/whatfast-backend.exe`
   en Windows y `<directorio de la app>/whatfast-backend` en Linux y macOS.
   Es la regla del paquete instalado, donde los dos binarios viajan juntos.
3. **El directorio del repositorio**, en desarrollo: `<repo>/backend/bin/whatfast-backend[.exe]`.
   Solo sirve para `cargo run` desde el repo; el paquete instalado nunca
   lleva el árbol de fuentes.

Si ninguna de las tres existe, la app falla al arrancar con un mensaje que
dice las tres rutas que probó. El sidecar no se arranca nunca sin ruta
absoluta: un path relativo se resuelve contra el directorio de trabajo, que
en un acceso directo de Windows es el directorio del usuario.

### Variables y flags del sidecar

El sidecar exige los dos flags, en este orden:

| Flag | Qué es |
|---|---|
| `--stdio` | obligatorio: sirve protobuf enmarcado por stdin/stdout. Sin el flag, el proceso sale con error. |
| `--data-dir` | obligatorio: directorio del perfil (sesión, base de chats, avatares, medios). El directorio se crea si no existe y se le restringen los permisos. |
| `--log-file` | opcional: archivo de logs. La app lo apunta a `<data-dir>/backend.log`. |

El directorio del perfil no se puede compartir entre dos procesos: el
sidecar toma un lock y el segundo proceso sale con error.

## El contrato de transporte

- **Encuadre**: 4 bytes big-endian con el tamaño del payload, y después el
  payload protobuf. El máximo es 8 MiB.
- **Sobre el papel**: `proto/bridge.proto`, generado en `gen/bridgev1/`.
- **Versión del protocolo**: 21. El `Hello` es siempre el primer request; el
  cliente manda `minimum_protocol_version` y `maximum_protocol_version`, y el
  sidecar responde `unsupported_protocol` si no hay solapamiento.
- **Los eventos** del sidecar solo empiezan a salir después de que se
  escribió la respuesta del `Hello`.

El códec del lado Go está en `internal/bridge/codec.go`. El lado Rust está en
`crates/whatfast-protocol/src/transport.rs` (`write_frame` / `read_frame`).

## Estructura

| Ruta | Qué es |
|---|---|
| `cmd/whatfast-backend/` | el entrypoint: parsea los flags, abre el perfil, levanta el servidor |
| `internal/app/` | el servidor del bridge: despacha los 64 commands y emite los eventos |
| `internal/bridge/` | el códec del framing |
| `internal/wa/` | el cliente de `whatsmeow`: conexión, sincronización, medios |
| `internal/store/` | la base de datos de chats y mensajes |
| `internal/domain/` | los tipos del dominio |
| `internal/linkpreview/`, `internal/search/`, `internal/securefs/`, `internal/profilelock/`, `internal/sqliteprivacy/`, `internal/logging/` | piezas de apoyo |
| `gen/bridgev1/` | el código generado del `.proto` |
| `tools/smoke/` | el smoke test que prueba el binario de verdad |

`cmd/rust-meow-backend/main.go` es un archivo obsoleto con `//go:build ignore`:
no compila y no se usa. Está ahí porque el volumen `G:` del repositorio de
origen no permite borrar archivos.

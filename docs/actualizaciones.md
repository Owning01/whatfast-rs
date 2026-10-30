# Actualizaciones de Whatfast

Whatfast se actualiza solo: pregunta a GitHub Releases si hay una versión nueva,
la baja en segundo plano y, cuando el usuario acepta, reemplaza sus dos
ejecutables y se reinicia.

El código vive en `crates/whatfast-ui/src/update.rs`. La sección **Updates** de
Ajustes muestra el estado y los botones.

## De dónde baja

La dirección base es:

```
https://github.com/Owning01/whatfast-rs/releases/latest/download
```

`releases/latest/download/<archivo>` siempre devuelve el archivo del release más
nuevo. La app hace una sola petición por actualización y no gasta cuota de la API
de GitHub.

La app pide un manifiesto por plataforma:

| Plataforma | Manifiesto |
| --- | --- |
| Windows x86_64 | `whatfast-windows-x86_64.json` |
| Linux x86_64 | `whatfast-linux-x86_64.json` |

macOS y las arquitecturas que no son x86_64 no tienen manifiesto. En esas copias
la actualización automática queda en `UpdatePhase::Unsupported` y el usuario
actualiza a mano.

## Formato del manifiesto

```json
{
  "version": "0.2.31",
  "app": {
    "asset": "whatfast-windows-x86_64.exe",
    "sha256": "…64 caracteres hex…"
  },
  "backend": {
    "asset": "whatfast-windows-x86_64-backend.exe",
    "sha256": "…64 caracteres hex…"
  }
}
```

Reglas del manifiesto:

1. `version` es un número con puntos. Un sufijo con `-` (por ejemplo
   `0.2.31-beta`) no es un release y el manifiesto se descarta.
2. `app` y `backend` son obligatorios.
3. `asset` es un nombre de archivo, nunca una ruta.
4. `sha256` tiene 64 dígitos hexadecimales.

La comparación es numérica por partes: `0.2.31` es más nuevo que `0.2.9`. Si la
versión publicada es igual o menor, la app responde "nada nuevo".

## Flujo

1. **Chequeo.** La app baja el manifiesto de su plataforma con un límite de
   64 KiB y 20 segundos.
2. **Descarga.** Baja los dos ejecutables a
   `%LOCALAPPDATA%\whatfast\update\<versión>\` en Windows, o
   `~/.local/share/whatfast/update/<versión>/` en Linux. El límite es 256 MiB por
   archivo y el plazo es 300 segundos. Verifica el SHA-256 de cada archivo.
3. **Marca.** Deja `ready.json` en la misma carpeta con la versión y las sumas.
4. **Instalación.** El usuario aprieta **Install**. La app corta el sidecar y
   copia los dos archivos nuevos junto al ejecutable actual.
5. **Reinicio.** La copia nueva arranca sola y la vieja cierra el proceso.

El paso 2 se conserva entre reinicios: si el usuario cierra la app con la
descarga lista, la sección **Updates** la ofrece de nuevo al abrir.

## Reemplazo y rollback

El orden del reemplazo es fijo:

1. La app se copia a `.<nombre>.new` y el sidecar a `.<nombre>.new`.
2. El ejecutable actual se renombra a `<nombre>.old-<segundos>`.
3. La copia nueva toma el nombre del ejecutable.
4. Se repite lo mismo con el sidecar.

Si falla el reemplazo del sidecar, la app se vuelve a renombrar a su nombre
original y no cambia nada. Si el reemplazo de la app falla, tampoco se toca el
sidecar.

Los archivos `*.old-<segundos>` se borran en el siguiente arranque
(`update::cleanup()`). Un archivo que otro proceso todavía tiene abierto se
saltea y se reintenta en el arranque siguiente.

La carpeta de descarga que no esté esperando un reinicio también se borra.

## Variables de entorno

| Variable | Efecto |
| --- | --- |
| `WHATFAST_UPDATE_URL` | Cambia la dirección base. Se usa para probar contra un servidor local. |
| `WHATFAST_NO_UPDATE` | Con cualquier valor distinto de `0`, la actualización automática queda apagada. La sección **Updates** informa que está apagada. |

## Cómo publicar una versión

1. Subir `version` en `Cargo.toml` (`[workspace.package]`).
2. Crear y empujar la etiqueta `v<versión>`, por ejemplo `v0.2.31`.
3. El workflow `.github/workflows/release.yml` compila, calcula las sumas y
   publica los archivos.

El workflow corta la publicación si la etiqueta y `Cargo.toml` no dicen la misma
versión. Ese chequeo evita que la app se actualice a una versión que no es la
publicada.

Archivos que publica cada plataforma:

- Windows: `whatfast-windows.zip`, `whatfast-setup.exe`, sus `.sha256`,
  `whatfast-windows-x86_64.exe`, `whatfast-windows-x86_64-backend.exe` y
  `whatfast-windows-x86_64.json`.
- Linux: `whatfast-linux.tar.gz`, su `.sha256`,
  `whatfast-linux-x86_64`, `whatfast-linux-x86_64-backend` y
  `whatfast-linux-x86_64.json`.

## Cómo probar en local

1. Armar un release falso con una versión mayor, por ejemplo `0.2.31`, y los dos
   ejecutables de `dist\`.
2. Calcular el SHA-256 de cada ejecutable.
3. Escribir el manifiesto de la plataforma en una carpeta servida por HTTP.
4. Arrancar Whatfast con `WHATFAST_UPDATE_URL=http://127.0.0.1:<puerto>`.
5. En Ajustes, apretar **Check for updates**, después **Download** y después
   **Install**.
6. Confirmar que la app vuelve a abrir con la versión nueva y que el archivo
   `.old-<segundos>` desaparece.

La misma prueba sirve con un manifiesto cuya suma no coincide: la descarga falla
y la app sigue con la versión vieja.

## Límites

| Dato | Valor |
| --- | --- |
| Tamaño del manifiesto | 64 KiB |
| Tamaño de cada ejecutable | 256 MiB |
| Plazo del chequeo | 20 segundos |
| Plazo de cada descarga | 300 segundos |

Ningún archivo del release lleva claves ni datos del perfil del usuario.

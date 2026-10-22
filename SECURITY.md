# Seguridad

## Cómo reportar un problema

No abras un issue público con los detalles. Usá el reporte privado de
GitHub: en la pestaña **Security** del repositorio, **Report a
vulnerability**. Ese canal avisa solo a los mantenedores.

Si el reporte privado no está disponible, abrí un issue que diga únicamente
"tengo un problema de seguridad, necesito un canal privado" y esperá la
respuesta.

## Qué cubre

- El cliente de escritorio (`whatfast-ui`) y el sidecar (`whatfast-backend`).
- El contrato y el transporte entre los dos procesos.
- El sitio publicado en `what-fast.web.app`.

## Cómo se guardan los datos

- Las llaves de la sesión de WhatsApp viven en `session.db`, en el
  directorio de datos del usuario, con permisos `0600` y el directorio con
  `0700`.
- El historial vive en `client.db`, con los mismos permisos.
- El cliente y el sidecar hablan por la entrada y la salida estándar del
  proceso: no hay puerto TCP abierto, no hay socket de red.
- Los adjuntos se guardan en `media/` con un nombre derivado del chat y del
  mensaje, nunca del nombre que manda el servidor.
- Las vistas previas de los enlaces se piden desde la app, pero solo a
  direcciones públicas: no se puede apuntar a `localhost` ni a la red local.

## Lo que el proyecto no hace

- No manda datos a ningún servidor propio: no hay telemetría ni analítica.
- No incluye claves ni credenciales en el repositorio. El CI las busca con
  `scripts/chequear-secretos.py` antes de cada commit y el repositorio tiene
  el escaneo de secretos y la protección de push activados.

## Paquetes publicados

Los binarios no están firmados con un certificado de código. Cada release
publica el `sha256` de lo que sube: sirve para comprobar que el archivo
bajado es el que se publicó, no para certificar quién lo compiló.

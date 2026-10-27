# Agentes en Whatfast

El servidor MCP de Whatfast y el socket local que lo alimenta. Un harness
(Claude Code, Codex, Cursor, OpenCode, Pi, Hermes) se conecta a este servidor y
trabaja sobre la cuenta de WhatsApp del usuario sin tocar la sesión.

## Las dos piezas

| Pieza | Qué hace |
|---|---|
| Socket local del backend | El sidecar (`whatfast-wa`) escucha el mismo contrato de sobres que habla la ventana, además de su stdin. Un Windows por perfil: tubería `\\.\pipe\whatfast-<huella>`. En Linux y macOS: `<perfil>/agent.sock`, solo dueño. |
| `whatfast mcp` | El subcomando del mismo `whatfast.exe` habla JSON-RPC por stdio, como pide el protocolo MCP, y traduce cada herramienta a un pedido del puente. |

El nombre del endpoint se deriva de la carpeta del perfil
(`whatfast_protocol::local::endpoint`): el backend y los clientes que resuelven
el mismo perfil se encuentran solos, sin configuración.

## Comando

```
whatfast.exe mcp --agent <id> [--data-dir <carpeta>] [--backend <sidecar>]
```

- `--agent`: el id del harness (`claude-code`, `codex`, `cursor`, `opencode`,
  `pi`, `hermes`, `claude-desktop`, `vscode`). Hoy solo nombra la conexión; los
  permisos por agente llegan con la base de agentes.
- `--data-dir`: el perfil. Por defecto `%LOCALAPPDATA%\whatfast` (o
  `$XDG_DATA_HOME/whatfast`, o `~/Library/Application Support/whatfast`), y
  respeta `WHATFAST_DATA_DIR`.
- `--backend`: el sidecar que arranca cuando no hay nadie escuchando. Por
  defecto el `whatfast-backend[.exe]` que está al lado de `whatfast.exe`.

## Cómo se comporta

1. Al primer pedido abre el endpoint derivado del perfil.
2. Si la ventana está abierta, se cuelga del backend que ya corre. La sesión de
   WhatsApp es una sola.
3. Si la ventana está cerrada, arranca `whatfast-backend --serve --idle-exit
   600`, sin ventana y sin consola. Ese backend se cierra solo cuando pasan diez
   minutos sin pedidos, o cuando este servidor termina (le manda `Shutdown`).
4. Manda `Hello` y después `Reconnect`: un backend que nadie más arrancó está
   offline hasta que se le pide conectarse, y offline no llega ningún mensaje.

La E/S es asíncrona a los dos lados. En Windows, una tubería abierta en modo
síncrono serializa sus operaciones: una lectura pendiente bloquea la escritura
siguiente (medido: el segundo pedido no salía del proceso). Por eso cada lado
tiene una sola tarea dueña del stream y hace una operación por vez.

## Las siete herramientas

| Herramienta | Argumentos | Devuelve |
|---|---|---|
| `chats_list` | `limit`, `cursor`, `include_archived` | Las conversaciones con su `chat_id`, título, sin leer y el anticipo del último mensaje. |
| `messages_read` | `chat_id`, `limit`, `before_timestamp_ms`, `before_message_id` | Los mensajes de un chat, del más viejo al más nuevo. |
| `messages_search` | `query`, `limit` | Mensajes, grupos y contactos que ya están en el caché local. |
| `chat_info` | `chat_id` | Dirección, about, descripción del grupo y miembros. |
| `attachment_get` | `chat_id`, `message_id` | Baja el archivo al caché y devuelve su ruta. |
| `messages_wait` | `chat_id?`, `timeout_ms`, `after_*` | Espera un mensaje nuevo (o el tope) y devuelve lo que llegó con su cursor. |
| `message_send` | `chat_id`, `text`, `reply_to_message_id?`, `client_message_id?` | Manda un texto a un chat, como el dispositivo vinculado. Si el chat no está habilitado para enviar, deja un borrador. |
| `draft_propose` | `chat_id`, `text`, `reply_to_message_id?`, `reason?`, `confidence?` | Deja un borrador para que el usuario lo apruebe. |
| `drafts_list` | `state?`, `limit` | Los borradores en la bandeja, con su chat, motivo y confianza. |
| `draft_decide` | `draft_id`, `send` | Aprueba (`send: true`) o descarta el borrador. Aprobar no envía: el envío es del usuario. |

Los resultados son JSON compacto. Cada mensaje trae `at` en ISO-8601 UTC y
`at_ms`; el cuerpo se corta a 4000 caracteres y avisa con `clipped`.

`messages_wait` consulta el caché cada 1,2 a 1,8 segundos: no depende del canal
de eventos, que sigue siendo de la ventana.

## Los permisos

El permiso se fija **por agente y por chat**, nunca global, y vive en
`agents.db` (en la carpeta del perfil), que el lado del backend abre al
arrancar. La tabla `agents` guarda la escala, los chats, el tope por hora, la
ventana horaria y el modo de ritmo; el token se guarda como su SHA-256.

| Pieza | Qué hace |
|---|---|
| Escala | `read` lee, `draft` lee y deja borradores, `send` lee, deja borradores y manda. Son acumulativas. |
| Chats | Una lista de `chat_id`, o todos. Los grupos quedan afuera salvo que se pidan. Una lista vacía no cubre ningún chat. |
| Tope por hora | Cuenta los envíos de la última hora; al llegar al tope responde `rate_limited`. |
| Ventana | `08:00-23:00` en hora local del perfil. Fuera de ella responde `out_of_window`: el mensaje no sale. |
| Ritmo | Con el modo humanización el texto sale en partes, con la espera de alguien escribiendo. Se apaga por agente. |

Un agente presenta su token en el saludo (`Hello.agent_id` y
`Hello.agent_token`); el backend lo compara y, si no coincide, contesta
`unauthorized` en todos los pedidos de esa conexión. La identidad se fija por
conexión, no por proceso: el saludo de la ventana no habilita a un agente.

Los pedidos que un agente no puede usar —los ajustes, la sesión, las llamadas,
los borrados— responden `forbidden`: un agente habla con Whatfast, nunca con la
sesión de WhatsApp. Las listas que sí puede pedir se recortan a sus chats antes
de salir del proceso.

**Trampa**: mientras el perfil no tenga ningún agente registrado, un harness que
se anuncia sin token se trata como el usuario que lo arrancó. Es lo que permite
probar la conexión antes de que exista la pantalla Agentes; en cuanto hay una
fila en `agents.db`, el token es obligatorio.

## El ritmo

`whatfast-auto::humanize` corta el texto donde lo cortaría una persona (fin de
oración, si no un espacio) y calcula la espera: 45 ms por carácter, con un piso
de 1,5 s y un techo de 6 s, más una variación sembrada con el
`client_message_id`. Antes de cada parte manda `SetTyping` y espera; al final lo
apaga. Dos envíos seguidos del mismo texto nunca esperan lo mismo.


## Las automatizaciones

Una tarea es un instante (`once`) o un cron de cinco campos (`cron`), con un
chat, un prompt y una salida (`read`, `draft` o `send`). La tabla `schedules`
las guarda; el backend mira cada 30 segundos las que vencieron, escribe su
próximo turno **antes** de correr y lanza el harness en un hilo aparte.

- Un harness sin modo headless (Cursor, Claude Desktop, VS Code, Hermes) no
  atiende tareas: el intento queda en `agent_calls` como fallo con el motivo.
- La respuesta queda **siempre como borrador**, con el motivo `automatización:
  <primera línea del prompt>`. El envío automático necesita la bandeja, y llega
  después.
- Fuera de la ventana horaria la tarea no dispara: el próximo turno se corre al
  momento en que la ventana abre.
- El plazo de un harness es de 120 segundos; pasado eso se lo mata.

## La guía del agente

`whatfast_agents::guide` instala el texto de reglas en el archivo que cada
harness lee solo: `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`,
`~/.config/opencode/AGENTS.md` y `~/.cursor/rules/whatfast.mdc`. El bloque va
entre `<!-- whatfast:inicio -->` y `<!-- whatfast:fin -->`: instalar dos veces
no duplica nada y `remove` deja el archivo como estaba (con copia
`.whatfast-backup` antes de escribir). Claude Desktop, VS Code, Pi y Hermes no
declaran archivo de reglas.

## Probarlo

```
cargo build -p whatfast-ui -p whatfast-wa
python scripts\probar-mcp.py
python scripts\probar-mcp.py --esperar-ms 20000
```

El guion arranca `whatfast.exe mcp --agent probar`, recorre `initialize`,
`tools/list`, `chats_list`, `messages_read`, `chat_info`, `messages_search` y,
si se le pide, una espera. Con `--enviar <chat_id> <texto>` manda un mensaje de
verdad: sin esa bandera no escribe nada.

Si la aplicación instalada todavía no tiene el endpoint (release anterior a
M1), el guion falla con `the backend did not listen`: la ventana abierta ocupa
el perfil con un backend sin socket. Para probar sin cerrarla, apuntá a una
copia del perfil:

```
copy %LOCALAPPDATA%\whatfast\chats.db* %TEMP%\wf-mcp-test\
python scripts\probar-mcp.py --data-dir %TEMP%\wf-mcp-test --esperar-ms 8000
```

## Conectarlo a mano

```
claude mcp add --scope user whatfast -- "C:\ruta\a\whatfast.exe" mcp --agent claude-code
```

Codex: la tabla `[mcp_servers.whatfast]` en `~/.codex/config.toml` con
`command` y `args = ["mcp", "--agent", "codex"]`. OpenCode: en
`~/.config/opencode/opencode.jsonc`, dentro de `mcp.servers`:

```
"whatfast": {
  "type": "local",
  "command": ["C:\\ruta\\a\\whatfast.exe", "mcp", "--agent", "opencode"]
}
```

La ventana escribe estas mismas entradas con un clic (catálogo en
`crates/whatfast-agents`). **OpenCode se escribe por archivo, no por su CLI**:
en Windows el comando del PATH puede ser un shim de Bun que no arranca desde
otro proceso, y el archivo es lo que el cliente lee igual.

## El modo rápido y las automatizaciones

Las dos piezas que le piden algo a un agente son el mismo mecanismo: un harness
sin ventana, con plazo. viven en `whatfast-auto::runner` (lanzarlo y leer su
respuesta) y en `whatfast-auto::ask` (armar el pedido con el chat como
contexto).

| Pedido del bridge | Qué hace |
|---|---|
| `AgentAsk` | El modo rápido del chat: le pasa al agente el pedido del usuario con los últimos mensajes del chat y devuelve el texto al cuadro de composición. No manda nada. Plazo: 90 segundos. |
| `ScheduleList` | Las automatizaciones del perfil, con su próximo turno. |
| `SchedulePut` | Crea o reescribe una tarea y calcula su primer turno (cron de cinco campos, ventana horaria y zona del sistema). |
| `SchedulePause` | La pausa; al reanudarla, el próximo turno se cuenta desde ese momento. |
| `ScheduleRemove` | La borra. |
| `DraftFinish` | La ventana avisa que mandó el borrador aprobado (o que no pudo, con su motivo) y el estado queda `sent` o `failed`. |
| `GuideInstall` / `GuideRemove` / `GuideStatus` | La guía del agente en el archivo de reglas de cada harness, con copia de respaldo. |

Solo el usuario edita automatizaciones: un agente del socket las recibe con
`forbidden`. El cronómetro del backend mira las tareas vencidas cada 30 segundos
(60 cuando lo arrancó un agente), escribe el turno siguiente **antes** de correr
y deja la respuesta del harness como borrador.

## La ventana

El riel tiene un destino **Agents** (`ctrl-shift-a`). Adentro:

- La lista de harnesses de esta máquina, con su estado (instalado, detectado,
  conectado) y el botón **Connect**. Conectar genera el token, escribe la fila en
  `agents.db`, registra el servidor MCP en la configuración del harness y avisa
  dónde quedó.
- **Choose config file…**: si el archivo no está donde la ficha lo busca (el
  cliente se movió o cambió de nombre), el usuario lo elige a mano. La ruta queda
  guardada en `agents/<id>.config` y el siguiente Connect escribe ahí sin
  preguntar. El formato lo decide la extensión: `.json`, `.jsonc`, `.toml` o
  `.yaml`.
- Los **permisos** del agente en frente: escala, chats, tope por hora, ventana y
  ritmo humano.
- La **bandeja de borradores**, con motivo y confianza. **Send** manda el texto
  con la misma ruta que el compositor y deja el borrador en `sent`; si falla,
  `failed` con la línea del backend.
- La sección **Automations**: alta, pausa y borrado de tareas, con el próximo
  turno a la vista.

En el chat, **Ask an agent** (`ctrl-shift-q`) abre la tarjeta del modo rápido:
escribís el pedido, el agente lo contesta con los últimos mensajes del chat como
contexto, y la respuesta aparece en el compositor. No se manda sola.

## Lo que todavía no está

- **Editar una tarea** desde la pantalla: se crea, se pausa y se borra; el
  backend ya acepta `id` para reescribir una.
- **Desconectar borra el registro** del servidor en la configuración del harness:
  hoy revoca el token y la fila, la entrada queda y hay que sacarla a mano.
- **Reintentar un borrador fallido** desde la bandeja.
- **El envío del borrador aprobado** desde el backend: `DraftDecide` lo deja
  `approved` y lo manda la ventana, que es donde está la sesión de WhatsApp.
- Cursor, Claude Desktop, VS Code y Hermes no tienen modo headless confirmado:
  en esos, la aplicación solo lee y propone; no atienden el modo rápido ni una
  tarea del cronómetro.

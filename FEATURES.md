# Catálogo de Funcionalidades — `whatfast-rs`

Inventario detallado de las funcionalidades implementadas en el sistema, organizadas por área funcional con referencia directa a los módulos responsables.

---

## 1. Mensajería y Conversaciones

- **Envío y recepción en tiempo real**: texto enriquecido con formato WhatsApp (`*negrita*`, `_cursiva_`, `~tachado~`, `` `código` ``, bloques de código y citas) procesado en [whatsapp_formatting.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-core/src/whatsapp_formatting.rs) y [send.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/send.rs).
- **Cola offline (`outbox`)**: si el usuario envía mensajes sin conexión, se persisten en SQLite (`outbox` en [db.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/db.rs)) y se reenvían automáticamente al recuperar el estado conectado ([app.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/app.rs)).
- **Respuestas y citas**: citar cualquier mensaje entrante o saliente con vista previa de contenido y salto al mensaje original.
- **Edición de mensajes**: edición de mensajes propios de texto dentro de la ventana de 20 minutos (`EDIT_WINDOW_MS` en [message_actions.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/message_actions.rs)).
- **Eliminación granular**: *Eliminar para mí* y *Eliminar para todos* con confirmación previa y posicionamiento adaptativo del menú contextual hacia arriba para no ocultar mensajes inferiores.
- **Reenvío y selección múltiple**: modo de selección múltiple de mensajes para copiar, destacar o reenviar a uno o varios chats mediante el selector [message_actions.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/message_actions.rs).
- **Fijado y destacados**: fijar/desfijar mensajes en la cabecera del chat y marcar/desmarcar mensajes con estrella ([starred.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-model/src/starred.rs)).
- **Reacciones con emoji**: añadir, cambiar o quitar reacciones y consultar el detalle de quién reaccionó.
- **Encuestas**: creación de encuestas con múltiples opciones y votación en vivo.
- **Indicadores de presencia y lectura**: estado de escritura (*typing*), estados de envío (`Pending`, `Sent`, `Delivered`, `Read`, `Failed`) y envío de acuses de lectura.
- **Mensajes efímeros y View Once**: soporte de temporizadores de desaparición por chat y detección/manejo de mensajes *Ver una sola vez*.
- **Borradores persistentes por chat**: guardado automático con debounce de 500 ms y escritura atómica en `drafts.json` ([composer_drafts.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/composer_drafts.rs)).

---

## 2. Organización, Carpetas y Búsqueda Avanzada

- **Filtros rápidos de lista de chats**: `All`, `Unread`, `Favorites`, `Groups` y `Mentions` (`@`) definidos en [model.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-model/src/model.rs).
- **Carpetas personalizadas de chats**: agrupación configurable de conversaciones (`ChatFolder` en [settings.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-model/src/settings.rs)) con chips de filtrado rápido.
- **Gestión de chats**: fijar, archivar, silenciar (con duración), bloquear/desbloquear contactos, ver información detallada del chat y listar participantes de grupos ([members.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/members.rs)), además de creación de grupos nuevos (`CreateGroup`).
- **Pestañas de conversaciones y switcher rápido**: barra de pestañas abiertas ([tabs.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/tabs.rs)) y conmutador de chats recientes con `Ctrl+Tab` / `Ctrl+Shift+Tab`.
- **Búsqueda global FTS5 y facetada**: índice trigrama en [db.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/db.rs) con soporte de facetas:
  - `has:photo`, `has:video`, `has:doc`, `has:audio`, `has:link`
  - `from:<nombre o remitente>`
  - Navegación con flechas y salto directo al mensaje encontrado en la conversación.

---

## 3. Multimedia, Audio, Video y Editor de Imágenes

- **Visor de imágenes**: apertura a pantalla completa sobre la ventana, carrusel entre fotos del chat con `←`/`→`, zoom, información del archivo y copia directa de imagen al portapapeles (`Ctrl+C`, [clipboard_image.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/clipboard_image.rs)).
- **Reproductor de video**: reproducción embebida en burbuja y pantalla completa mediante Windows Media Foundation / `ffmpeg` en Unix ([video.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/video.rs)), con controles de pausa, volumen, `Seek` y velocidad (`0.5x`–`2.5x`).
- **Mini-reproductor flotante persistente**: barra superior ([mini_player.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/mini_player.rs)) que permite controlar notas de voz o videos activos mientras se navega por otros chats, con botón de salto directo a la conversación.
- **Notas de voz**:
  - **Grabación**: captura de micrófono y codificación nativa en Rust a Ogg Opus (`opus-pure` en [recording.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/recording.rs)).
  - **Reproducción**: control de velocidad (`1x`, `1.5x`, `2x`) sin distorsión de tono mediante algoritmo **WSOLA** y seek con `←`/`→` ([audio.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/audio.rs)).
  - **Transcripción local**: transcripción de notas de voz desde la tarjeta de audio invocando `whisper` / `whisper-cli` / `whisper.cpp` (`transcribe_voice_note`).
- **Editor de imágenes integrado**: recorte libre y proporciones, rotación, dibujo de formas, texto y stickers/emojis rasterizados sobre la imagen antes de enviar ([media_editor.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/media_editor.rs), [media_overlays.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/media_overlays.rs)).
- **Adjuntos múltiples y Drag & Drop**: bandeja de previsualización de hasta **32 adjuntos** ([composer_attachments.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/composer_attachments.rs)) y soporte de arrastrar y soltar archivos desde el explorador del sistema (`.on_drop`).
- **Stickers, GIFs, enlaces y ubicaciones**:
  - Bandeja de stickers enviados/recibidos ([stickers.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/stickers.rs)).
  - Buscador e inserción de GIFs ([gif_tray.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/gif_tray.rs)).
  - Vistas previas ricas de enlaces ([link_preview.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/link_preview.rs)).
  - Tarjetas de ubicación con apertura en mapa y tarjetas de contactos vCard ([ubicaciones.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/ubicaciones.rs)).
- **Gestión de caché de medios**: inspección de espacio ocupado por tipo de archivo y limpieza selectiva de `<data-dir>/media` sin borrar mensajes ([storage.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/storage.rs)), además de límites de auto-descarga (`auto_download_max_mb`) y omisión en redes medidas (`skip_auto_download_metered`).

---

## 4. Estados y Canales (Newsletters)

- **Estados (Status)**:
  - Visor dedicado con barra de progreso, avance/retroceso (`←`/`→`), pausa automática al comenzar a responder un estado ([statuses.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/statuses.rs)).
  - **Auto-descarga de video** al abrir un estado multimedia.
  - **Expiración automática a 2 días** para limpiar estados antiguos.
  - **Publicación de estados** de texto, imagen y video (con recorte automático a 30 segundos vía `ffmpeg`).
  - **Silencio por contacto** desde el menú contextual y persistencia de estados vistos con anillo indicador en la lista de chats ([status_seen.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/status_seen.rs)).
- **Canales (Newsletters)**: vista dedicada para listar canales suscritos, leer publicaciones, suscribirse por clave de invitación y cancelar suscripciones ([newsletters.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/newsletters.rs)).

---

## 5. Llamadas VoIP de Audio y Video

- **Llamadas de voz y video** sobre `whatsapp-rust` (`voip`) y el puente v22 ([calls.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/calls.rs)):
  - Captura y reproducción PCM a 16 kHz ([call_audio.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/call_audio.rs)).
  - **Cancelador de eco adaptativo NLMS** en Rust puro y puerta de ruido (*noise gate*).
  - Captura de cámara web vía `ffmpeg` y **compartición de pantalla** multiplataforma (`gdigrab` en Windows, `x11grab` en Linux en [call_video.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/call_video.rs)).

---

## 6. Productividad, Composición y Traducción

- **Paleta de comandos (`Ctrl+K` / `Cmd+K`)**: salto instantáneo a cualquier chat o ejecución de comandos globales con filtrado en vivo y navegación por teclado ([command_palette.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/command_palette.rs)).
- **Atajos de formato en el compositor**: `Ctrl+B` (negrita `*...*`), `Ctrl+I` (cursiva `_..._`), `Ctrl+Shift+X` (tachado `~...~`).
- **Plantillas rápidas (`/templates`)**: inserción de respuestas reutilizables configuradas en `Settings::quick_templates`.
- **Autocompletado de menciones (`@`)**: popup navegable con `↑`/`↓`/`Enter`/`Tab` en grupos.
- **Corrector ortográfico**: verificación con `hunspell` multiplataforma ([spellcheck.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/spellcheck.rs)).
- **Traducción de mensajes**: acción *Translate* en el menú de mensajes con caché local ([translate.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/translate.rs)).
- **Exportación de conversaciones**: exportación a **TXT**, **JSON** y **HTML** desde Ajustes o la paleta de comandos ([export.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/export.rs)).

---

## 7. Ecosistema de Agentes IA, MCP y Automatizaciones

- **Integración con 9 harnesses de IA** ([agents_panel.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/agents_panel.rs), [spec.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-agents/src/spec.rs)): `claude-code`, `codex`, `opencode`, `cursor`, `claude-desktop`, `vscode`, `antigravity`, `pi` y `hermes`, con detección automática, configuración por panel individual, escritura segura de archivos de configuración y guía de reglas.
- **Servidor MCP (`whatfast.exe mcp --agent <id>`)** ([tools.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-mcp/src/tools.rs)) con **16 herramientas**:
  - Lectura y espera: `chats_list`, `messages_read`, `messages_search`, `chat_info`, `attachment_get`, `messages_wait`, `statuses_list`.
  - Borradores y aprobación: `draft_propose`, `drafts_list`, `draft_decide`.
  - Acciones en WhatsApp: `message_send`, `message_send_media`, `message_react`, `poll_create`, `edit_message`, `pin_message`.
- **Gobierno y seguridad (`Guard` + `humanize`)**: permisos por chat, exclusión de grupos, cuota máxima por hora, ventana horaria y fragmentación de mensajes largos con pausas humanas.
- **Bandeja de borradores en chat**: barra dedicada para enviar, editar o descartar borradores propuestos por agentes ([chat_drafts.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/chat_drafts.rs)).
- **Quick Ask AI (`Ctrl+Shift+Q`)**: preguntas rápidas sobre el chat abierto con respuesta directa al compositor y botón de 1 clic **Summarize unread** ([quick_ask.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/quick_ask.rs)).
- **Automatizaciones programadas y reactivas** ([automations_panel.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/automations_panel.rs), [schedules.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/schedules.rs), [scheduler.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/scheduler.rs)):
  - Tareas cron de 5 campos ejecutadas por agente IA o envíos literales directos (`agent_id == "__direct__"`).
  - Disparadores reactivos por mensaje entrante en [ingest.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-wa/src/ingest.rs).

---

## 8. Accesibilidad, Navegación por Teclado y Personalización

- **Navegación completa por teclado**:
  - `F6` / `Shift+F6`: cicla el foco entre `NavRail`, `ChatList`, `MessageList` y `Composer`.
  - En `ChatList`: `↑`/`↓` cambia de chat, `Enter`/`→` entra a los mensajes.
  - En `MessageList`: `↑`/`↓` (o `K`/`J`) recorre mensajes con anillo de enfoque visible; `Enter`/`Espacio`/`Shift+F10` abre el menú del mensaje; `R` responde, `E` edita, `S` destaca, `Ctrl+C` copia, `Delete` elimina para mí, `I`/`Ctrl+E` enfoca el compositor.
  - Navegación por teclado en menús contextuales (`↑`/`↓`/`Enter`/`Esc`) y Paleta de Comandos.
  - Guía interactiva de atajos con `Ctrl+/` o `F1` ([shortcuts_help.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/shortcuts_help.rs)).
- **Accesibilidad visual y motora**:
  - **Modo Alto Contraste** (`high_contrast`): refuerza bordes, contraste de texto secundario y anillo de foco en [tokens.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/tokens.rs).
  - **Movimiento Reducido** (`reduced_motion`): desactiva/minimiza transiciones animadas.
  - **Zoom instantáneo de interfaz**: `Ctrl++` / `Ctrl+=`, `Ctrl+-` y `Ctrl+0` (`ui_scale` de `1.0` a `1.5`), además de escala independiente de fuente de mensajes (`message_font_scale`).
- **Temas, apariencia e idiomas**:
  - Temas claro/oscuro de WhatsApp + temas personalizados con editor de tokens (`editing_token`), densidad (`Comfortable`/`Compact`), radio y ancho de burbujas, colores y fondos de chat.
  - Fuente de emojis Noto Color Emoji embebida (o fuente Apple/personalizada) con parche `GsubLigatures` en [direct_write.rs](file:///G:/Proyectos/17-whatfast-rs/vendor/gpui_windows/src/direct_write.rs) para renderizar correctamente variantes de tono de piel y secuencias ZWJ.
  - Interfaz traducida al **100%** en **Inglés (`en`)**, **Español (`es`)** y **Chino (`zh`)** ([whatfast-i18n](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-i18n)).

---

## 9. Seguridad, Escritorio y Operaciones

- **App Lock (`Ctrl+L`)**: bloqueo por PIN local con PBKDF2-SHA256, bloqueo automático tras inactividad (`auto_lock_minutes`), desenfoque de privacidad al perder foco (`privacy_blur_on_unfocus`) y verificación biométrica en Linux (`fprintd-verify`, [app_lock.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/app_lock.rs)).
- **Integración de escritorio**:
  - Bandeja del sistema con contador de no leídos (incluyendo D-Bus `Unity.LauncherEntry` en Linux, [tray.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/tray.rs)).
  - Notificaciones nativas respetando modo **No Molestar** (`do_not_disturb`, [notifications.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/notifications.rs)): toast de WinRT propio en Windows (AUMID `Owning01.Whatfast` en el acceso directo, sin sonido del sistema) y D-Bus en Linux.
  - **Sonidos de los mensajes** ([sounds.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/sounds.rs)): sonido distinto al recibir y al enviar, con presets sintetizados (Pop, Ding, Ping, Campanilla), un archivo propio del usuario (`file:<ruta>`) o silencio, y un interruptor general.
  - Arranque automático (`autostart` en Registro de Windows o `~/.config/autostart/whatfast.desktop` en Linux).
  - Multi-perfil mediante variable de entorno `WHATFAST_PROFILE` ([paths.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-model/src/paths.rs)).
  - Soporte de proxy HTTP/SOCKS5 (`proxy_url`).
- **Actualizaciones y diagnóstico**:
  - Auto-actualizador verificado por SHA-256 desde GitHub Releases ([update.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/update.rs)).
  - **DevStats Inspector** (`Ctrl+Shift+D`, [dev_stats.rs](file:///G:/Proyectos/17-whatfast-rs/crates/whatfast-ui/src/dev_stats.rs)) con métricas de arranque, caché, SQLite y registro de llamadas de agentes.

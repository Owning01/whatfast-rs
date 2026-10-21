# Referencias visuales

Las capturas de **WhatsApp Desktop** que mandaste. Son la referencia de cómo
tiene que verse la app, y se revisan una por una mientras se trabaja.

| Archivo | Qué muestra |
|---|---|
| `01-whatsapp-chats.png` | La columna de chats: título "Chats", botón `⋮`, botón verde `+`, buscador "Buscar un chat o iniciar uno nuevo", fila de filtros (Todos / No leídos / Favoritos / `▾`) y la lista con la hora a la derecha. |
| `02-whatsapp-estados.png` | La vista de **Estados**: título "Estados", "Mi estado", sección "RECIENTE" con los estados de cada contacto, y a la derecha "Comparte estados" con "Estado de texto" y "Foto y video". |
| `03-whatsapp-archivados.png` | La vista de **Archivados**: título "Archivados", la explicación de arriba y la lista de chats archivados. |
| `04-whatsapp-perfil.png` | La vista del **perfil**: la foto, el buscador y la lista de ajustes (General, Perfil, Cuenta, Privacidad, Chats, Video y voz, Notificaciones, Ajustes del teclado). |
| `05-whatsapp-canales.png` | La vista de **Canales**: título "Canales", botón `+`, buscador, la lista de canales con la hora y lapreview, "Busca canales para seguir" con los botones "Seguir", y a la derecha "Descubre canales". |
| `06-whatsapp-llamadas.png` | La vista de **Llamadas**: título "Llamadas", buscador, "Favoritos" con "Ver todo", "Recientes" y a la derecha "Llamadas y videollamadas". |

## Cómo se usan

- Cada cambio de la interfaz se compara contra estas imágenes.
- Lo que la imagen muestra y la app todavía no tiene, queda anotado en
  `PROJECT_MEMORY.md` como pendiente, con el archivo de la captura al lado.
- Cuando una vista queda igual que su referencia, se marca en el índice de
  este archivo con la fecha del día en que quedó.

## Estado de cada vista

| Vista | Estado |
|---|---|
| Chats (`01`) | La barra quedó con el título "Chats", el buscador "Search or start a new chat", la fila de filtros, la hora a la derecha de cada fila, el prefijo "You:" y los indicadores de silencio, favorito y pin. |
| Estados (`02`) | Tiene su propio destino en el riel: "My status", la sección "RECENT" y el panel "Share status" con los botones "Text status" y "Photo and video". |
| Canales (`05`) | Tiene su propio destino en el riel: el buscador, la lista de canales con la hora a la derecha y los botones "Follow" y "New local channel". |
| Archivados (`03`) | Tiene su propio destino en el riel, con la explicación de arriba y la lista. |
| Perfil (`04`) | El panel de ajustes del riel. |
| Llamadas (`06`) | Todavía no: el ladocar no tiene RPC de llamadas. |

## Reglas que salen de las capturas

1. **Los estados y los canales no son chats.** La lista de chats muestra solo
   conversaciones: los estados (`status@broadcast`) y los canales
   (`*@newsletter`) quedan fuera de la lista y viven en su propia vista.
2. **La columna de chats** lleva el título "Chats", los botones de la derecha,
   el buscador arriba del todo y la fila de filtros debajo del buscador.
3. **Cada cosa en su vista**: chats, estados, archivados, canales y llamadas
   son vistas separadas del riel de la izquierda, no secciones mezcladas en la
   misma columna.

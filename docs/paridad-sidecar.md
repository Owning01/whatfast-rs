# Paridad del sidecar: Go (oráculo) contra Rust

Estado del reemplazo de `backend/` (Go, congelado) por `crates/whatfast-wa/`
(Rust) bajo el mismo contrato `proto/bridge.proto` (v22). El juez que produce
esta tabla es `scripts/paridad_sidecar.py`; se re-corre con cada cambio.

## El contrato de RPC (50 RPC del oráculo)

| Estado | Cantidad | Qué es |
|---|---|---|
| Brazo real | 50 | El dispatch de `server.rs` sirve el RPC con la semántica del Go. |
| Pendiente declarado | 0 | El ledger está vacío: no hay deuda. |
| Sin declarar | 0 | Un RPC del Go sin brazo ni declaración **frena el gate**. |
| Extra del puerto | 6 | `CallPlace`…`CallAudio`: las llamadas, que el oráculo nunca sirvió. |

No queda ningún `not_implemented` en el sidecar: los últimos 7
(`VotePoll`, `RequestOlderHistory`, `GetChatHistoryCoverage`,
`GetHistoryOverview`, `GetIntegrityStatus`, `RepairLocalCache`,
`RepairRecentReactions`) ganaron brazo real. El `match` del dispatch es
exhaustivo a propósito: un RPC nuevo del contrato no compila hasta tener
su brazo, así que el gate ya no depende de declarar deuda sino del
compilador. El brazo `Some(other)` se eliminó por inalcanzable.

## Métodos del oráculo

El inventario de métodos del `Client` y del `Store` del Go contra las
funciones del puerto es un **reporte**, no un gate: el puerto renombró y
fusionó (p. ej. `EnsureConversation` vive como `ensure_chat` +
`adopt_address` + `merge_chats`), así que la paridad semántica de cada
traducción la auditaron las particiones (proceso/puente, store, ingesta,
envíos/medios, server/identidad), no un nombre. El juez deja ver los
candidatos: `store` 83 métodos (32 con homónimo), `client` 53 (25).

## Qué auditó cada partición

| Partición | Archivos | Resultado |
|---|---|---|
| A · proceso/puente | `main.rs`, `bridge.rs`, `tests/bridge_smoke.rs` | Flags y exit codes del Go, doble lock (`backend.lock` + `.profile.lock`), framing con máximo de 8 MiB, EOF truncado que ya no pasa por cierre limpio, handshake y `Shutdown`. 16 tests. |
| B · store | `db.rs`, `tests/store.rs` | Reducer de mensajes (antes `INSERT OR IGNORE` perdía ediciones y revokes), tabla de `SetMessageStatus`, tombstone de reacciones, ventana de 25/50, búsqueda difusa del Go, merge sin pérdida, `busy_timeout`. 27 tests. |
| C · ingesta | `ingest.rs`, `tests/ingest.rs` | Tabla de clasificación tipo a tipo, `unread` entrante, puntero forward-only, reply/forwarding, revoke con hora, receipts y presence, reacciones planas y encriptadas. 12 tests de integración. |
| D · envíos/medios | `send.rs`, `media.rs`, `meta.rs` | Idempotencia por `client_message_id` (la reserva no funcionaba por un `UNIQUE` con `''`), contexto de respuesta con autor, ventanas de edición/revocación, recibos por autor, sniffing y límites de imagen, voz Ogg/Opus, caché de avatares con poda. 24 tests. |
| E · server/identidad | `server.rs`, `session.rs`, `wire.rs`, `identity.rs`, `reconcile.rs` | Tabla RPC a RPC, los 8 RPC nuevos, `wire` campo por campo (adjuntos heredados de `image_*`, encuestas desde `polls`), sync inicial con la regla del Go, fold LID/PN idempotente y concurrido. 19 tests de server y 8 de identidad. |
| 7 RPC | `db.rs`, `ingest.rs`, `send.rs`, `server.rs`, `tests/server.rs`, `tests/store.rs` | Cobertura del historial (`history_coverage`, `save`, `overview`, `oldest`), integridad y reparación (`integrity_status`, `repair_local_cache`), clave de encuestas (`creator_jid` + `message_secret`, guardada al crear, al ingerir y migrada con `ALTER TABLE`), votos (`apply_poll_vote`, `vote_raw` real), historial bajo demanda (`fetch_message_history` + máquina de estados + expiración de 30 s) y reparación de reacciones (reserva + PDO por ítem). 11 tests nuevos. |

## Hallazgos que quedan fuera del gate

- **Rollback**: el Go congelado sirve v21 y la UI negocia 22; volver al Go
  exige bajar la negociación de la UI. El camino está documentado en
  `MIGRATION.md`, no es automático.
- **Lock**: el Rust toma `backend.lock` **y** `.profile.lock` (el nombre del
  Go), así ningún binario viejo corre en paralelo sobre el mismo perfil.
- **Log**: texto (tracing) en el Rust, JSON (slog) en el Go; nada lo parsea.
- **Pool de medios**: el Go servía las descargas con un pool acotado; el
  loop del Rust serializa los RPCs mientras descarga.
- **FTS5**: el puerto busca con trigramas `LIKE` + scoring, no con la tabla
  virtual del Go. Encuentra typos y captions, pero no ignora acentos y
  hace scan completo.
- **Adjuntos de un `chats.db` heredado**: el Go guarda video/audio/documento
  en `image_*`; `wire.rs` y `media.rs` ya caen ahí, pero el resto del
  esquema del puerto escribe `media_*`.
- **Identidad de remitente en grupos**: `sender_name` y `sender_phone_number`
  salen vacíos en `wire.rs`; la UI los resuelve por su lado.
- **Cobertura tras historial bajo demanda**: si el historial llega, los
  mensajes se guardan pero la fila de cobertura sigue en `requesting`
  hasta que la expiración de 30 s la pasa a `more_available` (creció el
  conteo) o a `failed` con reintento. Ningún camino la deja colgada.

## Cómo verificar (la batería, en orden)

```sh
cargo fmt --all --check                       # 0 diferencias
cargo clippy -p whatfast-core -p whatfast-protocol -p whatfast-model \
             -p whatfast-i18n -p whatfast-wa --all-targets -- -D warnings
cargo clippy -p whatfast-ui --all-targets -- -D warnings   # 0 warnings
cargo test --workspace                        # todo en verde
python scripts/parity.py                      # oráculo del frontend: exit 0
python scripts/funcionalidad.py               # RPC y componentes: exit 0
python scripts/paridad_sidecar.py             # gate: 50/50, 0 sin declarar, exit 0
```

La paridad del perfil real se valida aparte, sobre una **copia** (nunca el
perfil vivo):

```sh
WHATFAST_WA_PROFILE=<copia> cargo test -p whatfast-wa \
  --test identity_reconcile the_real_profile_folds -- --ignored --nocapture
```

## Última corrida verificada

- `cargo fmt --all --check`: 0 diferencias.
- clippy de los cinco crates de lógica y de la UI: 0 warnings.
- `cargo test --workspace`: 685 tests pasados, 0 fallidos.
- `python scripts/parity.py`: 123/123 casos, exit 0.
- `python scripts/funcionalidad.py`: exit 0.
- `python scripts/paridad_sidecar.py`: 50/50 reales, 0 pendientes declarados,
  0 sin declarar, 6 extra del puerto, gate OK, exit 0.
- Perfil real (copia de 28,8 MB de `chats.db` y 11,3 MB de `session.db`):
  35.415 mensajes antes y después, 470 redirects coherentes con el mapa,
  0 folds pendientes, segunda pasada con 0 eventos.

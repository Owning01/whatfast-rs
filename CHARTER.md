# CHARTER — migración de Whatfast a Rust nativo sin webview

> Fase F0 de `repo-migrate`. **Firma pendiente del humano.**
> Las fases F1 a F3 están en `docs/`; el plan completo está en `PLAN.md`.

## Objetivo (una frase, medible)

**Que `whatfast-rs` tenga la interfaz 100 % Rust —sin webview, sin SolidJS,
sin Electron, sin Chromium— que arranque la ventana en menos de 0,5 s y ocupe
menos de 120 MB de RAM en reposo (working set del árbol de procesos completo,
medido con el sidecar real y 10.000 chats cargados), con paridad
comportamental del 100 % de la suite actual de 153 tests y de los 64 comandos
de la API del bridge.**

> **Excepción adoptada el 20/10/2026 (ver "Decisión D1" más abajo):** el
> protocolo de WhatsApp **sigue en Go** (sidecar `whatsmeow`). La interfaz, el
> núcleo, la lógica y el transporte son Rust. La razón es que la librería
> Rust del protocolo no puede reusar una sesión ya emparejada, y obligar a
> los usuarios a volver a escanear el QR es una regresión de producto que no
> compra RAM: el sidecar ocupa 31,8 MB de los 120 MB de la meta, y el 85 % de
> la RAM que se quiere eliminar es el WebView2, que sí se elimina entero.

## Baseline medido (origen, 20/10/2026, esta máquina)

| Métrica | Origen (`whatfast`, Tauri + SolidJS + WebView2) |
|---|---|
| Ventana visible tras lanzar | 0,25 s |
| Lista de chats lista | 1,16 s (build de dev, Chromium headless) |
| RAM del árbol completo | **432 MB** (núcleo Rust 25,6 + sidecar Go 31,8 + WebView2 366,8) |
| RAM del código propio | 57,4 MB |
| JS de la UI | 411 KB (127 KB gzip) |
| Descarga para instalar | 33,1 MB |
| Motor de render | WebView2 (Chromium del sistema, 6 procesos) |

**El 85 % de la RAM es el motor del navegador.** Ese es el objetivo real del
trabajo: el WebView2 son 6 procesos y 367 MB que existen solo para pintar
nuestro DOM.

## Qué NO se migra

1. **El protocolo de WhatsApp NO se migra.** Decidido con evidencia en el
   spike B-R1 (`docs/protocol-spike.md`, 20/10/2026), verificado de forma
   independiente:
   - `whatsapp-rust` 0.7.0 (MIT, 928 estrellas, 146.955 descargas) cubre la
     capa de WhatsApp: Noise, Signal propio, app-state, newsletters,
     multimedia, polls.
   - **Pero su esquema SQLite es incompatible** (`device`, `identities`,
     `sessions`, `prekeys`) con el de `whatsmeow` (`whatsmeow_device`,
     `whatsmeow_session`), y su store (423 KB de `sqlite_store.rs`) tiene
     **0 menciones de `whatsmeow`**: no hay importador. Un usuario ya
     emparejado tendría que volver a escanear el QR.
   - `wa-rs` 0.2.0 está parado desde el 17/02/2026, con 8 estrellas y un
     solo mantenedor: no es una base sobre la que construir.
   - Reescribir el sidecar son 13.526 líneas de Go que no mueven la meta de
     RAM y sí rompen el requisito más duro (no perder la sesión).
   - **Consecuencia**: el sidecar Go se queda. La app es un binario Rust que
     habla con un sidecar Go por protobuf sobre stdio, igual que hoy, pero
     **sin webview**. Esto sigue siendo un cliente de escritorio nativo, no
     una página web empaquetada.
2. **El mock de navegador** (`tauri/src/lib/browser-mock.ts`, 1.411 líneas): es
   un artefacto de desarrollo web, sin equivalente ni sentido en una app
   nativa. Se reemplaza por un backend fake en Rust.
3. **El CSS.** Las 4.439 líneas de CSS se reinterpretan como un sistema de
   tokens de GPUI. No hay equivalencia lineal.
4. **No se tocan los tests de paridad del origen.** Los 153 tests TS y los 64
   tests Rust de `desktop/` quedan congelados en `whatfast` como oráculo.
5. **No se migra `tauri/src-tauri/`** tal cual: el núcleo Rust se reutiliza
   (staging, editor de imagen, rutas, permisos), pero se reescribe como
   crate de `whatfast-rs` con crates propios, no como un bloque Tauri.

## Orden de magnitud

| Magnitud | Cantidad |
|---|---|
| Origen a migrar | ~25.600 líneas (21.192 TS/TSX + 4.439 CSS) + 10.187 líneas Rust ya escritas en `desktop/` |
| Tests a preservar | 153 TS + 64 Rust, de los cuales 24 suites tienen o no equivalente |
| Componentes de UI a portar | 21 (7 con equivalente parcial, 11 inexistentes) |
| Comandos de la API del bridge | 64 (el crate GPUI hoy cubre 22) |
| Esfuerzo | ~14 semanas con 1 persona a tiempo completo + 1 líder revisando los gates (ancla del skill: 800K líneas ≈ 14 semanas) |

## Restricciones y riesgos aceptados por escrito

| Riesgo | Mitigación |
|---|---|
| GPUI arrastra el repositorio de Zed por git: compila lento y pesado | el crate de lógica pura (`whatfast-core`) no depende de GPUI; el loop de migración rápido corre ahí |
| `gpui` cambia de API entre commits | se **pinea** el rev (`031555662e99a1b5a549990b47f246d475b8288a`) y el pin es parte del rulebook |
| No hay librería Rust de protocolo maduro y probada | spike en F4 con el criterio de gate escrito: si no pasa, el sidecar Go se queda |
| La RAM de GPUI (Skia) puede comerse parte del ahorro | la meta de 120 MB es un gate de F8 medido, no una promesa; si no se cumple, se publica el número real y se documenta el gap |
| Un store reactivo de 113 acciones no tiene traducción directa | `state/app.ts` se rediseña (no se porta): `Entity<Store>` + `cx.notify()`, con las acciones repartidas en submódulos |

## Decisiones tomadas (con su evidencia)

| # | Decisión | Evidencia | Fecha |
|---|---|---|---|
| **D1** | **El sidecar Go se queda.** El objetivo es "interfaz 100 % Rust sin webview", no "100 % Rust". | Spike B-R1: `whatsapp-rust` no puede reusar una sesión emparejada (esquema incompatible, sin importador) y `wa-rs` está parado | 20/10/2026 |
| **D2** | **Los usuarios ya emparejados no pierden su sesión.** No se acepta re-emparejamiento. | Consecuencia directa de D1 | 20/10/2026 |
| **D3** | **No se abre el lote de reescritura del sidecar** (13.526 líneas de Go, 4-6 semanas). | No mueve la meta de RAM (31,8 MB de 120 MB) y rompe D2 | 20/10/2026 |
| **D4** | **El transporte sigue siendo protobuf sobre stdio** entre el binario Rust y el sidecar Go. | Ya existe, ya está versionado (v21) y falla cerrado; un puente nuevo no aporta | 20/10/2026 |
| **D5** | **El spike de 2 días de `whatsapp-rust`** (emparejar en un crate aparte, medir RSS, probar si el `SessionRecord` de libsignal-go se decodifica) queda **para atrás**. | El gate de D3 no se cumple si D1 no se cumple | 20/10/2026 |

## Gates (resumen)

- **F0**: este charter firmado. → *pendiente de firma*
- **F1**: `docs/dependency-map.md` + `docs/dependency-map.json` generado por script, con citas `file:line`.
- **F2**: suite de paridad congelada + juez validado (pasa contra el origen, falla contra una rotura deliberada).
- **F3**: `RULEBOOK.md` + `docs/type-map.md` + `docs/api-mapping.md` firmados.
- **F4**: puente montado (el crate `whatfast-protocol` + el transporte sobre stdio) + baseline medido + **un componente real de destino corriendo detrás del puente**.
- **F5**: slice duro (el parser de formato) verde + defectos del rulebook corregidos.
- **F6**: traducción paralela por lotes, main siempre verde.
- **F7**: repair + paridad (L1 sintaxis, L2 compilación/lints, L3 paridad) + mutation ≥ 80 %.
- **F8**: cutover por componente, del borde al núcleo; cero líneas de SolidJS; números antes/después publicados.

## Cómo se decide lo dudoso

- **El humano** (Octavio) es el control plane: firma charter, rulebook y gates caros, y decide los cortes.
- **Los agentes** no inventan semántica: si el TypeScript no dice qué hacer, se escribe un test de caracterización en el origen primero, y se anota la decisión en `RULEBOOK.md` (sección "Decisiones pendientes").
- **Nada de stubs.** Un agente que deja un `todo!()`, un `unimplemented!()` o un `unwrap()` para que compile es rechazado.

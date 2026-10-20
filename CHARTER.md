# CHARTER — migración de Whatfast a Rust nativo sin webview

> Fase F0 de `repo-migrate`. **Firma pendiente del humano.**
> Las fases F1 a F3 están en `docs/`; el plan completo está en `PLAN.md`.

## Objetivo (una frase, medible)

**Que `whatfast-rs` sea un cliente de escritorio de WhatsApp 100 % Rust —sin
webview, sin SolidJS, sin Electron— que arranque la ventana en menos de 0,5 s
y ocupe menos de 120 MB de RAM en reposo (working set del árbol de procesos
completo, medido con el sidecar real y 10.000 chats cargados), con paridad
comportamental del 100 % de la suite actual de 153 tests y de los 64 comandos
de la API del bridge.**

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

1. **El protocolo de WhatsApp no se reescribe a mano.** Se evalúa
   `wa-rs` / `whatsapp-rust` (crates Rust existentes, 50K SLoC, MIT) contra
   los 50 RPC de `proto/bridge.proto` en F1/F4. Si ninguno cubre lo que
   necesita el cliente, el sidecar Go (`whatsmeow`) **se queda** y el trabajo
   termina en "UI en Rust, protocolo en Go". Eso es un outcome válido, no un
   fracaso: el ahorro de RAM (el WebView2) ya estáLogging hecho.
2. **No se migra el mock de navegador** (`tauri/src/lib/browser-mock.ts`,
   1.411 líneas): es un artefacto de desarrollo web, sin equivalente ni
   sentido en una app nativa. Se reemplaza por un backend fake en Rust.
3. **No se migra el CSS.** Las 4.439 líneas de CSS se reinterpretan como un
   sistema de tokens de GPUI. No hay equivalencia lineal.
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

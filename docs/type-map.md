# TYPE MAP — par TypeScript/SolidJS → Rust

> F3 de `repo-migrate`. Plantilla general en
> `C:\Users\perca\.agents\backupo\repo-migrate\references\type-map.md`,
> instanciada para este par. **Firma pendiente del humano.**

## Los 5 ejes (resueltos, con el detalle en `RULEBOOK.md` §1)

| Eje | Origen | Destino | Decisión |
| :--- | :--- | :--- | :--- |
| **Números** | `number` (siempre `f64` en JS) | `i64` / `u32` / `f64` según el dominio | El dominio decide el tipo; si el origen puede ver `NaN`/`Infinity`, va `f64` o `Option<f64>`. `Math.round` no equivale a `f64::round` en empates negativos |
| **Nulabilidad** | `T \| undefined`, `T \| null` | `Option<T>` | `null` y `undefined` se unifican en `None` salvo que un test demuestre que se distinguen. Una clave presente con `null` **se conserva** |
| **Errores** | `throw` / `return null` / excepción implícita | `Result<T, E>` (`thiserror`) / `Option<T>` | `unwrap`, `expect`, `panic!` prohibidos fuera de tests. `anyhow` solo en binarios |
| **Concurrencia** | un solo hilo, event loop del navegador, `async/await`, `setTimeout` | `whatfast-core`: síncrono y puro. Capas con efecto: `async` explícito | El origen con timers se divide: decisión pura aparte, efecto aparte. El instante se inyecta como parámetro |
| **Memoria** | GC, objetos mutables, proxies de Solid | ownership de Rust | `String`/`Vec` por defecto; sin `Rc<RefCell>`; **sin lifetimes** en `whatfast-core` |

## Toolchain

| | Origen | Destino |
|---|---|---|
| Build | `pnpm build` (Vite 8) | `cargo build --workspace` |
| Package manager | pnpm 10.28.2 | cargo, `Cargo.lock` versionado |
| Test | `node --test tests/*.test.ts` | `cargo test --workspace` (wrapper `scripts/test.cmd`) |
| Lint | `tsc --noEmit` | `cargo clippy --all-targets --locked -- -D warnings` (wrapper `scripts/lint.cmd`) |
| Formatter | prettier | `cargo fmt` |
| Watch | `vite` | `cargo watch` (opcional) |
| fast-check | — | `cargo test`; `proptest` solo si el módulo lo pide (hoy ninguno) |

## Dependencias: una fila por reemplazo

| Origen + versión | Destino + versión pineada | Diferencia de errores | Diferencia de defaults | Bucket |
|---|---|---|---|---|
| `solid-js` 1.9.14 | GPUI (`gpui` rev `031555662e99a1b5a549990b47f246d475b8288a`) | Solid: error en tiempo de render; GPUI: `panic` en el hilo de render | Solid re-renderiza por granularidad; GPUI hay que llamar `cx.notify()` | reescritura |
| `@tanstack/solid-virtual` 3.13.34 | `gpui` `v_virtual_list` / `uniform_list` | Solid: devuelve items; GPUI: callback de render | ventanas de overscan distintas | reemplazo |
| `qrcode` 1.5.4 (JS) | `qrcode` 0.14 (Rust) | JS: lanza; Rust: `Result` | mismo contenido | reemplazo |
| `lucide-solid` 1.25.0 | `gpui-component` `IconName` (rev pineado) | n/a | el set de nombres es distinto: hay que mapear 38 iconos | reemplazo |
| (catálogo de emoji propio, 469 l.) | `emojis` 0.9.0 | n/a | `emojis` incluye shortcodes: cubre `emoji-autocomplete` | reemplazo |
| `@tauri-apps/api` 2.11.1 | crate `whatfast-protocol` (propio) | Tauri: `invoke` serializa; el crate nuevo: RPC Rust directo | — | reescritura |
| `@tauri-apps/plugin-dialog` 2.7.2 | `rfd` / `gpui-component` file dialog | — | — | reescritura |
| `@tauri-apps/plugin-notification` 2.3.3 | `notify-rust` 4.18 (ya en el crate Tauri) | — | — | reescritura |
| `@tauri-apps/plugin-clipboard-manager` 2.3.2 | `gpui-component` clipboard | — | — | reescritura |
| `@tauri-apps/plugin-opener` 2.5.4 | **eliminable**: no se usa | — | — | eliminable |
| (CSS, 4.439 l.) | tokens de GPUI | — | — | reescritura |
| `localStorage` | archivo JSON en el perfil | — | — | reescritura |
| `lib/browser-mock.ts` (1.411) | backend fake en Rust | — | — | eliminable |
| `lib/types.ts` (490) | **ya existe**: `proto/bridge.proto` | — | — | directa |

## Intraducibles (se rediseñan, no se emulan)

| Intraducible | Qué era | Qué pasa | Ticket |
|---|---|---|---|
| Store reactivo de Solid (`createStore`, `batch`, `reconcile`) | `state/app.ts`, 3.023 l., ~113 acciones | Rediseño a `Entity<Store>` + `cx.notify()`, con las acciones repartidas en submódulos por dominio | U2 |
| `createEffect` / `createMemo` | derivaciones reactivas | Cálculo bajo demanda en el render, o caché invalidada a mano | U2 |
| Proxies de Solid (lectura/escritura granular) | performance del store | GPUI notifica de a todo el árbol: hay que acotar los invalidadores | U2 |
| `document.documentElement.style.setProperty` | `lib/theme.ts` (292) | Sistema de tokens propio de GPUI | U1 |
| `localStorage` | preferencias, recientes, snapshot del workspace | Archivo JSON en el perfil, con escritura atómica | U1 |
| `window.addEventListener("keydown")` | atajos globales (`App.tsx`) | `FocusHandle` + `on_key_down` de GPUI | U2 |
| `<canvas>` para el QR | `Screens.tsx` | Render del QR como imagen en GPUI | U4 |
| `getCurrentWebview().onDragDropEvent` | drag & drop de archivos | `on_drop` nativo de GPUI | U5 |
| `convertFileSrc` / protocolo `asset:` | carga de medios en el render | Rutas de archivo locales servidas por el crate UI | U3 |
| `import.meta.env.DEV` | mock de navegador | crate de features (`debug`) | R1 |
| Timers (`setTimeout`, `setInterval`) | lifecycle, notificaciones, scroll | `async-channel` + un runtime explícito | U2 |

## Criterio de "no traducible"

Un constructo es intraducible cuando su semántica depende del DOM, del
ciclo de vida del navegador o de la reactividad de Solid. No se emula con un
`std::cell::RefCell` ni con un `Rc<RefCell<dyn Fn>>`: se rediseña.

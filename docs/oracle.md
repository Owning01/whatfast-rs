# ORACLE — la fuente de verdad de la paridad

## Qué es el juez

El juez **es el repo de origen**, `Owning01/whatfast`, congelado. No se
modifica ni un test. Los agentes de migración nunca lo editan: si un test
del origen está mal, se anota y se arregla en un PR aparte, después.

## Los cuatro物权

| Pieza | Ubicación | Qué fija |
|---|---|---|
| 153 tests TS | `whatfast/tauri/tests/` | el comportamiento de la lógica de producto |
| 64 tests Rust | `whatfast/desktop/src/` | el modelo de datos, el transporte y el ciclo de vida de los RPC |
| 50 RPC + 51 respuestas | `whatfast/proto/bridge.proto` | el contrato de la API |
| 9 paquetes de test Go | `whatfast/backend/` | el comportamiento del sidecar |

## Cómo se declara la paridad de un módulo

Para cada módulo TS `X.ts` con suite `X.test.ts`:

1. El puerto en Rust tiene un test con **los mismos casos**, con el nombre
   traducido (`snake_case`).
2. El juez compara la **lista de nombres de caso**, no el texto de las
   aserciones (el lenguaje cambia).
3. La paridad del módulo es `casos verdes / casos del oráculo`.

Un módulo está *done* cuando: paridad 100 %, cero `unwrap`, clippy limpio,
`cargo fmt` aplicado, y `git diff` limitado a la partición del lote.

## Qué NO se puede usar como juez

- El mock de navegador (`tauri/src/lib/browser-mock.ts`): es un artefacto de
  desarrollo web y sus datos no son comportamiento de producto.
- El CSS: no hay judge de estilos; la paridad visual se revisa a ojo con las
  capturas de referencia de `whatfast/docs/screenshots/`.
- La app de WhatsApp real: la paridad se mide contra el oráculo, no contra
  el producto de Meta.

## Validación del juez (gate de F2)

Antes de escribir la primera línea de destino:

1. El juez corre contra el origen → **verde**.
2. Se rompe el origen a propósito (un `>=` por `>` en un módulo de lógica
   pura) → el juez tiene que **fallar**. Si no falla, hay que reforzar el
   juez primero.

## LaFrozen lista de casos por módulo (a completar en F2)

El mapeo exacto de suites del origen a módulos del destino vive en
`docs/dependency-map.md`. La lista de casos se extrae de los archivos
`tauri/tests/*.test.ts` con un script, no a mano.

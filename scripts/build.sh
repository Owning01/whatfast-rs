#!/usr/bin/env bash
# ============================================================================
#  Whatfast - build de distribucion para Linux y macOS
#
#  Hermano de scripts/build.cmd. Deja en dist/ los dos ejecutables que forman
#  la aplicacion:
#    whatfast            la interfaz nativa (GPUI, Rust)
#    whatfast-backend    el sidecar en Go (la sesion de WhatsApp)
#
#  El sidecar va al lado porque la interfaz lo busca ahi cuando no esta
#  definida la variable WHATFAST_BACKEND.
#
#  Se puede correr desde cualquier carpeta:
#    scripts/build.sh
#    /ruta/al/repo/scripts/build.sh
#
#  Variables que respeta:
#    CARGO_HOME         lo usa cargo tal cual (no se toca).
#    CARGO_TARGET_DIR   donde deja los artefactos cargo. Si no esta, cargo
#                       usa <repo>/target.
#    WHATFAST_TARGET_DIR  alias de CARGO_TARGET_DIR, igual que en build.cmd.
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ -z "${CARGO_TARGET_DIR:-}" ] && [ -n "${WHATFAST_TARGET_DIR:-}" ]; then
    export CARGO_TARGET_DIR="$WHATFAST_TARGET_DIR"
fi

# cargo resuelve un CARGO_TARGET_DIR relativo contra la carpeta actual, que
# aca ya es la raiz del repo: se hace lo mismo para poder leer el artefacto.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    mkdir -p "$CARGO_TARGET_DIR"
    TARGET_DIR="$(cd "$CARGO_TARGET_DIR" && pwd)"
else
    TARGET_DIR="$ROOT/target"
fi
export CARGO_TARGET_DIR="$TARGET_DIR"

for tool in cargo go; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "FALLO: falta $tool en el PATH" >&2
        echo "       cargo: https://rustup.rs   go: https://go.dev/dl/" >&2
        exit 1
    fi
done

echo "[1/4] compilando el sidecar en Go ..."
mkdir -p backend/bin
# CGO_ENABLED=0: el sidecar no usa nada nativo (la base es modernc.org/sqlite,
# que es Go puro). Asi el binario sale estatico y no depende de libc.
(
    cd backend
    CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o bin/whatfast-backend ./cmd/whatfast-backend
)

echo "[2/4] compilando la interfaz en modo release ..."
cargo build --release -p whatfast-ui --config "build.rustc-wrapper=''"

echo "[3/4] armando dist/ ..."
mkdir -p dist
cp -f "$TARGET_DIR/release/whatfast-ui" dist/whatfast
cp -f backend/bin/whatfast-backend dist/whatfast-backend
chmod +x dist/whatfast dist/whatfast-backend

echo "[4/4] copiando los iconos ..."
mkdir -p dist/icons
for icon in 32x32.png 64x64.png 128x128.png 128x128@2x.png icon.png; do
    if [ -f "assets/icons/$icon" ]; then
        cp -f "assets/icons/$icon" "dist/icons/$icon"
    fi
done

# La entrada de escritorio solo sirve en Linux: en macOS el paquete es un
# Whatfast.app y el lanzador lo arma el workflow del release.
if [ "$(uname -s)" = "Linux" ] && [ -f assets/linux/whatfast.desktop ]; then
    cp -f assets/linux/whatfast.desktop dist/whatfast.desktop
fi

echo
echo "listo. dist/ :"
ls -l dist
echo
echo "los dos ejecutables tienen que quedar en la misma carpeta:"
echo "  dist/whatfast            la interfaz"
echo "  dist/whatfast-backend    el sidecar"

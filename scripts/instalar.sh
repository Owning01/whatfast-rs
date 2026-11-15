#!/usr/bin/env bash
# ============================================================================
#  Whatfast - instalador local para Linux (usuario actual, sin root)
#
#  Hermano de scripts/instalar.ps1: instala dist/whatfast y
#  dist/whatfast-backend en ~/.local/bin, los iconos en hicolor y la entrada
#  de escritorio en ~/.local/share/applications/whatfast.desktop.
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS_DIR="$DATA_HOME/applications"
ICONS_DIR="$DATA_HOME/icons/hicolor"

if [ ! -x "$ROOT/dist/whatfast" ] || [ ! -x "$ROOT/dist/whatfast-backend" ]; then
    echo "Faltan binarios en dist/. Ejecutando scripts/build.sh..."
    "$ROOT/scripts/build.sh"
fi

mkdir -p "$BIN_DIR" "$APPS_DIR"
cp -f "$ROOT/dist/whatfast" "$BIN_DIR/whatfast"
cp -f "$ROOT/dist/whatfast-backend" "$BIN_DIR/whatfast-backend"
chmod +x "$BIN_DIR/whatfast" "$BIN_DIR/whatfast-backend"

install_icon() {
    local size="$1"
    local src="$2"
    if [ -f "$src" ]; then
        mkdir -p "$ICONS_DIR/${size}/apps"
        cp -f "$src" "$ICONS_DIR/${size}/apps/whatfast.png"
    fi
}

install_icon "32x32" "$ROOT/assets/icons/32x32.png"
install_icon "64x64" "$ROOT/assets/icons/64x64.png"
install_icon "128x128" "$ROOT/assets/icons/128x128.png"
install_icon "256x256" "$ROOT/assets/icons/icon.png"

if [ -f "$ROOT/assets/linux/whatfast.desktop" ]; then
    sed "s|^Exec=.*|Exec=$BIN_DIR/whatfast|;s|^TryExec=.*|TryExec=$BIN_DIR/whatfast|" \
        "$ROOT/assets/linux/whatfast.desktop" > "$APPS_DIR/whatfast.desktop"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$ICONS_DIR" >/dev/null 2>&1 || true
fi

echo "Instalado en $BIN_DIR/whatfast y registrado en $APPS_DIR/whatfast.desktop"

"""Genera el juego de iconos de Whatfast a partir del logo maestro.

El logo maestro es `assets/icons/icon.png` (512x512, el mismo del cliente
web). De ahí salen:

- los PNG que usa el instalador y el paquete (`32x32.png`, `64x64.png`,
  `128x128.png`, `128x128@2x.png`),
- el `.ico` de Windows con los tamaños que la barra de tareas y el
  Explorador piden (de 16 a 256),
- el `.icns` de macOS (el que ya estaba, que no se regenera).

Uso:
    python scripts\\iconos.py
"""

from __future__ import annotations

import pathlib
import sys

from PIL import Image

RAIZ = pathlib.Path(__file__).resolve().parent.parent
ICONOS = RAIZ / "assets" / "icons"
MAESTRO = ICONOS / "icon.png"

# Los tamaños del .ico: 16 y 24 para la barra de tareas, 32 para el
# Explorador, 48 para el "icono mediano" y los grandes para el alt-tab.
TAMANOS_ICO = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]

# Los PNG sueltos que el instalador y el paquete de distribucion usan.
PNG = {
    "32x32.png": 32,
    "64x64.png": 64,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
}


def principal() -> int:
    if not MAESTRO.exists():
        print(f"  no existe el logo maestro {MAESTRO}")
        return 1

    with Image.open(MAESTRO) as maestro:
        maestro = maestro.convert("RGBA")
        print(f"  el logo maestro mide {maestro.width}x{maestro.height}")

        for nombre, lado in PNG.items():
            resized = maestro.resize((lado, lado), Image.LANCZOS)
            resized.save(ICONOS / nombre, "PNG", optimize=True)
            print(f"  {nombre:<16} {lado}x{lado}")

        ico = maestro.save(ICONOS / "icon.ico", sizes=[(s, s) for s in TAMANOS_ICO])
        del ico
    return 0


if __name__ == "__main__":
    sys.exit(principal())

"""Dice si la vista de los estados esta abierta en una captura.

La vista de los chats tiene el chip verde del filtro activo; la de los
estados no tiene filtros. El detector mira el verde de esa fila.

Uso:
    python detectar-estados.py CAPTURA.png
"""
import sys

import numpy as np
from PIL import Image


def main() -> None:
    ruta = sys.argv[1]
    imagen = np.asarray(Image.open(ruta).convert("RGB"), dtype=np.int16)
    alto, ancho = imagen.shape[:2]
    # La fila de filtros: el chip activo es verde.
    y0, y1 = int(alto * 0.10), int(alto * 0.18)
    x0, x1 = int(ancho * 0.06), int(ancho * 0.22)
    caja = imagen[y0:y1, x0:x1]
    r, g, b = caja[:, :, 0], caja[:, :, 1], caja[:, :, 2]
    verdes = ((g > 120) & (g - r > 40) & (g - b > 20)).sum()
    # El veredicto va primero: el guion de captura lee la primera linea.
    print("1" if verdes < 40 else "0")
    print(f"pixeles verdes de los filtros: {verdes}")


if __name__ == "__main__":
    main()

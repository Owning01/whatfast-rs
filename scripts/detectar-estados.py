"""Dice si la vista de los estados esta abierta en una captura.

La vista de los estados no tiene el titulo "Chats" en la cabecera de la
columna: si esa caja de pixeles esta vacia, la vista de los estados esta
enfrente.

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
    caja = imagen[12:48, 60 : min(300, ancho)]
    brillo = int((caja.sum(axis=2) > 240).sum())
    # Solo el veredicto: el guion de captura lee la ultima linea.
    print("1" if brillo == 0 else "0")


if __name__ == "__main__":
    main()

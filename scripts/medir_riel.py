"""Mide en una captura donde esta el boton de los estados del riel.

El boton es el tercero de la columna de la izquierda. La foto puede estar
escalada por la escala de pixels de la pantalla, asi que la medida se
devuelve en pixeles de la foto y el guion de captura la convierte.

Uso:
    python medir_riel.py CAPTURA.png
"""
import sys
from collections import Counter

import numpy as np
from PIL import Image

# La columna de la izquierda: el riel de los iconos. La primera columna es
# el borde de la ventana y la ultima el separador con la columna de chats:
# las dos se dejan fuera porque difieren del fondo en todas las filas.
X_INICIO = 2
X_FIN = 70


def fondo_de(franja: np.ndarray) -> np.ndarray:
    """El color mas comun de la franja: el fondo del riel."""
    pixeles = [tuple(pixel) for pixel in franja.reshape(-1, 3)]
    return np.array(Counter(pixeles).most_common(1)[0][0], dtype=np.int16)


def main() -> None:
    ruta = sys.argv[1]
    imagen = np.asarray(Image.open(ruta).convert("RGB"), dtype=np.int16)
    alto, ancho = imagen.shape[:2]
    riel = imagen[:, X_INICIO : min(X_FIN, ancho)]
    fondo = fondo_de(riel)

    # Un icono es un pixel que se aparta del fondo del riel. Las columnas
    # que difieren del fondo en casi todas las filas son el separador con la
    # columna de chats: no son iconos y se descartan.
    diferencia = np.abs(riel - fondo).sum(axis=2)
    columnas_vivas = (diferencia > 40).sum(axis=0) < alto * 0.5
    filas = np.flatnonzero((diferencia[:, columnas_vivas] > 40).any(axis=1))

    franjas: list[list[int]] = []
    for fila in filas:
        if franjas and fila - franjas[-1][-1] <= 3:
            franjas[-1].append(int(fila))
        else:
            franjas.append([int(fila)])

    print(f"--- bandas del riel en {ruta} ({ancho}x{alto}) ---")
    for franja in franjas:
        columnas = np.flatnonzero(
            (diferencia[franja[0] : franja[-1] + 1] > 40).any(axis=0) & columnas_vivas
        )
        if columnas.size == 0:
            continue
        print(
            f"y {franja[0]:4d}-{franja[-1]:4d} centro {(franja[0] + franja[-1]) // 2:4d} "
            f"x {columnas.min() + X_INICIO:3d}-{columnas.max() + X_INICIO:3d}"
        )

    # El boton de los estados es el tercero del grupo de arriba. El borde de
    # la ventana y los botones de abajo (la privacidad y los ajustes) se
    # descartan.
    iconos = [
        franja
        for franja in franjas
        if len(franja) >= 12 and franja[0] > 8 and franja[-1] < alto // 2
    ]
    if len(iconos) >= 3:
        franja = iconos[2]
        columnas = np.flatnonzero(
            (diferencia[franja[0] : franja[-1] + 1] > 40).any(axis=0) & columnas_vivas
        )
        x = int((columnas.min() + columnas.max()) // 2) + X_INICIO
        print(f"posicion {x} {(franja[0] + franja[-1]) // 2}")
    else:
        print("posicion 0 0")


if __name__ == "__main__":
    main()

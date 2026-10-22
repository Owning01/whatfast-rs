"""Busca secretos en los archivos que van a entrar al repositorio.

El repositorio es publico: una clave commiteada queda en el historial para
siempre. Este guion corre en el hook de pre-commit y tambien a mano:

    python scripts/chequear-secretos.py            # todo lo versionado
    python scripts/chequear-secretos.py --staged   # solo lo preparado

Sale con 1 si encuentra algo. No imprime el valor encontrado completo: deja
las primeras letras y tapa el resto, que alcanza para ubicarlo sin copiarlo.
"""

import io
import os
import re
import subprocess
import sys

# Formatos de clave conocidos por su prefijo.
PATRONES = [
    ("clave privada", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
    ("AWS access key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("GitHub token", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}")),
    ("Google API key", re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")),
    ("OpenAI key", re.compile(r"\bsk-[A-Za-z0-9]{20,}")),
    ("Slack token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{10,}")),
    ("Stripe key", re.compile(r"\b(sk|rk)_(live|test)_[A-Za-z0-9]{10,}")),
    ("JWT", re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}")),
]

# Asignaciones del tipo `password = "algo largo"`. Se pide una palabra clave
# pegada a un signo igual y un valor de 12 caracteres o mas, para no marcar
# `password: false` ni un texto de ayuda.
ASIGNACION = re.compile(
    r"(?i)\b(api[_-]?key|apikey|secret|password|passwd|token|access[_-]?key|"
    r"client[_-]?secret|auth[_-]?token)\b\s*[:=]\s*[\"']([^\"'\s]{12,})[\"']"
)

# Lo que no tiene sentido escanear: binarios y archivos generados.
SALTAR_EXT = {
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".icns", ".svg",
    ".zip", ".gz", ".tar", ".exe", ".dll", ".so", ".dylib", ".pdf",
    ".woff", ".woff2", ".ttf", ".db", ".lock",
}
SALTAR_RUTA = ("Cargo.lock", "package-lock.json", "backend/go.sum")


def archivos_versionados():
    salida = subprocess.run(
        ["git", "ls-files"], capture_output=True, text=True, check=True
    ).stdout
    return [linea for linea in salida.splitlines() if linea.strip()]


def archivos_preparados():
    salida = subprocess.run(
        ["git", "diff", "--cached", "--name-only", "--diff-filter=ACM"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [linea for linea in salida.splitlines() if linea.strip()]


def tapar(valor):
    if len(valor) <= 8:
        return "*" * len(valor)
    return valor[:4] + "*" * (len(valor) - 4)


def revisar(ruta):
    bajo = ruta.replace("\\", "/")
    if os.path.splitext(bajo)[1].lower() in SALTAR_EXT:
        return []
    if any(bajo.endswith(nombre) for nombre in SALTAR_RUTA):
        return []
    try:
        texto = io.open(ruta, encoding="utf-8", errors="strict").read()
    except (OSError, UnicodeDecodeError):
        return []

    hallazgos = []
    for numero, linea in enumerate(texto.splitlines(), start=1):
        for nombre, patron in PATRONES:
            encontrado = patron.search(linea)
            if encontrado:
                hallazgos.append((ruta, numero, nombre, tapar(encontrado.group(0))))
                continue
        asignacion = ASIGNACION.search(linea)
        if asignacion:
            hallazgos.append(
                (ruta, numero, "asignacion con valor largo", tapar(asignacion.group(2)))
            )
    return hallazgos


def main():
    solo_preparado = "--staged" in sys.argv[1:]
    rutas = archivos_preparados() if solo_preparado else archivos_versionados()

    hallazgos = []
    for ruta in rutas:
        hallazgos.extend(revisar(ruta))

    if not hallazgos:
        alcance = "lo preparado" if solo_preparado else len(rutas)
        if solo_preparado:
            print("secretos: nada en %s" % alcance)
        else:
            print("secretos: %s archivos revisados, nada" % alcance)
        return 0

    print("SECRETOS ENCONTRADOS: no commitees esto.")
    for ruta, numero, nombre, muestra in hallazgos:
        print("  %s:%s  %s  %s" % (ruta, numero, nombre, muestra))
    return 1


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Reporta las claves que usa la interfaz y las que le faltan a cada idioma.

La clave de un texto es su frase en ingles: `i18n::t("Type a message")`. Un
idioma nuevo es un archivo `crates/whatfast-i18n/locales/<codigo>.json` con su
`meta` y sus `strings`; el `build.rs` lo descubre solo. Este script dice que
claves faltan, para que traducir un idioma sea completar una lista.

Uso:
    python scripts/idiomas.py            # informe
    python scripts/idiomas.py --strict   # falla si falta alguna clave

Nota: las cadenas que llegan a `t` por una variable (las filas del menu de un
mensaje, por ejemplo) no se pueden leer del codigo. El informe cuenta solo las
llamadas literales, que son la mayoria.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UI = ROOT / "crates" / "whatfast-ui" / "src"
LOCALES = ROOT / "crates" / "whatfast-i18n" / "locales"

PATTERN = re.compile(r'i18n::t(?:_args)?\(\s*"((?:[^"\\]|\\.)*)"')


def unescape(text: str) -> str:
    """Decodifica los escapes de Rust que pueden aparecer en un texto."""
    out: list[str] = []
    i = 0
    while i < len(text):
        if text[i] != "\\":
            out.append(text[i])
            i += 1
            continue
        nxt = text[i + 1]
        if nxt == "u" and text[i + 2] == "{":
            end = text.index("}", i + 3)
            out.append(chr(int(text[i + 3 : end], 16)))
            i = end + 1
        elif nxt == "n":
            out.append("\n")
            i += 2
        else:
            out.append(nxt)
            i += 2
    return "".join(out)


def used_keys() -> set[str]:
    keys: set[str] = set()
    for path in sorted(UI.glob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="surrogateescape")
        for match in PATTERN.finditer(text):
            keys.add(unescape(match.group(1)))
    return keys


def load_locales() -> dict[str, tuple[str, set[str]]]:
    locales: dict[str, tuple[str, set[str]]] = {}
    for path in sorted(LOCALES.glob("*.json")):
        data = json.loads(path.read_text(encoding="utf-8"))
        meta = data.get("meta") or {}
        code = meta.get("code") or path.stem
        name = meta.get("name") or code
        locales[code] = (name, set((data.get("strings") or {}).keys()))
    return locales


def main() -> int:
    # The console of Windows is not UTF-8: the names of the languages would
    # crash the report.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    keys = used_keys()
    locales = load_locales()
    print(f"{len(keys)} claves en la interfaz, {len(locales)} idiomas")
    missing_any = False
    for code, (name, strings) in sorted(locales.items()):
        missing = sorted(keys - strings)
        covered = len(keys & strings)
        percentage = covered * 100 // max(len(keys), 1)
        print(f"{code:3} {name:12} {len(strings):4} textos, {percentage:3}% de la interfaz")
        if missing:
            missing_any = True
            for key in missing:
                print(f"    falta: {key}")
    if "--strict" in sys.argv and missing_any:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

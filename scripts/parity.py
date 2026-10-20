"""F2 — el juez de paridad.

Compara, módulo por módulo, los casos de test del oráculo (TypeScript) con
los casos de test del puerto (Rust). Solo compara NOMBRES de caso: el
lenguaje cambia, la semántica no.

    python scripts/parity.py [--origin RUTA] [--out RUTA] [--verbose]

El juez busca los casos del oráculo en TODOS los archivos de test del
destino (no solo en el archivo homónimo): un módulo puede tener su suite
dividida, y varias suites del oráculo pueden vivir en un solo archivo.

Salida: una tabla por módulo con `ok / faltan / sobran` y el total global.
Sale con código 1 si algún módulo está incompleto, para usarlo como gate.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

DEFAULT_ORIGIN = r"G:\Proyectos\16-whatsapp2"
DEFAULT_OUT = r"G:\Proyectos\17-whatfast-rs"

RE_TEST = re.compile(r"""(?:^|\s)(?:it|test)\s*\(\s*(['"`])(.+?)\1""", re.M)
RE_RUST_TEST = re.compile(
    r"#\[(?:tokio::)?test[^\]]*\]\s*(?:async\s+)?fn\s+([A-Za-z0-9_]+)")

STOP = {"a", "an", "the", "of", "to", "in", "is", "for", "and", "when", "with",
        "that", "it", "as", "on", "by", "be", "does", "not", "no", "s", "t"}


def snake(name: str) -> str:
    name = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name)
    name = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    name = re.sub(r"[^A-Za-z0-9]+", "_", name)
    return name.strip("_").lower()


def normalize(name: str) -> str:
    """Normaliza para comparar: minusculas, sin separadores, sin palabras
    vacias. El nombre del caso puede estar en ingles en el oraculo y en
    snake_case en el puerto."""
    tokens = [t for t in re.split(r"[^a-z0-9]+", snake(name)) if t]
    return " ".join(t for t in tokens if t not in STOP)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    origin = Path(args.origin)
    out = Path(args.out)
    tests_dir = out / "crates" / "whatfast-core" / "tests"

    # Todos los casos del destino, con el archivo donde viven.
    destino = {}   # nombre normalizado -> [casos rust...]
    archivo_de = {}  # nombre normalizado -> archivo
    for path in sorted(tests_dir.glob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for case in RE_RUST_TEST.findall(text):
            key = normalize(case)
            destino.setdefault(key, []).append(case)
            archivo_de.setdefault(key, path.name)

    rows = []
    total_oracle = 0
    total_ok = 0
    consumidos = set()

    for suite_path in sorted((origin / "tauri" / "tests").glob("*.test.ts")):
        module = suite_path.name[: -len(".test.ts")]
        text = suite_path.read_text(encoding="utf-8", errors="replace")
        oracle = [m[1] for m in RE_TEST.findall(text)]

        faltan = []
        for case in oracle:
            key = normalize(case)
            if destino.get(key):
                destino[key].pop(0)
                consumidos.add(key)
            else:
                faltan.append(case)

        ok = len(oracle) - len(faltan)
        total_oracle += len(oracle)
        total_ok += ok
        rows.append({
            "modulo": module,
            "oracle": len(oracle),
            "ok": ok,
            "faltan": faltan,
            "en": sorted({archivo_de[normalize(c)] for c in oracle
                          if normalize(c) in archivo_de}),
        })

    # Casos del destino que ninguna suite del oraculo reclama.
    # - Si el archivo tiene suite equivalente en el oraculo, sus casos sin
    #   reclamar son tests inventados: se reportan como "sobra".
    # - Si el archivo NO tiene suite en el oraculo, sus casos son tests de
    #   caracterizacion (el module no tenia cobertura): son legitimos.
    suites_del_oraculo = {r["modulo"].replace("-", "_") for r in rows}
    sobra_global = []
    caracterizacion = []
    for key, cases in destino.items():
        archivo = archivo_de.get(key, "?")
        stem = Path(archivo).stem
        for c in cases:
            if stem in suites_del_oraculo:
                sobra_global.append(f"{c} ({archivo})")
            else:
                caracterizacion.append(f"{c} ({archivo})")

    width = max((len(r["modulo"]) for r in rows), default=10)
    print(f"{'modulo'.ljust(width)}  oracle  ok  faltan  estado")
    print("-" * (width + 28))
    for r in rows:
        estado = "OK" if not r["faltan"] else "FALTAN"
        print(f"{r['modulo'].ljust(width)}  {r['oracle']:>6}  {r['ok']:>2}  "
              f"{len(r['faltan']):>7}  {estado}")
        if args.verbose:
            for case in r["faltan"]:
                print(f"    falta: {case}")
    print("-" * (width + 28))
    pct = (total_ok / total_oracle * 100) if total_oracle else 0.0
    print(f"paridad: {total_ok}/{total_oracle} casos del oraculo ({pct:.1f}%)")
    if sobra_global:
        print(f"casos del destino que el oraculo no reclama: {len(sobra_global)}")
        for item in sobra_global:
            print(f"    sobra: {item}")
    if caracterizacion:
        print(f"tests de caracterizacion (modulos sin suite en el oraculo): "
              f"{len(caracterizacion)}")

    report = {
        "modulos": rows,
        "total_oracle": total_oracle,
        "total_ok": total_ok,
        "paridad_pct": round(pct, 2),
        "sobran": sobra_global,
        "caracterizacion": caracterizacion,
    }
    (out / "docs").mkdir(parents=True, exist_ok=True)
    (out / "docs" / "parity.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    completo = all(not r["faltan"] for r in rows)
    return 0 if completo and not sobra_global else 1


if __name__ == "__main__":
    sys.exit(main())

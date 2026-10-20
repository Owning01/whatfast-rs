"""F2 — el juez de paridad.

Compara, módulo por módulo, los casos de test del oráculo (TypeScript) con
los casos de test del puerto (Rust). Solo compara NOMBRES de caso: el
lenguaje cambia, la semántica no.

    python scripts/parity.py [--origin RUTA] [--out RUTA] [--verbose]

Salida: una tabla por módulo con `ok / faltan / sobran` y el total global.
Sale con código 1 si algún módulo está incompleto, para poder usarlo como
gate en CI.
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


def snake(name: str) -> str:
    """Convierte un nombre de caso a la convencion de Rust."""
    name = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name)
    name = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    name = re.sub(r"[^A-Za-z0-9]+", "_", name)
    return name.strip("_").lower()


def normalize(name: str) -> str:
    """Normaliza para comparar: minusculas y sin separadores.

    Los tests en ingles y el nombre del modulo pueden diferir en palabras
    (por ejemplo `media crop` vs `media_crop`): se comparan por los tokens
    significativos, en orden.
    """
    tokens = [t for t in re.split(r"[^a-z0-9]+", snake(name)) if t]
    stop = {"a", "an", "the", "of", "to", "in", "is", "for", "and", "when",
            "with", "that", "it", "as", "on", "by", "be", "does", "not", "no"}
    return " ".join(t for t in tokens if t not in stop)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    origin = Path(args.origin)
    out = Path(args.out)
    tests_dir = out / "crates" / "whatfast-core" / "tests"

    rows = []
    total_oracle = 0
    total_ok = 0

    for suite_path in sorted((origin / "tauri" / "tests").glob("*.test.ts")):
        module = suite_path.name[: -len(".test.ts")]
        text = suite_path.read_text(encoding="utf-8", errors="replace")
        oracle = [m[1] for m in RE_TEST.findall(text)]

        rust_path = tests_dir / f"{module}.rs"
        rust_cases = []
        if rust_path.exists():
            rtext = rust_path.read_text(encoding="utf-8", errors="replace")
            rust_cases = RE_RUST_TEST.findall(rtext)

        # Emparejamiento por nombre normalizado.
        index = {}
        for case in rust_cases:
            index.setdefault(normalize(case), []).append(case)
        used = set()
        faltan = []
        for case in oracle:
            key = normalize(case)
            if key in index and index[key]:
                index[key].pop(0)
                used.add(key)
            else:
                faltan.append(case)
        sobran = [c for cases in index.values() for c in cases]

        ok = len(oracle) - len(faltan)
        total_oracle += len(oracle)
        total_ok += ok
        rows.append({
            "modulo": module,
            "oracle": len(oracle),
            "ok": ok,
            "faltan": faltan,
            "sobran": sobran,
            "puerto": rust_path.exists(),
        })

    width = max((len(r["modulo"]) for r in rows), default=10)
    print(f"{'modulo'.ljust(width)}  oracle  ok  faltan  sobran  estado")
    print("-" * (width + 34))
    for r in rows:
        estado = "OK" if not r["faltan"] and not r["sobran"] else (
            "FALTA" if r["faltan"] else "SOBRA")
        if not r["puerto"]:
            estado = "SIN PUESTO"
        print(f"{r['modulo'].ljust(width)}  {r['oracle']:>6}  {r['ok']:>2}  "
              f"{len(r['faltan']):>7}  {len(r['sobran']):>6}  {estado}")
        if args.verbose:
            for case in r["faltan"]:
                print(f"    falta: {case}")
            for case in r["sobran"]:
                print(f"    sobra: {case}")
    print("-" * (width + 34))
    pct = (total_ok / total_oracle * 100) if total_oracle else 0.0
    print(f"paridad: {total_ok}/{total_oracle} casos del oraculo ({pct:.1f}%)")

    report = {
        "modulos": rows,
        "total_oracle": total_oracle,
        "total_ok": total_ok,
        "paridad_pct": round(pct, 2),
    }
    (out / "docs").mkdir(parents=True, exist_ok=True)
    (out / "docs" / "parity.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    return 0 if all(not r["faltan"] and not r["sobran"] for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())

"""F1 — genera el mapa de dependencias del origen.

Escanea el repo de origen (el oráculo, solo lectura) y escribe
`docs/dependency-map.json` y `docs/dependency-map.md` en el repo de destino.

    python scripts/dependency-map.py [--origin RUTA] [--out RUTA]

El mapa NO se escribe a mano: si cambia el origen, se regenera y el diff
aparece en el PR.
"""
from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path

DEFAULT_ORIGIN = r"G:\Proyectos\16-whatsapp2"
DEFAULT_OUT = r"G:\Proyectos\17-whatfast-rs"

TEXT_EXT = {".ts", ".tsx", ".css", ".rs", ".go", ".proto", ".toml", ".json", ".md"}
SKIP_DIRS = {
    ".git", "node_modules", "target", "dist", "build", "gen", "perf-results",
    ".pnpm", ".vite-temp",
}

# Clasificacion por modulo del frontend. La clave es el nombre del modulo TS
# (sin extension); el valor es (bucket, modulo destino en Rust, nota).
CLASSIFICATION = {
    "types": ("directa", "proto/bridge.proto", "el contrato ya vive en Rust"),
    "whatsapp-formatting": ("reescritura", "whatsapp_formatting", "no hay soporte en GPUI"),
    "media-crop": ("reemplazo", "media_crop", "geometria pura"),
    "format": ("reemplazo", "message_format", "sin suite: tests de caracterizacion"),
    "polls": ("reemplazo", "polls", ""),
    "chat-merge": ("reemplazo", "chat_merge", ""),
    "unsupported": ("reemplazo", "unsupported", ""),
    "file-drop": ("reemplazo", "file_drop", ""),
    "unread": ("reemplazo", "unread", ""),
    "pairing": ("reemplazo", "pairing", ""),
    "workspace": ("reemplazo", "workspace", "31 tests en el origen"),
    "app-helpers": ("reemplazo", "app_helpers", "sin suite: tests de caracterizacion"),
    "tab-keyboard": ("reemplazo", "tab_keyboard", ""),
    "notification-policy": ("reemplazo", "notification_policy", ""),
    "backend-lifecycle": ("reemplazo", "backend_lifecycle", ""),
    "scroll-restoration": ("reemplazo", "scroll_restoration", ""),
    "browser-mock": ("eliminable", "-", "backend fake en Rust"),
    "bridge": ("reescritura", "whatfast-protocol", "la fachada de comandos desaparece"),
    "app": ("reescritura", "whatfast-ui", "store reactivo -> Entity<Store>"),
    "preferences": ("reescritura", "whatfast-ui", "localStorage -> archivo JSON"),
    "theme": ("reescritura", "whatfast-ui", "variables CSS -> tokens de GPUI"),
    "i18n": ("reemplazo", "whatfast-ui", "datos puros"),
    "spotlight": ("reescritura", "whatfast-ui", "ranking + localStorage"),
    "send-idempotency": ("reemplazo", "whatfast-ui", ""),
    "emoji-autocomplete": ("reemplazo", "whatfast-ui", "offsets UTF-16"),
    "performance": ("reemplazo", "whatfast-ui", ""),
    "notification-routing": ("reescritura", "whatfast-ui", "timers"),
    "participant-avatar-queue": ("reescritura", "whatfast-ui", "concurrencia"),
    "voice-note": ("reescritura", "whatfast-ui", "binario"),
    "notifications": ("reescritura", "whatfast-ui", "plugin de Tauri"),
    "participant-roster": ("reemplazo", "whatfast-ui", ""),
}

RE_TEST = re.compile(r"""(?:^|\s)(?:it|test)\s*\(\s*(['"`])(.+?)\1""", re.M)
RE_RUST_TEST = re.compile(r"#\[(?:tokio::)?test[^\]]*\]\s*(?:async\s+)?fn\s+([A-Za-z0-9_]+)")
RE_CSS_CLASS = re.compile(r"^\s*\.[A-Za-z][A-Za-z0-9_-]*", re.M)
RE_CSS_VARS = re.compile(r"--[a-z0-9-]+\s*:", re.I)
RE_CSS_VAR_USE = re.compile(r"var\(\s*(--[a-z0-9-]+)")
RE_RUST_UNWRAP = re.compile(r"\.unwrap\(\)|\.expect\(|panic!\(|todo!\(|unimplemented!\(")


def code_lines(text: str) -> int:
    """Lineas de codigo: no vacias y no solo comentarios."""
    total = 0
    in_block = False
    for raw in text.splitlines():
        line = raw.strip()
        if in_block:
            if "*/" in line:
                in_block = False
                line = line.split("*/", 1)[1].strip()
            else:
                continue
        if not line or line.startswith("//"):
            continue
        if line.startswith("/*"):
            if "*/" not in line:
                in_block = True
                continue
            line = line.split("*/", 1)[1].strip()
            if not line:
                continue
        total += 1
    return total


def walk(root: Path):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            yield Path(dirpath) / name


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args()

    origin = Path(args.origin)
    out = Path(args.out)
    src = origin / "tauri" / "src"

    modules = []
    totals = {"ts": 0, "css": 0, "files": 0}

    for path in sorted(walk(src)):
        if path.suffix.lower() not in TEXT_EXT:
            continue
        rel = path.relative_to(src).as_posix()
        text = path.read_text(encoding="utf-8", errors="replace")
        loc = code_lines(text)
        stem = path.stem
        bucket, dest, note = CLASSIFICATION.get(stem, ("reescritura", "whatfast-ui", ""))
        entry = {
            "origen": rel,
            "loc": loc,
            "bucket": bucket,
            "destino_rust": dest,
            "nota": note,
        }
        if path.suffix == ".css":
            totals["css"] += loc
            entry["css_clases"] = len(set(RE_CSS_CLASS.findall(text)))
            entry["css_variables"] = len(set(RE_CSS_VAR_USE.findall(text)))
        else:
            totals["ts"] += loc
        totals["files"] += 1
        modules.append(entry)

    # Suites del origen y sus casos
    suites = []
    for path in sorted((origin / "tauri" / "tests").glob("*.test.ts")):
        text = path.read_text(encoding="utf-8", errors="replace")
        cases = [m[1] for m in RE_TEST.findall(text)]
        suites.append({
            "suite": path.name,
            "modulo": path.name.replace(".test.ts", ""),
            "casos": cases,
            "total": len(cases),
        })

    # Estado del destino
    dest_modules = []
    unwrap_hits = []
    for path in sorted(walk(out / "crates")):
        if path.suffix != ".rs":
            continue
        rel = path.relative_to(out).as_posix()
        text = path.read_text(encoding="utf-8", errors="replace")
        tests = RE_RUST_TEST.findall(text)
        dest_modules.append({
            "archivo": rel,
            "loc": code_lines(text),
            "tests": len(tests),
        })
        for i, line in enumerate(text.splitlines(), 1):
            if RE_RUST_UNWRAP.search(line) and "#[cfg(test)]" not in line:
                unwrap_hits.append(f"{rel}:{i}: {line.strip()[:100]}")

    doc = {
        "generado_por": "scripts/dependency-map.py",
        "origen": str(origin),
        "destino": str(out),
        "totales": {
            "archivos_frontend": totals["files"],
            "loc_ts": totals["ts"],
            "loc_css": totals["css"],
            "suites_origen": len(suites),
            "casos_origen": sum(s["total"] for s in suites),
            "modulos_destino": len(dest_modules),
            "tests_destino": sum(m["tests"] for m in dest_modules),
        },
        "modulos": modules,
        "suites": suites,
        "destino": dest_modules,
        "violaciones": unwrap_hits,
    }

    (out / "docs").mkdir(parents=True, exist_ok=True)
    (out / "docs" / "dependency-map.json").write_text(
        json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    # Markdown legible
    lines = [
        "# DEPENDENCY MAP — generado por script",
        "",
        "NO editar a mano: `python scripts/dependency-map.py` lo regenera.",
        "",
        "## Totales",
        "",
        "| Métrica | Valor |",
        "|---|---|",
    ]
    for key, value in doc["totales"].items():
        lines.append(f"| {key} | {value} |")
    lines += ["", "## Módulos del origen", "",
              "| Origen | LOC | Bucket | Destino Rust | Nota |", "|---|---|---|---|---|"]
    for m in modules:
        lines.append(f"| `{m['origen']}` | {m['loc']} | {m['bucket']} | "
                     f"`{m['destino_rust']}` | {m['nota']} |")
    lines += ["", "## Suites del oráculo", "",
              "| Suite | Módulo | Casos |", "|---|---|---|"]
    for s in suites:
        lines.append(f"| `{s['suite']}` | {s['modulo']} | {s['total']} |")
    lines += ["", "## Estado del destino", "",
              "| Archivo Rust | LOC | Tests |", "|---|---|---|"]
    for m in dest_modules:
        lines.append(f"| `{m['archivo']}` | {m['loc']} | {m['tests']} |")
    lines += ["", "## Violaciones del rulebook (gate de F7)", ""]
    if unwrap_hits:
        lines += ["```"] + unwrap_hits + ["```"]
    else:
        lines.append("Ninguna: sin `unwrap`, `expect`, `panic!`, `todo!()` ni "
                     "`unimplemented!()` fuera de los tests.")
    lines.append("")
    (out / "docs" / "dependency-map.md").write_text("\n".join(lines), encoding="utf-8")

    print(f"modulos: {len(modules)} | suites: {len(suites)} | "
          f"casos del oraculo: {doc['totales']['casos_origen']} | "
          f"tests en destino: {doc['totales']['tests_destino']} | "
          f"violaciones: {len(unwrap_hits)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

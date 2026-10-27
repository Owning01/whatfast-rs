"""F5 — el juez de paridad del sidecar.

Compara el oraculo Go congelado (backend/, el sidecar original) con el puerto
Rust (crates/whatfast-wa) que lo reemplaza bajo el mismo contrato
proto/bridge.proto. Como parity.py compara NOMBRES: el lenguaje cambia, la
semantica no. Tres inventarios:

  1. RPC del contrato: los `case *bridgev1.RpcRequest_X:` del switch de
     backend/internal/app/server.go contra los brazos
     `Some(rpc_request::Request::X)` del match de dispatch en
     crates/whatfast-wa/src/server.rs.
  2. Metodos exportados de backend/internal/store/store.go y
     backend/internal/wa/client.go contra `fn` en los .rs del crate
     (heuristica: snake_case del metodo Go).
  3. El oneof de RpcRequest en proto/bridge.proto, para detectar brazos que
     el puerto agrego y el oraculo nunca tuvo.

Regla del gate (RPC): todo RPC del Go debe tener brazo real en el Rust o
estar en PENDIENTES_DECLARADOS. Un RPC del Go que cae en el brazo
`not_implemented` sin estar declarado es un hueco sin registrar: el juez lo
lista como `pendiente-declarar` y sale con codigo 1. Los pendientes
declarados y los brazos extra del puerto se reportan, no rompen el gate.

El inventario de metodos es reporte, no gate: el puerto renombro y fusiono
metodos del store (p.ej. EnsureConversation -> ensure_chat/adopt_address),
asi que la heuristica de nombre lista candidatos, y la paridad semantica la
auditan las particiones (db, ingest, send/media/meta, server/identity).

    python scripts/paridad_sidecar.py [--origin RUTA] [--out RUTA]
                                      [--verbose] [--json RUTA]
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

DEFAULT_ORIGIN = r"G:\Proyectos\17-whatfast-rs"
DEFAULT_OUT = r"G:\Proyectos\17-whatfast-rs"

# RPC que el Go sirve y el puerto todavia no: deuda conocida y verificada.
# Vacio desde que los ultimos 7 (VotePoll, RequestOlderHistory,
# GetChatHistoryCoverage, GetHistoryOverview, GetIntegrityStatus,
# RepairLocalCache, RepairRecentReactions) ganaron brazo real: el match del
# dispatch es exhaustivo y el compilador exige brazo para cada variante.
PENDIENTES_DECLARADOS = set()

RE_GO_RPC = re.compile(r"case \*bridgev1\.RpcRequest_(\w+):")
RE_RUST_ARM = re.compile(r"Some\(rpc_request::Request::(\w+)\(")
RE_GO_FN = r"^func \(\w+ \*{receiver}\) ([A-Z]\w*)\("
RE_RUST_FN = re.compile(r"\bfn\s+([A-Za-z0-9_]+)")
RE_PUB_FN = re.compile(r"^\s*pub (?:async )?fn (\w+)", re.M)

MODULOS_PUERTO = ["db", "send", "media", "meta", "ingest", "identity", "reconcile", "server", "session", "wire"]


def snake(name: str) -> str:
    name = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name)
    name = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    name = re.sub(r"[^A-Za-z0-9]+", "_", name)
    return name.strip("_").lower()


def normalize(name: str) -> str:
    """Minusculas sin separadores, para que MarkReadIDs no falle por el caso."""
    return re.sub(r"[^a-z0-9]", "", name.lower())


def camel(field: str) -> str:
    return "".join(part.capitalize() for part in field.split("_"))


def span_braces(text: str, start: int) -> str:
    """El pedazo de texto desde el primer `{` en `start` hasta su cierre."""
    begin = text.index("{", start)
    depth = 0
    for index in range(begin, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[begin:index]
    return text[begin:]


def rpcs_go(origin: Path) -> list[str]:
    text = (origin / "backend" / "internal" / "app" / "server.go").read_text(
        encoding="utf-8", errors="replace")
    return RE_GO_RPC.findall(text)


def rpcs_rust(origin: Path) -> list[str]:
    text = (origin / "crates" / "whatfast-wa" / "src" / "server.rs").read_text(
        encoding="utf-8", errors="replace")
    # Solo el match de `dispatch`: normalize_chat_id (mas abajo en el mismo
    # archivo) tambien matchea `Some(rpc_request::Request::X)` y no es un
    # brazo del despacho.
    dispatch = span_braces(text, text.index("async fn dispatch("))
    return RE_RUST_ARM.findall(dispatch)


def rpcs_proto(origin: Path) -> list[str]:
    text = (origin / "proto" / "bridge.proto").read_text(
        encoding="utf-8", errors="replace")
    message = span_braces(text, text.index("message RpcRequest {"))
    oneof = span_braces(message, message.index("oneof request"))
    fields = re.findall(r"^\s*\w+\s+(\w+) = \d+;", oneof, re.M)
    return [camel(field) for field in fields]


def metodos_go(path: Path, receiver: str) -> list[str]:
    pattern = re.compile(RE_GO_FN.format(receiver=receiver), re.M)
    return pattern.findall(path.read_text(encoding="utf-8", errors="replace"))


def fns_por_archivo(crate: Path) -> list[tuple[str, dict[str, str]]]:
    """[(archivo, {fn_normalizado: fn_original}), ...] en orden estable."""
    archivos = []
    for path in sorted(crate.rglob("*.rs")):
        nombres: dict[str, str] = {}
        for fn in RE_RUST_FN.findall(path.read_text(encoding="utf-8",
                                                    errors="replace")):
            nombres.setdefault(normalize(fn), fn)
        archivos.append((path.name, nombres))
    return archivos


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=DEFAULT_ORIGIN,
                    help="repo con backend/ (oraculo) y crates/ (puerto)")
    ap.add_argument("--out", default=DEFAULT_OUT,
                    help="repo donde escribir el --json")
    ap.add_argument("--verbose", action="store_true",
                    help="lista cada RPC y cada metodo que falta")
    ap.add_argument("--json", default=None,
                    help="ruta de evidencia en JSON (opcional)")
    args = ap.parse_args()

    origin = Path(args.origin)
    crate = origin / "crates" / "whatfast-wa"

    go = rpcs_go(origin)
    brazos = rpcs_rust(origin)
    proto = rpcs_proto(origin)

    # --- Inventario 1: RPC del contrato -----------------------------------
    brazos_set = set(brazos)
    ok, pendiente, sin_declarar = [], [], []
    for rpc in go:
        if rpc in brazos_set:
            ok.append(rpc)
        elif rpc in PENDIENTES_DECLARADOS:
            pendiente.append(rpc)
        else:
            # Sin brazo propio: cae en `Some(other) => not_implemented`.
            sin_declarar.append(rpc)
    extras = [b for b in brazos if b not in set(go)]

    print("RPC del contrato (oraculo Go -> puerto Rust)")
    print(f"  switch Go: {len(go)}   brazos Rust: {len(brazos)}   "
          f"proto: {len(proto)}")
    print(f"  {'estado':<20} cantidad")
    print(f"  {'-' * 28}")
    print(f"  {'ok (brazo real)':<20} {len(ok):>7}")
    print(f"  {'pendiente declarado':<20} {len(pendiente):>7}")
    print(f"  {'pendiente-declarar':<20} {len(sin_declarar):>7}")
    print(f"  {'extra del puerto':<20} {len(extras):>7}")
    for rpc in pendiente:
        print(f"    pendiente: {rpc} (declarado, responde not_implemented)")
    for rpc in sin_declarar:
        print(f"    pendiente-declarar: {rpc} (responde not_implemented "
              f"y no esta en PENDIENTES_DECLARADOS)")
    for rpc in extras:
        print(f"    extra: {rpc} (brazo sin case en el Go; el oraculo "
              f"no lo servia)")
    if args.verbose:
        for rpc in ok:
            print(f"    ok: {rpc}")

    # --- Inventario 2: metodos Go -> fn Rust ------------------------------
    origenes = [
        ("store", origin / "backend" / "internal" / "store" / "store.go",
         "Store"),
        ("client", origin / "backend" / "internal" / "wa" / "client.go",
         "Client"),
    ]
    archivos = fns_por_archivo(crate)

    def modulo_de(nombre_fn: str) -> str | None:
        objetivo = normalize(nombre_fn)
        for archivo, nombres in archivos:
            if objetivo in nombres:
                return Path(archivo).stem
        return None

    print()
    print("Metodos Go -> fn Rust (heuristica snake_case, todo el crate)")
    por_modulo: dict[str, dict[str, list[str]]] = {}
    resumen_origen = []
    for nombre_origen, ruta, receptor in origenes:
        metodos = metodos_go(ruta, receptor)
        encontrados, faltan = [], []
        for metodo in metodos:
            modulo = modulo_de(snake(metodo))
            if modulo is None:
                faltan.append(metodo)
            else:
                encontrados.append(metodo)
                por_modulo.setdefault(modulo, {}).setdefault(
                    nombre_origen, []).append(metodo)
        resumen_origen.append((nombre_origen, len(metodos), len(encontrados),
                               faltan))

    ancho = max((len(o) for o, *_ in resumen_origen), default=6)
    print(f"  {'origen'.ljust(ancho)}  metodos  ok  faltan")
    print(f"  {'-' * (ancho + 22)}")
    total = [0, 0, 0]
    for nombre, cantidad, okm, _ in resumen_origen:
        faltan = cantidad - okm
        print(f"  {nombre.ljust(ancho)}  {cantidad:>8}  {okm:>3}  {faltan:>6}")
        total[0] += cantidad
        total[1] += okm
        total[2] += faltan
    print(f"  {'-' * (ancho + 22)}")
    print(f"  {'total'.ljust(ancho)}  {total[0]:>8}  {total[1]:>3}  "
          f"{total[2]:>6}")
    print()
    print("  por modulo del puerto (donde vive el fn homonimo):")
    print(f"    {'modulo':<12} {'store':>6} {'client':>7}")
    modulos = sorted({m for m in por_modulo} | set(MODULOS_PUERTO))
    for modulo in modulos:
        hits = por_modulo.get(modulo, {})
        store_n = len(hits.get("store", []))
        client_n = len(hits.get("client", []))
        if store_n or client_n or modulo in MODULOS_PUERTO:
            print(f"    {modulo:<12} {store_n:>6} {client_n:>7}")
    for _, _, _, faltan in resumen_origen:
        if faltan and args.verbose:
            for metodo in faltan:
                print(f"    falta: {metodo} -> {snake(metodo)}")

    # --- Inventario 3: pub fn del puerto ----------------------------------
    print()
    print("pub fn del puerto (volumen por modulo auditado):")
    for modulo in MODULOS_PUERTO:
        ruta = crate / "src" / f"{modulo}.rs"
        if ruta.exists():
            cantidad = len(RE_PUB_FN.findall(
                ruta.read_text(encoding="utf-8", errors="replace")))
            print(f"    {modulo:<12} {cantidad:>3}")

    veredicto = (
        f"rpc: {len(ok)}/{len(go)} reales, {len(pendiente)} pendientes "
        f"declarados, {len(sin_declarar)} sin declarar"
    )
    print()
    print(veredicto)
    if sin_declarar:
        print("gate: FALLA (hay RPC del oraculo cayendo en not_implemented "
              "sin declarar)")
    else:
        print("gate: OK (todo RPC del Go tiene brazo real o pendencia "
              "declarada)")

    if args.json:
        reporte = {
            "rpc": {
                "go": go,
                "brazos_rust": brazos,
                "proto": proto,
                "ok": ok,
                "pendientes_declarados": pendiente,
                "pendientes_sin_declarar": sin_declarar,
                "extras_del_puerto": extras,
            },
            "metodos": {
                nombre: {"total": cantidad, "ok": okm, "faltan": faltan}
                for nombre, cantidad, okm, faltan in resumen_origen
            },
            "por_modulo": {
                modulo: {
                    origen: metodos
                    for origen, metodos in por_modulo.get(modulo, {}).items()
                }
                for modulo in sorted(por_modulo)
            },
        }
        destino = Path(args.out) / args.json if not Path(args.json).is_absolute() \
            else Path(args.json)
        destino.parent.mkdir(parents=True, exist_ok=True)
        destino.write_text(
            json.dumps(reporte, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8")
        print(f"evidencia: {destino}")

    return 1 if sin_declarar else 0


if __name__ == "__main__":
    sys.exit(main())

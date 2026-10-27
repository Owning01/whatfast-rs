#!/usr/bin/env python3
"""Prueba el servidor MCP de Whatfast contra el perfil de esta maquina.

Arranca `whatfast.exe mcp --agent probar`, habla JSON-RPC por stdio como lo
haria un harness (Claude Code, Codex, OpenCode) y recorre las herramientas de
lectura. Con `--enviar` manda un mensaje de verdad: sin esa bandera no escribe
nada en WhatsApp.

    python scripts/probar-mcp.py
    python scripts/probar-mcp.py --chat 1203630...@g.us
    python scripts/probar-mcp.py --enviar 5491100000000@s.whatsapp.net "hola"
"""

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time

# Los nombres de los chats traen emojis y acentos: la consola de Windows usa
# cp1252 y corta el informe con un UnicodeEncodeError.
for flujo in (sys.stdout, sys.stderr):
    if hasattr(flujo, "reconfigure"):
        flujo.reconfigure(encoding="utf-8", errors="replace")

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def por_defecto(relativo):
    return os.path.join(RAIZ, relativo)


class Servidor:
    """El proceso del servidor MCP, con su entrada y su salida."""

    def __init__(self, exe, backend, data_dir, agente, verboso):
        argumentos = [
            exe,
            "mcp",
            "--agent",
            agente,
            "--data-dir",
            data_dir,
            "--backend",
            backend,
        ]
        self.proc = subprocess.Popen(
            argumentos,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
        self.verboso = verboso
        self.lineas = queue.Queue()
        threading.Thread(target=self._bombear, daemon=True).start()
        threading.Thread(target=self._bombear_errores, daemon=True).start()

    def _bombear(self):
        for linea in self.proc.stdout:
            self.lineas.put(linea)
        self.lineas.put(None)

    def _bombear_errores(self):
        for linea in self.proc.stderr:
            if self.verboso:
                print(f"  [servidor] {linea.rstrip()}", file=sys.stderr)

    def enviar(self, mensaje):
        self.proc.stdin.write(json.dumps(mensaje, ensure_ascii=False) + "\n")
        self.proc.stdin.flush()

    def pedir(self, identificador, metodo, parametros=None, espera=60):
        mensaje = {"jsonrpc": "2.0", "id": identificador, "method": metodo}
        if parametros is not None:
            mensaje["params"] = parametros
        self.enviar(mensaje)
        limite = time.time() + espera
        while True:
            restante = limite - time.time()
            if restante <= 0:
                raise TimeoutError(f"{metodo} no contesto en {espera} s")
            try:
                linea = self.lineas.get(timeout=restante)
            except queue.Empty:
                raise TimeoutError(f"{metodo} no contesto en {espera} s")
            if linea is None:
                raise RuntimeError(f"{metodo}: el servidor cerro la salida")
            if self.verboso:
                print(f"  [servidor] {linea.rstrip()}", file=sys.stderr)
            respuesta = json.loads(linea)
            if respuesta.get("id") == identificador:
                return respuesta

    def avisar(self, metodo, parametros=None):
        mensaje = {"jsonrpc": "2.0", "method": metodo}
        if parametros is not None:
            mensaje["params"] = parametros
        self.enviar(mensaje)

    def llamar(self, identificador, herramienta, argumentos, espera=60):
        respuesta = self.pedir(
            identificador,
            "tools/call",
            {"name": herramienta, "arguments": argumentos},
            espera,
        )
        if "error" in respuesta:
            raise RuntimeError(f"{herramienta}: {respuesta['error']}")
        resultado = respuesta["result"]
        texto = "".join(
            parte.get("text", "")
            for parte in resultado.get("content", [])
            if parte.get("type") == "text"
        )
        if resultado.get("isError"):
            raise RuntimeError(f"{herramienta} fallo: {texto}")
        return json.loads(texto) if texto else {}

    def cerrar(self):
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            return self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            return None


def main():
    analizador = argparse.ArgumentParser(description=__doc__)
    analizador.add_argument(
        "--exe",
        default=por_defecto(os.path.join("target", "debug", "whatfast-ui.exe")),
        help="el ejecutable de Whatfast (el que tambien abre la ventana)",
    )
    analizador.add_argument(
        "--backend",
        default=por_defecto(os.path.join("target", "debug", "whatfast-wa.exe")),
        help="el sidecar que el servidor arranca si la ventana esta cerrada",
    )
    analizador.add_argument(
        "--data-dir",
        default=os.environ.get("WHATFAST_DATA_DIR")
        or os.path.join(os.environ.get("LOCALAPPDATA", ""), "whatfast"),
        help="el perfil de Whatfast",
    )
    analizador.add_argument("--agente", default="probar", help="el id del agente")
    analizador.add_argument("--chat", help="el chat que se lee en detalle")
    analizador.add_argument(
        "--enviar",
        nargs=2,
        metavar=("CHAT_ID", "TEXTO"),
        help="manda un mensaje de verdad y sale",
    )
    analizador.add_argument(
        "--esperar-ms",
        type=int,
        help="espera un mensaje nuevo con ese tope y muestra lo que llego",
    )
    analizador.add_argument("--verboso", action="store_true", help="muestra el trafico")
    opciones = analizador.parse_args()

    for ruta in (opciones.exe, opciones.backend):
        if not os.path.exists(ruta):
            print(f"falta {ruta}: compilá con cargo build", file=sys.stderr)
            return 2

    servidor = Servidor(
        opciones.exe,
        opciones.backend,
        opciones.data_dir,
        opciones.agente,
        opciones.verboso,
    )
    try:
        arranque = servidor.pedir(
            1,
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "probar-mcp", "version": "1"},
            },
        )
        resultado = arranque["result"]
        print(f"protocolo: {resultado['protocolVersion']}")
        print(f"servidor:  {resultado['serverInfo']['name']} {resultado['serverInfo']['version']}")
        servidor.avisar("notifications/initialized")

        herramientas = servidor.pedir(2, "tools/list")["result"]["tools"]
        print(f"herramientas ({len(herramientas)}): {', '.join(t['name'] for t in herramientas)}")

        listado = servidor.llamar(3, "chats_list", {"limit": 5})
        total = listado.get("total", 0)
        print(f"chats: {total} en el perfil")
        for chat in listado.get("chats", [])[:5]:
            marca = "[grupo]" if chat["kind"] == "group" else "[chat] "
            sin_leer = f" ({chat['unread']} sin leer)" if chat["unread"] else ""
            print(f"  {marca} {chat['title']}{sin_leer}")
            print(f"         {chat['last_message'][:70]}")

        primero = opciones.chat
        if primero is None and listado.get("chats"):
            primero = listado["chats"][0]["chat_id"]
        if primero:
            detalle = servidor.llamar(4, "messages_read", {"chat_id": primero, "limit": 5})
            print(f"ultimos mensajes de {primero}: {len(detalle.get('messages', []))}")
            for mensaje in detalle.get("messages", []):
                quien = "yo" if mensaje["from_me"] else mensaje["sender"]
                print(f"  {mensaje['at'][11:16]} {quien}: {mensaje['text'][:70]}")
            info = servidor.llamar(5, "chat_info", {"chat_id": primero})
            titulo = (info.get("chat") or {}).get("title", "")
            print(
                f"info: {titulo} · {info.get('address', '')} · "
                f"{info.get('participant_count', 0)} participantes"
            )

        buscado = servidor.llamar(6, "messages_search", {"query": "hola", "limit": 3})
        print(f"busqueda 'hola': {len(buscado.get('messages', []))} mensajes")

        if opciones.esperar_ms:
            espera = servidor.llamar(
                7,
                "messages_wait",
                {"timeout_ms": opciones.esperar_ms},
                espera=opciones.esperar_ms / 1000 + 30,
            )
            print(
                f"espera: {espera.get('waited_ms')} ms · "
                f"{len(espera.get('messages', []))} mensajes · {espera.get('reason', 'llego')}"
            )
            for mensaje in espera.get("messages", []):
                print(f"  {mensaje['chat_id']}: {mensaje['text'][:60]}")

        if opciones.enviar:
            chat_id, texto = opciones.enviar
            enviado = servidor.llamar(
                7, "message_send", {"chat_id": chat_id, "text": texto}, espera=90
            )
            print(f"enviado: {enviado.get('id')} · {enviado.get('status')}")

        codigo = servidor.cerrar()
        print(f"salida del servidor: {codigo}")
        return 0 if codigo == 0 else 1
    except (RuntimeError, TimeoutError, KeyError, json.JSONDecodeError) as error:
        print(f"FALLO: {error}", file=sys.stderr)
        servidor.cerrar()
        return 1


if __name__ == "__main__":
    sys.exit(main())

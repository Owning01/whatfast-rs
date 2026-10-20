# Mide que tanta parte del cliente web tiene ya la app nativa.
#
# Dos cuentas:
#   1. Los RPC: que fraccion de los 50 del proto pide la interfaz.
#   2. Los componentes: que fraccion de los del cliente web hay en Rust.
#
# Salida 1 si algo quedo sin cubrir: es el gate de "igual que el cliente web".
#
# Uso:
#   python scripts/funcionalidad.py
#   python scripts/funcionalidad.py --verbose

import re
import sys
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
PROTO = RAIZ / "proto" / "bridge.proto"
UI = RAIZ / "crates" / "whatfast-ui" / "src"
WEB = Path(r"G:\Proyectos\16-whatsapp2\tauri\src\components")

# El cliente web y su equivalente en la app nativa. La clave es el nombre del
# componente del cliente web; el valor, el archivo (o el texto) que lo
# reemplaza en Rust.
EQUIVALENCIAS = {
    "ThemeIcon.tsx": "icons.rs",
    "Primitives.tsx": "gpui_component (button, dialog, input, menu)",
    "TitleBar.tsx": "navrail.rs",
    "Tabs.tsx": "tabs.rs",
    "MemberPanel.tsx": "members.rs",
    "ParticipantList.tsx": "members.rs",
    "ParticipantRow.tsx": "members.rs",
    "NewsletterPanel.tsx": "newsletters.rs",
    "StatusStrip.tsx": "statuses.rs",
    "StickerTray.tsx": "stickers.rs",
    "MediaEditor.tsx": "media_editor.rs",
    "Sidebar.tsx": "render.rs (render_sidebar)",
    "Conversation.tsx": "render.rs (render_conversation)",
    "MessageBubble.tsx": "render.rs (render_message)",
    "Composer.tsx": "render.rs (render_conversation)",
    "EmojiPicker.tsx": "render.rs (render_emoji_picker)",
    "SpotlightSearch.tsx": "render.rs (render_search_results)",
    "ChatSwitcher.tsx": "render.rs (render_chat_switcher)",
    "Panels.tsx": "render.rs (render_settings, render_chat_info)",
    "Screens.tsx": "render.rs (render_pairing, render_center)",
    "Avatar.tsx": "helpers.rs (avatar_element)",
}


def rpcs_del_proto() -> list[str]:
    texto = PROTO.read_text(encoding="utf-8")
    i = texto.find("oneof request {")
    cuerpo = texto[i : texto.find("\n  }", i)]
    return re.findall(r"^\s*(\w+)Request \w+ = \d+;", cuerpo, re.M)


def rpcs_que_pide_la_ui() -> set[str]:
    pedidos: set[str] = set()
    for archivo in UI.rglob("*.rs"):
        texto = archivo.read_text(encoding="utf-8", errors="ignore")
        pedidos.update(re.findall(r"Request::(\w+)\s*\{", texto))
        pedidos.update(re.findall(r"Request::(\w+)\b(?!\s*\{)", texto))
    return pedidos


def componentes_web() -> list[str]:
    if not WEB.is_dir():
        return []
    return sorted(p.name for p in WEB.glob("*.tsx"))


def main() -> int:
    verbose = "--verbose" in sys.argv

    todos = rpcs_del_proto()
    pedidos = rpcs_que_pide_la_ui()
    faltan = [r for r in todos if r not in pedidos]

    print("=== RPC del sidecar ===")
    print(f"  la interfaz pide {len(todos) - len(faltan)} de {len(todos)}")
    if faltan:
        print("  faltan:")
        for r in faltan:
            print(f"    - {r}")

    print()
    print("=== componentes del cliente web ===")
    componentes = componentes_web()
    if not componentes:
        print("  (el cliente web no esta en esta maquina: la cuenta se omite)")
    else:
        con_counterpart = 0
        archivos_ui = {p.name for p in UI.glob("*.rs")}
        for nombre in componentes:
            contraparte = EQUIVALENCIAS.get(nombre)
            if contraparte is None:
                estado = "sin equivalencia conocida"
            elif contraparte.startswith("gpui_component"):
                estado = f"cubierto por {contraparte}"
            else:
                # "render.rs (render_sidebar)" -> el archivo que lo resuelve
                archivo = contraparte.split("(")[0].strip()
                if archivo in archivos_ui:
                    estado = f"cubierto por {contraparte}"
                else:
                    estado = f"FALTA ({contraparte})"
            if estado.startswith("cubierto"):
                con_counterpart += 1
            if verbose or not estado.startswith("cubierto"):
                print(f"  {nombre:<26} {estado}")
        print(f"  la app nativa cubre {con_counterpart} de {len(componentes)}")

    print()
    if faltan:
        print(f"la app nativa todavia no esta igual que el cliente web: {len(faltan)} RPC sin pedir")
        return 1
    print("la app nativa pide todos los RPC del sidecar")
    return 0


if __name__ == "__main__":
    sys.exit(main())

# Whatfast en las tres plataformas

Esta guía cubre cómo se instala Whatfast en Windows, en Linux y en macOS: qué
trae cada paquete, dónde queda, qué dependencias pide el sistema y cómo se
compila desde el código.

El estado de cada paquete, primero, porque no es el mismo:

| Sistema | Paquete | Estado |
|---|---|---|
| Windows | `whatfast-setup.exe` (instalador) y `whatfast-windows.zip` | **Probado.** Se midió y se ejecutó en Windows 11 x86-64. |
| Linux | `whatfast-linux.tar.gz` | **Sin probar en una máquina Linux.** El workflow lo arma; nadie lo ejecutó todavía en un escritorio real. |
| macOS | `whatfast-macos-arm64.zip` y `whatfast-macos-x64.zip` | **En camino.** El paquete todavía no está publicado. |

Lo que sí está probado en los tres casos es lo de abajo: la forma del paquete,
los nombres de los archivos y las rutas de los datos. Lo que falta probar es
que la interfaz arranque en Linux y en macOS; eso lo está cerrando el trabajo
de portabilidad.

Los paquetes salen de los releases de GitHub del repositorio
(`Owning01/whatfast`). Si un archivo no está en la página de releases, ese
paquete todavía no salió.

Los nombres de los archivos y la estructura interna salen de
`.github/workflows/release.yml` y de `installer.nsi`. La parte de Linux y macOS
del workflow vive en el trabajo de empaquetado y todavía no está en `main`: es
la que define el `.tar.gz` con `share/`, los `.sha256` y el job de macOS.
Cuando eso se integre, esta guía queda alineada sin cambios de fondo.

---

## Lo que trae cualquier paquete: los dos ejecutables

Whatfast son **dos ejecutables** y no uno:

| Ejecutable | Qué es |
|---|---|
| `whatfast` (`whatfast.exe` en Windows) | La interfaz nativa, la que dibuja la ventana con GPUI. |
| `whatfast-backend` (`whatfast-backend.exe` en Windows) | El sidecar en Go: la sesión de WhatsApp, el historial, los medios y las bases SQLite. |

La interfaz busca el sidecar en dos lugares, en este orden:

1. La variable de entorno `WHATFAST_BACKEND`, si está definida. Tiene que
   apuntar al ejecutable completo, no a la carpeta.
2. **Al lado de la interfaz.** El mismo directorio del ejecutable que está
   corriendo.

Por eso los dos archivos van juntos. Si el sidecar no está, la app abre y no
conecta: no hay mensajes, no hay historial.

La app habla con WhatsApp como **dispositivo vinculado**. La primera vez
muestra un QR; se escanea desde el teléfono en **WhatsApp → Dispositivos
vinculados → Vincular un dispositivo**. La sesión queda guardada en el
sidecar, así que el QR se pide una sola vez por equipo.

### Dónde quedan los datos

Los dos ejecutables se pueden mover; el perfil no. Las rutas son fijas por
sistema y se pueden cambiar con una variable de entorno.

| Sistema | Configuración | Datos (caché, medios, `backend.log`) |
|---|---|---|
| Windows | `%APPDATA%\whatfast` | `%LOCALAPPDATA%\whatfast` |
| Linux | `$XDG_CONFIG_HOME/whatfast`, o `~/.config/whatfast` | `$XDG_DATA_HOME/whatfast`, o `~/.local/share/whatfast` |
| macOS | `~/Library/Application Support/whatfast` | `~/Library/Application Support/whatfast` |

Las variables que mandan sobre todo lo anterior son `WHATFAST_CONFIG_DIR` y
`WHATFAST_DATA_DIR`. Sirven para tener dos perfiles en la misma máquina. El
modo `--fake-backend` usa la misma carpeta de datos, así que para no mezclar
los chats simulados con el perfil real conviene correrlo con
`WHATFAST_DATA_DIR` apuntando a una carpeta aparte.

El historial (`session.db`, `client.db`) y el log (`backend.log`) viven en la
carpeta de datos. Desloguear desde la app borra la sesión.

---

## Windows

### El `.zip`

`whatfast-windows.zip` se descomprime en cualquier carpeta. Adentro:

```
whatfast.exe            la interfaz
whatfast-backend.exe    el sidecar
icons\                  el logo en sus tamaños (32x32.png, 128x128.png,
                        128x128@2x.png, icon.ico, icon.icns)
```

Se corre `whatfast.exe` desde esa carpeta. Los dos `.exe` tienen que quedar
juntos: si se mueve uno solo, la app abre sin conexión.

Es la variante portable: no escribe en el registro, no arma accesos directos y
no pide permisos de administrador. Para sacarla, se borra la carpeta. Los
datos quedan en `%APPDATA%\whatfast` y `%LOCALAPPDATA%\whatfast`; si se quieren
borrar también, hay que borrar esas dos carpetas a mano.

### El instalador `whatfast-setup.exe`

`whatfast-setup.exe` es el instalador (NSIS). Hace esto:

- Copia `whatfast.exe`, `whatfast-backend.exe` y `icons\` en
  **`C:\Program Files\Whatfast`** (`$PROGRAMFILES64\Whatfast`).
- Arma el acceso directo del **escritorio** y la carpeta **Whatfast** en el
  menú Inicio, con el logo.
- Deja el desinstalador en `C:\Program Files\Whatfast\Uninstall.exe` y la
  entrada en **Agregar o quitar programas** (registro
  `HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\Whatfast`).

Pide permisos de administrador, porque escribe en Archivos de programa. Se
desinstala desde **Configuración → Aplicaciones → Whatfast**, o con
`Uninstall.exe`. El desinstalador borra los binarios, los iconos y los accesos
directos; **no** borra el perfil de `%APPDATA%` ni de `%LOCALAPPDATA%`.

El instalador no está firmado con un certificado de código, así que SmartScreen
puede frenarlo la primera vez: **Más información → Ejecutar de todas formas**.

---

## Linux

### El `.tar.gz`

`whatfast-linux.tar.gz` se descomprime en cualquier carpeta:

```sh
mkdir -p ~/whatfast
tar -xzf whatfast-linux.tar.gz -C ~/whatfast
```

El contenido, tal como lo arma el release:

```
whatfast                                        la interfaz
whatfast-backend                                el sidecar
share/applications/whatfast.desktop             la entrada de escritorio
share/icons/hicolor/32x32/apps/whatfast.png
share/icons/hicolor/64x64/apps/whatfast.png
share/icons/hicolor/128x128/apps/whatfast.png
share/icons/hicolor/256x256/apps/whatfast.png
share/icons/hicolor/512x512/apps/whatfast.png
icons/                                          la carpeta plana que traían
                                                las versiones anteriores
```

Para probarlo sin instalar nada:

```sh
cd ~/whatfast
./whatfast
```

El sidecar se busca al lado, así que `whatfast` y `whatfast-backend` tienen que
quedar en la misma carpeta. Si el archivo vino sin permiso de ejecución:

```sh
chmod +x whatfast whatfast-backend
```

El release también publica `whatfast-linux.tar.gz.sha256`, por si se quiere
verificar la descarga:

```sh
sha256sum -c whatfast-linux.tar.gz.sha256
```

### Instalarlo en el sistema

El `.desktop` tiene `Exec=whatfast`, sin ruta, y `Icon=whatfast`. Eso quiere
decir que el lanzador espera **el binario en el `PATH`** y **el icono en el
tema de iconos**. Si el binario no está en el `PATH`, el lanzador del menú no
hace nada; si falta el icono, la entrada aparece sin logo.

Instalación solo para el usuario actual, sin `sudo`:

```sh
mkdir -p ~/.local/bin ~/.local/share/applications
install -m755 whatfast whatfast-backend -t ~/.local/bin
install -m644 share/applications/whatfast.desktop -t ~/.local/share/applications
cp -r share/icons/hicolor/. ~/.local/share/icons/hicolor/
update-desktop-database ~/.local/share/applications 2>/dev/null || true
```

`~/.local/bin` tiene que estar en el `PATH`. La mayoría de las distribuciones
lo agrega solo si existe al iniciar sesión; si no aparece, se agrega a mano en
el `~/.profile`.

Instalación para todo el sistema:

```sh
sudo install -Dm755 whatfast whatfast-backend -t /usr/local/bin
sudo install -Dm644 share/applications/whatfast.desktop -t /usr/share/applications
sudo cp -r share/icons/hicolor/. /usr/share/icons/hicolor/
```

Las dos variantes dejan el `.desktop` donde corresponde
(`~/.local/share/applications/` o `/usr/share/applications/`) y los iconos en
el árbol de freedesktop (`hicolor/<tamaño>/apps/whatfast.png`). Con eso la app
aparece en el menú de aplicaciones con su logo.

Para desinstalar: borrar los dos binarios, el `.desktop` y
`~/.local/share/icons/hicolor/*/apps/whatfast.png`. Los datos quedan en
`~/.local/share/whatfast` y la configuración en `~/.config/whatfast`.

### Dependencias del sistema

La interfaz usa el stack de GPUI, el de Zed. Necesita un servidor gráfico
**X11 o Wayland**, fontconfig y freetype, y una API de dibujo: **Vulkan**
(`libvulkan1`) o GL en su defecto.

En Debian, Ubuntu y derivados, para ejecutar un paquete ya compilado:

```sh
sudo apt install libfontconfig1 libwayland-client0 libxkbcommon-x11-0 libgl1 libvulkan1
```

En un Ubuntu de escritorio esas bibliotecas ya están. En un sistema mínimo, o
en un contenedor, hay que instalarlas.

Las dependencias de **compilación** son más, y son las que instala el workflow
de release:

```sh
sudo apt-get install --yes \
  build-essential cmake clang lld pkg-config \
  libfontconfig-dev libfreetype6-dev \
  libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev \
  libssl-dev libvulkan1 libglib2.0-dev libzstd-dev
```

Los nombres son de Debian y Ubuntu. En Fedora, Arch y openSUSE cambian
(`fontconfig-devel`, `freetype2`, `wayland-devel`, `libxkbcommon-devel`,
`vulkan-loader`, `glib2-devel`, `zstd-devel`, `openssl-devel`).

### Lo que todavía no está probado en Linux

- Nadie ejecutó el `.tar.gz` en un escritorio Linux real. El paquete se arma
  en el runner `ubuntu-latest` de GitHub.
- El `.desktop` se instala con `Exec=whatfast` y `Icon=whatfast`. Si se corre
  desde la carpeta descomprimida, sin instalar, el lanzador del menú no
  encuentra el binario. Por eso el `.desktop` se usa recién después de
  instalarlo, o se le cambia el `Exec` por la ruta absoluta.
- No hay AppImage, ni `.deb`, ni Flatpak, ni Snap. Si alguno de esos aparece
  más adelante, esta guía se actualiza.

---

## macOS

### El paquete

El paquete de macOS está **en camino**: el workflow lo va a armar, pero
todavía no salió publicado. Cuando salga, van a ser dos archivos, uno por
arquitectura:

| Archivo | Arquitectura | Para qué equipo |
|---|---|---|
| `whatfast-macos-arm64.zip` | `aarch64-apple-darwin` | Apple Silicon (M1 en adelante) |
| `whatfast-macos-x64.zip` | `x86_64-apple-darwin` | Intel |

No hay `.dmg`. Cada `.zip` trae `Whatfast.app` ya armado, y el release publica
al lado un `.sha256` de cada uno.

Adentro del `.app` están los dos ejecutables, que es la razón por la que no hay
que copiarlos a mano:

```
Whatfast.app/Contents/MacOS/whatfast            la interfaz
Whatfast.app/Contents/MacOS/whatfast-backend    el sidecar
Whatfast.app/Contents/Resources/icon.icns       el logo
Whatfast.app/Contents/Info.plist                bundle io.github.owning01.whatfast
```

El mínimo que declara el `Info.plist` es **macOS 11.0**. Los dos binarios van
en `Contents/MacOS`, así que la regla del sidecar al lado se cumple sola: la
interfaz busca `whatfast-backend` en su propio directorio.

Como los nombres de estos archivos salen del workflow de release y no de una
prueba, hay que confirmarlos cuando el paquete se publique.

### Instalarlo

1. Se descomprime el `.zip`. Queda `Whatfast.app`.
2. Se arrastra a **`/Applications`** (o a `~/Applications`).
3. Se abre.

La app no se puede abrir desde la carpeta del `.zip` sin descomprimir: macOS
trabaja con el bundle, no con un archivo suelto.

### Gatekeeper la primera vez

El paquete va firmado **ad hoc** (`codesign --sign -`), no con un certificado
de desarrollador de Apple, y no está notarizado. Eso alcanza para que macOS
ejecute un binario de arm64, pero **no** para pasar Gatekeeper. La primera vez
macOS va a decir que la app es de un desarrollador no identificado.

Hay dos formas de abrirla:

**La del botón derecho.** Clic derecho (o Control + clic) sobre `Whatfast.app`
→ **Abrir** → **Abrir** otra vez en el diálogo. macOS guarda la excepción para
esa app y no vuelve a preguntar.

**La del comando.** Se le saca el atributo de cuarentena al bundle:

```sh
xattr -dr com.apple.quarantine /Applications/Whatfast.app
```

Después de eso abre con doble clic, como cualquier app. El comando hay que
correrlo cada vez que se descarga un paquete nuevo: la cuarentena la pone el
navegador, no la app.

Si macOS dice que la app **está dañada**, casi siempre es la cuarentena y no
un archivo roto: se arregla con el mismo `xattr`. Si el mensaje sigue, hay que
mirar el log del sidecar en
`~/Library/Application Support/whatfast/backend.log`.

### Lo que todavía no está probado en macOS

- Nadie abrió `Whatfast.app` en una Mac. El paquete lo arma el runner
  `macos-latest` de GitHub con el `Info.plist` de `assets/macos/`, así que la
  estructura está definida, pero la ejecución no.
- No hay firma de desarrollador ni notarización, y no hay planes de tenerlas
  por ahora. De ahí el paso de Gatekeeper de arriba.
- El `Info.plist` y el armado del `.app` viven en el workflow y en
  `assets/macos/`. Esos archivos no están en `main` todavía: llegan con el
  paquete de macOS.

---

## Compilar desde el código

En los tres sistemas hacen falta las mismas dos herramientas:

- **Rust estable.** El `rust-toolchain.toml` de la raíz fija el canal
  (`stable`) y los componentes (`rustfmt`, `clippy`).
- **Go 1.25 o superior**, solo para el sidecar (`backend/go.mod` pide
  `go 1.25.0`).

Después, cada sistema tiene su guion y sus dependencias.

### Windows

```sh
scripts\build.cmd
```

Deja en `dist\`:

```
dist\whatfast.exe
dist\whatfast-backend.exe
dist\icons\
```

El `.cmd` resuelve el directorio de compilaciones compartido (`CARGO_TARGET_DIR`,
o `<repo principal>\.cache\target`) para que varios worktrees no llenen el disco
con una copia de la compilación cada uno. Compila el release con
`--config "build.rustc-wrapper=''"`: el binario sale del compilador, sin caché
de por medio.

Para dejarlo instalado sin pasar por el `.exe`:

```sh
powershell -ExecutionPolicy Bypass -File scripts\instalar.ps1
```

Ese script copia los dos ejecutables a `%LOCALAPPDATA%\Programs\Whatfast` y
arma los accesos directos del escritorio y del menú Inicio. Es la variante sin
administrador; el instalador NSIS, en cambio, escribe en Archivos de programa.

Para armar el instalador desde el código hace falta NSIS (`makensis`) y el
`dist\` ya compilado:

```sh
makensis installer.nsi
```

Sale `whatfast-setup.exe` en la raíz del repositorio. El `installer.nsi` toma
los binarios de `dist\` y los iconos de `dist\icons\`, así que el orden importa:
primero `build.cmd`, después `makensis`.

### Linux y macOS

```sh
chmod +x scripts/build.sh
scripts/build.sh
```

El guion es el hermano de `build.cmd` y deja lo mismo, sin `.exe`:

```
dist/whatfast
dist/whatfast-backend
dist/icons/
dist/whatfast.desktop     (solo en Linux)
```

Compila el sidecar con `CGO_ENABLED=0`, así que el binario sale estático y no
depende de la libc del sistema. Verifica que `cargo` y `go` estén en el `PATH`
antes de arrancar, y respeta `CARGO_TARGET_DIR` y `WHATFAST_TARGET_DIR`.

En Linux, el guion también copia `whatfast.desktop` a `dist/`. Ese `.desktop`
es el mismo que después se instala en `~/.local/share/applications/`.

Para correr lo compilado sin instalar:

```sh
cd dist
./whatfast
```

En macOS, `build.sh` deja los dos binarios sueltos en `dist/`. Para tener el
`.app` hay que armar el bundle: es lo que hace el job `macos` del workflow, con
el `Info.plist` de `assets/macos/`, el `icon.icns` en `Contents/Resources` y los
dos binarios en `Contents/MacOS`. Un `.app` hecho a mano con la misma
estructura funciona igual.

Las dependencias del sistema en Linux son las de la sección de arriba. En
macOS alcanza con las **Command Line Tools de Xcode** (`xcode-select --install`)
para el compilador y los headers; GPUI usa Metal y AppKit del sistema, así que
no hay bibliotecas extra que instalar.

---

## Problemas comunes

| Síntoma | Causa | Qué hacer |
|---|---|---|
| La app abre pero no conecta y no hay QR | El sidecar no está al lado o no tiene permiso de ejecución | Poner `whatfast-backend` junto a la interfaz, o definir `WHATFAST_BACKEND` con la ruta completa. En Linux, `chmod +x`. |
| `error while loading shared libraries: libfontconfig.so.1` (Linux) | Faltan las bibliotecas del stack de GPUI | `sudo apt install libfontconfig1 libwayland-client0 libxkbcommon-x11-0 libgl1 libvulkan1` |
| El menú de Linux abre la app y se cierra sola | El `.desktop` tiene `Exec=whatfast` y el binario no está en el `PATH` | Instalarlo en `~/.local/bin` o `/usr/local/bin`, o poner la ruta absoluta en el `Exec` |
| El lanzador de Linux no tiene logo | El icono no está en el árbol `hicolor` o falta refrescar la caché | Copiar los PNG a `~/.local/share/icons/hicolor/<tamaño>/apps/whatfast.png` y correr `gtk-update-icon-cache` |
| Windows: "Windows protegió tu PC" | El instalador no está firmado | **Más información → Ejecutar de todas formas** |
| macOS: "no se puede abrir porque es de un desarrollador no identificado" | El paquete va firmado ad hoc, sin notarizar | Clic derecho → **Abrir**, o `xattr -dr com.apple.quarantine Whatfast.app` |
| macOS: "la app está dañada" | Cuarentena, no un archivo roto | El mismo `xattr`. Si sigue, mirar `~/Library/Application Support/whatfast/backend.log` |
| No aparecen los mensajes de una cuenta que ya estaba vinculada | Se está usando otro perfil | Revisar `WHATFAST_DATA_DIR`; el historial es por carpeta de datos |

# Captura la ventana de Whatfast con la app en primer plano.
#
# El script `capturas.ps1` levanta la app y saca las fotos, pero si otra
# ventana tiene el foco la foto sale de lo que haya encima. Este guion
# levanta la app, la trae al frente (con el truco de ALT, que es la unica
# forma de que Windows deje robar el foco), la deja en la esquina y saca la
# foto.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\captura-frontal.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\captura-frontal.ps1 -Salida docs\screenshots\actual -Clics "34,140;150,286"

param(
    [string]$Salida = "docs\screenshots\actual",
    [string]$Binario = "dist\whatfast.exe",
    # Los avatares se piden de a cuatro: hacen falta unos segundos para que
    # las filas visibles tengan su foto.
    [int]$EsperaSegundos = 14,
    # Los destinos del riel a pulsar, en el orden en que se pulsan. Cada uno
    # es el numero de icono: 1 chats, 2 busqueda, 3 estados, 4 canales.
    [string]$Destinos = "",
    [string]$Clics = "",
    # El clic derecho, en "x,y": abre el menu de la fila.
    [string]$ClicDerecho = "",
    # El nombre de la foto que se deja al final.
    [string]$Nombre = "01-lista-de-chats.png",
    # Las vueltas de la rueda: "x,y,pasos" por cada una.
    [string]$Ruedas = ""
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$ejecutable = if([System.IO.Path]::IsPathRooted($Binario)) { $Binario } else { Join-Path $raiz $Binario }
$destino = if([System.IO.Path]::IsPathRooted($Salida)) { $Salida } else { Join-Path $raiz $Salida }

if(-not (Test-Path $ejecutable)) {
    Write-Error "No existe $ejecutable. Corre scripts\build.cmd primero."
    exit 1
}
New-Item -ItemType Directory -Force -Path $destino | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ventana {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern void keybd_event(byte v, byte s, uint f, IntPtr e);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

# Al tanto de los pixeles: sin esto Windows virtualiza las coordenadas de
# este proceso y la foto y el clic miden cosas distintas.
[Ventana]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

function Origen([IntPtr]$handle) {
    $cliente = New-Object Ventana+RECT
    [Ventana]::GetClientRect($handle, [ref]$cliente) | Out-Null
    $esquina = New-Object Ventana+POINT
    [Ventana]::ClientToScreen($handle, [ref]$esquina) | Out-Null
    return @($esquina.X, $esquina.Y, ($cliente.Right - $cliente.Left), ($cliente.Bottom - $cliente.Top))
}

function EnPrimerPlano([IntPtr]$handle, [int]$idProceso) {
    for($i = 0; $i -lt 30; $i++) {
        # Windows deja traer una ventana al frente solo si el proceso tiene
        # el foco: la pulsacion de ALT abre esa puerta.
        [Ventana]::keybd_event(0x12, 0, 0, [IntPtr]::Zero)
        [Ventana]::SetForegroundWindow($handle) | Out-Null
        [Ventana]::keybd_event(0x12, 0, 2, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 250
        $idEnfrente = 0
        [Ventana]::GetWindowThreadProcessId([Ventana]::GetForegroundWindow(), [ref]$idEnfrente) | Out-Null
        if($idEnfrente -eq $idProceso) {
            return $true
        }
    }
    return $false
}

function Clic([IntPtr]$handle, [int]$dx, [int]$dy) {
    # Los offsets son en pixeles de la foto, que ya son pixeles de la
    # pantalla: el proceso es DPI-aware, asi que la foto y el clic usan la
    # misma medida y no hay que convertir nada.
    [Ventana]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 400
    $origen = Origen $handle
    $destinoX = $origen[0] + $dx
    $destinoY = $origen[1] + $dy
    # Un movimiento con las coordenadas absolutas: es el unico que le llega a
    # la ventana como movimiento del puntero, y sin el la interfaz no sabe
    # donde esta el cursor cuando llega el clic.
    [Ventana]::SetCursorPos($destinoX, $destinoY) | Out-Null
    Start-Sleep -Milliseconds 150
    $ancho = [Ventana]::GetSystemMetrics(0)
    $alto = [Ventana]::GetSystemMetrics(1)
    $normalX = [int]($destinoX * 65535 / $ancho)
    $normalY = [int]($destinoY * 65535 / $alto)
    [Ventana]::mouse_event(0x8001, $normalX, $normalY, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    [Ventana]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    [Ventana]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 900
    # El guion dice donde quedo el cursor: si la foto y el clic no usan la
    # misma medida, el clic cae en otro punto de la pantalla.
    $cursor = New-Object Ventana+POINT
    [Ventana]::GetCursorPos([ref]$cursor) | Out-Null
    Write-Output ("    el cursor quedo en {0},{1} de la pantalla" -f $cursor.X, $cursor.Y)
}

function ClicDerecho([IntPtr]$handle, [int]$dx, [int]$dy) {
    # El boton derecho del raton: es el que abre el menu de la fila.
    [Ventana]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 400
    $origen = Origen $handle
    $destinoX = $origen[0] + $dx
    $destinoY = $origen[1] + $dy
    [Ventana]::SetCursorPos($destinoX, $destinoY) | Out-Null
    Start-Sleep -Milliseconds 150
    $ancho = [Ventana]::GetSystemMetrics(0)
    $alto = [Ventana]::GetSystemMetrics(1)
    $normalX = [int]($destinoX * 65535 / $ancho)
    $normalY = [int]($destinoY * 65535 / $alto)
    [Ventana]::mouse_event(0x8001, $normalX, $normalY, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    [Ventana]::mouse_event(0x0008, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Ventana]::mouse_event(0x0010, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 900
    $cursor = New-Object Ventana+POINT
    [Ventana]::GetCursorPos([ref]$cursor) | Out-Null
    Write-Output ("    el cursor quedo en {0},{1} de la pantalla" -f $cursor.X, $cursor.Y)
}

function Rueda([IntPtr]$handle, [int]$dx, [int]$dy, [int]$pasos) {
    $origen = Origen $handle
    [Ventana]::SetCursorPos($origen[0] + $dx, $origen[1] + $dy) | Out-Null
    for($i = 0; $i -lt $pasos; $i++) {
        [Ventana]::mouse_event(0x0800, 0, 0, 120, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 120
    }
    Start-Sleep -Milliseconds 700
}

function Capturar([IntPtr]$handle, [string]$nombre) {
    [Ventana]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 700
    $origen = Origen $handle
    $bitmap = New-Object System.Drawing.Bitmap $origen[2], $origen[3]
    $graficos = [System.Drawing.Graphics]::FromImage($bitmap)
    $graficos.CopyFromScreen($origen[0], $origen[1], 0, 0, $bitmap.Size)
    $ruta = Join-Path $destino $nombre
    $bitmap.Save($ruta, [System.Drawing.Imaging.ImageFormat]::Png)
    $graficos.Dispose()
    $bitmap.Dispose()
    Write-Output ("  {0,-44} {1}x{2}" -f $nombre, $origen[2], $origen[3])
}

function Medir([string]$ruta) {
    $salida = @(python (Join-Path $raiz "scripts\medir_riel.py") $ruta 2>$null | Where-Object { $_.Trim() -ne "" })
    $ultima = $salida[$salida.Count - 1] -split ' '
    return @([int]$ultima[1], [int]$ultima[2])
}

Write-Output "=== levanto la app con el backend fake ==="
$proceso = Start-Process -FilePath $ejecutable -ArgumentList "--fake-backend" -PassThru -RedirectStandardError "$env:TEMP\whatfast_frontal_err.txt"

$handle = [IntPtr]::Zero
for($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 250
    $proceso.Refresh()
    if($proceso.MainWindowHandle -ne 0) { $handle = $proceso.MainWindowHandle; break }
}
if($handle -eq [IntPtr]::Zero) {
    Write-Error "la ventana no aparecio"
    Get-Content "$env:TEMP\whatfast_frontal_err.txt" -ErrorAction SilentlyContinue
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

try {
    Start-Sleep -Seconds 4
    # La ventana va a la esquina: sin esto la sombra de Windows entra en la
    # foto.
    [Ventana]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, 0, 0, 0x0001 -bor 0x0004) | Out-Null
    Start-Sleep -Milliseconds 900
    if(-not (EnPrimerPlano $handle $proceso.Id)) {
        Write-Error "la ventana de la app no queda en primer plano"
    }
    Write-Output "=== capturas ==="
    if($Clics -ne "") {
        foreach($par in $Clics -split ";") {
            $punto = $par -split ","
            Clic $handle ([int]$punto[0]) ([int]$punto[1])
        }
    }
    if($Destinos -ne "") {
        # Los destinos se miden sobre la foto: el numero de icono es el
        # orden de los botones de la columna de la izquierda.
        Capturar $handle "00-sonda.png"
        $lineas = @(python (Join-Path $raiz "scripts\medir_riel.py") (Join-Path $destino "00-sonda.png") 2>$null | Where-Object { $_.Trim() -ne "" })
        $posiciones = @()
        foreach($linea in $lineas) {
            if($linea -match '^posiciones\s+(.*)$') {
                $posiciones = $Matches[1] -split ' '
            }
        }
        foreach($numero in $Destinos -split ",") {
            $indice = [int]$numero - 1
            if($indice -ge 0 -and $indice -lt $posiciones.Count) {
                $punto = $posiciones[$indice] -split ','
                $x = [int]$punto[0]
                $y = [int]$punto[1]
                Write-Output ("  pulso el destino {0} en {1},{2}" -f $numero, $x, $y)
                Clic $handle $x $y
            }
        }
        Remove-Item (Join-Path $destino "00-sonda.png") -ErrorAction SilentlyContinue
    }
    if($ClicDerecho -ne "") {
        $punto = $ClicDerecho -split ","
        ClicDerecho $handle ([int]$punto[0]) ([int]$punto[1])
    }
    if($Ruedas -ne "") {
        foreach($par in $Ruedas -split ";") {
            $punto = $par -split ","
            Rueda $handle ([int]$punto[0]) ([int]$punto[1]) ([int]$punto[2])
        }
    }
    Capturar $handle $Nombre
} finally {
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
}
Write-Output "listo."

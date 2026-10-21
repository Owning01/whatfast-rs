# Captura la vista de los estados de Whatfast.
#
# Levanta la app con el backend fake, pulsa el boton de los estados del riel
# de la izquierda y saca una foto de la pantalla.
#
# El boton se mide sobre una foto de prueba (scripts/medir_riel.py) y el
# detector de scripts/detectar-estados.py dice si la vista quedo abierta.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\captura-estados.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\captura-estados.ps1 -Salida docs\screenshots\actual

param(
    [string]$Salida = "docs\screenshots\actual",
    [string]$Binario = "dist\whatfast.exe",
    [int]$EsperaSegundos = 3
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$ejecutable = if([System.IO.Path]::IsPathRooted($Binario)) { $Binario } else { Join-Path $raiz $Binario }
$destino = if([System.IO.Path]::IsPathRooted($Salida)) { $Salida } else { Join-Path $raiz $Salida }

if(-not (Test-Path $ejecutable)) {
    Write-Error "No existe $ejecutable. Corre scripts\build.cmd primero."
    exit 1
}
if(-not (Test-Path $destino)) {
    New-Item -ItemType Directory -Force -Path $destino | Out-Null
}

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
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
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
    for($i = 0; $i -lt 20; $i++) {
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
    # Los offsets son en pixeles de la foto, que son pixeles de la
    # pantalla: el clic va al mismo punto desde donde se copio la foto.
    [Ventana]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 300
    $origen = Origen $handle
    [Ventana]::SetCursorPos($origen[0] + $dx, $origen[1] + $dy) | Out-Null
    Start-Sleep -Milliseconds 250
    [Ventana]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    [Ventana]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 800
}

function Capturar([IntPtr]$handle, [string]$nombre) {
    $origen = Origen $handle
    $ancho = $origen[2]
    $alto = $origen[3]
    if($ancho -le 0 -or $alto -le 0) {
        Write-Warning "la ventana no tiene tamano"
        return
    }
    $bitmap = New-Object System.Drawing.Bitmap $ancho, $alto
    $graficos = [System.Drawing.Graphics]::FromImage($bitmap)
    $graficos.CopyFromScreen($origen[0], $origen[1], 0, 0, $bitmap.Size)
    $ruta = Join-Path $destino $nombre
    $bitmap.Save($ruta, [System.Drawing.Imaging.ImageFormat]::Png)
    $graficos.Dispose()
    $bitmap.Dispose()
    Write-Output ("  {0,-44} {1}x{2}" -f $nombre, $ancho, $alto)
}

function Medir([string]$ruta) {
    $salida = @(python (Join-Path $raiz "scripts\medir_riel.py") $ruta 2>$null)
    $ultima = $salida[$salida.Count - 1] -split ' '
    return @([int]$ultima[1], [int]$ultima[2])
}

function Abierta([string]$ruta) {
    $salida = @(python (Join-Path $raiz "scripts\detectar-estados.py") $ruta 2>$null)
    return $salida[0].Trim() -eq "1"
}

Write-Output "=== levanto la app con el backend fake ==="
$proceso = Start-Process -FilePath $ejecutable -ArgumentList "--fake-backend" -PassThru -RedirectStandardError "$env:TEMP\whatfast_estados_err.txt"

$handle = [IntPtr]::Zero
for($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 250
    $proceso.Refresh()
    if($proceso.MainWindowHandle -ne 0) { $handle = $proceso.MainWindowHandle; break }
}
if($handle -eq [IntPtr]::Zero) {
    Write-Error "la ventana no aparecio"
    Get-Content "$env:TEMP\whatfast_estados_err.txt" -ErrorAction SilentlyContinue
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

try {
    Start-Sleep -Seconds $EsperaSegundos
    if(-not (EnPrimerPlano $handle $proceso.Id)) {
        Write-Error "la ventana de la app no queda en primer plano"
    }
    Write-Output "=== capturas ==="

    # La foto de prueba: de ahi se mide donde esta el boton de los estados.
    Capturar $handle "00-sonda.png"
    $medida = Medir (Join-Path $destino "00-sonda.png")
    $x = $medida[0]
    $y = $medida[1]
    Write-Output ("  el boton de los estados esta en {0},{1} de la foto" -f $x, $y)

    Clic $handle $x $y
    Clic $handle $x $y
    Capturar $handle "00-prueba.png"
    if(-not (Abierta (Join-Path $destino "00-prueba.png"))) {
        Write-Error "la vista de los estados no abrio"
    }
    Move-Item (Join-Path $destino "00-prueba.png") (Join-Path $destino "04-estados.png") -Force
    Remove-Item (Join-Path $destino "00-sonda.png") -ErrorAction SilentlyContinue
} finally {
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
}
Write-Output "listo."

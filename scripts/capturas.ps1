# Captura las pantallas de Whatfast para el README.
#
# Levanta la app con el backend fake (sin cuenta, sin QR, sin red), espera a
# que la ventana aparezca y saca una foto de cada pantalla.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\capturas.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\capturas.ps1 -Salida docs\screenshots

param(
    [string]$Salida = "docs\screenshots",
    [string]$Binario = "dist\whatfast.exe",
    [int]$EsperaSegundos = 3
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$ejecutable = Join-Path $raiz $Binario
$destino = Join-Path $raiz $Salida

if(-not (Test-Path $ejecutable)) {
    Write-Error "No existe $ejecutable. Corré scripts\build.cmd primero."
    exit 1
}
if(-not (Test-Path $destino)) {
    New-Item -ItemType Directory -Force -Path $destino | Out-Null
}

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ventana {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

function Capturar([IntPtr]$handle, [string]$nombre) {
    $rect = New-Object Ventana+RECT
    if(-not [Ventana]::GetWindowRect($handle, [ref]$rect)) {
        Write-Warning "no se pudo leer el rectangulo de $nombre"
        return
    }
    $ancho = $rect.Right - $rect.Left
    $alto = $rect.Bottom - $rect.Top
    if($ancho -le 0 -or $alto -le 0) {
        Write-Warning "la ventana $nombre no tiene tamano"
        return
    }
    $bitmap = New-Object System.Drawing.Bitmap $ancho, $alto
    $graficos = [System.Drawing.Graphics]::FromImage($bitmap)
    $graficos.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $ruta = Join-Path $destino $nombre
    $bitmap.Save($ruta, [System.Drawing.Imaging.ImageFormat]::Png)
    $graficos.Dispose()
    $bitmap.Dispose()
    Write-Output ("  {0,-44} {1}x{2}" -f $nombre, $ancho, $alto)
}

function Clic([IntPtr]$handle, [int]$dx, [int]$dy) {
    $rect = New-Object Ventana+RECT
    [Ventana]::GetWindowRect($handle, [ref]$rect) | Out-Null
    [Ventana]::SetCursorPos($rect.Left + $dx, $rect.Top + $dy) | Out-Null
    Start-Sleep -Milliseconds 250
    [Ventana]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    [Ventana]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 700
}

Write-Output "=== levanto la app con el backend fake ==="
$proceso = Start-Process -FilePath $ejecutable -ArgumentList "--fake-backend" -PassThru -RedirectStandardError "$env:TEMP\whatfast_capturas_err.txt"

$handle = [IntPtr]::Zero
for($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 250
    $proceso.Refresh()
    if($proceso.MainWindowHandle -ne 0) { $handle = $proceso.MainWindowHandle; break }
}
if($handle -eq [IntPtr]::Zero) {
    Write-Error "la ventana no aparecio"
    Get-Content "$env:TEMP\whatfast_capturas_err.txt" -ErrorAction SilentlyContinue
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# The window goes to the top left corner: the shadow that Windows draws
# around it would otherwise be part of the capture.
[Ventana]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, 0, 0, 0x0001 -bor 0x0004) | Out-Null
[Ventana]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Seconds 4

Write-Output "=== capturas ==="
Capturar $handle "01-lista-de-chats.png"

# una conversacion
Clic $handle 150 220
Start-Sleep -Seconds 2
Capturar $handle "02-conversacion.png"

# el buscador: se hace clic en el campo y se escribe
Clic $handle 220 150
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait("fast")
Start-Sleep -Seconds 2
Capturar $handle "03-busqueda.png"
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
Start-Sleep -Seconds 1

# los ajustes (el boton de la rueda, en la cabecera del panel lateral)
Clic $handle 302 64
Start-Sleep -Seconds 2
Capturar $handle "04-ajustes.png"

Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
Write-Output "listo."

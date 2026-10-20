# Captura las ventanas de Whatfast para el README.
#
# Levanta la app con el backend fake (sin cuenta, sin QR, sin red), espera a
# que la ventana aparezca y guarda la imagen.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\capturas.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\capturas.ps1 -Salida docs\screenshots

param(
    [string]$Salida = "docs\screenshots",
    [string]$Binario = "dist\whatfast.exe",
    [int]$EsperaSegundos = 4
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
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ventana {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
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
    Write-Output ("  {0,-40} {1}x{2}" -f $ruta.Replace("$raiz\", ""), $ancho, $alto)
}

Write-Output "=== levanto la app con el backend fake ==="
$proceso = Start-Process -FilePath $ejecutable -ArgumentList "--fake-backend" -PassThru

$handle = [IntPtr]::Zero
for($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 250
    $proceso.Refresh()
    if($proceso.MainWindowHandle -ne 0) { $handle = $proceso.MainWindowHandle; break }
}
if($handle -eq [IntPtr]::Zero) {
    Write-Error "la ventana no aparecio"
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

[Ventana]::ShowWindow($handle, 3) | Out-Null   # maximizada
[Ventana]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Seconds $EsperaSegundos

Write-Output "=== capturas ==="
Capturar $handle "01-lista-de-chats.png"

Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
Write-Output "listo."

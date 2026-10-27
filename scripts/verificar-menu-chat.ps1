# Abre una conversación y el menú de tres puntos de su cabecera, y guarda la
# captura: el menú tiene filas que la app todavía no puede hacer y no puede
# caerse al dibujarlas.

param(
    [string]$Binario = "target\debug\whatfast-ui.exe"
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$ejecutable = if ([System.IO.Path]::IsPathRooted($Binario)) { $Binario } else { Join-Path $raiz $Binario }
$destino = Join-Path $raiz "capturas\review"
if (-not (Test-Path $destino)) { New-Item -ItemType Directory -Force -Path $destino | Out-Null }

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ventana {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

function Origen([IntPtr]$handle) {
    $cliente = New-Object Ventana+RECT
    [Ventana]::GetClientRect($handle, [ref]$cliente) | Out-Null
    $esquina = New-Object Ventana+POINT
    [Ventana]::ClientToScreen($handle, [ref]$esquina) | Out-Null
    $escala = [Ventana]::GetDpiForWindow($handle) / 96.0
    return @($esquina.X, $esquina.Y, $escala, ($cliente.Right - $cliente.Left), ($cliente.Bottom - $cliente.Top))
}

function Clic([IntPtr]$handle, [int]$dx, [int]$dy) {
    [Ventana]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 400
    $origen = Origen $handle
    [Ventana]::SetCursorPos($origen[0] + [int]($dx * $origen[2]), $origen[1] + [int]($dy * $origen[2])) | Out-Null
    Start-Sleep -Milliseconds 250
    [Ventana]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    [Ventana]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 900
}

function Capturar([IntPtr]$handle, [string]$nombre) {
    [Ventana]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 500
    $origen = Origen $handle
    $escala = $origen[2]
    $ancho = [int]($origen[3] * $escala)
    $alto = [int]($origen[4] * $escala)
    Write-Output ("client={0}x{1} escala={2} ancho={3} alto={4}" -f $origen[3], $origen[4], $escala, $ancho, $alto)
    $bitmap = New-Object System.Drawing.Bitmap $ancho, $alto
    $graficos = [System.Drawing.Graphics]::FromImage($bitmap)
    $graficos.CopyFromScreen($origen[0], $origen[1], 0, 0, $bitmap.Size)
    $bitmap.Save((Join-Path $destino $nombre), [System.Drawing.Imaging.ImageFormat]::Png)
    $graficos.Dispose()
    $bitmap.Dispose()
    Write-Output ("guardada {0} ({1}x{2})" -f $nombre, $ancho, $alto)
}

$err = Join-Path $env:TEMP "whatfast_menu_err.txt"
$out = Join-Path $env:TEMP "whatfast_menu_out.txt"
$proceso = Start-Process -FilePath $ejecutable -ArgumentList "--fake-backend" -PassThru -RedirectStandardError $err -RedirectStandardOutput $out
$handle = [IntPtr]::Zero
for ($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 250
    $proceso.Refresh()
    if ($proceso.HasExited) { break }
    if ($proceso.MainWindowHandle -ne 0) { $handle = $proceso.MainWindowHandle; break }
}
if ($handle -eq [IntPtr]::Zero) {
    Write-Output "la ventana no aparecio"
    Get-Content $err -ErrorAction SilentlyContinue
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
    exit 1
}
[Ventana]::ShowWindow($handle, 3) | Out-Null
Start-Sleep -Seconds 5
$proceso.Refresh()
Write-Output ("tras redimensionar: exited={0} code={1} handle={2} principal={3}" -f $proceso.HasExited, $proceso.ExitCode, $handle, $proceso.MainWindowHandle)
Get-Content $err -ErrorAction SilentlyContinue | Select-Object -Last 10
Get-Content $out -ErrorAction SilentlyContinue | Select-Object -Last 20
$origen = Origen $handle
Write-Output ("client tras redimensionar: {0}x{1}" -f $origen[3], $origen[4])

Clic $handle 150 286
Start-Sleep -Seconds 2
$proceso.Refresh()
Write-Output ("antes del menu: exited={0} principal={1}" -f $proceso.HasExited, $proceso.MainWindowHandle)
$origen = Origen $handle
Write-Output ("client antes del menu: {0}x{1}" -f $origen[3], $origen[4])
# El boton de tres puntos de la cabecera, pegado al borde derecho.
Clic $handle ($origen[3] - 36) 82
Start-Sleep -Seconds 1
Capturar $handle "07-menu-chat.png"

Start-Sleep -Milliseconds 500
$proceso.Refresh()
if ($proceso.HasExited) {
    Write-Output ("la app se cerro con codigo {0}" -f $proceso.ExitCode)
} else {
    Write-Output "la app sigue abierta"
}
Get-Content $err -ErrorAction SilentlyContinue | Select-Object -Last 20
Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue

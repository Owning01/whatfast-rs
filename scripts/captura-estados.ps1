# Captura la vista de los estados de Whatfast.
#
# Levanta la app con el backend fake, abre la vista de los estados con el
# atajo de teclado y saca la foto de la pantalla.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\captura-estados.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\captura-estados.ps1 -Salida docs\screenshots\actual

param(
    [string]$Salida = "docs\screenshots\actual",
    [string]$Binario = "dist\whatfast.exe",
    [int]$EsperaSegundos = 10,
    # El atajo: "ctrl-shift-s" abre los estados, "ctrl-shift-n" los canales y
    # "ctrl-shift-h" los archivados.
    [string]$Atajo = "ctrl-shift-s",
    # El nombre de la foto.
    [string]$Nombre = "04-estados.png"
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$ejecutable = if([System.IO.Path]::IsPathRooted($Binario)) { $Binario } else { Join-Path $raiz $Binario }
$destino = if([System.IO.Path]::IsPathRooted($Salida)) { $Salida } else { Join-Path $raiz $Salida }
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
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

[Ventana]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

# Las teclas del atajo: el control, el shift y la letra.
$letras = @{ "s" = 0x53; "n" = 0x4E; "h" = 0x48 }
$partes = $Atajo -split "-"
$control = $partes -contains "ctrl"
$shift = $partes -contains "shift"
$letra = $letras[$partes[-1].ToLower()]
if(-not $letra) {
    Write-Error "el atajo $Atajo no tiene una letra conocida"
    exit 1
}

function Origen([IntPtr]$handle) {
    $cliente = New-Object Ventana+RECT
    [Ventana]::GetClientRect($handle, [ref]$cliente) | Out-Null
    $esquina = New-Object Ventana+POINT
    [Ventana]::ClientToScreen($handle, [ref]$esquina) | Out-Null
    return @($esquina.X, $esquina.Y, ($cliente.Right - $cliente.Left), ($cliente.Bottom - $cliente.Top))
}

function EnPrimerPlano([IntPtr]$handle, [int]$idProceso) {
    for($i = 0; $i -lt 30; $i++) {
        [Ventana]::keybd_event(0x12, 0, 0, [IntPtr]::Zero)
        [Ventana]::SetForegroundWindow($handle) | Out-Null
        [Ventana]::keybd_event(0x12, 0, 2, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 250
        $idEnfrente = 0
        [Ventana]::GetWindowThreadProcessId([Ventana]::GetForegroundWindow(), [ref]$idEnfrente) | Out-Null
        if($idEnfrente -eq $idProceso) { return $true }
    }
    return $false
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
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

try {
    Start-Sleep -Seconds $EsperaSegundos
    [Ventana]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, 0, 0, 0x0001 -bor 0x0004) | Out-Null
    Start-Sleep -Milliseconds 900
    if(-not (EnPrimerPlano $handle $proceso.Id)) {
        Write-Error "la ventana de la app no queda en primer plano"
    }
    Write-Output "=== capturas ==="

    # El atajo de la vista.
    if($control) {
        [Ventana]::keybd_event(0x11, 0, 0, [IntPtr]::Zero)
    }
    if($shift) {
        [Ventana]::keybd_event(0x10, 0, 0, [IntPtr]::Zero)
    }
    [Ventana]::keybd_event([byte]$letra, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 120
    [Ventana]::keybd_event([byte]$letra, 0, 2, [IntPtr]::Zero)
    if($shift) {
        [Ventana]::keybd_event(0x10, 0, 2, [IntPtr]::Zero)
    }
    if($control) {
        [Ventana]::keybd_event(0x11, 0, 2, [IntPtr]::Zero)
    }
    Start-Sleep -Seconds 2

    $origen = Origen $handle
    $bitmap = New-Object System.Drawing.Bitmap $origen[2], $origen[3]
    $graficos = [System.Drawing.Graphics]::FromImage($bitmap)
    $graficos.CopyFromScreen($origen[0], $origen[1], 0, 0, $bitmap.Size)
    $ruta = Join-Path $destino $Nombre
    $bitmap.Save($ruta, [System.Drawing.Imaging.ImageFormat]::Png)
    $graficos.Dispose()
    $bitmap.Dispose()
    Write-Output ("  {0}  {1}x{2}" -f $Nombre, $origen[2], $origen[3])
} finally {
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
}
Write-Output "listo."

# Diagnostico del clic en el riel de Whatfast.
#
# Levanta la app, mueve el cursor al punto medido y saca la foto SIN
# hacer clic: si el icono queda resaltado, las coordenadas son correctas y
# el problema es el evento del clic.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\diagnostico-riel.ps1

param(
    [string]$Salida = "docs\screenshots\actual",
    [string]$Binario = "dist\whatfast.exe"
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
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

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

function Capturar([IntPtr]$handle, [string]$nombre) {
    $origen = Origen $handle
    $bitmap = New-Object System.Drawing.Bitmap $origen[2], $origen[3]
    $graficos = [System.Drawing.Graphics]::FromImage($bitmap)
    $graficos.CopyFromScreen($origen[0], $origen[1], 0, 0, $bitmap.Size)
    $ruta = Join-Path $destino $nombre
    $bitmap.Save($ruta, [System.Drawing.Imaging.ImageFormat]::Png)
    $graficos.Dispose()
    $bitmap.Dispose()
    Write-Output ("  {0,-30} {1}x{2}" -f $nombre, $origen[2], $origen[3])
}

$proceso = Start-Process -FilePath $ejecutable -ArgumentList "--fake-backend" -PassThru -RedirectStandardError "$env:TEMP\whatfast_diag.txt"
$handle = [IntPtr]::Zero
for($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 250
    $proceso.Refresh()
    if($proceso.MainWindowHandle -ne 0) { $handle = $proceso.MainWindowHandle; break }
}
try {
    Start-Sleep -Seconds 4
    EnPrimerPlano $handle $proceso.Id | Out-Null
    $origen = Origen $handle
    Write-Output "cliente $($origen[0]),$($origen[1]) $($origen[2])x$($origen[3])"

    Capturar $handle "diag-antes.png"
    $salida = @(python (Join-Path $raiz "scripts\medir_riel.py") (Join-Path $destino "diag-antes.png") 2>$null)
    $ultima = $salida[$salida.Count - 1] -split ' '
    $x = [int]$ultima[1]
    $y = [int]$ultima[2]
    Write-Output "el boton de los estados esta en $x,$y de la foto"

    $pantallaX = $origen[0] + $x
    $pantallaY = $origen[1] + $y
    [Ventana]::SetCursorPos($pantallaX, $pantallaY) | Out-Null
    Start-Sleep -Milliseconds 800
    $cursor = New-Object Ventana+POINT
    [Ventana]::GetCursorPos([ref]$cursor) | Out-Null
    Write-Output "el cursor quedo en $($cursor.X),$($cursor.Y) (se pidio $pantallaX,$pantallaY)"
    Capturar $handle "diag-hover.png"
} finally {
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
}
Write-Output "listo."

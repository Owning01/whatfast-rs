# Mide cuando aparece la lista de chats: la primera diferencia grande de la
# imagen de la barra lateral.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\medir-lista.ps1

param(
    [string]$Binario = "dist\whatfast.exe",
    [int]$Corridas = 3
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$ejecutable = Join-Path $raiz $Binario

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class VentanaMedida {
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

function Suma([System.Drawing.Bitmap]$bm) {
    $total = 0
    for($x = 0; $x -lt $bm.Width; $x += 2) {
        for($y = 0; $y -lt $bm.Height; $y += 2) {
            $pixel = $bm.GetPixel($x, $y)
            $total += $pixel.R + $pixel.G + $pixel.B
        }
    }
    return $total
}

$tiempos = @()
for($i = 1; $i -le $Corridas; $i++) {
    $reloj = [Diagnostics.Stopwatch]::StartNew()
    $proceso = Start-Process -FilePath $ejecutable -ArgumentList "--fake-backend" -PassThru
    $anterior = $null
    $cambio = 0.0
    while($reloj.Elapsed.TotalSeconds -lt 20) {
        Start-Sleep -Milliseconds 80
        $proceso.Refresh()
        if($proceso.MainWindowHandle -eq 0) { continue }
        $ventana = $proceso.MainWindowHandle
        $cliente = New-Object VentanaMedida+RECT
        [VentanaMedida]::GetClientRect($ventana, [ref]$cliente) | Out-Null
        $esquina = New-Object VentanaMedida+POINT
        [VentanaMedida]::ClientToScreen($ventana, [ref]$esquina) | Out-Null
        $escala = [VentanaMedida]::GetDpiForWindow($ventana) / 96.0
        # a piece of the list of the chats, in the logical pixels of the app
        $imagen = New-Object System.Drawing.Bitmap 60, 40
        $graficos = [System.Drawing.Graphics]::FromImage($imagen)
        $graficos.CopyFromScreen(
            $esquina.X + [int](150 * $escala),
            $esquina.Y + [int](300 * $escala),
            0, 0, $imagen.Size)
        $actual = Suma $imagen
        $graficos.Dispose()
        $imagen.Dispose()
        if($null -ne $anterior -and [math]::Abs($actual - $anterior) -gt 2000) {
            $cambio = $reloj.Elapsed.TotalSeconds
            break
        }
        $anterior = $actual
    }
    $tiempos += $cambio
    Write-Output ("  corrida {0}: la lista de chats aparece a los {1:N2} s" -f $i, $cambio)
    Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue
    Get-Process whatfast-backend -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
}

$orden = $tiempos | Sort-Object
$mediana = $orden[[int]($orden.Count / 2)]
Write-Output ("  mediana: {0:N2} s" -f $mediana)

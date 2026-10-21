# Trae la ventana de Whatfast al frente y dice donde esta, en pixeles de la
# pantalla.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\ventana-abierta.ps1

param(
    [int]$IdProceso = 0
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ventana {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

# Al tanto de los pixeles: las coordenadas de la ventana son las de la
# pantalla, no las virtualizadas.
[Ventana]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

if($IdProceso -eq 0) {
    $proceso = Get-Process whatfast-ui -ErrorAction SilentlyContinue | Select-Object -First 1
    if(-not $proceso) {
        $proceso = Get-Process whatfast -ErrorAction SilentlyContinue | Select-Object -First 1
    }
} else {
    $proceso = Get-Process -Id $IdProceso -ErrorAction SilentlyContinue
}
if(-not $proceso) {
    Write-Error "no esta corriendo la app"
    exit 1
}

$proceso.Refresh()
$handle = $proceso.MainWindowHandle
if($handle -eq 0) {
    Write-Error "la ventana no aparece todavia"
    exit 1
}

[Ventana]::ShowWindow($handle, 9) | Out-Null
[Ventana]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Milliseconds 400

$rect = New-Object Ventana+RECT
[Ventana]::GetWindowRect($handle, [ref]$rect) | Out-Null
$cliente = New-Object Ventana+RECT
[Ventana]::GetClientRect($handle, [ref]$cliente) | Out-Null
$esquina = New-Object Ventana+POINT
[Ventana]::ClientToScreen($handle, [ref]$esquina) | Out-Null
$escala = [Ventana]::GetDpiForWindow($handle) / 96.0

Write-Output "ventana  $($rect.Left),$($rect.Top) - $($rect.Right),$($rect.Bottom)"
Write-Output "cliente  $($esquina.X),$($esquina.Y)  $($cliente.Right - $cliente.Left)x$($cliente.Bottom - $cliente.Top)"
Write-Output "escala   $escala"

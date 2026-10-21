# Mide el arranque y la RAM de Whatfast.
#
# Lo que hace:
#   1. lanza el binario de release con el backend fake (sin cuenta, sin QR)
#   2. cronometra hasta que aparece la ventana
#   3. espera 30 s y suma el working set del arbol de procesos
#   4. imprime los tamanos de los dos ejecutables
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\medir.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\medir.ps1 -Binario dist\whatfast.exe

param(
    [string]$Binario = "dist\whatfast.exe",
    [int]$EsperaSegundos = 30,
    # Con el sidecar real: los dos ejecutables de verdad, como en la tabla del
    # README. Sin el, se usa el backend de demostracion (sin cuenta, sin QR).
    [switch]$ConSidecar
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$ejecutable = Join-Path $raiz $Binario

if(-not (Test-Path $ejecutable)) {
    Write-Error "No existe el binario $ejecutable. Corré scripts\build.cmd primero."
    exit 1
}

$sidecar = Join-Path (Split-Path $ejecutable) "whatfast-backend.exe"

Write-Output "=== tamano ==="
foreach($f in @($ejecutable, $sidecar)) {
    if(Test-Path $f) {
        $item = Get-Item $f
        Write-Output ("  {0,-46} {1,10:N0} bytes" -f $item.Name, $item.Length)
    }
}
$total = 0
foreach($f in @($ejecutable, $sidecar)) {
    if(Test-Path $f) { $total += (Get-Item $f).Length }
}
Write-Output ("  {0,-46} {1,10:N0} bytes" -f "TOTAL", $total)

Write-Output ""
Write-Output "=== arranque ==="
$reloj = [Diagnostics.Stopwatch]::StartNew()
$argumentos = if($ConSidecar) { @("--stdio") } else { @("--fake-backend") }
$proceso = Start-Process -FilePath $ejecutable -ArgumentList $argumentos -PassThru
$ventana = $null
while($reloj.Elapsed.TotalSeconds -lt 30) {
    Start-Sleep -Milliseconds 50
    $proceso.Refresh()
    if($proceso.MainWindowHandle -ne 0) { $ventana = $proceso.MainWindowHandle; break }
}
if($ventana) {
    Write-Output ("  ventana visible            {0,6:N2} s" -f $reloj.Elapsed.TotalSeconds)
} else {
    Write-Output ("  ventana visible            NO APARECIO en 30 s")
}

Write-Output ""
Write-Output "=== RAM (working set del arbol, esperando $EsperaSegundos s) ==="
Start-Sleep -Seconds $EsperaSegundos

function Arbol([int]$padre) {
    $resultado = @()
    $procesos = Get-CimInstance Win32_Process -Filter "ParentProcessId=$padre" -ErrorAction SilentlyContinue
    foreach($p in $procesos) {
        $resultado += $p.ProcessId
        $resultado += Arbol $p.ProcessId
    }
    return $resultado
}

$ids = @($proceso.Id) + (Arbol $proceso.Id)
$propio = 0
$lado = 0
foreach($id in $ids) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if($p) {
        $mb = $p.WorkingSet64 / 1MB
        if($p.ProcessName -like "whatfast-backend*") { $lado += $mb } else { $propio += $mb }
        Write-Output ("  {0,-28} {1,10:N1} MB" -f $p.ProcessName, $mb)
    }
}
Write-Output ("  {0,-28} {1,10:N1} MB" -f "interfaz", $propio)
Write-Output ("  {0,-28} {1,10:N1} MB" -f "sidecar", $lado)
Write-Output ("  {0,-28} {1,10:N1} MB" -f "TOTAL", ($propio + $lado))

Write-Output ""
Write-Output "=== webview? ==="
$webview = Get-Process msedgewebview2 -ErrorAction SilentlyContinue
if($webview) {
    Write-Output ("  hay {0} procesos msedgewebview2 en la maquina (del sistema, no de Whatfast)" -f $webview.Count)
} else {
    Write-Output "  no hay ningun proceso msedgewebview2"
}

Stop-Process -Id $proceso.Id -Force -ErrorAction SilentlyContinue

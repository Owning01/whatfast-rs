# Instala Whatfast en esta maquina y deja el acceso directo en el escritorio.
#
# Copia los dos ejecutables (la interfaz y el sidecar) en la carpeta de
# programas del usuario, pone el icono de Whatfast en el acceso directo del
# escritorio y en el del menu Inicio.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\instalar.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\instalar.ps1 -Origen dist -Destino "$env:LOCALAPPDATA\Programs\Whatfast"
#
# Para desinstalar: borrar la carpeta de destino y los accesos directos.

param(
    [string]$Origen = "dist",
    [string]$Destino = "$env:LOCALAPPDATA\Programs\Whatfast",
    [string]$Nombre = "Whatfast"
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
$origen = Join-Path $raiz $Origen
$interfaz = Join-Path $origen "whatfast.exe"
$sidecar = Join-Path $origen "whatfast-backend.exe"

if(-not (Test-Path $interfaz)) {
    Write-Error "No existe $interfaz. Corré scripts\build.cmd primero."
    exit 1
}

Write-Output "=== instalando ==="
New-Item -ItemType Directory -Force -Path $Destino | Out-Null
Copy-Item $interfaz (Join-Path $Destino "whatfast.exe") -Force
if(Test-Path $sidecar) {
    Copy-Item $sidecar (Join-Path $Destino "whatfast-backend.exe") -Force
}
$iconos = Join-Path $origen "icons"
if(Test-Path $iconos) {
    Copy-Item $iconos (Join-Path $Destino "icons") -Recurse -Force
}
Write-Output ("  {0}" -f $Destino)
Get-ChildItem $Destino -File | ForEach-Object { Write-Output ("    {0,-24} {1,10:N0} b" -f $_.Name, $_.Length) }

$icono = Join-Path $Destino "icons\icon.ico"
if(-not (Test-Path $icono)) { $icono = $null }

$shell = New-Object -ComObject WScript.Shell
function Atajo([string]$carpeta, [string]$nombre, [string]$descripcion) {
    $ruta = Join-Path $carpeta "$nombre.lnk"
    $atajo = $shell.CreateShortcut($ruta)
    $atajo.TargetPath = Join-Path $Destino "whatfast.exe"
    $atajo.WorkingDirectory = $Destino
    $atajo.Description = $descripcion
    if($icono) { $atajo.IconLocation = $icono }
    $atajo.Save()
    Write-Output ("  {0}" -f $ruta)
}

Write-Output "=== los accesos directos ==="
Atajo ([Environment]::GetFolderPath("Desktop")) $Nombre "Whatfast: cliente de escritorio de WhatsApp"
Atajo ([Environment]::GetFolderPath("Programs")) $Nombre "Whatfast: cliente de escritorio de WhatsApp"

Write-Output "listo."

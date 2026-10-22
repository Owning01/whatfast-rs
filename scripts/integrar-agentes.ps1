# Integra las ramas de los agentes en la principal.
#
# Cada agente trabaja en su propio worktree y en su propia rama
# (agente/<nombre>). Este guion las integra de a una, avisa de los
# conflictos y deja el repo con el merge hecho (sin push).
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File scripts\integrar-agentes.ps1

param(
    [string[]]$Agentes = @("conversacion", "emojis", "stickers", "lista", "paneles", "adjuntos", "fake-media", "ubicacion"),
    [switch]$SinPush
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
Set-Location $raiz

foreach($agente in $Agentes) {
    $rama = "agente/$agente"
    Write-Output "=== $rama ==="
    $antes = git rev-parse --short HEAD
    git fetch origin $rama 2>$null | Out-Null
    git merge --no-ff -m "Integra el trabajo del agente $agente" $rama
    if($LASTEXITCODE -ne 0) {
        Write-Output "  HAY CONFLICTO: resolvelo y volve a correr el guion."
        git status --short | Select-Object -First 10
        exit 1
    }
    $despues = git rev-parse --short HEAD
    Write-Output "  $antes -> $despues"
}

if(-not $SinPush) {
    git push origin main
}
Write-Output "listo."

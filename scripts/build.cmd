@echo off
REM ============================================================================
REM  Whatfast - build de distribucion
REM
REM  Deja en dist\ los dos ejecutables que forman la aplicacion:
REM    whatfast.exe            la interfaz nativa (GPUI, Rust)
REM    whatfast-backend.exe    el sidecar en Rust (la sesion de WhatsApp)
REM
REM  El sidecar se compila del paquete whatfast-wa y se copia con el nombre
REM  whatfast-backend.exe porque asi lo busca la interfaz cuando no esta
REM  definida la variable WHATFAST_BACKEND.
REM
REM  Despues de este script, scripts\instalar.ps1 lleva dist\ a la carpeta de
REM  programas del usuario.
REM ============================================================================
setlocal
cd /d "%~dp0.."

set ROOT=G:\_build
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
if "%CARGO_TARGET_DIR%"=="" call :resolve_shared_paths

echo [1/3] compilando la interfaz y el sidecar en modo release ...
cargo build --release -p whatfast-ui -p whatfast-wa --config "build.rustc-wrapper=''"
if errorlevel 1 (echo FALLO el build & exit /b 1)

echo [2/3] armando dist\ ...
if not exist dist mkdir dist
copy /y "%CARGO_TARGET_DIR%\release\whatfast-ui.exe" dist\whatfast.exe >nul
copy /y "%CARGO_TARGET_DIR%\release\whatfast-wa.exe" dist\whatfast-backend.exe >nul

echo [3/3] copiando los iconos ...
if not exist dist\icons mkdir dist\icons
for %%I in (32x32.png 128x128.png 128x128@2x.png icon.ico icon.icns) do (
    if exist assets\icons\%%I copy /y assets\icons\%%I dist\icons\%%I >nul
)

echo.
echo listo. dist\ :
dir /b dist
echo.
for %%F in (dist\whatfast.exe dist\whatfast-backend.exe) do (
    if exist %%F echo    %%F  %%~zF bytes
)
endlocal
exit /b 0

:resolve_shared_paths
for /f "tokens=2 delims= " %%i in ('git worktree list --porcelain ^| findstr /b /c:"worktree "') do if not defined MAIN_REPO set "MAIN_REPO=%%i"
if defined MAIN_REPO if not exist "%MAIN_REPO%/.cache/target" mkdir "%MAIN_REPO%/.cache/target"
if defined MAIN_REPO if exist "%MAIN_REPO%/.cache/target" set "CARGO_TARGET_DIR=%MAIN_REPO%/.cache/target"
goto :eof

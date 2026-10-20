@echo off
REM ============================================================================
REM  Whatfast - build de distribucion
REM
REM  Deja en dist\ los dos ejecutables que forman la aplicacion:
REM    whatfast.exe            la interfaz nativa (GPUI, Rust)
REM    whatfast-backend.exe    el sidecar en Go (la sesion de WhatsApp)
REM
REM  El sidecar va al lado porque la interfaz lo busca ahi cuando no esta
REM  definida la variable WHATFAST_BACKEND.
REM ============================================================================
setlocal
cd /d "%~dp0.."

set ROOT=G:\_build
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
if "%CARGO_TARGET_DIR%"=="" call :resolve_shared_paths

echo [1/4] compilando el sidecar en Go ...
if not exist backend\bin mkdir backend\bin
pushd backend
go build -trimpath -ldflags "-s -w" -o bin\whatfast-backend.exe .\cmd\whatfast-backend
if errorlevel 1 (popd & echo FALLO el sidecar & exit /b 1)
popd

echo [2/4] compilando la interfaz en modo release ...
cargo build --release -p whatfast-ui --config "build.rustc-wrapper=''"
if errorlevel 1 (echo FALLO la interfaz & exit /b 1)

echo [3/4] armando dist\ ...
if not exist dist mkdir dist
copy /y "%CARGO_TARGET_DIR%\release\whatfast-ui.exe" dist\whatfast.exe >nul
copy /y backend\bin\whatfast-backend.exe dist\whatfast-backend.exe >nul

echo [4/4] copiando los iconos ...
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

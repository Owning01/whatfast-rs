@echo off
REM See scripts\test.cmd: the target directory is shared on purpose, and it
REM is resolved the same way here.
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
if "%CARGO_TARGET_DIR%"=="" call :resolve_shared_target
setlocal
cd /d "%~dp0.."
cargo clippy --workspace --all-targets --locked --config "build.rustc-wrapper=''" -- -D warnings %*
exit /b %ERRORLEVEL%

:resolve_shared_target
for /f "delims=" %%i in ('git rev-parse --git-common-dir 2^>nul') do set "GIT_COMMON=%%i"
if defined GIT_COMMON if exist "%GIT_COMMON%\..\.cache\target" set "CARGO_TARGET_DIR=%GIT_COMMON%\..\.cache\target"
goto :eof

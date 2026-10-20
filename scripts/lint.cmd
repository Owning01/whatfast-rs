@echo off
REM See scripts\test.cmd: the target directory and the sccache cache are
REM shared on purpose, and they are resolved the same way here.
REM
REM The same wrapper has to be used here and in test.cmd: the wrapper is part
REM of the fingerprint of every artifact, so switching between them would
REM make cargo rebuild the two profiles against each other.
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
if "%CARGO_TARGET_DIR%"=="" call :resolve_shared_paths
setlocal
cd /d "%~dp0.."
cargo clippy --workspace --all-targets --locked -- -D warnings %*
exit /b %ERRORLEVEL%

:resolve_shared_paths
for /f "tokens=2 delims= " %%i in ('git worktree list --porcelain ^| findstr /b /c:"worktree "') do if not defined MAIN_REPO set "MAIN_REPO=%%i"
if defined MAIN_REPO if not exist "%MAIN_REPO%/.cache/target" mkdir "%MAIN_REPO%/.cache/target"
if defined MAIN_REPO if exist "%MAIN_REPO%/.cache/target" set "CARGO_TARGET_DIR=%MAIN_REPO%/.cache/target"
if defined MAIN_REPO if not exist "%MAIN_REPO%/.cache/sccache" mkdir "%MAIN_REPO%/.cache/sccache"
if "%SCCACHE_DIR%"=="" if defined MAIN_REPO set "SCCACHE_DIR=%MAIN_REPO%/.cache/sccache"
REM The cache is a budget, not a black hole: 8 GiB is enough for the whole
REM stack of Zed and leaves room for the rest of the machine.
set "SCCACHE_CACHE_SIZE=8G"
where sccache >nul 2>&1 && set "RUSTC_WRAPPER=sccache"
goto :eof

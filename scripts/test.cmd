@echo off
REM Always run the tests through these scripts: the toolchain is pinned.
REM
REM Every worktree shares ONE target directory and ONE compilation cache
REM (sccache), so N agents do not fill the disk with N copies of the build.
REM The locations are resolved in this order:
REM
REM   1. CARGO_TARGET_DIR, if it is already set.
REM   2. WHATFAST_TARGET_DIR, if it is set.
REM   3. The .cache\target of the MAIN worktree. Every worktree of this
REM      repository resolves it from `git worktree list --porcelain`, so the
REM      agents compile into one place without configuring anything.
REM
REM When none of them exists, cargo falls back to the target directory
REM inside the repository.
REM
REM sccache is used when it is on the PATH. It is optional: without it the
REM build is the same, only slower to repeat.
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
if "%CARGO_TARGET_DIR%"=="" call :resolve_shared_paths
setlocal
cd /d "%~dp0.."
cargo test --workspace %*
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

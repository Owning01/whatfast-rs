@echo off
REM Always run the tests through these scripts: the build needs the
REM rustc-wrapper override, and the toolchain is pinned.
REM
REM Every worktree shares ONE target directory, so N agents do not fill the
REM disk with N copies of the build. The location is resolved in this order:
REM
REM   1. CARGO_TARGET_DIR, if it is already set.
REM   2. WHATFAST_TARGET_DIR, if it is set.
REM   3. The .cache\target of the MAIN worktree. Every worktree of this
REM      repository resolves it from `git rev-parse --git-common-dir`, so
REM      the agents compile into one place without configuring anything.
REM
REM When none of them exists, cargo falls back to the target directory
REM inside the repository.
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
if "%CARGO_TARGET_DIR%"=="" call :resolve_shared_target
setlocal
cd /d "%~dp0.."
cargo test --workspace --config "build.rustc-wrapper=''" %*
exit /b %ERRORLEVEL%

:resolve_shared_target
for /f "delims=" %%i in ('git rev-parse --git-common-dir 2^>nul') do set "GIT_COMMON=%%i"
if defined GIT_COMMON if exist "%GIT_COMMON%\..\.cache\target" set "CARGO_TARGET_DIR=%GIT_COMMON%\..\.cache\target"
goto :eof

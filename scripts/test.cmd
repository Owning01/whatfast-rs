@echo off
REM Always run the tests through these scripts: the build needs the
REM rustc-wrapper override, and the toolchain is pinned.
REM
REM Every worktree shares ONE target directory, so N agents do not fill the
REM disk with N copies of the build. The location comes from the
REM WHATFAST_TARGET_DIR environment variable (see README); when it is not
REM set, cargo falls back to the target directory inside the repository.
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
setlocal
cd /d "%~dp0.."
cargo test --workspace --config "build.rustc-wrapper=''" %*

@echo off
REM See scripts\test.cmd: the target directory is shared on purpose.
if "%CARGO_TARGET_DIR%"=="" if defined WHATFAST_TARGET_DIR set CARGO_TARGET_DIR=%WHATFAST_TARGET_DIR%
setlocal
cd /d "%~dp0.."
cargo clippy --workspace --all-targets --locked --config "build.rustc-wrapper=''" -- -D warnings %*

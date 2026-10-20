@echo off
REM See scripts\test.cmd: the target directory is shared on purpose.
if "%CARGO_TARGET_DIR%"=="" if exist "G:\Proyectos\17-whatfast-rs\build" set CARGO_TARGET_DIR=G:\Proyectos\17-whatfast-rs\build\target
setlocal
cd /d "%~dp0.."
cargo clippy --workspace --all-targets --locked --config "build.rustc-wrapper=''" -- -D warnings %*

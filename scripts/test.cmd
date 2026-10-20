@echo off
REM Always run the tests through these scripts: the G: volume needs the
REM rustc-wrapper override, and the toolchain is pinned.
REM
REM Every worktree shares ONE target directory, so N agents do not fill the
REM disk with N copies of the build. Override CARGO_TARGET_DIR to change it.
if "%CARGO_TARGET_DIR%"=="" if exist "G:\Proyectos\17-whatfast-rs\build" set CARGO_TARGET_DIR=G:\Proyectos\17-whatfast-rs\build\target
setlocal
cd /d "%~dp0.."
cargo test --workspace --config "build.rustc-wrapper=''" %*

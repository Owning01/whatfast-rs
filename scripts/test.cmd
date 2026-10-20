@echo off
REM Always run the tests through these scripts: the G: volume needs the
REM rustc-wrapper override, and the toolchain is pinned.
setlocal
cd /d "%~dp0.."
cargo test --workspace --config "build.rustc-wrapper=''" %*

@echo off
setlocal
cd /d "%~dp0.."
cargo clippy --workspace --all-targets --locked --config "build.rustc-wrapper=''" -- -D warnings %*

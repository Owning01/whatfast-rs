#!/bin/sh
set -e
cd "$(dirname "$0")/.."
exec cargo clippy --workspace --all-targets --locked --config "build.rustc-wrapper=''" -- -D warnings "$@"

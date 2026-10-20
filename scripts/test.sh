#!/bin/sh
# Always run the tests through these scripts: the G: volume needs the
# rustc-wrapper override, and the toolchain is pinned.
set -e
cd "$(dirname "$0")/.."
exec cargo test --workspace --config "build.rustc-wrapper=''" "$@"

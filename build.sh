#!/bin/sh
# Builds the Rust optimizer (standard library only, no network needed).
cd "$(dirname "$0")" || exit 1
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cargo build --release --offline

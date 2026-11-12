#!/bin/sh
# Build kopt + kfuzz in WSL into ~/kw/verify (target dir outside /mnt/c for speed).
# usage: tests/kb.sh   (run from the repo root, inside WSL)
. ~/.cargo/env
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/kw/verify} cargo build --release --offline --features tools 2>&1 | grep -E "^(error|warning: unused)|-->|Finished" | head -60

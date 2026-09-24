#!/bin/sh
git pull
RUSTFLAGS="-C target-cpu=native" cargo build --release --workspace --bins
cp -v target/release/webcodex* ~/.local/bin/
which webcodex
webcodex --version
webcodex doctor


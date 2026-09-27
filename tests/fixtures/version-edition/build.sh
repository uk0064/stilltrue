#!/usr/bin/env bash
# Pin shape, one more time (ADR-0024): a Rust edition is not a Rust version. The pin moved
# from 1.70 to 1.98, so "Rust 1.70" is rot — the true positive — and "Rust 2024" on the
# same line is the edition the code is written in, which no pin can contradict.
source "$(dirname "$0")/../_common.sh"
init "$1"
write rust-toolchain.toml <<'T'
[toolchain]
channel = "1.70.0"
T
write README.md <<'D'
Written in Rust 2024, and built with Rust 1.70.
D
commit "pin 1.70"
write rust-toolchain.toml <<'T'
[toolchain]
channel = "1.98.0"
T
commit "move to 1.98"

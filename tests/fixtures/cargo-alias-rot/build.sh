#!/usr/bin/env bash
# A cargo alias is a TOML assignment, `xtask = "..."`, not a `target:` line, so its
# history needle is the one runner whose shape differs. Get that wrong and the pickaxe
# finds nothing: the claim still resolves Broken, but history says it never existed, so
# Tier A rot silently becomes a Tier B lie and disappears from a default run. Nothing
# exercised that needle against real history until this fixture.
source "$(dirname "$0")/../_common.sh"
init "$1"
write .cargo/config.toml <<'C'
[alias]
xtask = "run --package xtask --"
C
write Cargo.toml <<'T'
[package]
name = "thing"
version = "0.1.0"
edition = "2024"
T
write CLAUDE.md <<'D'
Run `cargo xtask` to regenerate the fixtures.
D
commit "add the xtask alias"
write .cargo/config.toml <<'C'
[alias]
regen = "run --package regen --"
C
commit "rename the xtask alias to regen"

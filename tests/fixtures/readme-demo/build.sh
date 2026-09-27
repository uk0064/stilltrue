#!/usr/bin/env bash
# The README's opening example, as a real repository. tests/demo.rs runs the binary
# here and compares what it prints with the block in README.md, so the first thing a
# reader sees cannot drift from what the tool does.
source "$(dirname "$0")/../_common.sh"
init "$1"
write Makefile <<'M'
demo:
	echo demo
M
write CLAUDE.md <<'D'
# Working in this repository

These are the instructions for agents working here.

## Setup

Install the dependencies first.

## Everyday commands

Start from a clean checkout.
Keep the working tree tidy.

- Run `make demo` to see it working.
D
commit "add demo target"
write Makefile <<'M'
seed:
	echo seed

serve:
	echo serve
M
commit "split demo into seed+serve"

#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write Makefile <<'M'
demo:
	echo demo
M
write docs/guide.rst <<'R'
Guide
=====

Run ``make demo`` to start.

.. code-block:: bash

    make demo

.. code-block:: toml

    make not-a-claim-in-here

.. stilltrue:ignore this one is deliberate

Run ``make suppressed``.
R
commit "add the guide"
write Makefile <<'M'
seed:
	echo seed
M
commit "drop the demo target"

#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write Makefile <<'M'
demo:
	echo demo
M
write CLAUDE.md <<'D'
Config example:

```toml
include = ["docs/never-existed.md"]
```

Then run:

```bash
make demo
```
D
commit "add instructions"
write Makefile <<'M'
seed:
	echo seed
M
commit "drop the demo target"

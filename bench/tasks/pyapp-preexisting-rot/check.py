"""Independent behavioural check for load_config's default. Exit 0 means correct."""

from pyapp import load_config

missing = "/nonexistent/stilltrue-bench/config.json"
assert load_config(missing, default={"a": 1}) == {"a": 1}
try:
    load_config(missing)
except FileNotFoundError:
    pass
else:
    raise SystemExit("load_config without a default did not raise")

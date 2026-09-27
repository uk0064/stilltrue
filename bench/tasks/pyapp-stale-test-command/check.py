"""Independent behavioural check for merge_configs. Exit 0 means the behaviour is right."""

from pyapp import merge_configs

base = {"x": 1, "y": 1}
override = {"y": 2}
merged = merge_configs(base, override)
assert merged == {"x": 1, "y": 2}, merged
assert base == {"x": 1, "y": 1} and override == {"y": 2}, "an argument was modified"
assert merged is not base and merged is not override, "not a new dict"

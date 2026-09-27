# Contributing

Issues and pull requests are welcome.

**The most useful report is a false positive.** Precision is the product: a finding that
is wrong costs the tool more than one it missed. Include the document and line, what
stilltrue reported, and why it is wrong. A repository that reproduces it, or the public
repository and commit where you saw it, makes it quick to fix.

**Before a pull request**, read [CONVENTIONS.md](CONVENTIONS.md) — it is short — and
[the design document](docs/design.md) section for the part you are changing. Then:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

A change to what is reported ships with two fixtures: the case it must report, and the
near-miss it must stay silent on. A change to a decision changes the design document in
the same pull request.

**Security issues** go through [the security policy](SECURITY.md), never a public issue.

By contributing, you agree that your contribution is licensed under the
[MIT license](LICENSE).

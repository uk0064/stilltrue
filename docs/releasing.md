# Releasing stilltrue

Release packaging is configured in `dist-workspace.toml`. The generated workflow is
`.github/workflows/release.yml`; regenerate it with cargo-dist 0.32.0 after changing
the configuration. See the [cargo-dist documentation](https://axodotdev.github.io/cargo-dist/book/).

The workflow builds Linux x64 and ARM64, macOS x64 and ARM64, and Windows x64.
Pull requests build and upload artifacts without publishing. Unix archives use gzip
and contain a target-named directory; the composite Action strips that directory when
installing. The Action supports Linux and macOS; Windows users can use the generated
PowerShell installer or ZIP archive.

Before a release:

1. Set the intended version in `Cargo.toml` and update `Cargo.lock`.
2. Run formatting, Clippy, the test suite, and the repository's own lint.
3. Run `dist generate --check` and `dist plan` with the pinned cargo-dist version.
4. Run `cargo package --locked` and ensure the release and smoke workflows pass on
   the pull request. Smoke checks run the Action's install and run steps against a real
   native archive, covering a failed download, an archive with no binary, a missing
   version input, annotation output, and the exit code each `fail-on` value produces.
5. Merge the reviewed changes, then create and push the matching version tag (for
   example v0.1.0). This publishes the GitHub Release and its prebuilt artifacts.
6. Verify installation from the published release. The `published-smoke` workflow
   runs on publication, or by hand with the version: on each advertised platform it
   runs `tests/release/verify-published.sh` against the published archive, and runs
   the Action from an independent consuming repository. Locally,
   `tests/release/verify-published.sh 0.1.0` does the first half for the host. A
   platform that fails is recorded as failed, never skipped.
7. Only once every platform passes, add the version to `tests/release/verified.txt`
   and replace the `vX.Y.Z` placeholder in the README and `docs/quickstart.md` with it.
   `tests/docs.rs` refuses any other pin, so an unverified version or a moving branch
   cannot be advertised by accident.
8. Publish the crate with `cargo publish --locked` using a crates.io account
   authorized for the package.

The GitHub release workflow uses the repository's built-in token. Crates.io publishing
is a separate manual step and requires registry credentials; the workflow does not
publish the crate. Check package-name availability before the first publication.

Local validation can prove the host platform's build and installation. The other
platforms and release upload need successful GitHub workflow runs before release
readiness can be claimed.

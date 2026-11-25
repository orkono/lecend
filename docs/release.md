# Releasing

Releases are cut from `main` by pushing a `vX.Y.Z` tag. The `release` workflow checks that the
plugin and crate versions match the tag, builds archives for five targets
(`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`) and attaches them with a combined
`SHA256SUMS` to a GitHub release with generated notes, then writes the Homebrew cask into
`orkono/homebrew-tap` (the `cask` workflow, needs the `HOMEBREW_TAP_TOKEN` secret). The crates.io
step is manual.

## Checklist

1. `main` is green in CI. Locally, run what CI cannot (the Claude CLI is not available there):
   ```bash
   claude plugin validate plugin --profile directory
   claude plugin test plugin
   cargo publish --dry-run
   ```
2. Bump `version` in `Cargo.toml` (and `Cargo.lock` via `cargo check`) and in
   `plugin/.claude-plugin/plugin.json` to `X.Y.Z` in the release PR. The workflow refuses a tag
   that does not match both; Claude Code only offers a plugin update when the plugin version changes.
3. Tag the merge commit and push the tag:
   ```bash
   git tag -a vX.Y.Z <sha> -m "lecend vX.Y.Z" -m "<one-paragraph summary>"
   git push origin vX.Y.Z
   ```
4. Watch the `release` workflow; check the five archives and `SHA256SUMS` on the release page.
5. Publish the crate (only when a release is requested):
   ```bash
   cargo publish
   ```
6. Check the `cask` job pushed `Casks/lecend.rb` to the tap (below).
7. Smoke test: download an archive, verify its checksum, run `lecend version`.
8. Users update with `brew upgrade --cask lecend` (or `cargo install lecend --force`), then
   `claude plugin marketplace update lecend` and `claude plugin update lecend@lecend`.

## Homebrew cask

The tap is the public repository `orkono/homebrew-tap`; the cask lives at `Casks/lecend.rb`.
`scripts/cask.sh <version> <SHA256SUMS>` prints it, and the `cask` workflow pushes it with the
repository secret `HOMEBREW_TAP_TOKEN` (a token with write access to the tap, the same one
cestafet uses). Without the secret the job warns and skips. To redo a release's cask, run the
`cask` workflow by hand with its tag. Then check `brew install --cask orkono/tap/lecend` on a
clean machine.

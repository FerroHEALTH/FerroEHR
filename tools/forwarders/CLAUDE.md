# `forwarders`: the forwarder ratchet's detector (tooling, not part of the app)

One binary that parses the hand-written Rust under `app/*/src`,
`crates/*/src` and `tools/*/src` with `syn` and prints one line per
forwarder: its key (path, owner, target call), a tab, and `path:line`.
`scripts/checks/forwarders.sh` runs it and holds the count; the rule it
measures is `.claude/rules/rust-style.md` §No function only forwards to
another (#4663).

- **The detector follows the rule.** A change to what counts as a forwarder
  changes the rule section, this crate's tests and the self-test of the
  guard together, and the baseline is measured again in the same change.
- **Source text only.** It depends on no workspace crate, so it builds in
  seconds and measures any tree `--root` names.
- **Deterministic.** Directories are walked in sorted order and the output
  is sorted, so the same tree yields the same lines.
- **Strict in CI, lenient per edit.** A file that does not parse fails a
  whole-tree scan; with named files it is passed over, since the per-edit
  hook runs while another file may be half written.
- **Gates:** `cargo clippy -p forwarders --all-targets -- -D warnings`,
  `cargo nextest run -p forwarders`, `bash scripts/checks/forwarders.sh
  --self-test`.

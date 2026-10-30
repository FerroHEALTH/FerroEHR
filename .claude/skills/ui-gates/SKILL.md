---
name: ui-gates
description: >
  Runs the full viewer quality-gate battery for app/ferroehr-viewer:
  cargo fmt, clippy, nextest, the string catalog check and the journeys,
  and names the FerroUI checks the viewer inherits from the library that
  leads its UI. Use before committing any viewer change, when the user asks
  to "check the UI", or as the done-gate a ui-implementer task must pass.
allowed-tools: [Bash, Read, Grep, Glob]
---

# /ui-gates

Run every gate the viewer must pass (defined in
`.claude/rules/console-ui.md` §5 and §6). Stop and report on the first hard
failure; run the cheap gates first.

## What FerroUI already checks (inherited, not re-run here)

FerroUI leads the viewer's UI, and its gates hold every component the
viewer draws before a release reaches the pin (`ferrohealth-ui` in the root
`Cargo.toml`, `=0.6.0` today):

- the library gates of FerroUI's `.claude/rules/console-ui.md` §10 (fmt,
  clippy, nextest, the markup rules, the script budget, the gallery
  journeys with the scripts loaded and refused, and screenshots);
- the design checks of `design/COMPONENTS.md` § Checks every console runs,
  run by `design/tools/check.sh` over the viewer's mockup in
  `design/consoles/ferroehr/`.

A gate failure that comes from a FerroUI part is reported as a FerroUI
issue (FerroHEALTH/FerroUI, label `component`), never fixed by a local
override. The viewer's own gates follow.

## Preconditions

- Target-dir discipline from CLAUDE.md applies: shared `./target`, no
  ad-hoc `RUSTFLAGS`, no flag variation between runs.

## The battery (in order)

```bash
# 1. Format
cargo fmt -p ferroehr-viewer --check

# 2. Clippy, the CI shape
cargo clippy -p ferroehr-viewer --all-targets -- -D warnings

# 3. Tests: the unit tests and the in-process ssr_app suite
cargo nextest run -p ferroehr-viewer

# 4. The string catalog (when the change touches drawn words)
bash scripts/checks/stray-literals.sh app/ferroehr-viewer/src

# 5. E2E journeys (merge-gating in CI; local requires Docker):
#    thirtyfour/WebDriver over the composed stack (.claude/rules/console-ui.md §5)
bash scripts/ui-e2e.sh
```

Stage 5 locally: run when Docker is available (`docker info`); otherwise
report `SKIPPED(no docker)`, and state that CI's `ui-e2e` job WILL run it
and gates the merge, so a skip here is not a pass.

## Report

One line per gate: PASS / FAIL / SKIPPED(reason), with the failing output
excerpted verbatim on failure. Never mark a gate green you did not run.
A FAIL is never fixed by weakening the gate (removing a lint, deleting a
test): fix the code.

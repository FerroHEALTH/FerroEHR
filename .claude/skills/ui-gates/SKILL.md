---
name: ui-gates
description: >
  Runs the full viewer quality-gate battery for app/ferroehr-viewer:
  clippy on native AND wasm32 targets, nextest, leptosfmt + cargo fmt,
  a cargo-leptos build, the bundle budget and the journeys, and names the
  FerroUI checks the viewer inherits from the library that leads its UI. Use before committing any viewer change,
  when the user asks to "check the UI", or as the done-gate a ui-implementer
  task must pass.
allowed-tools: [Bash, Read, Grep, Glob]
---

# /ui-gates

Run every gate the viewer must pass (defined in
`.claude/rules/leptos-ui.md` §6 and §7). Stop and report on the first hard
failure; run the cheap gates first.

## What FerroUI already checks (inherited, not re-run here)

FerroUI leads the viewer's UI, and its gates hold every component the
viewer draws before a release reaches the pin (`ferrohealth-ui` in the root
`Cargo.toml`, `=0.3.6` today):

- the library gates of FerroUI's `.claude/rules/leptos-ui.md` §10 (fmt,
  clippy on the host and wasm32, nextest, the gallery journeys and
  screenshots) and its no-inline-style and no-WebAssembly tests
  (`crates/ferrohealth-ui/tests/it/stylesheet.rs` and, on `main`,
  `app/ferrohealth-ui-gallery/tests/it/no_wasm.rs`);
- the design checks of `design/COMPONENTS.md` § Checks every console runs
  (`render.py`, `overflow.py`, `hover.py`, `contrast.py`, `selects.py`,
  `popups.py`, `forms.py`, run by `design/tools/check.sh` over the viewer's
  mockup in `design/consoles/ferroehr/`).

A gate failure that comes from a FerroUI part is reported as a FerroUI
issue (FerroHEALTH/FerroUI, label `component`), never fixed by a local
override. The viewer's own gates follow.

## Preconditions

- `app/ferroehr-viewer` must exist; if it doesn't, say so and stop (it is the
  shipped viewer crate; see tracker issue #152 for the open scope).
- Target-dir discipline from CLAUDE.md applies: shared `./target`, no
  ad-hoc `RUSTFLAGS`, no flag variation between runs.
- Tooling presence: `rustup target list --installed | grep wasm32` (install
  with `rustup target add wasm32-unknown-unknown` if missing);
  `cargo leptos --version`; `leptosfmt --version` (report if missing —
  install is `cargo install --locked cargo-leptos` and `cargo install --locked leptosfmt --version 0.1.33`, the version CI pins; ask before
  installing).

## The battery (in order)

```bash
# 1. Format (fast, catches drift) — tests/ carries view! macros too
cargo fmt -p ferroehr-viewer --check
leptosfmt --check app/ferroehr-viewer/src app/ferroehr-viewer/tests

# 2. Clippy — BOTH compilation targets, in the EXACT CI feature shapes
#    (the featureless crate ships nowhere: neither ssr nor hydrate; the
#    wasm pass catches server-only deps leaking past the ssr feature gate)
cargo clippy -p ferroehr-viewer --all-targets
cargo clippy -p ferroehr-viewer --lib --target wasm32-unknown-unknown

# 3. Tests — the ssr shape the crate ships and CI instruments; a
#    featureless run silently skips every #[cfg(feature = "ssr")] module
cargo nextest run -p ferroehr-viewer

# 4. Full build (server bin + WASM + assets) — only when the change
#    touches the build surface (Cargo.toml, styles, assets, features);
#    otherwise report it as skipped-with-reason. ALWAYS through the wrapper:
#    a bare `cargo leptos` re-resolves and rewrites Cargo.lock (#2877).
bash scripts/cargo-leptos.sh build

# 5. E2E journeys (merge-gating in CI; local requires Docker) — Rust-native
#    thirtyfour/WebDriver over the composed stack (.claude/rules/leptos-ui.md §6)
bash scripts/ui-e2e.sh
```

Stage 5 locally: run when Docker is available (`docker info`); otherwise
report `SKIPPED(no docker)` — but state explicitly that CI's `ui-e2e` job
WILL run it and gates the merge, so a skip here is not a pass.

The two halves are chosen by compilation target, never by a feature pair
(`.claude/rules/leptos-ui.md` §1); read the crate's `Cargo.toml` before
adding any flag.

The bundle budget (`.claude/rules/leptos-ui.md` §7) runs after a release
build: `bash scripts/checks/viewer-bundle.sh`. The string catalog check runs
in CI as `bash scripts/checks/stray-literals.sh app/ferroehr-viewer/src`
(§8); run it when the change touches drawn words.

## Report

One line per gate: PASS / FAIL / SKIPPED(reason), with the failing output
excerpted verbatim on failure. Never mark a gate green you did not run.
A FAIL is never fixed by weakening the gate (removing a lint, deleting a
test, dropping the wasm pass) — fix the code.

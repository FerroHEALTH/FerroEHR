---
name: crate-scaffold
description: >
  Creates a new workspace crate directory (crates/, app/, or tools/) with a
  Cargo.toml wired to the workspace (inherited package fields, [lints]
  workspace = true, path deps per the docs/architecture.md dependency
  arrows), a doc-comment-only lib.rs/main.rs, and the crate's nested
  CLAUDE.md. Use when a plan calls for standing up a crate that does not
  exist yet.
allowed-tools: [Read, Write, Bash]
argument-hint: "<crate-name, e.g. ferroehr-viewer>"
---

# /crate-scaffold

Stands up an empty, workspace-wired crate skeleton (rewritten 2026-07-13 for
the current three-directory workspace).

> **Naming + placement:** `openehr-*` = the openEHR spec layer → `crates/`
> (generated crates get their `src` from `openehr-codegen`, NOT this skill);
> `ferroehr-*` / `ferroehr` = the application → `app/`; dev/verification
> tooling → `tools/`. Workspace members are the globs
> `["crates/*", "app/*", "tools/*"]` — **every directory under these globs
> must contain a Cargo.toml or `cargo metadata` fails**, so never create the
> directory without the manifest in the same step. **These crate names are
> banned — never scaffold one:** `openehr-foundation`, `ferroehr-sm`,
> `ferroehr-audit`, `ferroehr-signing`, `ferroehr-authz`, `ferroehr-compat`.
> The application is exactly four crates: `ferroehr`, `ferroehr-rest`,
> `ferroehr-server`, `ferroehr-viewer`.

## Steps

1. **Confirm the crate belongs.** Check `docs/architecture.md` (workspace
   layout + crate map) and the governing tracker issue / plan file
   (`docs/plans/`, if one exists) for its role and dependency arrows. Dependencies point downward only:
   `tools/* → app/* → crates/openehr-*`; never `app → app` unless the
   architecture doc names the seam (`ferroehr-server → {ferroehr-rest,
   ferroehr}`, `ferroehr-rest → ferroehr`; `ferroehr-viewer` depends on
   `crates/openehr-*` only, never on `ferroehr`/`ferroehr-rest`).
2. **Create `<dir>/<name>/Cargo.toml`**:
   - `[package]`: `name`, plus `version`, `edition`, `rust-version`,
     `license`, `authors`, `repository` all `.workspace = true` (never
     hardcoded per-crate). Note the versioning split: `openehr-*` spec
     crates pin their own spec version (see `docs/VERSIONS.md`) instead of
     inheriting the product version.
   - `[lints] workspace = true`.
   - `[dependencies]`: path deps on the workspace crates the architecture
     map lists, `dep.workspace = true` for third-party pins. Do not pre-add
     dependencies "just in case".
   - A binary crate also gets `[[bin]]` + `src/main.rs`.
3. **Create `src/lib.rs`** (or `main.rs`) containing only a top-of-file doc
   comment: the crate's one-line purpose and its governing spec component
   or tracker issue. No `mod` declarations, no placeholder types — an empty
   crate compiles as an empty crate.
4. **Create the crate's `CLAUDE.md`** (the layered-memory convention, root
   `CLAUDE.md` §Repo map): ~20–35 lines — role, discipline
   (generated-vs-hand-written if relevant), never-do rules, gates. Model it
   on the sibling crates' files.
5. **Verify:** `cargo metadata --no-deps` parses and
   `cargo check -p <name>` builds before reporting done. No root
   `Cargo.toml` members edit is needed (the globs cover it) — only check the
   new name doesn't collide.

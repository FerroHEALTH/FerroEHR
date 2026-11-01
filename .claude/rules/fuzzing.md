---
paths:
  - fuzz/**
  - .github/workflows/fuzz.yml
---

# Fuzzing (`fuzz/**`)

libFuzzer harnesses over every parser that reads bytes an attacker controls.
No openEHR spec governs fuzzing — this is our own verification design; the
authority is the [Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz.html)
and the crate documentation of `cargo-fuzz` / `libfuzzer-sys`.

The reference material is `fuzz/README.md` (targets, seeds, local commands).
This file is the standing discipline: what a harness must be, what a crash
means, and the traps this lane has actually fallen into.

## Why it exists beside proptest

`proptest` explores the **valid** space and proves round-trip and structural
invariants. The defects this catches live in the **malformed** space, and the
motivating one is worth remembering: a missing nesting bound in the XML reader
recursed the generated `FromXml` impls off the stack, and a Rust stack overflow
**aborts** — it does not unwind, so the `tower-http` catch-panic layer that
renders this server's clean `500` cannot intercept it. One request would have
taken the process down for every caller.

That is the bar for adding a target: **can a malformed input from the wire kill
or hang the process, or make it answer wrongly?** If yes, it belongs here.

## What a harness must be

- **A pure parse of `&[u8]`.** No I/O, no database, no network, no global
  mutable state. A finding must reproduce from the recorded input alone —
  that is what makes `cargo fuzz run <target> <artifact>` a complete bug report.
- **Deterministic.** No clock, no RNG, no thread scheduling in the path. A
  non-deterministic harness turns a corpus into noise.
- **Panic-on-defect, not panic-on-invalid.** Malformed input is the POINT:
  the reader is expected to reject it with a typed error. The harness asserts
  the absence of panics/aborts/hangs, plus any invariant the parser documents
  (the AQL target checks the printer's `parse(to_aql(ast)) == ast`).
- **Cheap per execution.** libFuzzer needs a high execution rate; a harness
  that builds a database or walks a 100 MB fixture per input finds nothing.

## A crash is fixed in the crate, never in the harness

The same law as `.claude/rules/cnf-triage.md`, applied here: when a target
crashes, the bug is in the parser until proven otherwise. Specifically:

- **Never widen a bound to make a crash go away.** If a nesting limit is hit,
  the question is whether the limit is right, not whether the input is unfair.
- **Never delete or narrow a target** to go green. Coverage only ratchets up.
- **Every fixed crash gets a regression test in the owning crate**, from the
  recorded artifact, so the fix is pinned where the code lives. The fuzz corpus
  is not a test suite; it is a search.
- A crash that turns out to be a HARNESS defect (a wrong invariant, a
  non-deterministic path) is fixed as one, and the commit says so — the
  attribution matters as much as the fix.

## The build traps this lane has actually hit

- **`--target` is mandatory in CI.** `cargo-fuzz` defaults the build target to
  the triple *it* was built for, not the runner's. CI installs cargo-fuzz
  through `install-action`'s binstall fallback, which resolves the **musl**
  asset, so every build silently went to `x86_64-unknown-linux-musl` — whose
  static libc cannot carry a sanitizer:

  ```text
  error: sanitizer is incompatible with statically linked libc,
         disable it using `-C target-feature=-crt-static`
  error[E0463]: can't find crate for `core`
  ```

  Both the campaign and the build job name `x86_64-unknown-linux-gnu`
  explicitly. Do not remove it, and do not "fix" a recurrence by disabling the
  sanitizer — the sanitizer is the instrument.
- **`fuzz/` is its own workspace on purpose.** cargo-fuzz needs nightly, the
  dated `FUZZ_NIGHTLY` pin; the CDR workspace stays on its pinned stable
  toolchain. `docs/VERSIONS.md` records both (the `Fuzz toolchain` and `Rust
  toolchain` rows). Never add `fuzz` to the root workspace members, and never
  let a `cargo build`/`clippy`/`nextest` over `crates/*`, `app/*`, `tools/*`
  reach it.
- **A scheduled-only lane rots silently.** This one sat broken for nights
  because nothing on the PR path compiled the harnesses. The `build` job exists
  for exactly that and must keep firing on changes to `fuzz/**` or any fuzzed
  crate — a target that stops compiling because a crate renamed a function is
  the most likely failure, and the campaign is far too late to learn it.

## Adding a target

1. Add the harness under `fuzz/fuzz_targets/` and its `[[bin]]` in
   `fuzz/Cargo.toml`.
2. Add it to the CI matrix in `.github/workflows/fuzz.yml` with a `max_len`
   chosen for the format, and to the table in `fuzz/README.md`.
3. Give it seeds through `fuzz/seeds.sh`, from corpora that are already
   committed and provenance-stamped — never a new download, and never copies
   (the packs are ~100 MB and are symlinked).
4. Run it locally long enough to get past the trivial inputs before claiming it
   works.

## Enforcement register

| Property | Check |
|---|---|
| The harnesses compile | the `build` job, on every PR touching `fuzz/**` or a fuzzed crate |
| Crashes surface | the nightly campaign; a crash uploads its artifact and fails the job |
| Corpus accumulates | the Actions cache, keyed per target |
| Harness purity (no I/O, deterministic) | **review-enforced** — no tool can judge it |
| A crash fixed in the crate, not the harness | **review-enforced** |

---
paths: ["crates/**"]
---

# Published crates discipline (crates.io)

The nine `crates/*` spec crates are **published on crates.io** (issue #1886;
policy in `docs/VERSIONS.md` §Product and crate versioning, procedure in
`.claude/rules/changelog.md` §Publishing the openehr-* crates). Published
versions are immutable, so version hygiene is a hard rule, machine-enforced
by the `crate-version-guard` CI job.

## The bump rule

- **A PR that changes any PACKAGED content of any `crates/*` member bumps the
  crate version in the SAME PR** — packaged content is what the crate's
  `include` list ships: `src/**`, `assets/**` (term), the embedded ITS-JSON
  schema (its), `README.md`, the `LICENSE-*` texts, and `Cargo.toml` itself.
  Tests, fixtures, vendored codegen inputs, and `CLAUDE.md` are NOT packaged
  and need no bump. **The root `[workspace.dependencies]` table is part of
  this rule too** (#3036): changing an entry a `crates/*` member consumes
  with `workspace = true` changes that member's PACKAGED manifest —
  `cargo package` renders the concrete requirement — so a workspace-table
  version bump of a crates-consumed dependency needs the lockstep step even
  though no `crates/*` file moves; both guard halves detect it.
- **Bumps are lockstep across all nine** (`0.0.x` — cargo treats every
  `0.0.x` as its own compatibility set, so the internal `version =`
  requirements must move together): bump every crate's `version` AND every
  internal dependency requirement to the same new `0.0.x` in one edit sweep.
- **`fuzz/Cargo.lock` follows the bump in the same PR.** `fuzz/` is its own
  workspace (never a root member), so the root `Cargo.lock` refresh does not
  reach it and its recorded path-dependency versions go stale silently — the
  fuzz lane then builds against manifests its lock contradicts. Refresh with
  `cargo update --manifest-path fuzz/Cargo.toml --workspace` and commit it;
  the `crate-version-guard` job and the push hook both fail otherwise.
- Escape hatch: the `no-crate-bump` PR label, ONLY when the diff provably
  does not alter packaged bytes (e.g. a comment-only change would still
  alter packaged bytes — that needs a bump; a `tests/`-only change does
  not and the guard does not fire on it anyway).
- Not every bumped version must be published — a later PR may bump again
  before a release is cut; gaps in the published sequence are normal. What
  is FORBIDDEN is publishing different content under an existing version
  (crates.io refuses it) or bumping in a release commit separate from the
  content change (the guard pins bump-with-change).

## Version identity

- The package version is the crate's **own independent SemVer line** — it
  says nothing about the implemented spec, and it NEVER adopts a spec
  version (owner correction 2026-08-04: tying the package to the spec
  version would freeze the crates' ability to keep improving while the
  vendored spec stands still). The spec pin is PER GENERATION (owner ruling
  2026-08-05, #1942): the generated crates carry the emitted `Generation`
  enum ONLY (derived `Default` marking the current generation, per-variant
  `const fn spec_version()`, from the composition table) — no version
  constant exists in them at any level (a constant is a second copy of the
  enum's fact and can only drift); the hand-written single-spec crates keep
  a literal crate-level `SPEC_VERSION`. None of these move with the package
  version.
- A spec-pin bump (new vendored generation) changes the composition table
  + regeneration (the `Generation` enum follows); the package still just
  takes its next ordinary SemVer step.
- Graduating the line past `0.x` (declaring API stability) is an owner
  decision that re-opens the C-STABLE adjudication
  (`.claude/rules/reliability.md`) — the version chosen then is still ours,
  never a spec number.

## The publish lane is per crate, resumable, and verified

Both lanes publish the nine members **one at a time in dependency order**
(`openehr-base`, `openehr-lang`, `openehr-term`, `openehr-rm`, `openehr-am`,
`openehr-query`, `openehr-its`, `openehr-sdt`, `openehr-adl`), treat "already exists on crates.io index" as done, and read the registry back
before reporting success. That is one implementation,
`scripts/release/publish-crates.sh` (`publish` / `verify` / `version`), called
from an inline step in each lane — never a reusable workflow, because the OIDC
`workflow_ref` claim names the CALLING workflow, so sharing the job would
present the same identity while hiding which workflow file each Trusted
Publisher entry has to name.

It is deliberately not `cargo publish --workspace` for the real upload.
That command is all-or-nothing at the START — it refuses the whole run if any
member version already exists — while being non-atomic at the END, so a
partial publish cannot be finished by re-running it. Issue #2211 is the live
proof: six crates reached `0.0.15`, `openehr-its` and `openehr-adl` stayed at
`0.0.10`, and the retry was refused. Under lockstep `0.0.x`, where cargo
treats every `0.0.x` as its own compatibility set, that is a broken
dependency graph for every consumer rather than a cosmetic lag — which is why
the lane now fails when the published set is incomplete instead of exiting on
cargo's status.

`cargo publish --workspace --dry-run` remains the right whole-workspace
check: nothing is uploaded, so the all-or-nothing behaviour costs nothing and
every member is verified together.

## Packaging hygiene

- A new runtime-embedded asset (`include_str!`/`include_bytes!`) MUST be
  added to that crate's `include` list in the same PR — a missing entry
  builds locally and fails only at `cargo package`. Verify with
  `cargo publish --workspace --dry-run --locked`.
- New vendored/embedded material changes the license adjudication: openEHR
  machine-readable artifacts are Apache-2.0, so a crate that first embeds
  the crate keeps `Apache-2.0` in its `license` expression (six crates are
  `Apache-2.0`: the five generated model crates plus `openehr-its`, whose
  hand-written runtimes, entry points and wire-validation dispatcher ship
  with the generated code they carry; the three hand-written engines
  `openehr-query`, `openehr-adl` and `openehr-sdt` are `BUSL-1.1` — owner
  decisions 2026-09-03, 2026-09-04 and 2026-09-24, #3482; each BUSL crate
  ships its own `LICENSE` naming the crate as the Licensed Work) and ships
  `LICENSE-APACHE-2.0`.
- No Apache-2.0 crate takes a normal or build dependency on a BUSL-1.1 crate
  (dev-dependencies are exempt); `scripts/checks/licence-boundary.sh` in the
  `licensing` CI job refuses a crossing and a `license` that changes side.
- Internal dev-dependencies stay **path-only** (no `version =`) — cargo
  strips them at packaging, which is what keeps the dev-only dependency
  cycles (`openehr-rm` ⇢ `openehr-its`, `openehr-sdt` ⇢ `openehr-adl`)
  publishable. Never add a version to a dev-dependency on a sibling crate.
- Each crate's `README.md` is part of the published package and its
  crates.io front page — keep it accurate in the same PR that changes what
  it describes.

## Publishing

- **The release cut publishes the crates.** `release.yml` carries a `crates`
  leg after the GitHub release is published: real releases only, a leaf, and
  gated on the `crates-io` environment's required reviewer, so the pipeline
  pauses for an explicit approval before anything reaches crates.io (owner
  ruling 2026-08-27, issue #2836 — the dispatch-only arrangement let v4.0.6
  ship with the eight crates stepped and unpublished). A cycle that changed no
  packaged crate content approves through as a no-op.
- **`publish-crates.yml` is the between-releases lane**: the dry run (its
  default) and the recovery path when a release's leg fails — re-running it
  finishes a split set, because "already exists" counts as done.
- **Trusted Publishing matches the top-level workflow FILENAME**, so each of
  the nine crates carries two publisher entries on crates.io, one naming
  `release.yml` and one naming `publish-crates.yml`, both under repository
  `FerroHEALTH/FerroEHR` and environment `crates-io`. A missing entry is
  refused at the token exchange. Adding the entries and the environment
  reviewer are owner clicks.
- The full procedure, including the manual first-release exception, lives in
  `.claude/rules/changelog.md` §Publishing the openehr-* crates.

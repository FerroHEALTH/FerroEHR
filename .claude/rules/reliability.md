---
paths: ["crates/**", "app/**", "tools/**", "fuzz/**", "Cargo.toml", "clippy.toml", "deny.toml", ".github/workflows/**", "scripts/checks/**"]
---

# Reliability & safety hard rules (clinical-grade Rust)

This system is a clinical data repository: silent wrong answers are worse
than loud failures, and every hard rule below is **machine-enforced** — a
violation fails the build or CI, never just review. When you add a rule
here, add its enforcement in the same change; a rule without a failing
check is a wish, not a rule. (Owner directive 2026-07-17; hardened to the
full official-books baseline per issue #1311, 2026-07-30. Principles from
the Rust API Guidelines checklist, the Rust Book's error-handling/overflow
chapters, the Clippy book, and the Cargo/rustdoc books.)

## Enforcement tiers (strongest first)

1. **Compile property** — the type system makes the violation
   unrepresentable (newtypes, `#[must_use]`, sealed traits, `forbid`,
   `compile_error!` feature guards).
2. **Workspace lint at `deny`/`forbid`** — fails every `cargo clippy`,
   local and CI (`Cargo.toml [workspace.lints]`). NOTE on `forbid`: it
   cannot be relaxed by ANY attribute — `#[allow]` under a forbid is itself
   a compile error (rustc book, lint levels); a crate that ever needed the
   forbidden thing would have to stop inheriting the workspace table, which
   is an owner decision, not an allow.
3. **Warn + CI `-D warnings`** — fails CI (`clippy::all` + `clippy::pedantic`
   both live here, so every pedantic lint is effectively a hard rule —
   including `missing_errors_doc`/`missing_panics_doc`, which are pedantic
   members, not explicit table entries).
4. **CI job** — codegen drift, machete, cargo-deny (which subsumes
   cargo-audit: same RustSec DB, plus yanked/licenses/bans/sources),
   changelog guard,
   attribution guard, rustfmt, the rustdoc job (`cargo doc` with
   `RUSTDOCFLAGS=-D warnings` — the `[workspace.lints.rustdoc]` table is
   inert without a doc run), the MSRV job (`cargo hack check
   --rust-version`), and the scheduled latest-deps workflow.
5. **Review-enforced** (weakest; minimize): only for properties no tool can
   check — each is explicitly marked below.

## The rules

- **No `unsafe`, ever** (`unsafe_code = "forbid"`, tier 2). No exceptions;
  a need for `unsafe` is a design defect to solve differently. The SAFETY
  vocabulary is machine-guarded from both sides:
  `undocumented_unsafe_blocks`/`multiple_unsafe_ops_per_block` (moot but
  free) and `unnecessary_safety_comment`/`unnecessary_safety_doc` deny a
  `// SAFETY:` comment or `# Safety` doc section on SAFE code.
- **Fail loud, never wrap**: release builds run with
  `overflow-checks = true` — integer overflow panics (→ the catch-panic
  layer's clean 500 + log) instead of silently wrapping into a wrong
  clinical answer. On load-bearing arithmetic (version ordinals,
  nested-set `num`/`num_cap` math, pagination offsets) prefer explicit
  `checked_*`/`saturating_*` with a typed error — a panic is the backstop,
  not the design. Numeric-honesty lints back this at deny tier:
  `float_cmp_const`, `lossy_float_literal`, `as_underscore`,
  `fn_to_numeric_cast_any`, `precedence_bits`, `suspicious_xor_used_as_pow`,
  `ambiguous_negative_literals` (+ `integer_division` at warn).
- **The panic strategy is `unwind`, pinned in `[profile.release]`, and must
  never become `abort`** (tier 1 by profile + comment): the clean-500
  contract is `tower-http` CatchPanic = `std::panic::catch_unwind`, which
  "only catches unwinding panics" — and Cargo documents that tests IGNORE
  the `panic` setting, so an abort regression is untestable by construction
  (https://doc.rust-lang.org/cargo/reference/profiles.html#panic). Release
  builds carry `debug = "line-tables-only"` so a production panic names its
  file:line; `strip` stays `"none"` (rustc: stripping symbols makes traces
  "incomprehensible").
- **No `unwrap`/`expect`/`panic!`/`unreachable!`/`unimplemented!` in
  application code** (deny-tier lints; `todo!` was already denied). Tests
  keep the `clippy.toml` `allow-*-in-tests` scoping (unwrap/expect/panic/
  print/indexing — the Book ch11 doctrine: a panicking assertion is exactly
  how a test fails). Recoverable failures return typed errors (`thiserror`
  in libraries, `anyhow` only in binaries) — the Rust Book's ch9 split:
  panic is for states that cannot happen, `Result` for everything that can.
- **The ONE sanctioned escape for a logically-impossible `Err`/`None`**
  (Book ch9: "perfectly acceptable to call `expect` … and document the
  reason you think you'll never have an `Err` variant"): a narrowly-scoped
  `#[expect(clippy::expect_used, reason = "…")]` on the smallest item, whose
  reason states the inspection proving unreachability, plus a
  *should*-phrased message (`.expect("hardcoded IP address should be
  valid")`). Dodging the lint with `unwrap_or_default()`/`unwrap_or_else`
  instead is FORBIDDEN — that converts a loud impossible state into a
  silent wrong clinical value, the exact failure class this file exists to
  prevent.
- **No panicking indexing on request paths** (deny tier:
  `indexing_slicing`, `string_slice`): `.get(..)`/pattern matching over
  `x[i]` and `&s[a..b]` — `string_slice` panics on a UTF-8 boundary and
  clinical text is full of multi-byte content. Tests are scoped out via
  `clippy.toml`; a hot-path site PROVEN in-bounds uses the `#[expect]`
  escape above, never a bare index.
- **Guards are never silently dropped**: `let _ = lock/tx/handle;` is
  denied (`let_underscore_drop` + `let_underscore_lock`) — bind guards to
  named variables that live to scope end. `unused_result_ok` (deny) closes
  the `.ok();` variant — it looks like a check but only silences
  `#[must_use]`. Edition-2024 corollary (review-enforced; from the Edition
  Guide's own deadlock example): **a guard/transaction/borrow produced in an
  `if let` scrutinee is dropped BEFORE the `else` branch runs**
  (https://doc.rust-lang.org/edition-guide/rust-2024/temporary-if-let-scope.html)
  — never rely on it inside `else`; rewrite as `match` when the guard must
  span both arms. Tail-expression temporaries drop at end of block, before
  locals (temporary-tail-expr-scope.html) — same discipline.
- **`#[source]` over an `Option<Arc<…>>` or `Option<Box<…>>` yields the SMART
  POINTER as the source hop, not the error inside it** (verified first-hand on
  the pinned 1.96.1 toolchain against this workspace's `thiserror` 2, while
  landing #2115). The failure is invisible in a log — `Display` forwards, so the
  chain reads correctly — and `downcast_ref::<sqlx::Error>()` returns `None`,
  which is the entire point of carrying a cause, silently lost. A non-`Option`
  `Box<dyn Error + Send + Sync>` derives correctly; the optional form must
  hand-write `Display` + `Error` and return `self.source.as_deref()`. There is no
  lint for this: the only thing that catches it is a test that DOWNCASTS to the
  concrete error type rather than asserting `source().is_some()`, so a new
  source-carrying error type gets one.
- **`#[error(transparent)]` removes its own type from the cause chain** —
  verified first-hand on the pinned toolchain while landing #2034. Transparent
  forwards `Display` AND `source()`, so the wrapper is not a hop: walking from
  a `ServiceError` through a transparent `SignError` lands on the underlying
  `pgp::errors::Error` directly, and a test looking for the wrapper fails while
  the chain is perfectly intact. Assert the ROOT cause, not an intermediate
  type, or the test measures thiserror's forwarding rather than our chaining.
- **`Result → Option` inside a chain is a DECISION, and it carries NO
  automated guard** (review-enforced; the honest no-guard record, issue
  #1733). `.filter_map(|x| f(x).ok())`, `.and_then(|x| f(x).ok())` and
  `f(x).ok()?` turn an error into a missing element with no trace — the
  silent-data-loss shape that dropped a client-supplied attestation from a
  commit and a non-decoding version uid from an admin merge list. The rule:
  **a fallible conversion whose failure means "the input is DEFECTIVE"
  propagates a typed error; only a fallible conversion whose failure means
  "this input is legitimately ABSENT / not of this form" may become
  `Option`, and it carries a `// NOTE:` saying so.** Enforcement is honest
  about its own limits: this is the one rule in this file with no failing
  check, because there is no lint (the Clippy book lists none —
  https://rust-lang.github.io/rust-clippy/master/) and a grep gate cannot
  make the distinction the rule turns on. The two shapes are
  *textually identical* — a grammar probe where a parse failure IS the
  answer (`CaptureName::parse(a).ok()?` in the reference-grammar reader)
  reads exactly like a defect swallowed in a codec — and the repo carries
  ~300 `.ok()` sites, the vast majority legitimate, so a pattern gate would
  be ~99% false positives and would be blanket-suppressed within a release.
  A wish honestly labelled beats a check that trains people to ignore it.
- **Determinism is lint-backed** (deny tier: `iter_over_hash_type`):
  HashMap/HashSet iteration order is undefined; anything that feeds
  canonical JSON/XML, SQL generation, or emitted code iterates ordered
  structures (`BTreeMap`/sorted vecs) — byte-determinism is a codegen
  emitter invariant and a wire-parity requirement.
- **No debug/print output from libraries** (`dbg_macro`, `print_stdout`,
  `print_stderr` denied): libraries speak `tracing`; only the binaries and
  `tools/*` write to stdio (crate-root `#![allow]`/`#![expect]` relaxations
  there, each with a reason). `pointer_format` (deny) keeps addresses out
  of logs/Debug output.
- **Banned APIs are compile-time bans, not review notes** (`clippy.toml`
  `disallowed-methods`/`disallowed-types`, owner rulings 2026-07-30):
  `std::time::SystemTime::now` (wall-clock comes from jiff; `Instant` stays
  fine for latency), `std::env::var`/`var_os` (config flows through the
  config tree), `uuid::Uuid::new_v4` (DB keys are uuidv7), the
  `chrono::*` types (jiff is the one time library), and
  `Option::as_slice`/`as_mut_slice` (on an `Option<Vec<T>>` receiver they
  yield `&[Vec<T>]` — a slice of 0-or-1 *vectors*, not `&[T]` — and keep
  compiling after a field's shape flips between `Vec<T>` and
  `Option<Vec<T>>`; spell it `.as_deref().unwrap_or_default()` or match on
  the `Option` — added 2026-08-02, issue #1718; `Vec::as_slice` stays
  fine). A legitimate exception
  site carries a scoped `#[expect(clippy::disallowed_methods, reason)]`.
- **Errors are types, not strings, at every boundary that branches**
  (C-GOOD-ERR): a caller that needs to distinguish outcomes gets an enum
  variant, not a substring match. String context belongs in the display
  text, not the discriminant. (Review-enforced at the seam; the
  status-mapping tests pin the wire outcome.)
- **Ids are distinct types where confusion is fatal** (C-NEWTYPE): distinct
  id newtypes (`EhrId`, `VoId`, …) are used throughout, so the type system
  rejects a swapped-argument mistake at compile time (tier 1). Never pass a
  bare `Uuid` where a typed id belongs, and never add a function that takes
  two adjacent bare `Uuid` parameters. (This is the API Guidelines
  C-VALIDATE preference order made structural: static beats dynamic beats
  assertion beats opt-out.)
- **Every public item: documented, `Debug`, with concrete
  `# Errors`/`# Panics` sections** (C-DOC, C-DEBUG, C-FAILURE):
  `missing_docs` (rustc, tier 3) requires the doc comment;
  `missing_debug_implementations` requires Debug;
  `missing_errors_doc`/`missing_panics_doc` (pedantic members, tier 3)
  require the sections. Generated crates get their docs FROM THE EMITTER
  (BMM `documentation` propagation) — never hand-edit a `// @generated`
  file to document it. Doc quality is lint-backed too: the
  `[workspace.lints.rustdoc]` table (broken/private intra-doc links,
  invalid codeblock attributes, bare URLs at deny) + the CI doc job;
  doctests are copy-paste templates and deny warnings via
  `#![doc(test(attr(deny(warnings))))]` (C-QUESTION-MARK: `?`, never
  unwrap). **PHI caveat** (review-enforced): `Debug` impls and `tracing`
  fields must never carry clinical payload content — log identifiers and
  shapes, not bodies.
- **Visibility is deliberate** (C-STRUCT-PRIVATE): private by default,
  scoped visibility only at real module boundaries, zero re-exports
  (every import names its defining module), `unreachable_pub` watched at
  CI. Struct fields private unless the type IS a plain record.
- **Constructors and conversions follow the standard shapes** (C-CTOR,
  C-CONV/C-CONV-TRAITS, C-GETTER, C-BUILDER): `new`/`with_*` builders,
  `From`/`TryFrom` over ad-hoc `to_x()` where the conversion is total/
  fallible, getters without `get_` prefixes. (Tier 3 via pedantic +
  review; `avoid-breaking-exported-api = false` in `clippy.toml` keeps the
  API-shape lints live. The justification is NOT "nothing publishes" — the
  nine `openehr-*` crates publish to crates.io — it is the line they publish
  on: `0.0.x`, where cargo treats every patch as its own compatibility set
  (https://doc.rust-lang.org/cargo/reference/semver.html), so no consumer
  version range spans a break, and any packaged-content change already bumps
  all nine in the same PR. The `ferroehr-*` crates remain unpublished.
  Re-adjudicate if the line graduates past `0.x` — the same trigger the
  C-STABLE entry below carries.)
- **Blocking never hides in async**: no `std::sync` locks held across
  `.await` (clippy `await_holding_lock`, tier 3 — active via `clippy::all`),
  no synchronous I/O on the runtime; `spawn_blocking` for the rare
  CPU-heavy transform.
- **Dependencies are pinned, locked, and vetted** (tier 4): workspace-table
  only, `cargo deny check` green at all times (its advisories check reads
  the same RustSec DB as cargo-audit and adds yanked/licenses/bans/sources,
  so CI runs deny alone), no new dependency for what
  the pinned set already provides. CI builds run `--locked` (the Cargo FAQ's
  determinism rationale: CI fails on new commits, never on registry drift);
  the scheduled `latest-deps` workflow is the official strategy for
  discovering in-range upstream breakage on schedule. Provenance, labeled
  honestly: `cargo audit`/RustSec = the Rust Secure Code WG (the closest
  thing to official); `cargo deny` = third-party (Embark) — our own tooling
  choice; `cargo vet` = Mozilla, not adopted (a standing-commitment decision
  for the tracker if ever). Feature discipline: mutually exclusive features
  carry a `compile_error!` guard (the Cargo book's prescription — the
  viewer's `hydrate`/`ssr` pair), and `--all-features` lanes exclude the
  guarded crate, testing it per-feature.
- **Comment style is machine-enforced** (owner rulings 2026-07-17,
  2026-08-01, 2026-08-04; the full guide is `comments.md` — RFC 505 +
  RFC 1574): line comments only, unfinished work is `// TODO(#NNNN):` (issue
  reference mandatory), a settled spec-silent decision is `// NOTE:` as a
  citation + one sentence (≤3 lines), plain `//` runs ≤8 lines (essays live
  on the PR/issue), `// SAFETY:` reserved for `unsafe`
  (`unnecessary_safety_comment` guards misuse), and no unsanctioned marker
  vocabulary (the guard's list is the authority — only `TODO`, `NOTE`, and
  `SAFETY` are sanctioned).
  Enforcement (tier 4): `scripts/checks/comment-style.sh` — per-edit
  via the `rust_fmt_clippy.sh` hook, per-PR via the `comment-style` CI job
  (`--all`, the whole tree — the legacy sweep #1870 closed) — plus
  `clippy::too_long_first_doc_paragraph` (tier 3) for the RFC 1574 doc
  summary line. Change-narration and prose deferrals stay review-enforced
  (no tool can judge them).
- **An HTTP status is compared as a `StatusCode`, never as a number** (owner
  directive 2026-08-06). `status.as_u16() == 401` discards the type the `http`
  crate exists to provide, and `403` versus `404` is a one-character typo no
  compiler can catch. Rendering the number (a log field, a metric label, a
  recorded outcome) stays legal — only comparison against a literal is refused.
  Enforcement (tier 4): `scripts/checks/typed-status.sh`, per-edit via the
  hook and per-PR via CI at `--all`.
- **A browser journey never holds a raw WebDriver handle, and never waits on a
  timer** (issue #3296; the rule is `leptos-ui.md` §10). thirtyfour bounds a
  request only by its 120 s per-request default, which outlasts every poll loop
  in the E2E suite — so `button.is_enabled().await.unwrap_or(false)` on a raw
  `WebElement` reads a stalled driver as an inert control, and the journey
  skips the save it was written to prove and goes green on the prior state.
  Four such sites were live when this landed. The fix is a type: journeys hold
  `common::Element`, which bounds every command and panics on anything but an
  absence, and `thirtyfour::components::SelectElement` is wrapped there too
  (its commands issue their own requests). A `sleep` is the same defect in the
  time domain — it asserts nothing and is either a flake or waste — so
  conditions are waited for through the harness's `wait_*`/`poll_*` helpers.
  Enforcement (tier 4): `scripts/checks/e2e-waits.sh`, per-edit via the
  `rust_fmt_clippy.sh` hook and per-PR via the `e2e-waits` CI job, refusing
  `WebElement`, `SelectElement` and any `sleep` in
  `app/ferroehr-viewer/tests/it/e2e_*.rs`; its detector is mutation-proven by
  its own `--self-test`, which the CI job runs first. The harness
  (`tests/it/common/mod.rs`) is deliberately out of scope: it is where the raw
  handle is wrapped and where the poll loops sleep.
- **A field's default value lives in its struct's `Default` impl, inline**
  (owner directive 2026-08-06; the shape of RFC 3681, whose own syntax is
  nightly-only — feature `default_field_values`,
  <https://github.com/rust-lang/rust/issues/132162>). Banned: the per-field
  `#[serde(default = "path")]` form (it lets `Default::default()` and a
  deserialized value disagree about one field — a silent wrong configuration
  value, the failure class this file exists to prevent), zero-argument
  `fn default_x()` constructors, and single-reader `const DEFAULT_X`. A
  constant with several consumers stays a constant and may be read from inside
  the `Default` impl. Enforcement (tier 4):
  `scripts/checks/default-style.sh` — per-edit via the `rust_fmt_clippy.sh`
  hook, per-PR via the `default-style` CI job (`--all`).
- **The build pipeline is code, and it is analysed like code** (issue #2007,
  audited against the OWASP GitHub Actions Security Cheat Sheet,
  <https://cheatsheetseries.owasp.org/cheatsheets/GitHub_Actions_Security_Cheat_Sheet.html>).
  Four properties hold across every workflow: every `uses:` is pinned to a full
  commit SHA with its version in a trailing comment; `permissions: {}` at
  workflow level with the minimum granted per job; `persist-credentials: false`
  on every `actions/checkout` that does not use git against the remote; and no
  context value is interpolated into a `run:` block — it arrives through `env:`.
  A lane that PUBLISHES (a release, an image, a crate) additionally restores no
  build cache, unless no untrusted run can write its cache keys and the proof is
  recorded at the step. Enforcement (tier 4): the `zizmor` CI job over
  `.github/workflows/` at `--min-severity=low` with online audits enabled — the
  `unpinned-uses`, `excessive-permissions`, `artipacked`, `template-injection`,
  `cache-poisoning` and `impostor-commit` rules are the failing checks — plus
  CodeQL's `actions` language on every pull request. An accepted finding is an
  inline `# zizmor: ignore[rule]` carrying its reason, never a silent
  suppression.
- **A migration that has shipped is never edited** (owner rulings 2026-09-09
  and 2026-09-15; the policy is `sqlx-conventions.md` §Migrations). Shipped
  means present at the latest release tag: sqlx refuses a database whose
  recorded checksum no longer matches the migration file, so editing one an
  installation has applied does not revise history, it locks that installation
  out of its own database at boot, reporting a checksum rather than the edit.
  A file in no release yet has been applied by nobody and is fixed in place
  (the rewrite is breaking changes only, no rename migrations or placeholders).
  For a shipped file a schema change is a NEW file; a shipped migration that
  was wrong is superseded, never rewritten. A whole shipped SET may be retired,
  and only in the shape that keeps an installation told rather than locked
  out: every shipped file under `app/ferroehr/migrations/<schema>/` goes at
  once (a new set may take the directory), and `<schema>` is named in
  `FIRST_GENERATION_SETS` (`app/ferroehr/src/db/mod.rs`) in the same change, so
  the boot refusal names that database and states the remedy — by the
  bookkeeping's existence where this build owns no set of that name, and by the
  description of its version 1 where the name survives the rewrite. Enforcement
  (tier 4): `scripts/checks/migration-immutability.sh`, run per-PR by the
  `migration-immutability` CI job against the pull request's merge base over
  the files the highest release tag in the repository carries (ancestry not
  required: a rewritten `main` leaves the tags on the old lineage), and refusing any
  modification or partial turnover of those under
  `app/ferroehr/migrations/`; it accepts a retirement only after verifying both
  halves itself — every shipped file of that directory gone at head, the schema
  named in that table — so the acceptance cannot be claimed in prose. Its detector is mutation-proven
  by its own `--self-test`, which the CI job runs first. There is deliberately
  no escape-hatch label: the checksum makes the rule absolute, so an exception
  could only ever be a broken deployment.
- **The shell programs are analysed like code too** (issue #2785). The two
  tooling languages here are bash and Rust, and only the Rust half was ever
  lint-gated. actionlint shellchecks the `run:` blocks EMBEDDED in workflows;
  the scripts those blocks call — every CI guard, every vendor fetcher, every
  deploy probe, the release machinery, the git hooks — were checked by nothing.
  ShellCheck now runs at `--severity=style`, its lowest floor, so every finding
  gates. A finding is FIXED, or it carries a per-line
  `# shellcheck disable=SCnnnn` directive with its reason on the same line; a
  blanket exclusion is refused, and no `.shellcheckrc` exists, because a file
  that can turn a code off tree-wide eventually does. Discovery is derived
  rather than listed: every tracked `*.sh`, plus every tracked extensionless
  file whose first line is a shell shebang (which is what covers
  `.githooks/commit-msg` and the next hook after it), minus the vendored trees
  nobody here may edit. Enforcement (tier 4):
  `scripts/checks/shellcheck-lane.sh`, run per-PR by the `shellcheck` CI job
  against the pinned upstream release binary (`shellcheck@0.11.0` — never the
  runner's distro package, whose version would drift under the adjudicated
  directives at every runner-image refresh).
- **A published compliance claim is generated from its source, never typed**
  (issue #3164). The control matrix on the documentation site states which
  legal control each issue delivers and whether it is shipped; typed by hand it
  is wrong the first time a control lands, and a stale "planned" beside a
  shipped control is exactly the misstatement the page exists to prevent.
  `scripts/render/control-matrix.sh` renders it from the tracker, resolves
  every legal source through a registry inside the generator (an undeclared
  short name stops the render rather than becoming a free-text cell), and is a
  pure function of tracker state: no clock, no build commit, nothing that
  differs between two runs of an unchanged tracker. Enforcement (tier 4): the
  `control-matrix` CI job runs the generator with `--check` and fails on any
  difference from the committed page.
- **No real personal data enters the tree, and the pseudonymisation boundary is
  reviewed when it moves** (issue #3167; the rules are `personal-data.md`,
  restated for contributors in `CONTRIBUTING.md` § Personal data). Tests and
  fixtures use synthetic values; logs, traces and `Debug` output carry record
  identifiers and shapes rather than a subject's data; no database grant spans
  the clinical and demographic domains. Enforcement (tier 4):
  `scripts/checks/privacy-boundary.sh`, run per-PR by the
  `privacy-boundary-guard` CI job — it refuses a diff touching the boundary
  paths whose pull request body carries no ticked "Privacy boundary" checklist,
  and it reads the added lines for a nine-digit value passing the BSN
  eleven-test or a Dutch postcode followed by a house number. The two data
  detectors run under `--self-test` before every judgement, so the arithmetic is
  exercised rather than trusted. Honest about its own limits: no check can tell
  an invented name from a real one or read intent out of a `tracing` field, so
  the synthetic-data and telemetry rules stay review-enforced at the boundary.
- **No Apache-2.0 crate depends on a BUSL-1.1 crate** (owner request
  2026-09-25, issue #3485). The six Apache-2.0 crates (`openehr-base`,
  `openehr-lang`, `openehr-term`, `openehr-rm`, `openehr-am`, `openehr-its`)
  publish so any Rust project can take them; a normal or build dependency on
  `openehr-query`, `openehr-adl` or `openehr-sdt`, or a feature naming one,
  would make every consumer of the wire layer compile BUSL code.
  Dev-dependencies are exempt, because cargo strips them at packaging. Each
  crate's `license` must also match its side, so relicensing a crate cannot
  move it across silently, and a new crate under `crates/` must be placed on
  one side before it passes. Enforcement (tier 4):
  `scripts/checks/licence-boundary.sh`, run per-PR by the `licensing` CI job
  over every `crates/*/Cargo.toml`, including `[target.*]` tables and
  dependencies renamed with `package =`. Its detector is mutation-proven by its
  own `--self-test`, which the job runs first.

## Recorded deviations from the API Guidelines (deliberate, owner-adjudicated)

- **C-SERDE — satisfied via emitted MANUAL impls** (#1702): the spec types implement
  `serde::Serialize`/`Deserialize` through explicit generated code
  (per-crate `json_serde.rs` from `emit-json`), never derives and never
  serde attributes — the manual form is what lets the strict reader,
  `_type` dispatch, and validated-constructor parsing live in auditable
  code (root `CLAUDE.md` §Code generation). The wire contract stays pinned
  by the canonical-output gates.
- **RFC 0356 (no module-name repetition) — UNSATISFIABLE under two of our own
  hard rules, and currently unenforced** (adjudicated 2026-08-06). The RFC
  (<https://rust-lang.github.io/rfcs/0356-no-module-prefixes.html>) offers
  exactly two ways to cope with the name collisions it creates: qualify by
  module (`io::Error`) or rename on import (`use io::Error as IoError`).
  Import renaming is banned outright (`rust-style.md`), and under zero
  re-exports the qualified path is not `io::Error` but
  `crate::config::server::Config` — so with eleven modules each defining a
  `Config`, every use site would carry a full path inline. Renaming the 273
  hand-written candidates would make the code LESS readable under our own
  import rules, and the 141 generated ones are immovable (the emitter mirrors
  BMM class names, and `codegen.md` forbids trimming the model). Enforcement
  status is honest: `clippy::module_name_repetitions` moved from `pedantic` to
  `restriction` in clippy 1.84, so `pedantic = deny` does NOT cover it — the
  lint is off, deliberately, and this entry is why.
- **RFC 2008 `#[non_exhaustive]` / `clippy::exhaustive_enums` — REJECTED for
  the spec model** (adjudicated 2026-08-06). openEHR subtype sets are closed
  BY THE BMM; a forced `_ =>` arm would turn a modelled fact into a silent
  fallthrough and defeat the exhaustiveness checking that makes a spec-model
  change a compile error. 1672 sites, and the rejection is on merit, not cost.
- **C-PERMISSIVE — not followed; the project's own code is under the Business
  Source License 1.1** (owner decision 2026-09-03, superseding the MIT decision
  of 2026-07-31; coverage corrected 2026-08-04, issue #1883). BUSL-1.1 is
  The six generated-or-wire `openehr-*` crates (`base`, `rm`, `am`, `lang`,
  `term`, `its`) are the EXCEPTION: they publish under Apache-2.0 (the licence
  of their openEHR inputs; `openehr-its` carries the hand-written runtimes,
  entry points and wire-validation dispatcher its generated code cannot ship
  without) so any Rust project can consume them; C-PERMISSIVE holds for them.
  The three hand-written engines (`openehr-query`, `openehr-adl`,
  `openehr-sdt`) carry BUSL-1.1 like the application (owner decisions
  2026-09-04 and 2026-09-24, #3482). The application is source-available, not open source: non-production use is free, production
  use is free for Non-Commercial Purposes only (research, teaching, personal
  use, non-profit or public bodies outside the course of a business), and any
  other production use, hosting for third parties, or distribution for a fee
  needs a commercial licence from the Licensor (Cadasto B.V., which is
  also the sole copyright holder every SPDX header names — owner ruling
  2026-09-16, issue #3435, and 2026-10-05, issue #3567; Ruben Talstra stays the maintainer credit the
  banner, `MAINTAINERS.md` and the OCI `authors` label carry), and each version
  becomes Apache License 2.0 four years after publication (`LICENSE` carries
  the parameters; releases up to v4.0.17 and crate versions up to 0.0.56 stay
  MIT as published, under the holder they were published with, which is what
  `LICENSES/MIT.txt` reproduces). The chosen licence deliberately blocks resale and hosted
  offering by others; contributions carry the relicensing grant in
  `CONTRIBUTING.md` so the work stays one work under one licensor. The
  Apache-2.0 carve-out that covered the
  conformance instrument (#2787) left with it — the instrument is now the
  separate Veredictum project, Apache-2.0 under its own root, pinned here by
  `scripts/lib/veredictum.sh` (#2811). Vendored third-party material keeps its upstream
  terms — Apache-2.0 for the openEHR machine-readable artifacts and the
  vendored test corpora (`LICENSE-APACHE-2.0`), CC-BY-SA 3.0 for the openEHR
  spec docs text (`LICENSE-CC-BY-SA-3.0`), and BOTH CC-BY-SA **4.0** and
  CC-BY-SA **3.0** for the CKM-derived clinical models — corrected 2026-08-12
  by a first-hand count over the vendored corpus, which finds 4.0 in the
  majority and several hundred 3.0 files beside it, so the earlier "one
  version per corpus" claim was false in the direction it warned about. Each
  archetype's own `licence` metadata is the authority for that file;
  `REUSE.toml` declares the trees as `CC-BY-SA-3.0 AND CC-BY-SA-4.0` because
  neither version alone is a true statement about them. The vendored ADL2 regression corpus
  additionally carries three ISO 13606 BMM reference models offered under an
  MPL 1.1 / GPL 2.0 / LGPL 2.1 tri-license; **we take them under MPL 1.1**, so
  no GPL or LGPL obligation attaches (the election is recorded in that
  corpus's `PROVENANCE.md`). Every vendored tree's
  `PROVENANCE.md` names its license, with the upstream `LICENSE` vendored
  alongside. Dependencies stay license-gated by `deny.toml`.
- **C-STABLE — re-adjudicated for publication (owner rulings 2026-08-04,
  issue #1886 + same-day correction)**: the `openehr-*` crates publish to
  crates.io on their **own independent SemVer line**, currently `0.x` and
  permanently decoupled from the vendored spec versions
  (`docs/VERSIONS.md` §Product and crate versioning), so pre-1.0 crates
  exposing pre-1.0 public deps (`jiff 0.2`, `chumsky 0.13`, …) is
  semver-honest and the original blocker does not bite. It RE-ARMS if the
  line ever graduates past `0.x`: declaring stability requires
  re-adjudicating every pre-1.0 public dependency in the published API
  first (the version chosen then is still ours, never a spec number).

## When a lint fights a legitimate case

**`#[expect(lint, reason = "…")]` is the default suppression** — it
self-reports the moment the expectation stops being fulfilled
(`unfulfilled_lint_expectations`), so stale suppressions cannot accumulate.
`#[allow(lint, reason = "…")]` is reserved for cases where the lint fires
only in SOME configurations (cfg/feature-dependent code, macro expansions) —
an `#[expect]` there would itself warn in the quiet configuration. Both
forms MUST carry `reason = "…"` (`allow_attributes_without_reason` = deny;
`allow_attributes` = warn steers toward `#[expect]`). Scope every
suppression to the smallest item. A file-level or crate-level suppression
needs the owner's sign-off in the PR.

---
paths: ["crates/**", "app/**", "tools/**"]
---

# Testing discipline

Test discipline is non-negotiable (a standing hard rule; see `CLAUDE.md`). It
applies to every crate — generated and hand-written alike.

## The hard rule

- **Never** silently weaken, skip, or delete an existing test to make a
  build pass.
- **Never** edit a test to route around a runtime bug it exposes. If a test
  fails and the fix is unclear, leave it failing and record a
  `// TODO(#NNNN):` naming its issue — do not touch the test to make it
  green.
- Conformance/corpus tests assert the **openEHR specifications**:
  cite the spec clause a test encodes; never adjust an expectation to match
  an implementation bug. Corpus/fixture defects are ADJUDICATED with a
  first-hand spec/schema citation (an expected-rejection entry in the owning
  gate, or, for spec silence, an `upstream-report` issue that the
  conformance instrument's option or `report:` citation carries —
  `.claude/rules/cnf-triage.md`), never routed around by editing the case.

## Tooling

- **Runner:** `cargo-nextest` (`cargo nextest run --workspace`), not
  `cargo test`.
- **Snapshots:** `insta` pins canonical JSON/XML output against golden
  vectors — the key tool for serialization parity. Redact volatile fields
  (timestamps, generated UUIDs) before snapshotting. Review intentional
  changes with `cargo insta review`; never accept a snapshot change you have
  not read.
- **Properties:** `proptest` for RM round-trips (serialize → parse → equal),
  parser stability, and AQL parse/print round-trips.
- **Database:** every DB-backed test takes its database from the shared
  harness — `testkit::db()` (`tools/testkit`): one PostgreSQL 18 server
  (`FERROEHR_TEST_PG_URL` in CI, a reusable `ferroehr-testkit-pg18`
  testcontainer locally), one migrated template database per migration
  fingerprint, one `CREATE DATABASE … TEMPLATE` clone per call. **Never
  start a per-test PostgreSQL container or run migrations in a test** — it
  costs ~5–10 s + Docker contention per test.
  Broker/blob tests (RabbitMQ, SeaweedFS) still run real testcontainers,
  serialized via the nextest `containers` group.
- **HTTP mocking:** `wiremock` for the terminology/FHIR client and any
  external integration test.
- **Benches:** `criterion` + `divan`, kept separate from correctness tests.

## Oracles and the acceptance instrument

- **The acceptance instrument is Veredictum, the CNF 2.0 runner** (an
  independent project, pinned in `scripts/lib/veredictum.sh` and driven by
  `scripts/conformance.sh`) — the data-driven interpreter over the committed
  machine-readable catalogue, with pure-function verdicts. Issue-close runs
  must show **zero drift** vs the committed baseline
  (`docs/conformance/ferroehr/results.json` and `summary.md`); the
  baseline only ratchets upward.
- **The vendored CNF text is the STALLED structural GUIDE the instrument's
  COVERAGE derives from — NOT the correctness oracle** (owner ruling
  2026-07-24; openEHR CNF never released a stable version):
  `docs/specs/openehr/CNF/docs/platform_test_schedule/` names WHICH behaviours
  to exercise, but the CORRECT behaviour is derived from the RELEASED spec
  components (RM / BASE / AM / QUERY / TERM / ITS-XML / ITS-REST docs text) —
  where the schedule and a released spec conflict, the released spec wins. The
  upstream Robot suites under `CNF/tests/platform/robot/` are stalled reference
  material; their official DATA fixtures are adopted into the runner corpus only
  as provenance-stamped re-adjudications (never blind imports, never as an
  oracle).
- Golden vectors: openEHR conformance corpora, the vendored canonical-JSON
  corpus, Better's `web-template-tests`, openEHR reference archetypes.
  Prefer an existing golden vector over a hand-written fixture. A test that
  encodes a spec rule cites the spec/CNF section it asserts
  (spec-adherence.md).
- **Material two sides read lives under `corpus/`, and a test in a published
  crate never reads a path under `app/`** (#3408). The dependency arrows run
  `app/* -> crates/*`, and a fixture path is a string the compiler cannot
  check, so a `crates/` test opening `../../app/...` makes the crate
  untestable from its own package and pins a fixture the application is then
  not free to move. Each `corpus/` subtree carries its own `PROVENANCE.md`.
  Enforced by `scripts/checks/crate-tests-read-no-app.sh` (the
  `spec-citations` CI job, with a `--self-test` that runs first).

## Where tests live

Unit tests live beside the code they test (`#[cfg(test)] mod tests` in the
same file) — and ONLY there: **dedicated test FILES under `src/` are banned**
(owner ruling 2026-07-17). A test
that drives the public API belongs in the owning crate's `tests/` directory
(`crates/*/tests/`, `app/*/tests/`, `tools/*/tests/`); a test of private
internals stays a small inline module next to the code it tests. If an
internals test grows large, that is a design signal to test through the
public seam, not to split the tests into a src file. Do not invent a third
location.

**One integration-test binary per crate** (issue #1311, 2026-07-30): the
`tests/` directory is `tests/it/main.rs` + one `mod` per topic file — NOT
one top-level `.rs` per topic. Cargo compiles and links every top-level
`tests/*.rs` as its own crate ("each integration test results in a separate
executable binary … this can be inefficient, as it can take longer to
compile" — https://doc.rust-lang.org/cargo/reference/cargo-targets.html).
nextest still runs each test
as its own process, so isolation is unchanged; only the compile/link waste
goes. Shared helpers live in a plain module under `tests/it/` (the
`tests/common/mod.rs` rule generalizes: helper modules are never top-level
test files). A second binary in one crate needs a real reason (e.g. a
different harness) stated in a comment.

**A binary-only crate is untestable by construction** (Book ch11.3): its
`main.rs` cannot be imported from `tests/`. The wiring binary
(`app/ferroehr-server`) therefore keeps a thin `main.rs` over a testable
`lib.rs` run path (Book ch12.3), and its integration tests import the lib.
Never park tests for crate X under crate Y's `tests/` directory.

## Test shapes (the Book ch11 doctrine)

- **`Result`-returning tests are the preferred shape**: `fn t() ->
  Result<(), E>` with `?` instead of unwrap chains — the officially blessed
  way to keep test bodies panic-idiom-free
  (https://doc.rust-lang.org/book/ch11-01-writing-tests.html). The
  `clippy.toml` `allow-*-in-tests` scoping keeps assertion panics legal, but
  plumbing failures should propagate with `?`, not `.unwrap()`.
  **`clippy::panic_in_result_fn` (deny, workspace-wide) fires on this shape
  and clippy offers NO `allow-…-in-tests` knob for it** (verified
  empirically on the then-pinned 1.97 toolchain (re-verified at the 1.96→1.97 bump): `allow-panic-in-tests = true` is
  already set and the lint still fires inside a `#[test] fn -> Result<…>`
  that asserts; the clippy lint-configuration page lists no option for this
  lint —
  https://doc.rust-lang.org/clippy/lint_configuration.html). Adjudication:
  **the Book shape wins in tests, the lint keeps its full strength in
  production code.** A Result-returning test that also asserts carries
  `clippy::panic_in_result_fn` in the same scoped relaxation its file
  already uses for `panic`/`unwrap`/`expect`
  (`#![allow(…, reason = "test assertions/diagnostics/fixtures")]` at the
  test-file root, or a `#[expect(…, reason)]` on the single test). It is
  never relaxed at the workspace level, and never in a non-test module.
- **`#[should_panic]` always carries `expected = "…"`** — bare
  `should_panic` passes when the code panics for the WRONG reason (Book
  ch11.1), unacceptable in a suite that adjudicates spec behaviour.
  Constraint: `should_panic` is illegal on Result-returning tests — assert
  `value.is_err()` there instead.
- **Assertions**: `assert_eq!`/`assert_ne!` over bare `assert!` for
  comparisons (they print both values); production-code asserts carry a
  message (`missing_assert_message` — the lint ignores test fns by design).
- **Doctests are copy-paste templates**: `?` via a hidden `# Ok::<(), E>(())`
  tail or hidden `fn main` wrapper, never `unwrap` (C-QUESTION-MARK;
  enforced by `#![doc(test(attr(deny(warnings))))]` on library roots).
  `no_run` for examples that would touch Postgres/HTTP, `text` for
  non-code — **never `ignore`** ("almost never what you want",
  https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html).
  Edition 2024 merges compatible doctests; a doctest asserting line numbers
  or panic locations must be marked `standalone_crate`. The five generated
  spec crates keep `doctest = false` DELIBERATELY (generated doc text is not
  curated examples) — do not "fix" that.

## CNF coverage (breadth is a mandate, not just pass rate)

A green pipeline over a thin catalogue proves almost nothing — the real
acceptance bar is COVERAGE. The CNF catalogue (Veredictum's `artifacts`)
must exercise EVERYTHING the spec defines on the wire: every ITS-REST operation,
every status-code branch (200/201/204/400/404/409/412/422/…), every
required/conditional header (`ETag`, `Location`, `Last-Modified`, `Prefer`,
`If-Match`), content-negotiation variants (JSON + XML, `Accept` q-values),
precondition and error families, and every RM/AQL behaviour — each as its own
small, ISOLATED case so a red row localizes to one behaviour. Every small
use-case counts; the goal is total behavioural coverage.

- **A spec-defined wire behaviour with no case is a COVERAGE GAP, never an
  acceptable omission.** Close it (a new spec-cited case) — or, only where the
  spec genuinely puts a behaviour off-wire, record the honest boundary
  (a statement-declared capability or option arm, or an `upstream-report`
  issue the catalogue cites). Silence is not coverage.
- **Coverage only ratchets up.** Cases are added, never removed to go green;
  narrowing coverage needs an adjudicated, spec-cited reason.
- **One behaviour per case** — many small isolated cases beat one broad case,
  because a failure then names exactly one defect (which is also what makes the
  attribution law tractable, `.claude/rules/cnf-triage.md`).
- **An adjudicated spec-correct refusal always yields BOTH twins (owner hard
  rule, 2026-07-29).** When triage attributes a red row to a defective
  fixture that the SUT was spec-RIGHT to refuse, fixing the fixture is only
  half the job: the invalid shape is preserved as its own corpus entry
  (`validity: invalid` with the defect + spec_ref) plus a refusal case, so
  the catalogue carries the valid twin (acceptance proven) AND the invalid
  twin (the refusal pinned — a lenient server fails it). Deleting the
  invalid shape silently narrows coverage; the first instance is the
  undefined-ac-code OPT pair (VATDF/VACDF, 2026-07-29).
- Same completeness discipline the vendored corpora carry (100% exercised,
  coverage-gated — never partial coverage that silently narrows the claim).

## Target

A green CNF pipeline is the standing bar (CORE + STANDARD PASS) —
every change preserves it; the baseline only ratchets upward, and green comes
ONLY from fixing the guilty component after spec-adjudicated attribution
(`.claude/rules/cnf-triage.md`), NEVER from bending the catalogue or runner to
match this server. Every change ships compiling, clippy-clean, tested
increments — this whole rule is fully active at all times.

## Test-fixture construction: typed by default, raw JSON only where raw is the point

Three classes (owner question 2026-08-03; the canonical_json_literals gate
deliberately scopes to production code, so this is the test-side rule):

1. **Refusal/negative fixtures: raw JSON, MANDATORY.** An invalid shape
   (missing mandatory, empty `1..*` list, undeclared key) is unrepresentable
   in the typed model — raw bytes are the only
   way to author what the reader must reject.
2. **Client-simulation inputs** (bodies posted through a REST/service seam):
   raw JSON permitted — independently-authored bytes catch codec bugs that
   typed-then-serialized values cannot, and the strict reader validates the
   fixture on the way in (an invalid "valid" fixture fails loudly).
3. **Everything else** (expected values, non-wire construction, values the
   test only manipulates in memory): build the typed `openehr-rm` value and
   serialize via `to_canonical_value` — compile-time-correct across pin
   bumps. Do not hand-roll `json!` here.

---
paths: ["sonar-project.properties", ".github/workflows/sonar.yml", "scripts/sonar/**"]
---

# Machine review (SonarQube Cloud) — what it is and what it is not

Every pull request and every main push is analyzed by SonarQube Cloud
(`.github/workflows/sonar.yml`; scope in `sonar-project.properties`; #2630).
It exists because the local gates catch what a lint can catch, review is one
person, and a deterministic multi-language sweep also covers trees the Rust
gates never see: shell scripts, workflow YAML, the website's JS/HTML/CSS,
Dockerfiles.

It is a **second opinion**. It is not authority, and it gates no merge.

## Precedence — a finding never outranks the sources

1. The vendored openEHR spec text (`docs/specs/openehr/`) — the oracle.
2. The hard rules: root `CLAUDE.md`, the crate `CLAUDE.md` files, `.claude/rules/*.md`.
3. The local gates: `cargo fmt`, `clippy`, `cargo nextest`, the CNF suite, the CI guards.
4. The analyzer.

A finding that contradicts a spec citation is wrong by construction — the
spec text is never a suspect (`spec-adherence.md`). A finding that asks for
something the rules forbid is wrong the same way. Nothing it reports relaxes
`testing.md`: never weaken a test, never adjust a CNF expectation, and never
edit a corpus fixture because a finding suggested it. A rule that contradicts
a deliberate recorded decision (the RFC 2008 rejection, zero re-exports, …)
is rejected as a class in the quality profile, with the reason recorded on
the triage program (#2640).

## Metric-scope adjudications (#2862)

Two scope decisions keep the dashboard numbers meaning something, both
duplication/coverage-ONLY — the affected trees stay fully analyzed for
findings:

- **`sonar.cpd.exclusions`** carries the ITS-REST wire-declaration surface
  (the per-group `openapi_routes.rs` files and the declaration-dominated
  handler files) and the codegen decision maps/templates. The released party
  CRUD is five byte-identical operation quintets and every `#[utoipa::path]`
  block mirrors its released operation file 1:1 for spec review; the
  template twins are near-identical by the #1964 design. Measured before the
  exclusion: ~20.5k of the 30.8k duplicated lines (67%) sat in that class.
  Extracting or macro-izing it would hide the citation surface, not remove
  repetition the wire does not itself carry.
- **`sonar.coverage.exclusions`** carries `app/ferroehr-viewer/**`: the
  viewer's acceptance instrument is the browser journey battery
  (`scripts/ui-e2e.sh`) and the published-image login probe, which no lcov
  run can observe (browser/wasm execution) — the metric misstated verified
  code, and view-macro unit tests would be the line-execution-only class
  `testing.md` forbids.

Both were adjudicated per-cluster from the live MCP ranking on #2862, where
the before/after numbers are recorded. Do not widen either list without the
same per-cluster case. #2915 widened `sonar.cpd.exclusions` by two pinned
expectation TABLES — `vendor_bmm_schema.rs` (the per-fixture adjudication
register over the 43 vendored BMM schemas) and `model_query.rs` (the
model-query report pinned row-for-row) — on the decision-map argument: each
"duplicated" run is a decided record, and compressing the rows hides the
adjudication the file exists to carry.

## New Code = since the last release (#2657)

The project's New Code definition is **"Previous version"**, anchored by
`sonar.projectVersion`: the sonar lane reads the workspace `version` from the
root `Cargo.toml` at scan time and appends it to the properties file — never
a second hand-maintained copy. The workspace version bumps at every release
cut (milestones are releases), so the quality gate's `new_*` conditions, PR
decoration, and the coverage program (#2656) all measure "since the last
release" — the same window the changelog section and the conformance baseline
describe. Do not switch the definition to days/reference-branch without
re-adjudicating that alignment.

## How Rust is analyzed

Rust is first-party in SonarQube Cloud (announced 2025-04-17): the analyzer
runs Clippy itself — 85 Clippy rules managed as quality-profile rules, plus
complexity metrics. That run uses the workspace DEFAULT features, a
deliberately independent second Clippy configuration beside our deny-tier
lanes; for pure Rust it mostly re-reports what the gates already enforce,
and its added value is the multi-language sweep, PR decoration on new code,
and a stable rule taxonomy to adjudicate class-by-class. Coverage IS wired
(owner directive 2026-08-24, reversing the #2630 deferral): the sonar lane
runs the instrumented nextest suites against a real PG18 and imports the
merged lcov via `sonar.rust.lcov.reportPaths`; the former ci.yml coverage
job and the badges-branch machinery are gone — the README's coverage badge
is Sonar's own, measured over the hand-written scan scope. SQL is excluded from scope entirely: the
PLSQL analyzer assumes Oracle and this tree's SQL is PostgreSQL (#2643).

## Sonar Architecture — adjudicated unavailable (2026-08-24, #2655)

The Architecture feature (current/intended architecture maps, deviations as
issues) is not in play here, for two independent reasons verified first-hand:
the organization is on the FREE plan and the feature requires Team/Enterprise
(`api/navigation/organization` → `"subscription":"FREE"`), and its language
support is C#/Java/JavaScript/Python/TypeScript — no Rust — so even a plan
upgrade would only ever cover the website's JS/TS. Re-evaluate if Sonar ships
Rust support for it AND the plan changes; the architecture record remains
`docs/architecture.md`, which no analyzer output outranks.

## The findings mirror into GitHub code scanning (#3032)

Sonar's own GitHub App uploads SECURITY-only code-scanning analyses, which
arrive empty here — the tools page listed SonarCloud with 0 results while
the dashboard carried the real set. A push to `main` therefore exports the
project's OPEN and CONFIRMED findings as SARIF
(`scripts/sonar/issues-to-sarif.sh`) and uploads them under the `sonarcloud`
category. The dashboard stays canonical: the query excludes ACCEPTED and
FALSE_POSITIVE, so a disposition recorded there closes its GitHub alert on
the next push, and the two surfaces cannot disagree for longer than one
analysis. Pull requests keep Sonar's own decoration; the mirror gates
nothing. The App's native (empty) security uploads stay enabled — they cost
nothing and would cover the vulnerability class if the mirror lane ever
broke. Dependabot pull requests skip the scan entirely: a dependabot-actor
run reads Dependabot secrets, not the Actions `SONAR_TOKEN`, and the
post-merge main push analyzes the merged result anyway.

## It does not gate a merge, and it never writes

No quality gate blocks a merge. Findings worth acting on are written by
hand in a normal change — never applied through any UI that would attribute
a commit to a bot (the no-AI-attribution rule has no exceptions). Promotion
to a gating check would follow a precision measurement, the same bar every
reviewer here has been held to.

## False positives are data

Record a wrong finding on the triage program (#2640) rather than silencing
it; a scope or profile change is made when the scope is actually wrong,
never to make a number go down.

## History: CodeRabbit (removed 2026-08-24, #2638)

CodeRabbit (an LLM reviewer) held this role from #2142. Its measured
precision over 25 PRs was ~60% true positives (#2148) — kept advisory for
exactly that reason — and it was removed in favor of the deterministic
analyzer once SonarQube Cloud landed. The measurement record stays in those
closed issues. CodeQL continues to run separately as the security scanner
(`.github/workflows/codeql.yml`).

Official documentation (durable citations):
<https://docs.sonarsource.com/sonarqube-cloud/> ·
<https://www.sonarsource.com/knowledge/languages/rust/> ·
<https://docs.sonarsource.com/sonarqube-cloud/analyzing-source-code/languages/rust>

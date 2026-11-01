---
name: run-conformance
description: >
  Runs the openEHR conformance pipeline (scripts/conformance.sh): the pinned
  Veredictum release's catalogue against one composed FerroEHR, under the
  released and development specification profiles, and the measured
  performance stages beside it. Use when the user asks to check conformance,
  run the conformance/CNF suite, verify spec compliance, measure performance,
  or at phase close for the zero-drift gate.
allowed-tools: [Read, Bash]
argument-hint: "[review | perf | perf-aa | CONF_DOCUMENTS=\"<document id> …\"]"
---

# /run-conformance

Runs [Veredictum](https://github.com/Vernum-Projecten/Veredictum), pinned in
`scripts/lib/veredictum.sh` (`VEREDICTUM_VERSION` and the release image by
digest), against one FerroEHR deployment. CI is the record: the Conformance
workflow (`.github/workflows/conformance.yml`) runs the pinned Veredictum
GitHub Action against the build of every pull request, merge group, push to
`main` and release that touches the server, the party set or the pipeline.
`scripts/conformance.sh` runs the same catalogue locally through the same
image.

## Ground rules (before touching ANYTHING)

**The oracle-and-attribution law (`.claude/rules/cnf-triage.md`).** The
vendored openEHR specification text (`docs/specs/openehr/`) is the reference
and is never a suspect. Everything we did not take from it is a suspect,
none privileged: FerroEHR (`app/*` + `crates/openehr-*`), the party set
(`docs/conformance/party/ferroehr/statement.json` and `ixit.json`), and
Veredictum (its catalogue, the judge that reads an exchange, the runner that
drives it). A red row is decided by reading the specification, never assumed.
**FerroEHR is NOT presumed correct because "we wrote it carefully to the
spec"; it is the most common real culprit.** Two reflexes are FORBIDDEN:

- *"Our code is right, so Veredictum must be wrong"*: reaching for the
  catalogue or the runner to make a red row green. A red row is not evidence
  the catalogue is wrong.
- *"Let me check our server"* to decide what the expected value should be.
  The server's response is **evidence** for the comparison, NEVER the
  reference. The reference is the specification text, read first-hand.

- **Never weaken a case to pass.** A failing row against our server is a
  correct instrument outcome until the specification says otherwise.
- **Coverage only ratchets up.** A specification-defined behaviour the
  catalogue does not yet exercise is a gap to close in Veredictum, never an
  acceptable omission (`.claude/rules/testing.md` §CNF coverage).
- **The record has no tier verdict.** It counts rows per part: BASE (what
  every claimant of a capability must do) and OPTIONS (the arms the statement
  selects). `scripts/render/conformance-stats.sh` reads the counts from
  `results.json`; never hand-type a count, a badge or a tier claim anywhere.

## The two profiles

- **`released`** reads every citation against the frozen openEHR release. It
  is the record of record: the committed `docs/conformance/ferroehr/` and
  the job's verdict, which fails a pull request and the merge queue on any
  failed or errored row.
- **`development`** reads the same citations against the pinned master of
  each openEHR repository, which FerroEHR builds on. It is published beside
  the record (its own job-summary heading, artifact and badge branch, and
  `docs/conformance/ferroehr-development/` locally) and never counted toward
  it. It fails the job only when it could not run at all.

## Steps

1. **Preflight:** Docker or Podman available; no conflicting
   `ferroehr-cnf` compose project running. The instrument always drives a
   deployed server over HTTP.
2. **Review the statement** (no deployment):
   ```bash
   bash scripts/conformance.sh review                         # released
   CONF_PROFILE=development bash scripts/conformance.sh review
   ```
   `veredictum validate --statement` must report `0 finding(s)`.
3. **Run.** Our server, built from the current tree, FerroTERM beside it, the
   SMART documents included (the committed test issuer under
   `docs/conformance/party/smart/` signs every scoped token in memory):
   ```bash
   bash scripts/conformance.sh                                # released
   CONF_PROFILE=development bash scripts/conformance.sh
   CONF_DOCUMENTS="<document id> …" bash scripts/conformance.sh   # a narrowed working run, never the record
   ```
   It writes `results.json`, `recording.har` (every exchange, response bodies
   included: gitignored, never published or uploaded), `junit.xml`,
   `ctrf.json` and `summary.md`. Exit 0 means every row passed, was skipped
   or is only a warning; 1 means a row failed or errored; 2 means the run
   could not start. `veredictum show results.json` prints the BASE and
   OPTIONS view in the terminal. `SKIP_BUILD=1` with `FERROEHR_IMAGE` judges
   a published image.
4. **Compare against the committed record**
   (`docs/conformance/ferroehr/results.json` and its `summary.md`): the only
   permitted delta is newly green rows, **zero drift**. Report passed,
   failed, warning, errored and skipped per part (BASE, OPTIONS), every
   skip by its reason (`not-claimed`, `option-not-selected`, `not-declared`,
   `cited-text-absent`), and any drift row by row.
5. **On any red row, ATTRIBUTE before you touch anything**
   (`.claude/rules/cnf-triage.md`; **delegate a red run to the `cnf-triage`
   agent**). Per row: read what was sent and received (the row in
   `results.json` with its expected and received values, and the exchange in
   `recording.har`); read the workflow the row comes from and its
   `x-veredictum-cite`; read the cited specification section first-hand and
   derive what a conformant server must answer; compare the three and
   attribute to exactly one of FerroEHR, the party set, Veredictum or the
   specification itself. Then act the one sanctioned way for that outcome
   (the rule lists them). Never adjust an expectation to match observed
   behaviour, never change FerroEHR without a reproduced exchange, never
   assume FerroEHR is the correct one.
6. **At phase close:** the committed `results.json` and `summary.md` under
   `docs/conformance/ferroehr/` are the released-profile run of the current
   release (the `conformance-views-released` artifact of its Conformance
   run). The Conformance workflow pushes the badges to the
   `veredictum-badges` and `veredictum-badges-development` branches; they are
   never committed.

## The measured performance stages

Separate from conformance, they never touch `results.json`:

```bash
bash scripts/conformance.sh perf      # veredictum perf over the pinned workload → docs/conformance/ferroehr/perf-result.json
bash scripts/conformance.sh perf-aa   # the same deployment against itself, interleaved: the record carries its A/A noise floor
```

Each stage composes the deployment with `docker/sut-measurement.yml` on fresh
volumes, runs the pinned binary on the host over the pin's `perf/` tree
(`CONF_PERF_WORKLOAD`, `CONF_PERF_POSTURE`, `CONF_PERF_HOST`, `CONF_PERF_OUT`),
re-derives the record with `perf-judge` and redraws the book's charts with
`scripts/render/perf-assets.sh`. A published record is measured on the idle
reference host only (`.claude/memory/measurement-environment-discipline.md`).
The full CLI lives in Veredictum's book (`veredictum.eu/docs/`).

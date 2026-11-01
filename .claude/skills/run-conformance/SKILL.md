---
name: run-conformance
description: >
  Runs the CNF 2.0 conformance pipeline (scripts/conformance.sh) — the
  reference runner over the committed machine-readable catalogue and the
  acceptance instrument — against the Docker-composed server (or any BYO
  SUT). Use when the user asks to check conformance, run the conformance/CNF
  suite, verify spec compliance, or at phase close for the zero-drift gate.
allowed-tools: [Read, Bash]
argument-hint: "[case-id-filter | --sut byo --base-url <url>]"
---

# /run-conformance

Runs the **CNF 2.0 pipeline** (Veredictum) via
`scripts/conformance.sh` — compose up --build on FRESH volumes (the
exclusive-server ground) → the committed catalogue → pure-function verdicts
→ artefacts under `docs/conformance/<sut>/` (our server:
`docs/conformance/ferroehr/`).

## Ground rules (before touching ANYTHING)

**The oracle-and-attribution law — the whole point of this instrument
(`.claude/rules/cnf-triage.md`).** The vendored openEHR spec
(`docs/specs/openehr/`) is the ONLY oracle and is ALWAYS right. The three
things WE built — the **application** (`app/*` + `crates/openehr-*`), the
**runner** (Veredictum's `src`), the **catalogue**
(Veredictum's `artifacts`) — are ALL suspects, none privileged. A red row
means exactly one of the three is wrong, and WHICH one is decided by reading
the spec, never assumed. **The application is NOT presumed correct because "we
wrote it carefully to the spec" — it is the single most common real culprit,
and this instrument exists precisely to catch it.** Two reflexes are
FORBIDDEN:

- *"Our code is right, so the CNF must be wrong"* → editing the catalogue or
  runner to make a red row green. This is the mistake we keep making; it is
  banned. A red row is not evidence the catalogue is wrong.
- *"Let me check our SUT"* to decide what the expected value should be. The
  SUT's observed response is **evidence** for the three-way comparison, NEVER
  the reference. The reference is the spec text, read first-hand, every time.

- The catalogue is **authored from the CNF 2.0 framework** (official
  schedule case ids; spec-text-only expectations; ambiguities through the
  typed register — Veredictum's own `CLAUDE.md`). The upstream Robot suites
  are reference text; their official DATA fixtures live in the corpus as
  provenance-stamped re-adjudications.
- **Never weaken a case to pass.** A failing case against our server is a
  correct instrument outcome; corpus/authoring defects are fixed with spec
  citations or registered as ambiguities — never bent to observed behaviour.
- **Coverage only ratchets up.** A spec-defined behaviour the catalogue does
  not yet exercise is a GAP to close (a new spec-cited case), never an
  acceptable omission — see `.claude/rules/testing.md` §CNF coverage.
- The record has no tier verdict. Its counts per part (BASE and OPTIONS) are
  read from `results.json` by `scripts/render/conformance-stats.sh`; never
  hand-type a count or a tier claim anywhere.

## Steps

1. **Preflight:** Docker available (`docker info`); no conflicting compose
   stack already running. The runner always drives a deployed SUT over HTTP
   — there is no in-process mode (owner ruling).
2. **Run.** Default (our server, from the current tree):
   ```bash
   bash scripts/conformance.sh          # optionally: CONF_DOCUMENTS="<id> …"
   ```
   It drives one FerroEHR deployment through the pinned Veredictum release
   image with Basic login. The measured performance lane is a separate,
   non-conformance stage: `bash scripts/conformance.sh perf` (or `perf-aa`
   for the A/A noise floor) composes the deployment with
   `docker/sut-measurement.yml`, runs `veredictum perf` over the pinned
   workload from the pin's `perf/` tree, re-derives the record with
   `perf-judge` and redraws the book's charts. It writes
   `docs/conformance/ferroehr/perf-result.json` only, never `results.json`,
   and a published record is measured on the idle reference host only
   (`.claude/memory/measurement-environment-discipline.md`). The full CLI
   lives in Veredictum's book (`veredictum.eu/docs/`).
3. **Compare against the committed baseline**
   (`docs/conformance/ferroehr/results.json`): the only permitted delta is
   newly-green rows, **zero drift**. Report: passed / failed / warning /
   errored / skipped counts per part, and any drift row by row.
4. **On any non-green result, ATTRIBUTE before you touch anything**
   (`.claude/rules/cnf-triage.md`; **delegate a red run to the `cnf-triage`
   agent**). Per red row, in order — no shortcuts:
   1. Read the observed wire exchange (`CNF_DEBUG_EXCHANGES=1` dumps every
      exchange sent/received/classified).
   2. Read what the catalogue expects and its cited `spec_refs` source.
   3. **Read the governing `docs/specs/openehr/...` section FIRST-HAND** and
      derive independently what a conformant server must return — status,
      headers, body — from the spec text alone. Never from the SUT's response,
      never from memory, never from EHRbase.
   4. **Three-way compare** spec-required vs catalogue-expected vs SUT-observed
      and attribute to exactly ONE suspect. Only THEN fix the guilty party the
      one sanctioned way:
      - **Application defect** (catalogue matches spec, SUT ≠ spec) → fix
        `app/*`, or the `openehr-codegen` emitter + regenerate for
        `crates/openehr-*` (never a hand-edit or consumer workaround), carrying
        a reproduced `curl` exchange + the quoted spec sentence.
      - **Catalogue defect** (catalogue-expected ≠ spec, regardless of what the
        SUT did) → fix the artifact WITH a new spec-cited source.
      - **Runner defect** (SUT spec-correct but mis-driven/mis-classified) →
        fix the module in VEREDICTUM, release, bump the pin here.
      - **Spec silence/ambiguity** → a Veredictum
        `artifacts/registers/ambiguities.yaml` entry with a typed disposition.
   Never adjust an expectation to match observed behaviour; never change app
   code without a reproduced exchange; never assume the app is the correct one.
5. **At phase close:** the ratcheted released-profile `results.json` and
   `summary.md` under `docs/conformance/<sut>/` are committed with the phase.
   The badges are pushed by the Conformance workflow, never committed.

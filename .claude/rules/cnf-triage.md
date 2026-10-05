---
paths: ["docs/conformance/**", "scripts/conformance*", "scripts/lib/veredictum.sh"]
---

# CNF failure triage (the attribution law)

When a CNF run goes red, the failure is attributed BEFORE anything is
changed. **The vendored openEHR spec text (`docs/specs/openehr/`) is always
right and is never a suspect** — every red row is a defect in something WE
wrote, in exactly one of three bins. Run the `cnf-triage` agent (or follow
this protocol in-session) for every red row; never "fix" a red run by
guessing which side is wrong.

**The two reflexes we keep making — both BANNED.** This is the single most
common process failure in this repo, and the reason this instrument exists:

1. **"Our code is right, so the CNF must be wrong."** The application
   (`app/*` + `crates/openehr-*`) is NEVER presumed correct because "we wrote
   it carefully to the spec". It is a suspect equally with the runner and the
   catalogue — and, wire-visible bugs being the point of the instrument, it is
   frequently the real culprit. Do not reach for the catalogue/runner to make
   a red row green.
2. **"Let me check our SUT."** The SUT's observed response is *evidence* for
   the three-way comparison — it is NEVER the reference for what the expected
   value should be. You never derive an expectation from what the server did;
   you derive it from the spec text, first-hand, and then check whether the
   server matched. Reading the SUT to *decide* the expectation is the bug.

The reference is the spec, every time. This instrument validates OUR setup
(the app AND the runner AND the catalogue) against that spec — it is not a
test suite that presumes our code and questions the tests.

## The three suspects (and the only fix path for each)

| Bin | What it means | Fix path |
|---|---|---|
| **Application** | The SUT violates the spec | `app/*`; for `crates/openehr-*` shapes: the `openehr-codegen` emitter + regeneration, never hand-edits or consumer workarounds |
| **Runner machinery** | The SUT behaved correctly but Veredictum's `src/**` misdrove the case or misjudged the response (driver, provisioning, resolver, outcome classification, comparator, verdicts) | Fix the runner module IN VEREDICTUM, cut a release, bump the pin here; the affected rows were inconclusive, not SUT failures |
| **Catalogue artifact** | The hand-authored schedule is wrong vs the spec (case core, operation binding, corpus, vocabulary) | Edit the artifact IN VEREDICTUM, WITH a new spec-cited source for the corrected expectation, then bump the pin here |

Two of the three bins now live in another repository (the instrument is
Veredictum, pinned in `scripts/lib/veredictum.sh`). The attribution work is
unchanged — read the case core, the binding and the spec first-hand in the
pinned checkout — but the FIX lands there and reaches this repository as a pin
bump, re-proven by a full `scripts/conformance.sh` run against the committed
baseline, unless the catalogue (`artifacts/`), `specs/`, `schemas/` and runner
(`app/veredictum/src/`) trees are all unchanged between the two tags
(`scripts/checks/veredictum-pin.sh`). A red row attributed away from the application never becomes a reason
to leave the row red here: the FerroEHR issue carries a native `blocked-by`
edge to the pin-bump work.

## The protocol (per red row)

1. Read the observed wire exchange (results.json / transcript — what was
   actually sent, received, classified).
2. Read the case core + operation binding (what the catalogue expects, and
   its cited spec source — docs text first; the released OAS is citable ONLY
   for behaviour the docs text is silent on, and loses on any conflict).
3. Read the governing RELEASED spec text FIRST-HAND (`/spec-lookup`; the
   ITS-REST docs text + overview `Requests_and_responses`, SM interface,
   RM/QUERY/BASE/AM/TERM/ITS-XML). The CNF schedule is only a GUIDE to WHICH
   behaviour to check — re-derive the correct answer from the released
   component, never from the schedule's own assertion, the OAS, the Robot set,
   memory, or EHRbase behaviour.
4. Derive independently what a conformant server must return for that
   exchange; compare three-way: spec-required vs catalogue-expected vs
   SUT-observed. The mismatching side is the defect.
5. An attribution that changes application code carries a reproduced wire
   exchange (`curl` against the composed SUT) + the spec citation (file +
   section, decisive sentence quoted).
6. Spec silence/ambiguity → an `artifacts/registers/ambiguities.yaml` entry
   with a typed `disposition` — never a private resolution.

## Hard rules

- **Never edit `docs/specs/openehr/**` or adjust a catalogue expectation to
  match observed SUT behaviour** — expectations trace to spec text only.
- **Only the RELEASED spec components are the oracle (owner rulings
  2026-07-24 + 2026-07-28).** Adjudicate against the released components
  (RM / BASE / AM / QUERY / TERM / ITS-XML / **SM** / ITS-REST **docs text**),
  with one ordered supplement: **the vendored released OAS**
  (`crates/openehr-its/vendor/rest-oas/`) is part of the release's own
  specification artifacts (ITS-REST overview `Specifications.md`) and grounds
  an expectation **where the docs text is SILENT** — but it **loses to the
  docs text on any conflict**, and an OAS-only ground is always cited AS the
  OAS (file + element), never passed off as docs text. NEVER treat as
  authority: the CNF Platform Conformance Test Schedule (never released
  stable — a GUIDE to which behaviour to test, not the correct answer); or
  the Robot suites/data sets (stalled/broken, e.g. AMB-47). Where any of
  these conflicts with a released component, the released component wins. An
  "ambiguity" that exists ONLY because a guide source is
  stale/incomplete/self-contradictory — with no released-component ground —
  is not a spec gap: re-ground it on a released component, or drop it and
  make the case gating. **SM and the ITS-REST docs
  text are BOTH oracles** (owner ruling 2026-07-24) — SM anchors the operation
  + the naming the case cores use, ITS-REST binds it to the wire; an SM
  operation the released ITS-REST does not yet realize is a genuine SM↔ITS
  realization gap (verdict N/A-with-citation on this ITS + an upstream
  alignment candidate), NOT a REFUTE.
- **A red run is not presumptive evidence of an app bug** (the first live
  triage attributed 7/7 defects to the runner) — nor of a runner bug.
  Every row gets the full derivation; no attribution without a citation.
- Transport faults / step-resolution failures classify as inconclusive
  runner-side rows, never SUT failures.
- **Ambiguity-register entries are spec-PROVEN, never assumed.** Every
  `artifacts/registers/ambiguities.yaml` entry is CONFIRMED first-hand against
  the vendored spec (the register is a suspect like any catalogue artifact) —
  re-adjudicate before trusting one or attaching an upstream report. A claimed
  ambiguity the spec actually DEFINES is a catalogue defect: remove the entry
  and make the case gating; do NOT report it upstream. `report_only` and
  `editorial` entries MUST carry an `upstream_issue` (schema-enforced) so a
  carried divergence is always reported to openEHR, never silently absorbed.
- Standing test discipline applies: never weaken a test or expectation to
  go green (`.claude/rules/testing.md`).

## Upstream reports (owner ruling 2026-08-01)

An outbound report of a released-spec defect/contradiction/silence is a
**GitHub issue labeled `upstream-report`** (dark red) — never a markdown
ledger file. One issue per defect; the register entry points at it via
`upstream_issue: <number>`; the narrative lives ONLY on the issue.

- **Shape** (never ticket-draft framing — no Channel/Status/Ask fields): a
  plain opening summary, `## What the released spec says` (citations +
  quotes), `## What this implementation does` (our behaviour + the register
  disposition), `## Resolution sought upstream`.
- **Grounding**: docs text first; the released OAS is citable only where the
  docs text is silent, always cited AS the OAS, and loses every conflict. A
  behaviour the OAS DEFINES is not a reportable silence; a "defect" that
  exists only because a stalled guide source (CNF schedule, Robot data) is
  wrong has no released-component ground and is not reportable.
- **Lifecycle (owner ruling 2026-08-21 — verification is TERMINAL)**: a NEW
  report is created UNVERIFIED and enters the current verification
  milestone. Once RE-VERIFIED first-hand as genuine, it gains
  `upstream-confirmed` and — when its divergence is fully adjudicated
  in-repo (the implementation, register entry or NOTE its own
  implementation section names), with nothing further pending on our side —
  it is **CLOSED as the standing outbound record**: the closed issue stays
  the durable, linkable target of the register's `upstream_issue` (nothing
  is silently absorbed by closing), and the open set stays near zero by
  design. A confirmed report stays OPEN only while something in-repo is
  genuinely blocked on it (a native `blocked-by` edge). When it was a docs
  misreading, it is CLOSED as refuted, its register entry removed or
  re-grounded, and the affected case made gating. When a report is filed on
  an openEHR channel (Jira / spec repo), the returned key
  (SPECPR-…/SPECQUERY-…) is recorded on the issue — reopen-free; when
  upstream later resolves one, file the inbound `spec-update` (the closed
  report is its provenance) and reopen only if the resolution requires
  in-repo changes.
- **Labels**: `upstream-report` + `spec:<component>` (+ `upstream-confirmed`
  once verified). `blocked-upstream` is NOT for reports — it keeps its
  narrower spec-update meaning (resolved in Jira, normative text not yet
  in the public spec repos). An in-repo work item waiting on a reported
  defect adds a native `blocked-by` edge to the report issue
  (`.claude/rules/issue-relationships.md`).

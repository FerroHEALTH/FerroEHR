---
paths: ["docs/conformance/**", "scripts/conformance*", "scripts/lib/veredictum.sh"]
---

# CNF failure triage (the attribution law)

When a conformance run goes red, every failed and errored row is attributed
BEFORE anything changes. **The vendored openEHR specification text
(`docs/specs/openehr/`) is the reference and is never a suspect.** Each red
row is a defect in exactly one of the things we did not take from it:
FerroEHR, the party set, or Veredictum; or the specification is itself
silent or self-contradictory there. Run the `cnf-triage` agent (or follow
this protocol in-session) for every red row; never "fix" a red run by
guessing which side is wrong.

**The two reflexes we keep making, both BANNED.** This is the most common
process failure in this repo, and the reason the instrument exists:

1. **"Our code is right, so Veredictum must be wrong."** FerroEHR (`app/*` +
   `crates/openehr-*`) is NEVER presumed correct because "we wrote it
   carefully to the spec". It is a suspect equally with the party set and
   Veredictum, and, wire-visible bugs being the point of the instrument, it
   is frequently the real culprit. Do not reach for the catalogue or the
   runner to make a red row green.
2. **"Let me check our server."** The server's observed response is
   *evidence* for the comparison, NEVER the reference for what the expected
   value should be. Derive the expectation from the specification text,
   first-hand, then check whether the server matched.

Neither FerroEHR nor Veredictum is presumed right. Veredictum is an
independent project (pinned in `scripts/lib/veredictum.sh`) whose verdicts
are evidence to adjudicate, never authority.

## The suspects and where each attribution ends

| Outcome | What it means | Where it ends |
|---|---|---|
| **FerroEHR is wrong** | The server violates the cited specification | A Bug filed here with the row, the decisive sentence quoted and the reproduced exchange, fixed in `app/*` (for `crates/openehr-*` shapes: the `openehr-codegen` emitter + regeneration, never a hand-edit or consumer workaround) |
| **The party set is wrong** | The statement claims a capability or selects an option arm the server does not realize, or the IXIT misdescribes the deployment (`docs/conformance/party/ferroehr/`) | Fixed in `statement.json` or `ixit.json`, with the reason in the commit; `veredictum validate --statement` stays at zero findings |
| **Veredictum is wrong** | The server answered as the specification requires, and the catalogue expects otherwise (a workflow under `catalogue/cases/`, its criteria or its `x-veredictum-cite`, an option in `catalogue/capabilities.yaml`), or the engine under `app/veredictum/src/` misdrove the exchange or misjudged it | An issue on Vernum-Projecten/Veredictum with the row, the cited sentence, what the catalogue expects and why the specification says otherwise; the fix reaches this repository as a pin bump |
| **The specification is silent or contradicts itself** | No released text decides the behaviour | An `upstream-report` issue here (§Upstream reports), with the row recorded as such in the triage log; a catalogue that asserts the behaviour anyway is also a Veredictum attribution (it should carry an option or a `report:`) |

The triage log is a comment on the tracking issue: one row per red row, its
outcome and the specification section that decides it. A row attributed
away from FerroEHR never becomes a reason to leave it red: the FerroEHR
issue carries a native `blocked-by` edge to the pin bump or the report. The
bumping pull request runs the new pin's catalogue against its own build in
the Conformance workflow.

## The protocol (per red row)

1. Read the observation: the row in `results.json` (status, what was
   checked, expected and received) and its exchange in `recording.har` from a
   local run of `scripts/conformance.sh` (CI never uploads the recording,
   which holds response bodies). A row whose record names the instrument as
   the one that could not judge it is inconclusive, never a server failure.
2. Read the encoding: the workflow the row comes from, at the pinned tag of
   Veredictum (`catalogue/cases/<folder>/*.arazzo.yaml`, read with
   `gh api repos/Vernum-Projecten/Veredictum/contents/<path>?ref=v<pin>`),
   what it expects and its `x-veredictum-cite` (source,
   section, quote, and a `report:` where the text is reported upstream). For
   an option row, the option and its arms in `catalogue/capabilities.yaml`
   and the arm the statement selects.
3. Read the governing RELEASED specification text FIRST-HAND
   (`/spec-lookup`; the ITS-REST docs text + overview
   `Requests_and_responses`, RM/QUERY/BASE/AM/TERM/ITS-XML). Under the
   `development` profile, the pinned master text the profile names. The CNF
   schedule is only a GUIDE to WHICH behaviour to check: re-derive the
   correct answer from the released component, never from the schedule's own
   assertion, the OAS, the Robot set, memory, or EHRbase behaviour.
4. Derive independently what a conformant server must return for that
   exchange; compare the specification, Veredictum's expectation and
   FerroEHR's answer, and check the party set's claim for the row. The
   mismatching side is the defect.
5. An attribution that changes FerroEHR carries a reproduced wire exchange
   (`curl` against the composed server) and the specification citation
   (file + section, decisive sentence quoted).
6. Specification silence or contradiction is never resolved privately: it is
   an `upstream-report` issue, and a released-text option is the statement's
   choice of arm, never a guess.

## Hard rules

- **Never edit `docs/specs/openehr/**` or ask Veredictum to adjust an
  expectation to match observed server behaviour**: expectations trace to
  specification text only.
- **Only the RELEASED spec components are the oracle (owner rulings
  2026-07-24 + 2026-07-28).** Adjudicate against the released components
  (RM / BASE / AM / QUERY / TERM / ITS-XML / ITS-REST **docs text**),
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
  make the case gating. **The Service Model (SM) is not an oracle** (owner
  ruling 2026-10-27, superseding the 2026-07-24 ruling that made it one): it
  describes internal service interfaces openEHR holds no implementation to.
  Veredictum keeps SM operation names as identifiers only (its
  `catalogue/id-map.yaml`); an SM operation the released ITS-REST does not
  realize has no wire contract to test, never a gap FerroEHR owes.
- **A red run is not presumptive evidence of a FerroEHR bug** (the first
  live triage attributed 7/7 defects to the runner), nor of a Veredictum bug.
  Every row gets the full derivation; no attribution without a citation.
- Transport faults and unresolvable steps are inconclusive rows on
  Veredictum's side, never server failures.
- **A claimed silence is proven, never assumed.** A Veredictum option or a
  `report:` citation is a suspect like any catalogue artifact: re-read the
  released text before trusting one. A claimed silence the specification
  actually DEFINES is a Veredictum attribution (the workflow should gate),
  and is NOT reported upstream. Every carried divergence has an outbound
  report, never a silent absorption.
- Standing test discipline applies: never weaken a test or expectation to
  go green (`.claude/rules/testing.md`).

## Upstream reports (owner ruling 2026-08-01)

An outbound report of a released-spec defect/contradiction/silence is a
**GitHub issue labeled `upstream-report`** (dark red) — never a markdown
ledger file. One issue per defect; the triage log and, where Veredictum
carries the divergence, its `report:` citation point at it; the narrative
lives ONLY on the issue.

- **Shape** (never ticket-draft framing — no Channel/Status/Ask fields): a
  plain opening summary, `## What the released spec says` (citations +
  quotes), `## What this implementation does` (our behaviour and the
  statement arm it selects, where the release leaves an option),
  `## Resolution sought upstream`.
- **Grounding**: docs text first; the released OAS is citable only where the
  docs text is silent, always cited AS the OAS, and loses every conflict. A
  behaviour the OAS DEFINES is not a reportable silence; a "defect" that
  exists only because a stalled guide source (CNF schedule, Robot data) is
  wrong has no released-component ground and is not reportable.
- **Lifecycle (owner ruling 2026-08-21 — verification is TERMINAL)**: a NEW
  report is created UNVERIFIED and enters the current verification
  milestone. Once RE-VERIFIED first-hand as genuine, it gains
  `upstream-confirmed` and — when its divergence is fully adjudicated
  in-repo (the implementation, statement arm or NOTE its own
  implementation section names), with nothing further pending on our side —
  it is **CLOSED as the standing outbound record**: the closed issue stays
  the durable, linkable target of every citation of it (nothing is
  silently absorbed by closing), and the open set stays near zero by
  design. A confirmed report stays OPEN only while something in-repo is
  genuinely blocked on it (a native `blocked-by` edge). When it was a docs
  misreading, it is CLOSED as refuted, every citation of it removed or
  re-grounded, and the affected workflow made gating (a Veredictum issue). When a report is filed on
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

---
paths: ["crates/**", "app/**", "scripts/conformance*", "docs/specs/**"]
---

# openEHR spec adherence (the vendored specs are the oracle)

The full normative openEHR spec text is vendored at **`docs/specs/openehr/`**
(pinned to `docs/VERSIONS.md`; index in its `README.md`). The openEHR
specifications are the conformance authority — not EHRbase, not memory, not
intuition.

**Hard rules:**

- **Before implementing or changing any spec-facing behaviour** (RM semantics,
  invariants, versioning, REST endpoints/status codes/headers, AQL semantics,
  canonical JSON/XML shapes, template/OPT handling, terminology), **read the
  relevant vendored spec section first** — grep the class/attribute/endpoint
  name under the owning component dir (map in `docs/specs/openehr/README.md`),
  or run `/spec-lookup <topic>`.
- **The CNF schedule + Robot suites are STALLED GUIDES, not the oracle (owner
  ruling 2026-07-24).** openEHR CNF never released a stable version, and the
  Robot suites/data sets are stalled/broken, so
  `docs/specs/openehr/CNF/docs/platform_test_schedule/` and
  `CNF/tests/platform/robot/` tell you WHICH behaviours to cover — they are NOT
  authority for the correct answer. Derive every enforceable expectation from
  the RELEASED spec component (RM / BASE / AM / QUERY / TERM / ITS-XML /
  ITS-REST docs text); where the schedule or a Robot data set conflicts with a
  released spec, the RELEASED SPEC WINS, and an expectation with no
  released-spec ground is not enforceable.
- **The Service Model (SM) is not an authority (owner ruling 2026-10-27).**
  SM describes internal service interfaces, and openEHR holds no
  implementation to them. The oracles are what goes in and comes out: BASE,
  RM, AM, TERM, QUERY/AQL, ITS-REST, ITS-JSON, ITS-XML and the serialisation
  formats. How FerroEHR is built inside is its own design: never justify, add
  or keep a feature because SM names it, never cite SM as the ground for a
  decision, and never mark something "not realised" against SM as if that
  were a conformance gap. The vendored SM text under `docs/specs/openehr/SM/`
  stays as reference.
- **The ITS-REST wire-oracle order: docs text first; the released OAS fills
  docs-text SILENCE; the register only when BOTH are silent (owner rulings
  2026-07-24 + 2026-07-28).** Read `docs/specs/openehr/ITS-REST/` prose (esp.
  overview `Requests_and_responses`) for required wire behaviour; **where the
  OAS and the docs text disagree, the docs text wins** (the 2026-07-24
  conflict rule stands — real conflicts exist and were adjudicated). Where
  the docs text is SILENT — not in conflict — the vendored released OAS
  (`crates/openehr-its/vendor/rest-oas/`) grounds the expectation: the
  release's own docs text presents the OAS files as its computable
  specification artifacts (overview `Specifications.md`: "Specifications can
  be downloaded as YAML files in OpenAPI Specification 3.0 format"; each API
  Preface repeats it). Only a behaviour BOTH sources withhold is spec
  silence, an `upstream-report` issue (`.claude/rules/cnf-triage.md`). The OAS remains `emit-rest` codegen
  input and is NEVER served (`served-openapi` rule unchanged).
- **CNF red-run triage is spec-adjudicated** (`.claude/rules/cnf-triage.md`;
  the `cnf-triage` agent): when the CNF runner and the application disagree,
  the vendored spec text decides — it is always right and never a suspect.
  The failure is attributed to the application, the runner machinery, or the
  catalogue artifacts by three-way comparison against the spec text, never
  by assuming either side; no fix lands before the attribution, and every
  attribution carries the spec citation.
- **Cite the source:** conformance-relevant decisions name the spec file +
  section heading in the commit/PR description. A deliberate deviation or gap
  gets a `// NOTE:` with the spec reference and the reason.
- **Cite ONLY the vendored specs + official external docs — never an
  internal markdown file (owner hard rules, 2026-07-11 + 2026-07-17).**
  In code comments, SQL schema comments, and doc comments, justify
  behaviour by citing the openEHR spec file + section (`docs/specs/openehr/`)
  or official external documentation (the PostgreSQL docs, the Rust
  book/reference, the docs.rs/crates.io docs of a pinned crate), or — for a
  compliance-relevant decision — the vendored regulation corpus
  (`docs/law/<jurisdiction>/<act>/<file> Art. N Abs. M`, the article-level
  form `.claude/rules/law-corpus.md` fixes; never a publisher's live page or
  a consolidation the tree does not carry) — never an
  internal doc, because internal docs move or die. **The ADR layer has been
  deleted** (it caused more confusion than value): no file may instruct anyone
  to read, write, or cite an ADR. Internal plan/design markdown is likewise
  never a citable authority — a plan or design file is deleted in the same PR
  that implements it, and the durable record is the closed issues + PR descriptions,
  `CHANGELOG.md`, git history, and the living reference docs
  (`docs/architecture.md`, `docs/VERSIONS.md`). Where
  the specs are SILENT on a decision (storage mechanics, indexing, infra,
  extension features), flag it explicitly: "no openEHR spec governs this — our
  own design/extension". Scrub any ADR or internal-doc citation from a file you
  touch.
- **NEVER LAX. Strictness is a hard rule (owner directive, 2026-08-01):**
  the CDR accepts EXACTLY what the released spec admits — nothing more,
  nothing less.
  - Everything the spec REFUSES, we refuse — and every refusal is an
    ASSERTED NEGATIVE TEST pinning its error code (the corpus/CNF gates fail
    if a refusal stops happening or fires differently), so a silently
    loosened reader/validator is a failing build, never a quiet drift. This
    assert-the-refusal pattern IS the machine enforcement of this rule.
  - Everything the spec DEPRECATES is enforced at exactly the deprecation's
    strength: accepted + a Warning-severity finding naming the preferred
    form (e.g. #1470) — never silently absorbed.
  - A spec-SILENT form is accepted only with a first-hand citation chain
    grounded in the released docs text (including a chapter's own embedded
    normative grammar) — NEVER on stalled reference material alone (.g4
    grammars, regression fixtures, Robot suites: corroboration at most,
    decision never). Each such acceptance is recorded on a tracker issue,
    and any upstream contradiction it exposes is filed as a `spec-update`
    report — stalled or contradictory upstream information is never carried
    silently.
  - Weakening ANY existing refusal requires a docs-text-grounded
    adjudication recorded on an issue (the #1465 pattern: re-derived
    citations, the flipped gate updated to assert the new expected outcome,
    the accepting twin fixture added). Inventing a prohibition the spec does
    not contain is the same defect class as leniency — strict means
    exact, in both directions.
- **Never resolve a spec question from EHRbase behaviour alone.** EHRbase is
  prior art; if it and the spec text disagree, the spec text wins and the
  divergence is worth a note.
- **Never hand-edit `docs/specs/openehr/**`** (except its top-level README) —
  re-vendor with `scripts/vendor/spec-docs.sh`; version pins live in that
  script and `docs/VERSIONS.md`.
- Subagents doing spec-facing work must be handed the relevant
  `docs/specs/openehr/...` paths in their prompt, and reviewers verify claims
  against those files.

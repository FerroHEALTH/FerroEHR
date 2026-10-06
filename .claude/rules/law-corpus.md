---
paths: ["docs/law/**", "scripts/vendor/law-*.sh", "scripts/checks/law-corpus.sh", "website/book/src/compliance/**", "website/book/src/security/**", ".claude/agents/compliance-*.md", ".claude/skills/compliance-audit/**", ".claude/skills/law-lookup/**"]
---

# The regulation corpus and how compliance claims are made

`docs/law/` is the vendored text of every regulation the compliance pages
cite: EU acts from the Publications Office, Dutch acts from
wetten.overheid.nl, German federal law from gesetze-im-internet.de and
recht.bund.de, Swiss federal law from Fedlex, each at a named consolidation
with its `PROVENANCE.md` and `SHA256SUMS` (`docs/law/README.md` is the index;
`scripts/checks/law-corpus.sh` is the guard). It exists so that a claim about
what the law asks resolves to bytes this repository carries, the way an
openEHR claim resolves to `docs/specs/openehr/`. This file is the discipline
for reading it, citing it and writing against it. The vendoring mechanics
(scripts, pins, licences) are in `vendored-corpora.md`.

## Two instruments, kept apart (owner direction 2026-09-12)

- **`compliance-researcher`** (read-only) answers "what does the law
  require" from `docs/law/` alone, with an article-level citation, and says
  so when the text is silent. **`/law-lookup`** is the in-session form of the
  same question.
- **`/compliance-audit`** walks one act article by article against the code,
  the configuration and the book, files one issue per article with a gap,
  and records the status per article on the tracker.
- **`compliance-implementer`** is the worker for the deliverables those two
  produce: compliance pages, provenance records, vendor scripts, `Control:`
  declarations on issues.

None of them is `spec-researcher`, `cnf-triage` or `/spec-audit`, and none
of them is ever folded into those: the openEHR specifications are the
conformance oracle for the wire, the regulations are the measure of the
compliance posture, and a review that mixes the two cites neither correctly.

## The text is the authority, and it is read, never remembered

- Every statement of what a regulation requires is read from the vendored
  file at the pinned consolidation, first-hand, at the moment of writing.
  Never from memory, never from a summary page, never from another
  implementation's compliance documentation, never from the publisher's live
  page (that is a different, undated text). If the vendored text does not
  say it, the honest answer is "the text is silent", and a silent text is
  never filled in from general knowledge.
- The authentic language wins. Swiss acts are cited in German (the English
  is Fedlex's non-binding translation, vendored for reading only); Dutch and
  German acts in their own language; EU acts in the English of the OJ. A
  quote is verbatim in the source language, with a rendering beside it if
  the reader needs one.
- Scope is read before substance: which Part, which addressee, which date.
  A section that binds a health insurer, an ePA operator, gematik, a
  supervisory authority or a law-enforcement body is not a duty on a care
  provider's CDR, however well the software happens to fit it (BDSG Part 3
  and SGB V §§ 342–345 are the standing examples). The compliance pages say
  who a provision binds whenever it is not the deploying organisation.

## Citation form

A citation names the vendored file and the provision, so a reader lands on
the words:

- Issues, PRs, comments and code comments:
  `docs/law/<jurisdiction>/<act>/<file> Art. N Abs. M` (or `§ N Abs. M`,
  `Art. N(M)` for EU acts, `Art. N lid M` for Dutch acts) — e.g.
  `docs/law/de/bdsg/BJNR209710017.xml § 22 Abs. 2`,
  `docs/law/ch/dpo/text-de.html Art. 4 Abs. 1`,
  `docs/law/eu/gdpr/text.html Art. 32(1)(a)`.
- The published book pages: the act's short name and the provision in
  bold at the start of the row, linked to the publisher's own page for that
  article where one exists (`gesetze-im-internet.de/<slug>/__N.html`,
  `wetten.overheid.nl/<BWB>`, `fedlex.admin.ch/eli/cc/...`,
  `eur-lex.europa.eu/eli/...`); the page's warning box points at
  `docs/law/` as the text the page was written against.
- `docs/law/` is a durable reference in the sense of the citation rule in
  `spec-adherence.md`: it may be cited from code, SQL comments and doc
  comments beside `docs/specs/openehr/` and official external
  documentation. A `Control:` line on an issue names a legal source from the
  registry in `scripts/render/control-matrix.sh`; a short name that registry
  does not declare stops the render.
- Never cite a consolidation the tree does not carry. When the pages need a
  newer text, the pin moves first (the vendor script, the diff, the re-read),
  then the citation.

## Wording the published claim (the standing policy)

Software supports obligations; controllers and processors meet them.
Therefore, on every page and in every issue:

- Never "compliant", "certified", "conformant", "GDPR-ready" or any phrase
  that says the product satisfies a law. Say what the product ships, name the
  provision it serves, and say what the deploying organisation must still
  do. The national-law tables are four columns for that reason: provision,
  what FerroEHR ships, tracker, what the deploying organisation must do.
- A control's status comes from the tracker: "shipped, #N" for a closed
  issue, "planned, #N" for an open one, "not planned" with the reason when
  the product deliberately does nothing (a prescription transport outside
  the German telematics infrastructure is prohibited, so the row says
  "nothing, and nothing planned"). `scripts/checks/tracker-status.sh`
  refuses a "planned" beside a closed issue and a "shipped" beside an open
  one; the control matrix page is generated from the tracker, never typed.
- Where a provision names an open list ("können insbesondere gehören",
  "such as"), the page says the duty is to choose appropriate measures and
  that the list is a menu, and does not present the list as a checklist the
  product ticks.
- A provision the product cannot serve at all is a row too, with "nothing"
  in the ships column and the organisation's duty beside it. Silence about
  a provision that reaches a CDR is what the audit exists to find.
- Prose follows `writing-style.md`; legal prose is where the banned tells
  breed fastest.

## Layering by jurisdiction

The EU layer (GDPR, ePrivacy, EHDS and its implementing acts 2026/2083 and
2026/2099, Recommendation 2019/243, the EDPB guidelines, NIS2, CRA with its
implementing act 2025/2392 and its delegated acts 2026/881 and 2025/1535,
Regulation 2019/1020 on market surveillance,
Regulation 765/2008 on accreditation and the CE marking, MDR) applies to every
EU deployment and is written once. Each country is one section on top of it,
in the shape of the Dutch one, read only by a deployment in that country: the
national acts as a four-column table, the national standards
(the NEN section shape; a paywalled standard gets a record, never text), and,
where the country issues a personal identifier with a published algorithm, a
rule in the identifier scanner citing the register that defines it.
Switzerland is not an EU member state: its section stands on the DSG, not on
the GDPR, and says so. Adding a country starts from the regulation issue
form and ends with the act vendored, the section written and the scanner rule
filed, in that order.

## The audit record (what `/compliance-audit` leaves behind)

One tracker issue per act audited, in the standard body shape (a plain
summary, then `## Acceptance criteria`), whose comments carry the
article-by-article status table: provision, what it requires of the software
(quoted), status (served / gap / organisation-only / not applicable, with
the scope reason), evidence (file:line, page anchor, test), issue filed. A gap
is filed as its own issue, typed and labelled `regulation`, milestoned in the
current milestone, and linked to the audit issue with `scripts/gh/rel.sh`;
the fix-first cadence of an audit programme (root `CLAUDE.md` §Issue
workflow) applies: a self-filed gap is fixed before the next act is audited.
The GDPR audit recorded on the tracker under #3295 is the shape every later
run follows.

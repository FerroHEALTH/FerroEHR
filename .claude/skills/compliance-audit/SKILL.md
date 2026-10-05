---
name: compliance-audit
description: >
  Audits FerroEHR against one regulation, article by article, from the
  vendored text at docs/law/ (the GDPR, the ePrivacy Directive, EHDS, NIS2,
  CRA, MDR, the EDPB guidelines (pseudonymisation, ePrivacy Art. 5(3)), the UAVG, Wabvpz, BW 7,
  the Telecommunicatiewet, the Dutch logging decree, Begz, the BDSG, SGB V, GDNG, StGB, the Swiss DSG, DSV, EPDG, EPDV,
  EPDV-EDI): builds the checklist from the text first, walks it against the
  code, the configuration and the book, classifies every article, files one
  issue per gap, and records the status per article on the tracker. Use when
  the user asks "are we meeting X", "audit against the GDPR/BDSG/DSG", before
  a compliance page is published or changed, or after a regulation is
  vendored or re-pinned. Not for openEHR conformance (/spec-audit).
argument-hint: "<act>   e.g. 'GDPR', 'BDSG', 'DSV', 'SGB V §§ 341-355', 'UAVG'"
---

# /compliance-audit

Systematic audit of the product against one regulation: the vendored text
versus the code, the configuration and the book. The output is a
**status per article** recorded on the tracker, and **one issue per gap**;
fixing is a separate step the fix-first cadence then demands. The discipline
is `.claude/rules/law-corpus.md`; this skill is its audit procedure, the way
`/spec-audit` is `spec-adherence.md`'s.

Software supports obligations; it never "complies". The audit therefore
never produces a verdict on the product. It produces, per article, what the
text requires, whom it binds, what the product ships against it with
evidence, and what is missing.

## Procedure

1. **Scope.** Resolve the argument to one act (or one chapter or section
   range of it) in `docs/law/README.md`, and read its `PROVENANCE.md`: the
   consolidation audited is the one vendored, and the record is written
   against that pin. Note what the record says is not in the corpus (an
   annex published by reference, a paywalled standard, an older English
   translation) so the audit can name its own boundary.
2. **Build the checklist from the text first, never from the code.** For
   every article (every section, for German acts) extract, from the vendored
   file in the authentic language: the obligation as a testable statement,
   the verbatim load-bearing sentence, the addressee (the controller, the
   processor, the software it runs, an insurer, an ePA operator, gematik, an
   EPD community, a supervisory authority, a law-enforcement body), the
   scope article that decides it, and whether a list of measures is closed
   or a menu. For a whole act, fan this out to `compliance-researcher`
   (one chapter per call, the act's path and the file format hints in the
   prompt) and merge; the checklist is the researcher's output, not a
   memory of the act.
3. **Walk the checklist against the product.** For each article that reaches
   software, find the evidence: the feature page and anchor in
   `website/book/src`, the configuration key in
   `app/ferroehr/assets/ferroehr.default.toml`, the handler, the migration,
   the test. Run a targeted test where it is cheap. Classify:
   - **served**: the software ships a control the article asks for, with
     evidence (file:line, page anchor, passing test) and the tracker issue
     that delivered it;
   - **gap**: the article reaches software and nothing, or too little,
     ships; what the text asks for versus what exists;
   - **organisation-only**: the duty is discharged by people, contracts or
     procedure; software can supply inputs (name them) but cannot serve it;
   - **other addressee**: the article binds a party the product is not, with
     the scope citation, so no page may claim it;
   - **not in corpus**: the provision's substance is published by reference
     or sits in an act not vendored, and the audit is silent on it;
   - **text silent**: the act does not address the topic the product has a
     control for.
4. **Cross-check the published pages.** Every row of the act's section in
   `website/book/src/compliance/index.md` and
   `shared-responsibility.md`, and every `Control:` line on an issue citing
   the act, is checked against the checklist: a row citing an article whose
   addressee is not the deployment, a "shipped" beside an open issue, a
   "compliant"/"certified" wording, a provision that reaches software and
   has no row. Each is a finding.
5. **Record.** Post the article-by-article table on the audit's tracker issue
   (one issue per act audited, in the standard body shape, a `Task` with
   `chore`, labelled `regulation`, in the current milestone): article | requirement
   (quoted) | addressee | status | evidence | issue. State coverage
   honestly: the articles not audited, the annexes not in the corpus, the
   consolidation the record is written against. Then file one issue per
   gap and per page finding (`scripts/gh/fields.sh new <type> <priority>
   <effort> …`, labelled
   `regulation`, milestoned in the current milestone, a `Control:` line
   naming the legal source where the fix delivers a control), and link each
   to the audit issue with `scripts/gh/rel.sh parent`. Never silently
   fix-and-forget; never adjust a page to match the product where the text
   asks for more.

## Rules

- The vendored text is the authority; a finding is never "the GDPR is
  generally understood to require X" but "Art. N(M) says X (citation)". A
  silent text is reported as silent, never filled in from general knowledge.
- Scope before substance: the addressee decides whether a gap exists at all
  (BDSG Part 3 is confined to law-enforcement bodies by § 45; SGB V § 309
  binds TI-application controllers; the EPDG binds certified communities).
- The product is never presumed to serve an article because a page says so;
  the page is one of the things audited.
- No legal advice: the audit reports what the text requires and what the
  product ships. Whether a deployment meets the obligation is the
  controller's judgement, and the record says so.
- Prose follows `writing-style.md`; the tracker record is public.
- The GDPR audit recorded under #3295 is the shape every later run follows.

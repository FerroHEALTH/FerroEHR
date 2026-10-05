---
name: law-lookup
description: >
  Finds and reads the authoritative text of a regulation provision (an
  article, section or paragraph of the GDPR, the ePrivacy Directive, EHDS,
  NIS2, CRA, MDR, the EDPB pseudonymisation guidelines, the UAVG, Wabvpz,
  BW 7, the Telecommunicatiewet, the Dutch logging decree, Begz, the BDSG, SGB V, GDNG, StGB, DigiG, the Swiss DSG, DSV, EPDG,
  EPDV or EPDV-EDI) in the vendored corpus at docs/law/. Use before writing
  or reviewing any compliance claim, provenance record or privacy-boundary
  change, when a "what does the law say about X" question comes up, or when
  a page cites a provision that needs checking against the text. Not for
  openEHR specification questions (/spec-lookup).
allowed-tools: [Read, Grep, Glob, Bash]
argument-hint: "<act> <article | § | topic>   e.g. 'BDSG § 22', 'DSV Art. 4 logging', 'GDPR Art. 32'"
---

# /law-lookup

Answer regulation questions from the vendored text at `docs/law/` — never
from memory, never from a publisher's live page. Every answer cites the
vendored file and the provision, quotes it verbatim in the authentic
language, and says who the provision binds. The discipline is
`.claude/rules/law-corpus.md`; this skill is its in-session form, and the
`compliance-researcher` agent is the same procedure run off the main context
for a whole act or a checklist.

## Procedure

1. **Route to the act** via `docs/law/README.md` (one table per jurisdiction,
   with the identifier, the consolidation vendored and the pages that cite
   it). Read the act's `PROVENANCE.md` first: it names the consolidation, the
   language, the sections the pages cite, and what is NOT vendored (a
   paywalled standard, an English translation at an older consolidation).
2. **Find the provision in the publisher's own format**, with grep/sed/perl
   over the vendored file — never a converted copy, because the numbering a
   citation resolves against must be the publisher's:
   - EU acts, `text.html`: `<div class="eli-subdivision" id="art_N">`; OJ
     texts also carry `<p class="oj-ti-art">Article N</p>`.
   - Dutch acts, `text.html`: `<div class="artikel" id="Hoofdstuk<H>_Artikel<N>">`.
   - German acts, `BJNR….xml` (the gii-norm DTD): one `<norm>` per section,
     `<enbez>§ N</enbez>` then `<titel>`, paragraphs as `<P>` inside
     `<Content>`. The DigiG is a PDF (`pdftotext`).
   - Swiss acts, `text-de.html` (authentic German) and `text-en.html` (the
     non-binding Fedlex English): `<article id="art_N">`, `<h6 class="heading">`,
     `<p class="absatz">`.
3. **Read the whole article and its neighbours**: the definitions article,
   the scope article (which Part of the act, which addressee, which date),
   and every article the provision cross-references. Scope before substance:
   a section that binds an insurer, an ePA operator, gematik, an EPD
   community, a supervisory authority or a law-enforcement body (BDSG Part 3
   is the standing trap) is not a duty on a care provider's CDR.
4. **Answer with**: the obligation as a testable statement; the verbatim
   quote in the authentic language (and the Fedlex English beside it where it
   exists, marked non-binding); the citation
   `docs/law/<jur>/<act>/<file> Art. N Abs. M` (`§ N Abs. M`, `Art. N(M)`,
   `Art. N lid M`); who it binds, with the scope citation; whether a list of
   measures is closed or a menu ("insbesondere", "such as"). If the text is
   silent or ambiguous, say so explicitly — a silent text is never filled in
   from general knowledge, and it is the signal for a "the text is silent"
   note on the page rather than a claim.
5. **Never give legal advice.** Report what the text says and what a software
   system would have to be able to do; whether a deployment meets it is the
   controller's judgement.

## What is deliberately not in the corpus

Standards sold under copyright (NEN 7510, 7512, 7513; ISO 27001/27799) hold
a record and no text: `docs/law/nl/nen-75xx/PROVENANCE.md` lists the clauses
the pages cite so a reader with a licensed copy can follow them. Do not
paraphrase a standard's clause from memory to fill the gap. State law
(German Länder, Swiss cantons) is out of scope and the README says so.

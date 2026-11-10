# NEN 7513:2024 — not vendored

**Nothing but this record is in this directory, and nothing else may be.**
NEN 7513 is sold under copyright by NEN (Koninklijk Nederlands
Normalisatie-instituut). It is not free to redistribute, so the standard is not
fetched, not vendored, not quoted and not paraphrased clause by clause here.

| | |
|---|---|
| NEN 7513:2024 | Medische informatica — Logging — Vastleggen van acties op persoonlijke gezondheidsinformatie |
| Publisher | NEN, https://www.nen.nl |
| Published | December 2024 |
| Shop | https://www.nen.nl/nen-7513-2024-nl-329182 |
| Recorded | 2026-11-10 (UTC) by `scripts/vendor/law-nl.sh` |

The 2024 edition follows NEN 7513:2018 (Medische informatica — Logging —
Vastleggen van acties op elektronische patiëntdossiers,
https://www.nen.nl/nen-7513-2018-nl-245399). The Besluit elektronische
gegevensverwerking door zorgaanbieders, Art. 5(1) (`docs/law/nl/begz/`),
names NEN 7513 without an edition. The edition and the shop link above are
the ones Logius's Logboek Dataverwerkingen NEN 7513 extension cites
(`docs/specs/nen-7513-sources/logius/js/config.mjs`).

## What the compliance pages cite

This is the one standard of the three whose citations reach a numbered clause.
The citations below were written against NEN 7513:2018, so their clause
numbers are that edition's until someone who holds the 2024 edition checks
each one and names its 2024 clause (#5136).

| Cited as | Where | What the page claims against it |
|---|---|---|
| §5, the content of a logged event | `website/book/src/audit.md` (the field-by-field table) | Every item the clause lists is mapped to the field of the access record that carries it, including the two a deployment must supply itself (`purpose`, `legal_basis`) |
| Actor role | `website/book/src/compliance/control-matrix.md`, `website/book/src/security/dpia.md` | The access record carries the roles the caller held at access time, as FHIR `agent.role` and as a DICOM `RoleIDCode` |
| Event content | `website/book/src/compliance/control-matrix.md`, `website/book/src/security/dpia.md`, `website/book/src/security/records-of-processing.md` | Per-domain access logging for reads and queries, aligned with this standard and EHDS Annex II 3.2 |
| Retention | `website/book/src/audit.md`, `website/book/src/compliance/control-matrix.md`, `website/book/src/security/dpia.md` | The retention floor for the NL jurisdiction, which the Besluit vaststelling bewaartermijn logging binds to this standard |

The retention chain is followable without holding the standard, because the
legal end of it is vendored: `docs/law/nl/begz/` (Art. 5 delegates the
period) and `docs/law/nl/besluit-bewaartermijn-logging/` (the period is five
years from the moment the log line is written).

## Why the citations still work without the text

The claims above are claims about THIS software, checkable in this repository:
the recorded fields are in the schema and in the tests, and the page names
which field answers which item. What the standard requires of the item list is
checkable only against the standard, by someone who holds it.

Do not add files to this directory. Re-run `scripts/vendor/law-nl.sh` to
rewrite this record.

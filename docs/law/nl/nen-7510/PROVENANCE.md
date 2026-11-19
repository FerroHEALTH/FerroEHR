# NEN 7510-1:2024 and NEN 7510-2:2024 — not vendored

**Nothing but this record is in this directory, and nothing else may be.**
NEN 7510 is sold under copyright by NEN (Koninklijk Nederlands
Normalisatie-instituut). It is not free to redistribute, so the standard is not
fetched, not vendored, not quoted and not paraphrased clause by clause here.

| | |
|---|---|
| NEN 7510-1:2024 | Medische informatica — Informatiebeveiliging in de zorg — Deel 1: Managementsysteem |
| NEN 7510-2:2024 | Medische informatica — Informatiebeveiliging in de zorg — Deel 2: Beheersmaatregelen |
| Publisher | NEN, https://www.nen.nl |
| Part 1 | https://www.nen.nl/nen-7510-1-2024-nl-331311 |
| Part 2 | https://www.nen.nl/nen-7510-2-2024-nl-331314 |
| Recorded | 2026-09-12 (UTC) by `scripts/vendor/law-nl.sh` |

## What the compliance pages cite

The citations are to a PART and a control THEME rather than to a numbered
clause: the pages state which half of the standard an obligation sits in and
which control family it belongs to, never a clause number. A reviewer with a
licensed copy follows them by control family.

| Cited as | Where | What the page claims against it |
|---|---|---|
| NEN 7510-1, the information security management system | `website/book/src/compliance/shared-responsibility.md`, `website/book/src/compliance/index.md` | The software supplies technical controls an ISMS can point at; running the ISMS, and holding any certificate, is the deployment's |
| NEN 7510-2, the control set | `website/book/src/compliance/shared-responsibility.md`, `website/book/src/compliance/index.md` | Access control, audit logging, cryptography in transit and for version signatures, supply-chain verification |

## Why the citations still work without the text

The claims above are claims about THIS software, checkable in this repository:
the control is in the code, its behaviour is in the tests, and the page names
which. What the standard requires is checkable only against the standard, by
someone who holds it. That split is the reason this record exists instead of a
copy.

Do not add files to this directory. Re-run `scripts/vendor/law-nl.sh` to
rewrite this record.

# Regulation (EU) 2016/679 — General Data Protection Regulation

The processing law every deployment answers to: the lawfulness, minimisation, security and rights obligations the pseudonymisation boundary, the access log and the records of processing are built against.

| | |
|---|---|
| CELEX | `02016R0679-20160504` |
| ELI (base act) | http://data.europa.eu/eli/reg/2016/679/oj |
| Consolidation vendored | 2016-05-04 |
| Fetched from | https://publications.europa.eu/resource/celex/02016R0679-20160504 (`Accept: application/xhtml+xml`) |
| Resolved to | https://publications.europa.eu/resource/cellar/5f2552c2-cc45-11e6-ad7c-01aa75ed71a1.0022.03/DOC_1 |
| Human-readable at | https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02016R0679-20160504 |
| Fetched | 2026-10-05 (UTC) |
| Vendored by | `scripts/vendor/law-eu.sh` |

## Files

| file | bytes | SHA-256 |
|---|---|---|
| `text.html` | 462324 | `a681ff22f32f125749af6e96947224369efc132edb8a95b12637e357df1056de` |
| `oj.html` | 806864 | `962539af03738bf552319ff4ce42d69e5f95a576307c4dfed7bf87e81b646b9d` |
| `corrigendum-2018-05-23.html` | 23861 | `292bd10918681ec090f23eafbc53afd4dd3bbb7cb976f6748df4a6254f2be54a` |

`text.html` is the English XHTML the Publications Office serves for this
CELEX, byte for byte. Nothing was converted or extracted. The same document is
what the EUR-Lex page above renders; that page is not the fetch source because
it wraps the act in site chrome carrying a per-request identifier, which would
make the digest above unreproducible.

The other files are OJ documents fetched the same way from their own
CELEX, byte for byte:

| file | what it is | fetched from | resolved to |
|---|---|---|---|
| `oj.html` | The Regulation as published in OJ L 119, 4.5.2016, p. 1, recitals included. | https://publications.europa.eu/resource/celex/32016R0679 (`Accept: application/xhtml+xml`) | https://publications.europa.eu/resource/cellar/3e485e15-11bd-11e6-ba9a-01aa75ed71a1.0006.03/DOC_1 |
| `corrigendum-2018-05-23.html` | The English corrigendum, OJ L 127, 23.5.2018, p. 2. | https://publications.europa.eu/resource/celex/32016R0679R%2802%29 (`Accept: application/xhtml+xml`) | https://publications.europa.eu/resource/cellar/683d5816-5e52-11e8-ab9c-01aa75ed71a1.0006.03/DOC_1 |

## Version

Consolidated text at **2016-05-04**, the version in force
on the fetch date. EUR-Lex prints its own disclaimer on a consolidation: it
"is meant purely as a documentation tool and has no legal effect". A citation
that has to be authoritative is therefore checked against the OJ act and its
amendments. This file is what the act says today, which is what a compliance
claim is read against.

## Recitals, the OJ text and the corrigendum

`text.html` is the consolidation and carries the enacting terms only: it has
no recital and no `id="rct_…"` anchor. Its own header gives the reason: "The
authentic versions of the relevant acts, including their preambles, are those
published in the Official Journal of the European Union". The recitals are
therefore vendored from the OJ text beside it.

- **Recitals** are cited from `oj.html`, the Regulation as published in OJ L
  119, 4.5.2016 (anchors `id="rct_1"` to `id="rct_173"`):
  `docs/law/eu/gdpr/oj.html Recital 63`.
- **Articles** are cited from `text.html`, as before:
  `docs/law/eu/gdpr/text.html Art. 15(1)`. The consolidation folds in the
  English corrigendum of 23 May 2018 (marked ►C1 in it); `oj.html` does not,
  so the article wording in `oj.html` is the uncorrected 2016 text and is not
  the citation referent.
- `corrigendum-2018-05-23.html` is that corrigendum. It corrects recital 71
  (fifth and sixth sentences) and points of Articles 37(1), 41(3), 41(5),
  42(7), 43(3), 43(6), 57(1), 64(1), 64(6) to (8), 65(1), 69(2) and 70(1). A
  citation of recital 71 reads `oj.html` together with it.

Both texts are kept: the consolidation for the articles, the OJ text for the
recitals. Replacing the consolidation with the OJ text would move every
existing article citation onto the uncorrected 2016 wording (#3581).

## Licence

- Commission reuse policy (Decision 2011/833/EU) for the act, and
  CC BY 4.0 for the consolidated editorial layer, which the EU owns. Both are
  quoted verbatim in `LICENSES/LicenseRef-EUR-Lex-Reuse.txt` and
  `LICENSES/CC-BY-4.0.txt`, and declared for this directory in `REUSE.toml`.
  The CC BY 4.0 layer covers `text.html` only. The other files,
  `oj.html` and `corrigendum-2018-05-23.html`,
  are OJ documents, not consolidated texts, so they come under the reuse
  policy alone, and `REUSE.toml` declares them that way.
- Copyright: © European Union, 1998-2026. Reuse is authorised
  provided the source is acknowledged; this record and the index at
  `docs/law/README.md` are that acknowledgement.

Do not hand-edit anything in this directory. Re-run
`scripts/vendor/law-eu.sh` instead — a hand edit makes the SHA256SUMS line a
lie, which is the one thing a vendored legal text may never be.

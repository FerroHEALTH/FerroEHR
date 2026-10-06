# Directive (EU) 2022/2555 — measures for a high common level of cybersecurity across the Union (NIS2)

The cybersecurity directive, transposed into national law, whose security and incident-reporting duties reach the essential and important entities that deploy this software, among them hospitals.

| | |
|---|---|
| CELEX | `32022L2555` |
| ELI (base act) | http://data.europa.eu/eli/dir/2022/2555/oj |
| Consolidation vendored | none — the OJ text |
| Fetched from | https://publications.europa.eu/resource/celex/32022L2555 (`Accept: application/xhtml+xml`) |
| Resolved to | https://publications.europa.eu/resource/cellar/9b84d482-85bd-11ed-9887-01aa75ed71a1.0006.03/DOC_1 |
| Human-readable at | https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32022L2555 |
| Fetched | 2026-10-06 (UTC) |
| Vendored by | `scripts/vendor/law-eu.sh` |

## Files

| file | bytes | SHA-256 |
|---|---|---|
| `text.html` | 702913 | `6ee66d5c419d9a056fb98d94083cacf9b818803bd6b416435718e0d4919cb589` |
| `corrigendum-2023-12-22.html` | 3257 | `fa8913a509771efdab0418a38f7381f1d01b8340d9638ff004b7d2f418172c75` |

`text.html` is the English XHTML the Publications Office serves for this
CELEX, byte for byte. Nothing was converted or extracted. The same document is
what the EUR-Lex page above renders; that page is not the fetch source because
it wraps the act in site chrome carrying a per-request identifier, which would
make the digest above unreproducible.

The other files are OJ documents fetched the same way from their own
CELEX, byte for byte:

| file | what it is | fetched from | resolved to |
|---|---|---|---|
| `corrigendum-2023-12-22.html` | The English corrigendum of OJ L, 2023/90206, 22.12.2023, to Article 19(1). | https://publications.europa.eu/resource/celex/32022L2555R%2804%29 (`Accept: application/xhtml+xml`) | https://publications.europa.eu/resource/cellar/8df003b8-a087-11ee-b164-01aa75ed71a1.0002.03/DOC_1 |

## Version

Not a consolidated text: this is the act as published in the
Official Journal. EUR-Lex lists no consolidation for it that resolves (checked
when the pin was set), and where an initial consolidation does exist it folds
in no amendment, so the authentic OJ text is pinned instead of a documentation-tool
version that carries a no-legal-effect disclaimer for the same words.

## The corrigendum, and which file to cite for what

The Publications Office records nine corrigenda to this Directive, CELEX
`32022L2555R(01)` to `32022L2555R(09)` (read 2026-10-06). One has an English
version, `R(04)`, and it is vendored beside the OJ text:

- `corrigendum-2023-12-22.html`, CELEX `32022L2555R(04)`, OJ L, 2023/90206,
  22.12.2023: Article 19(1), first sentence, reads "The Cooperation Group
  shall, by 17 January 2025, establish" in place of "on 17 January 2025".

The other eight correct other language versions only: `R(01)` Italian and
Dutch, `R(02)` Dutch, `R(03)` Slovenian, `R(05)` Croatian, Maltese, Romanian,
Slovak, Slovenian and Swedish, `R(06)` Estonian, `R(07)` Italian, `R(08)`
French and Hungarian, `R(09)` Estonian and Polish. `R(04)` also has German,
Estonian, Hungarian, Italian and Swedish versions; only the English one is
vendored.

`text.html` is the OJ text of 27 December 2022 and does not include the
corrigendum: its Article 19(1) still reads "on 17 January 2025". Every other
article is cited from `text.html`: `docs/law/eu/nis2/text.html Art. 23(1)`. A
citation of Article 19(1) reads `text.html` together with the corrigendum and
cites both.

## Why the OJ text stays pinned

The Publications Office lists one consolidated version of this Directive,
`02022L2555-20221227` (read 2026-10-06; its header prints the version
"000.004"). It folds in the English corrigendum, marked ►C1 in it, and no
amendment. The rule this script pins by moves an act to a consolidated CELEX
when an amendment has been folded in, and none has, so the OJ text stays the
pin, with the corrigendum beside it, and keeps the recitals the consolidation
does not carry.

## Licence

- Commission reuse policy (Decision 2011/833/EU), quoted verbatim
  in `LICENSES/LicenseRef-EUR-Lex-Reuse.txt` and declared for this directory
  in `REUSE.toml`.
- Copyright: © European Union, 1998-2026. Reuse is authorised
  provided the source is acknowledged; this record and the index at
  `docs/law/README.md` are that acknowledgement.

Do not hand-edit anything in this directory. Re-run
`scripts/vendor/law-eu.sh` instead — a hand edit makes the SHA256SUMS line a
lie, which is the one thing a vendored legal text may never be.

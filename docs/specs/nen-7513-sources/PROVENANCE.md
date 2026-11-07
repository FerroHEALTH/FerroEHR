# Provenance: public sources on the content of a NEN 7513 log record

NEN 7513:2024 ("Medische informatica - Logging - Vastleggen van acties op
elektronische patiëntdossiers", NEN, December 2024) is sold under copyright
and is not here. Its record is `docs/law/nl/nen-7513/PROVENANCE.md`. This
directory holds two public sources that state what the standard asks of a
log record, so the access log design can map it without the standard's text.

## Logboek Dataverwerkingen NEN7513 Extensie (committed, under `logius/`)

- Source: <https://github.com/Logius-standaarden/logboek-extensie-nen7513>, rendered at
  <https://logius-standaarden.github.io/logboek-extensie-nen7513/>
- Pin: commit `ac887b9584afab1c44f5ebc2cec14eb10194e6f7` of the `develop` branch (2026-04-20), each file
  checked against the sha256 below
- Document: Logius, "Logboek Dataverwerkingen NEN7513 Extensie", version
  0.0.1, status "werkversie" (draft), dated 2025-03-31 in `js/config.mjs`.
  `specificatie.md` lists the NEN 7513 fields "zoals verplicht gesteld"
  and maps them onto the Logboek Dataverwerkingen standard; `js/config.mjs`
  cites NEN 7513, December 2024. The document says it is a concept and not
  directly usable as a standard; the design reads it for the field list only.
- Retrieved: 2026-11-07
- Upstream licence: read for this copy on 2026-11-07. The repository's
  `LICENSE` is the W3C Software and Document Notice and License, vendored
  beside the files; `js/config.mjs` publishes the document under
  `license: "cc-by"`, which the rendered page states as Creative Commons
  Attribution 4.0. Either permits this copy with its notice.
  `REUSE.toml` declares the tree `CC-BY-4.0 OR W3C-20150513`.
- What is taken: the specification text (`abstract.md`,
  `specificatie.md`), the page that assembles it (`index.html`), its
  configuration (`js/config.mjs`) and the licence. The workflow files are
  not taken.

| File | sha256 |
|---|---|
| `logius/LICENSE` | `ce7d80d054cfbdd1f0ecab0b33af23f5869ed19797568943c327d655c4138c75` |
| `logius/abstract.md` | `5d15878f5e9fdd0b3eb9147bc890fca99c5c371757cb5da989337135d1f8ae47` |
| `logius/index.html` | `608127322c72f53c92245a54da5aaa2f9a6a0771dd7c5def486ae239fa696dfe` |
| `logius/js/config.mjs` | `069aac258ef44bcbcb63ad36913a595ab87434a7680f4624c19beddf0c2ded63` |
| `logius/specificatie.md` | `a7ba0221bf133b141a4a7b8e423627da2ef27742884d50c5c42a814608d3cbc6` |

## Kamerstuk 27529, nr. 228 (cache only)

- Source: <https://zoek.officielebekendmakingen.nl/kst-27529-228.xml>, the official XML of the letter of the Minister voor
  Medische Zorg to the Tweede Kamer on ICT in de Zorg, from
  zoek.officielebekendmakingen.nl
- What the design cites: the section "Logging en elektronisch ontsluiten
  logginggegevens", which states that NEN 7513 specifies the events logged
  and the data each log line records, and that the events are access to
  patient data, access to the logging, and events that can affect the
  meaning or reliability of the logging; and that the added data themselves
  stay in the record, not in the logging.
- Upstream licence: a document the government made public, which Auteurswet
  Art. 15b lets anyone reproduce unless copyright is reserved; this
  repository holds no licence record for parliamentary papers, so the letter
  is cache only, fetched into the git-ignored
  `.vendor-cache/nen-7513-sources/`.

| File | sha256 | Status |
|---|---|---|
| `kst-27529-228.xml` | `cebe41be636890fca033a9b64b40a15fa316b273d2a31c9ce1a22fd33a9fd8e1` | cache only |

- Fetched by: `scripts/vendor/nen-7513-sources.sh`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)

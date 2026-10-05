---
paths: ["scripts/vendor/*.sh", "corpus/**", "crates/openehr-adl/tests/corpus/**"]
---

# Vendoring external corpora (CKM, ADL libraries, upstream test material)

Every externally-sourced corpus in this repo is fetched by a **committed
`scripts/vendor/*.sh` script**, vendored **verbatim**, and stamped with a
`PROVENANCE.md` that records the source, the pin, **and the upstream
license** (with the upstream `LICENSE` file vendored alongside where the
source publishes one — issue #1883). Never hand-download into the tree,
never hand-edit a vendored file, never paste a corpus in from a chat
transcript. To refresh or extend a corpus: change the script, re-run it,
commit the result.

| corpus | script | destination |
|---|---|---|
| openEHR spec text + CNF schedule | `scripts/vendor/spec-docs.sh` | `docs/specs/openehr/` |
| CKM templates (OPT 1.4) — curated journey pack + full library | `scripts/vendor/ckm-templates.sh` | `corpus/templates/ckm/{,full/}` |
| CKM archetypes (**ADL 1.4**) | `scripts/vendor/ckm-archetypes.sh` | `corpus/archetypes/ckm/adl14/` |
| ADL **2** archetypes + their 1.4 twins | `scripts/vendor/adl2-archetypes.sh` | `crates/openehr-adl/tests/corpus/adl2-reference/`, `corpus/archetypes/adl2/` |
| CKM example skeletons (generated once vs a composed SUT) | `scripts/generate-ckm-examples.sh` | `…/templates/ckm/*.example.json` |
| EU legal acts + the EDPB pseudonymisation guidelines (the compliance citations' referents) | `scripts/vendor/law-eu.sh` | `docs/law/eu/` |
| Dutch acts and decrees (+ the NEN 75xx records, text absent by copyright) | `scripts/vendor/law-nl.sh` | `docs/law/nl/` |
| German federal law (gesetze-im-internet XML, pinned by builddate; the DigiG as its BGBl. PDF) | `scripts/vendor/law-de.sh` | `docs/law/de/` |
| Swiss federal law (Fedlex filestore HTML, German + non-binding English where published at the same consolidation) | `scripts/vendor/law-ch.sh` | `docs/law/ch/` |
| FerroPULSE report v1 JSON Schemas + examples (the usage report contract) | `scripts/vendor/ferropulse-schemas.sh` | `corpus/ferropulse/` |

## The openEHR CKM REST API — facts, verified 2026-08-01

Base: `https://ckm.openehr.org/ckm/rest/v1`. The API IS documented: a
Swagger 2.0 document at `/rest/v1/swagger.json` ("CKM REST API" 1.6.0, 39
paths) with Swagger UI at `/ckm/rest-doc/` (verified 2026-09-03; the 2026-08-01
probe tried `openapi.json` and `v3/api-docs` and wrongly concluded none
existed). Read it first, then re-verify on the wire before trusting a guess.

- **Pagination is `?size=M&offset=N` and nothing else** (corrected
  2026-09-03: the earlier `page` claim was wrong; `?page=0&size=10000` worked
  only because `size` did). `page`, `limit`, `pageSize`, `maxResults`, `count`,
  `rows`, `resultsPerPage`, `startIndex` are all silently IGNORED and you get a
  **20-row first page** — which reads exactly like "CKM only publishes 20
  resources". This has burned time twice. Every list fetch uses size/offset
  **and asserts the row count grew**
  (both vendor scripts fail loud on `<= 20` rows).
- Resource lists: `GET /templates`, `GET /archetypes`. Both return a flat JSON
  array of metadata (`cid`, `resourceType`, `resourceMainId`,
  `resourceMainDisplayName`, `status`, `modificationTime`,
  `versionAssetLatest`; archetypes also carry `revisionLatest`).
- Exports: `GET /templates/{cid}/opt` (OPT 1.4 XML),
  `GET /archetypes/{cid}/adl` (ADL text), `GET /archetypes/{cid}/xml`
  (AM 1.4 ARCHETYPE XML). `GET /{kind}/{cid}` returns the metadata.
- **ADL 2 exists only as Ocean's generated conversion.** `/rest/v1` has no
  ADL 2 export, but the legacy servlet
  `GET /ckm/retrieveArchetype?cid-archetype=<cid>&format=ADL2` returns an
  `.adls` whose header carries `generated` (verified 2026-09-03). It is a
  1.4->2 conversion, never an authored source, so the ADL 2 corpus stays
  `openEHR/adl-archetypes`. Bulk: `GET /ckm/retrieveResources` with no
  parameters ships the whole published archetype library as one zip.
- **A 404 on an export is usually not a bug**: resources held in a private
  CKM incubator are only exportable by a signed-in account with access. Record
  them as unreachable in `PROVENANCE.md` — never silently skip, never drop the
  row from the count.
- Naming: archetypes have a stable HRID in `resourceMainId`
  (`openEHR-EHR-CLUSTER.device.v1`) — use it as the file name. Templates have
  a **UUID** there, so template file names are slugs derived from the display
  name; those slugs are NOT stable across a display-name change, which is why
  the curated journey pack keeps a hand-pinned slug per cid (a contract read
  by the conformance instrument's corpus manifest and journey definitions and
  by `generate-ckm-examples.sh` — never rename or drop one).

## ADL 1.4 vs ADL 2.4 — where each dialect comes from

We support **both** generations (`openehr-adl`; `am14`/`am24`), so the corpus
must carry both. They come from **different sources**, and the split is not
negotiable:

- **ADL 1.4 → the live CKM.** `GET /archetypes/{cid}/adl` answers
  `adl_version=1.4`. That is the only ADL CKM publishes.
- **ADL 2 → `github.com/openEHR/adl-archetypes`, pinned by commit.** CKM has
  **no ADL 2 export**: `/adl2`, `/adl14`, `/adl2.4`, `/opt2`, `/source` all
  404, and `?format=ADL2` / `?version=2` are silently ignored and return
  byte-identical 1.4 text. Never label a CKM export "ADL 2".
- **Never generate the ADL 2 side with our own 1.4→2 converter.**
  `openehr_adl::adl14::convert` has no spec basis (it is our own design —
  archie is prior art only), so feeding its output back as the ADL 2 corpus
  would validate the converter against itself. Upstream's own
  `Reference/CKM_2013_12_09` tree carries `*.adl`/`*.adls` **pairs** of the
  same archetypes — that pairing is the independent conversion reference.
- The `openehr-adl` ADL 2 regression library
  (`crates/openehr-adl/tests/corpus/adl2-reference/` — the crate's OWN corpus,
  a different tree from the shared `corpus/`) is vendored from the same
  pinned commit;
  `scripts/vendor/adl2-archetypes.sh --check` proves the script still
  reproduces the committed tree byte-for-byte. Its provenance record stays
  `crates/openehr-adl/tests/corpus/PROVENANCE.md`.

## A vendored corpus is not done until it is exercised

Vendoring is half a change. The standing owner rule (`.claude/rules/testing.md`
§CNF coverage) applies to every corpus here:

- **100% of a vendored corpus is exercised**, with a coverage gate that fails
  when a file is present but unreferenced — a big pack that nothing parses is
  decoration, and it silently overstates what the suite proves.
- **Adjudicated skips only.** Real-world CKM content contains genuinely
  invalid artefacts (e.g. CKM cid `1013.26.61`, whose OPT carries an
  `assumed_value` outside its constrained code list — AM 1.4
  `Assumed_value_valid`). A file our conformant reader rejects is either a
  spec-cited expected-rejection entry in the owning gate, or an entry in the
  conformance instrument's ambiguity register — never a quiet exclusion and
  never a weakened gate. The valid/invalid twins rule holds (testing.md).
- The per-file verdict/defect manifest for catalogue fixtures lives with the
  instrument (Veredictum's `artifacts/corpus/MANIFEST.yaml`); the bulk breadth
  packs here record their inventory + adjudications in the pack's own
  `PROVENANCE.md` and are driven by a directory-walking gate.
- Size honesty: these packs are large (the full CKM template pack is ~100 MB
  of XML; the archetype packs ~25 MB). That is acceptable for text that git
  compresses well, but a pack is only worth its weight if a gate reads it.
- **The law corpus (`docs/law/`) is exercised by digest, not by a parser**
  (#3291/#3292): no reader consumes a legal text, so its gate is
  `scripts/checks/law-corpus.sh`, which reads every vendored byte against the
  recorded `SHA256SUMS`, refuses a file the record does not name (and a record
  naming a missing file), and requires every act directory to be indexed by
  `docs/law/README.md`, whose page-to-act table is the coverage record.

## EU and Dutch legal texts — fetch facts, verified 2026-09-12

- **The EUR-Lex `legal-content/…/TXT/HTML` page is not vendorable byte for
  byte**: it injects a bot-detection script with a per-request `agentId` and
  `rpid`, so two fetches of an unchanged act differ. Fetch the Publications
  Office CELEX URI instead, `https://publications.europa.eu/resource/celex/<celex>`
  with `Accept: application/xhtml+xml`, which serves the same bytes every time.
- **That URI answers `303` with an `http://` Location.** Resolve the header and
  re-issue over https (`resolve_celex()` in `law-eu.sh`); never widen
  `--proto-redir` to permit the plaintext hop.
- **wetten.overheid.nl stamps `Geraadpleegd op <date>` into the served markup**,
  so an NL re-run always changes the digests even when no act changed a word; a
  re-pin diff there is not evidence of an upstream change by itself.
- Only the OJ text has legal effect; a consolidation is vendored when EUR-Lex
  publishes one that folds in an amendment (GDPR, MDR), and the OJ text
  otherwise (EHDS, NIS2, CRA). Each `PROVENANCE.md` records which and why.

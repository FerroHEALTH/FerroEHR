# CKM template pack — provenance

Vendored from the official openEHR CKM (`https://ckm.openehr.org/ckm/rest/v1`) by
`scripts/vendor/ckm-templates.sh` on 2026-11-01T23:40:36Z.
Each file is CKM's own OPT export for the cited template, verbatim.
Example skeletons (`*.example.json`) are generated once against the
composed SUT by `scripts/generate-ckm-examples.sh` and committed
(byte-identical payload ground for every SUT; never fetched at run
time). The conformance instrument's measured workloads carry their
own copies of this pack under the same slugs (Veredictum's
`perf/payloads.yaml`).

The **curated pack** below is referenced by slug from this
repository's tests and benches, the sandbox reseed, the example
generator and Veredictum's `perf/payloads.yaml` — the slugs are a
stable contract. The **full library** is a separate pack under
`full/` with its own provenance file.

## Licensing

CKM publishes no repository-level license; each OPT embeds its source
archetypes' `licence` metadata, and the corpus is **mixed**: a count
over this directory on 2026-08-12 found **114 files carrying CC-BY-SA
4.0 and 10 carrying CC-BY-SA 3.0**. Read the individual file; its own
metadata is the authority. Vendored verbatim, so authorship and
licence metadata ride along; root reference copies:
`LICENSE-CC-BY-SA-3.0` and `LICENSE-CC-BY-SA-4.0`, and `REUSE.toml`
declares this tree as `CC-BY-SA-3.0 AND CC-BY-SA-4.0`.

| cid | slug | display name | status | modified | journey role |
|---|---|---|---|---|---|
| 1013.26.380 | vital-signs | Vital signs | DRAFT | 2021-03-08T13:30:44+01:00 | vitals_round (full observation round) |
| 1013.26.408 | generic-lab-test-result | Generic lab test result example simple | DRAFT | 2021-10-18T11:28:46+02:00 | lab_pipeline (result contribution) |
| 1013.26.2 | ereferral | eReferral | DRAFT | 2010-03-25T07:26:15+01:00 | lab_pipeline / imaging_pipeline (order) |
| 1013.26.386 | ccta-report | CCTA report | INITIAL | 2021-08-02T07:29:55+02:00 | imaging_pipeline (report) |
| 1013.26.80 | eprescription-fhir | ePrescription (FHIR) | DRAFT | 2016-05-23T23:01:02+02:00 | medication_round (order + administrations) |
| 1013.26.357 | medicines-list | Medicines list item R1 | INITIAL | 2020-07-22T09:13:37+02:00 | medicines_reconciliation (ward-seeded, updated) |
| 1013.26.191 | gp-data-set | GP data set | INITIAL | 2018-10-15T02:11:04+02:00 | correction target (ward-seeded, amended) |
| 1013.26.376 | international-patient-summary | International Patient Summary | DRAFT | 2020-08-18T04:28:14+02:00 | admission / discharge summary |
| 1013.26.360 | problem-list | Problem/Diagnosis list item R1 | INITIAL | 2020-07-22T09:15:48+02:00 | admission (problem list) |
| 1013.26.199 | bc-breast-cancer-report | British Columbia Cancer agency Breast Cancer Synoptic Report | INITIAL | 2018-11-12T12:27:58+01:00 | specialist_report (cancer synoptic report) |
| 1013.26.40 | treat-registry-report | TREAT Registry report | DRAFT | 2015-02-18T21:32:34+01:00 | registry_submission (registry export) |
| 1013.26.377 | sars-event-notification | SARS event notification | DRAFT | 2020-10-13T06:36:16+02:00 | public_health_notification (statutory notification) |
| 1013.26.282 | covid19-infection-report | openEHR confirmed COVID-19 infection report.v0 | DRAFT | 2020-08-07T07:19:11+02:00 | public_health_notification (confirmed-case follow-up) |
| 1013.26.988 | poisoning-case-investigation | Accidental poisoning case investigation form | INITIAL | 2024-12-30T03:35:42+01:00 | case_investigation |
| 1013.26.980 | diphtheria-case-investigation | Diphtheria case investigation form | INITIAL | 2024-12-30T01:55:25+01:00 | case_investigation |
| 1013.26.977 | congenital-syphilis-case-investigation | Congenital syphilis case investigation form | INITIAL | 2024-12-30T01:49:57+01:00 | case_investigation (largest published form — the large-payload scale probe) |

## Adjudicated boundaries

Two EHDS Annex I priority categories have no template in this pack, and the
gap is a boundary rather than an omission (#3206): CKM publishes no operational
template for **electronic dispensations** (Annex I 3) or **discharge reports**
(Annex I 6) as such at the pinned revision, and this pack vendors CKM content
verbatim, never a template authored here to fill a category. The
international-patient-summary template covers admission and discharge
summaries as a COMPOSITION shape, not the discharge-report category the
Regulation names. The question reopens when a CKM template for either category
is published, or when the Article 36 implementing acts fix the exchange format
those categories must be provided in.

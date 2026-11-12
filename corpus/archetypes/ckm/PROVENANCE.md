# CKM archetype library (ADL 1.4) — provenance

Every archetype the official openEHR CKM (`https://ckm.openehr.org/ckm/rest/v1`) lists, exported
by CKM itself and vendored verbatim by
`scripts/vendor/ckm-archetypes.sh` on 2026-11-12T14:25:48Z.

## The list

`GET /archetypes?size=10000&offset=0` with `resource-state` set to every
state the contract (`https://ckm.openehr.org/ckm/rest/v1/openapi.json`) enumerates: INITIAL, DRAFT, TEAMREVIEW, REVIEWSUSPENDED, RELEASECANDIDATE, PUBLISHED, REASSESS_DRAFT, REASSESS_TEAMREVIEW, REASSESS_REVIEWSUSPENDED, REJECTED, DEPRECATED.
CKM's default leaves out REJECTED and DEPRECATED, so those are asked
for by name. Each row below records the archetype's lifecycle state and
its CKM modification time as the list reported them; the state and the
time together pin the export, since a v0 archetype can change without
a new id. `adl14/SHA256SUMS` (and `xml/SHA256SUMS` with the XML twin)
holds the sha256 of every file vendored.

## Dialect

`adl14/` holds CKM's `GET /archetypes/{cid}/adl` response — **ADL 1.4**
text (`adl_version=1.4`). `/rest/v1` publishes NO ADL 2 (`/adl2`,
`/adl14`, `/opt2` 404; `?format=ADL2` is ignored); the legacy servlet's
`format=ADL2` export is Ocean's own GENERATED 1.4->2 conversion, so the **ADL 2.4 half of the corpus comes from
`scripts/vendor/adl2-archetypes.sh`** (openEHR/adl-archetypes). A CKM
export is never labelled ADL 2, and the ADL 2 side is never produced by
running our own 1.4->2 converter over these files — that would test the
converter against itself.

`xml/` holds the AM 1.4 ARCHETYPE XML twin of the same 1263 exports
(`GET /archetypes/{cid}/xml`), for the XML codec's read path.

## Exercised, with adjudicated refusals

The pack is parsed 100% by
`crates/openehr-adl/tests/it/ckm_archetype_packs.rs`
(`ckm_adl14_pack_parses`). A file the conformant reader REFUSES is listed
in that gate's `ADJUDICATED_CKM_ADL14` table with the syntax code the
refusal must carry plus the spec ground for it, and the gate asserts the
refusal HAPPENS and carries that code — a refusal is a negative test, not
a skip. Never delete a refused file: that drops the negative case
(`.claude/rules/vendored-corpora.md`, `.claude/rules/testing.md`).

## Licensing

CKM publishes no repository-level license; each archetype carries its
own `description` > `licence` metadata, and the corpus is **mixed**:
of the 1262 ADL texts, **685 state CC-BY-SA 4.0 and 468
CC-BY-SA 3.0** (counted by this script at the fetch). Read the
individual file; its own metadata is the authority. Vendored verbatim,
so the authorship and licence metadata ride along in every file; root
reference copies: `LICENSE-CC-BY-SA-3.0` and `LICENSE-CC-BY-SA-4.0`
(`LICENSES/CC-BY-SA-3.0.txt`, `LICENSES/CC-BY-SA-4.0.txt`), and
`REUSE.toml` declares this tree as `CC-BY-SA-3.0 AND CC-BY-SA-4.0`.

## Inventory

- listed by CKM: **1263**
- vendored: **1262**
- unreachable: **1**

| RM class | count |
|---|---|
| openEHR-EHR-CLUSTER | 533 |
| openEHR-EHR-OBSERVATION | 407 |
| openEHR-EHR-EVALUATION | 133 |
| openEHR-EHR-COMPOSITION | 34 |
| openEHR-EHR-SECTION | 32 |
| openEHR-EHR-ACTION | 28 |
| openEHR-EHR-INSTRUCTION | 27 |
| openEHR-EHR-ADMIN_ENTRY | 23 |
| openEHR-DEMOGRAPHIC-CLUSTER | 18 |
| openEHR-EHR-ITEM_TREE | 8 |
| openEHR-DEMOGRAPHIC-ADDRESS | 5 |
| openEHR-DEMOGRAPHIC-ROLE | 4 |
| openEHR-DEMOGRAPHIC-PARTY_IDENTITY | 3 |
| openEHR-DEMOGRAPHIC-PERSON | 2 |
| openEHR-EHR-ELEMENT | 2 |
| openEHR-DEMOGRAPHIC-CAPABILITY | 1 |
| openEHR-DEMOGRAPHIC-ITEM_TREE | 1 |
| openEHR-DEMOGRAPHIC-ORGANISATION | 1 |

| status | count |
|---|---|
| DRAFT | 408 |
| REJECTED | 283 |
| INITIAL | 256 |
| PUBLISHED | 241 |
| DEPRECATED | 34 |
| REVIEWSUSPENDED | 28 |
| TEAMREVIEW | 10 |
| REASSESS_DRAFT | 2 |

## Unreachable (recorded, not skipped)

CKM answers 404 for resources held in a private incubator; they are
only exportable by a signed-in account with access.

| cid | archetype | status | modified |
|---|---|---|---|
| 1013.1.4512 | openEHR-EHR-OBSERVATION.mental_state.v0 | INITIAL | 2020-03-24T03:52:55+01:00 |

## Vendored

| cid | archetype | display name | status | modified | revision |
|---|---|---|---|---|---|
| 1013.1.499 | `openEHR-DEMOGRAPHIC-ADDRESS.address-provider.v0` | Healthcare provider address | DRAFT | 2019-08-08T14:52:12+02:00 | 0.0.1-alpha |
| 1013.1.484 | `openEHR-DEMOGRAPHIC-ADDRESS.address.v0` | Address | REVIEWSUSPENDED | 2019-08-08T14:45:55+02:00 | 0.0.1-alpha |
| 1013.1.485 | `openEHR-DEMOGRAPHIC-ADDRESS.address_iso-health_provider.v1` | Address | REJECTED | 2009-08-28T09:56:50+02:00 |  |
| 1013.1.487 | `openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication-provider.v0` | Healthcare provider electronic address | DRAFT | 2019-08-08T15:11:59+02:00 | 0.0.1-alpha |
| 1013.1.486 | `openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication.v0` | Electronic address | DRAFT | 2019-08-08T15:08:00+02:00 | 0.0.1-alpha |
| 1013.1.824 | `openEHR-DEMOGRAPHIC-CAPABILITY.individual_credentials.v0` | Professional credentials | DRAFT | 2019-08-08T15:24:16+02:00 | 0.0.1-alpha |
| 1013.1.488 | `openEHR-DEMOGRAPHIC-CLUSTER.biometric_identifier_iso.v0` | Biometric identifier | DRAFT | 2019-08-12T14:56:05+02:00 | 0.0.1-alpha |
| 1013.1.834 | `openEHR-DEMOGRAPHIC-CLUSTER.birth_data_additional_detail_br.v0` | Other birth certificate data | DRAFT | 2019-10-01T04:15:58+02:00 | 0.0.1-alpha |
| 1013.1.495 | `openEHR-DEMOGRAPHIC-CLUSTER.high_level_address_other_data_br.v0` | Other high level address components | DRAFT | 2019-10-01T02:32:56+02:00 | 0.0.1-alpha |
| 1013.1.489 | `openEHR-DEMOGRAPHIC-CLUSTER.identifier_other_details.v0` | Additional identifier data | DRAFT | 2019-08-12T14:27:59+02:00 | 0.0.1-alpha |
| 1013.1.490 | `openEHR-DEMOGRAPHIC-CLUSTER.individual_credentials_iso.v0` | Professional credentials | DRAFT | 2019-08-12T14:39:17+02:00 | 0.0.1-alpha |
| 1013.1.500 | `openEHR-DEMOGRAPHIC-CLUSTER.individual_provider_credentials_iso.v0` | Individual healthcare provider credentials | DRAFT | 2019-08-09T15:08:43+02:00 | 0.0.1-alpha |
| 1013.1.504 | `openEHR-DEMOGRAPHIC-CLUSTER.person_additional_data_br.v0` | Extended personal demographics | DRAFT | 2019-10-01T02:40:03+02:00 | 0.0.1-alpha |
| 1013.1.503 | `openEHR-DEMOGRAPHIC-CLUSTER.person_additional_data_iso.v0` | Person additional demographic data | DRAFT | 2019-08-12T14:58:59+02:00 | 0.0.1-alpha |
| 1013.1.491 | `openEHR-DEMOGRAPHIC-CLUSTER.person_birth_data_iso.v0` | Birth data | DRAFT | 2025-09-02T01:09:47+02:00 | 0.0.1-alpha |
| 1013.1.492 | `openEHR-DEMOGRAPHIC-CLUSTER.person_death_data_iso.v0` | Death data | DRAFT | 2019-08-12T14:33:27+02:00 | 0.0.1-alpha |
| 1013.1.493 | `openEHR-DEMOGRAPHIC-CLUSTER.person_details_br.v1` | Person´s data | REJECTED | 2009-08-27T17:13:04+02:00 |  |
| 1013.1.494 | `openEHR-DEMOGRAPHIC-CLUSTER.person_details_iso.v1` | Person´s data | REJECTED | 2009-08-27T17:13:09+02:00 |  |
| 1013.1.833 | `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier-provider.v0` | Healthcare provider identifier | DRAFT | 2019-08-12T15:05:15+02:00 | 0.0.1-alpha |
| 1013.1.470 | `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v0` | Person identifier | DRAFT | 2018-12-20T10:41:29+01:00 | 0.0.1-alpha |
| 1013.1.471 | `openEHR-DEMOGRAPHIC-CLUSTER.person_other_birth_data_br.v0` | Other birth certificate data | DRAFT | 2019-10-01T02:27:12+02:00 | 0.0.1-alpha |
| 1013.1.472 | `openEHR-DEMOGRAPHIC-CLUSTER.person_other_death_data.v0` | Death additional data | DRAFT | 2019-08-12T14:49:49+02:00 | 0.0.1-alpha |
| 1013.1.473 | `openEHR-DEMOGRAPHIC-CLUSTER.provider_identifier.v0` | Healthcare provider identifier | DRAFT | 2019-08-09T15:03:48+02:00 | 0.0.1-alpha |
| 1013.1.474 | `openEHR-DEMOGRAPHIC-CLUSTER.registration_other_data.v0` | Other provider registration data | DRAFT | 2019-08-12T15:01:38+02:00 | 0.0.1-alpha |
| 1013.1.823 | `openEHR-DEMOGRAPHIC-ITEM_TREE.person_details.v0` | Person data | DRAFT | 2025-04-10T01:24:53+02:00 | 0.0.1-alpha |
| 1013.1.475 | `openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0` | Organisation | DRAFT | 2019-08-09T09:43:02+02:00 | 0.0.1-alpha |
| 1013.1.476 | `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.organisation_name.v0` | Organisation name | DRAFT | 2019-08-09T15:32:33+02:00 | 0.0.1-alpha |
| 1013.1.478 | `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name-individual_provider.v0` | Individual healthcare provider name | DRAFT | 2019-08-09T15:44:26+02:00 | 0.0.1-alpha |
| 1013.1.477 | `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0` | Person name | REVIEWSUSPENDED | 2019-08-09T15:36:49+02:00 | 0.0.1-alpha |
| 1013.1.821 | `openEHR-DEMOGRAPHIC-PERSON.person-patient.v0` | Patient | DRAFT | 2019-08-09T12:45:27+02:00 | 0.0.1-alpha |
| 1013.1.479 | `openEHR-DEMOGRAPHIC-PERSON.person.v0` | Person | DRAFT | 2022-01-20T11:01:19+01:00 | 0.0.1-alpha |
| 1013.1.502 | `openEHR-DEMOGRAPHIC-ROLE.healthcare_consumer.v0` | Healthcare consumer | DRAFT | 2019-08-09T13:50:08+02:00 | 0.0.1-alpha |
| 1013.1.501 | `openEHR-DEMOGRAPHIC-ROLE.healthcare_provider_organisation.v0` | Healthcare Provider Organisation | DRAFT | 2019-08-09T14:26:15+02:00 | 0.0.1-alpha |
| 1013.1.482 | `openEHR-DEMOGRAPHIC-ROLE.individual_provider.v0` | Individual healthcare provider | DRAFT | 2019-08-09T14:39:37+02:00 | 0.0.1-alpha |
| 1013.1.483 | `openEHR-DEMOGRAPHIC-ROLE.third_party_payer.v0` | Third-party payer | DRAFT | 2019-08-12T14:09:39+02:00 | 0.0.1-alpha |
| 1013.1.192 | `openEHR-EHR-ACTION.blood_transfusion_management.v0` | Blood transfusion management | DRAFT | 2025-01-14T10:53:44+01:00 | 0.0.1-alpha |
| 1013.1.7254 | `openEHR-EHR-ACTION.care_pathway.v1` | Care pathway management | PUBLISHED | 2026-09-21T13:28:24+02:00 | 1.0.0 |
| 1013.1.1655 | `openEHR-EHR-ACTION.care_plan.v0` | Care Plan | DRAFT | 2016-08-22T07:28:39+02:00 | 0.0.1-alpha |
| 1013.1.40 | `openEHR-EHR-ACTION.follow_up.v1` | Follow up action | REJECTED | 2015-02-05T01:29:42+01:00 |  |
| 1013.1.8158 | `openEHR-EHR-ACTION.health_assessment.v0` | Health assessment | INITIAL | 2026-01-19T01:35:00+01:00 | 0.0.1-alpha |
| 1013.1.1395 | `openEHR-EHR-ACTION.health_education.v1` | Health education | PUBLISHED | 2025-05-26T01:29:21+02:00 | 1.1.3 |
| 1013.1.122 | `openEHR-EHR-ACTION.imaging.v1` | Imaging investigation | REJECTED | 2015-02-05T01:31:22+01:00 |  |
| 1013.1.1597 | `openEHR-EHR-ACTION.imaging_exam.v0` | Imaging examination | DRAFT | 2022-01-17T13:45:27+01:00 | 0.0.1-alpha |
| 1013.1.1303 | `openEHR-EHR-ACTION.informed_consent.v0` | Informed consent | DRAFT | 2022-03-29T08:55:25+02:00 | 0.0.1-alpha |
| 1013.1.179 | `openEHR-EHR-ACTION.intravenous_fluid_administration.v1` | Intravenous fluid administration | REJECTED | 2015-02-05T01:31:38+01:00 |  |
| 1013.1.6558 | `openEHR-EHR-ACTION.laboratory_test.v0` | Laboratory test | INITIAL | 2022-10-20T07:08:10+02:00 | 0.0.1-alpha |
| 1013.1.7897 | `openEHR-EHR-ACTION.medical_equipment_supply.v0` | Medical equipment supply | INITIAL | 2025-05-29T11:53:31+02:00 | 0.0.1-alpha |
| 1013.1.123 | `openEHR-EHR-ACTION.medication.v1` | Medication management | PUBLISHED | 2026-02-03T13:31:18+01:00 | 1.6.0 |
| 1013.1.3226 | `openEHR-EHR-ACTION.notification.v0` | Notification | INITIAL | 2018-05-01T07:43:31+02:00 | 0.0.1-alpha |
| 1013.1.7899 | `openEHR-EHR-ACTION.physical_assistance.v0` | Physical assistance | INITIAL | 2025-05-29T11:54:29+02:00 | 0.0.1-alpha |
| 1013.1.1858 | `openEHR-EHR-ACTION.procedure-Hip_arthroplasty_previous_procedure_res.v1` | Primary hip arthroplasty - previous procedure | INITIAL | 2015-03-02T16:01:59+01:00 |  |
| 1013.1.1889 | `openEHR-EHR-ACTION.procedure-hip_arthroplasty_revision_previous_procedure_res.v1` | Revision hip arthroplasty- previous procedure | INITIAL | 2015-03-16T23:20:02+01:00 |  |
| 1013.1.1859 | `openEHR-EHR-ACTION.procedure-primary_hip_arthroplasty_res.v1` | Primary hip arthroplasty | INITIAL | 2015-03-02T16:10:27+01:00 |  |
| 1013.1.1860 | `openEHR-EHR-ACTION.procedure-revision_hip_arthroplasty_res.v1` | Revision hip arthroplasty | INITIAL | 2015-03-16T23:21:35+01:00 |  |
| 1013.1.204 | `openEHR-EHR-ACTION.procedure.v1` | Procedure | PUBLISHED | 2026-09-17T11:56:07+02:00 | 1.5.3 |
| 1013.1.7903 | `openEHR-EHR-ACTION.psychosocial_therapy.v0` | Psychosocial therapy | INITIAL | 2025-08-26T00:01:08+02:00 | 0.0.1-alpha |
| 1013.1.1665 | `openEHR-EHR-ACTION.referral.v1` | Referral | REJECTED | 2015-07-20T13:30:31+02:00 |  |
| 1013.1.1326 | `openEHR-EHR-ACTION.review.v0` | Review | DRAFT | 2019-09-24T07:45:07+02:00 | 0.0.1-alpha |
| 1013.1.3194 | `openEHR-EHR-ACTION.screening.v0` | Screening Activity | DRAFT | 2018-04-17T07:48:27+02:00 | 0.0.1-alpha |
| 1013.1.2374 | `openEHR-EHR-ACTION.service.v1` | Service | PUBLISHED | 2026-08-13T18:02:01+02:00 | 1.0.4 |
| 1013.1.8146 | `openEHR-EHR-ACTION.unit_transfusion_management.v0` | Unit transfusion management | DRAFT | 2026-01-07T15:11:03+01:00 | 0.0.1-alpha |
| 1013.1.7910 | `openEHR-EHR-ACTION.vaccination_management.v0` | Vaccination management | INITIAL | 2025-07-22T03:25:38+02:00 | 0.0.1-alpha |
| 1013.1.8145 | `openEHR-EHR-ACTION.whole_transfusion_management.v0` | Whole transfusion management | DRAFT | 2026-01-07T15:10:28+01:00 | 0.0.1-alpha |
| 1013.1.79 | `openEHR-EHR-ADMIN_ENTRY.admission.v0` | Patient admission | REJECTED | 2021-08-10T07:07:46+02:00 | 0.0.1 |
| 1013.1.4078 | `openEHR-EHR-ADMIN_ENTRY.arrival.v0` | Arrival | REJECTED | 2023-12-21T08:18:04+01:00 | 0.0.1 |
| 1013.1.8225 | `openEHR-EHR-ADMIN_ENTRY.citizenship.v1` | Citizenship | PUBLISHED | 2026-09-16T08:43:27+02:00 | 1.0.0 |
| 1013.1.7527 | `openEHR-EHR-ADMIN_ENTRY.contact_for_tracing.v0` | Exposure contact for tracing | INITIAL | 2024-12-29T01:10:51+01:00 | 0.0.1-alpha |
| 1013.1.5764 | `openEHR-EHR-ADMIN_ENTRY.contact_tracing.v0` | Contact tracing | INITIAL | 2022-07-25T14:04:31+02:00 | 0.0.1-alpha |
| 1013.1.4417 | `openEHR-EHR-ADMIN_ENTRY.covid_19_admission.v0` | Covid 19 Admission | INITIAL | 2020-03-18T09:50:10+01:00 | 0.0.1-alpha |
| 1013.1.4416 | `openEHR-EHR-ADMIN_ENTRY.covid__outcomes.v0` | COVID outcomes | INITIAL | 2020-03-18T09:49:35+01:00 | 0.0.1-alpha |
| 1013.1.5721 | `openEHR-EHR-ADMIN_ENTRY.demographics.v0` | Demographics container | DRAFT | 2021-09-24T07:04:29+02:00 | 0.0.1-alpha |
| 1013.1.4447 | `openEHR-EHR-ADMIN_ENTRY.episode_institution.v0` | Admission | REVIEWSUSPENDED | 2026-03-09T08:23:01+01:00 | 0.0.1-alpha |
| 1013.1.7551 | `openEHR-EHR-ADMIN_ENTRY.health_event_investigation_metadata.v0` | Health event investigation metadata | INITIAL | 2024-10-29T19:01:24+01:00 | 0.0.1-alpha |
| 1013.1.7522 | `openEHR-EHR-ADMIN_ENTRY.infectious_disease_investigation_metadata.v0` | Infectious disease investigation metadata | INITIAL | 2024-12-24T11:28:41+01:00 | 0.0.1-alpha |
| 1013.1.4831 | `openEHR-EHR-ADMIN_ENTRY.legal_authority.v0` | Legal authority | INITIAL | 2020-06-15T14:45:06+02:00 | 0.0.1-alpha |
| 1013.1.3185 | `openEHR-EHR-ADMIN_ENTRY.legal_constraint.v0` | Legal constraint | REJECTED | 2022-05-10T08:32:58+02:00 | 0.0.1 |
| 1013.1.6604 | `openEHR-EHR-ADMIN_ENTRY.outbreak_investigation.v0` | Outbreak investigation | INITIAL | 2023-07-19T06:28:31+02:00 | 0.0.1-alpha |
| 1013.1.6605 | `openEHR-EHR-ADMIN_ENTRY.outbreak_management.v0` | Outbreak management | INITIAL | 2023-01-24T11:02:47+01:00 | 0.0.1-alpha |
| 1013.1.2181 | `openEHR-EHR-ADMIN_ENTRY.procedure_efficiency.v0` | Efficiency of healthcare procedure | INITIAL | 2015-09-25T18:25:10+02:00 | 0.0.1-alpha |
| 1013.1.6391 | `openEHR-EHR-ADMIN_ENTRY.referral_review.v0` | Referral review | INITIAL | 2022-08-01T06:49:39+02:00 | 0.0.1-alpha |
| 1013.1.6434 | `openEHR-EHR-ADMIN_ENTRY.three_delays_model.v0` | Three Delays Model (3DM) | DRAFT | 2022-08-01T08:24:21+02:00 | 0.0.1-alpha |
| 1013.1.4438 | `openEHR-EHR-ADMIN_ENTRY.transfer_of_care.v0` | Transfer of care | DRAFT | 2020-03-15T02:07:18+01:00 | 0.0.1-alpha |
| 1013.1.6394 | `openEHR-EHR-ADMIN_ENTRY.transfer_review.v0` | Transfer review | INITIAL | 2022-07-11T05:53:43+02:00 | 0.0.1-alpha |
| 1013.1.3539 | `openEHR-EHR-ADMIN_ENTRY.translation_requirements.v1` | Translation requirement | PUBLISHED | 2025-05-22T00:37:38+02:00 | 1.1.2 |
| 1013.1.5089 | `openEHR-EHR-ADMIN_ENTRY.travel_event.v0` | Travel event | DRAFT | 2023-09-12T13:51:40+02:00 | 0.0.1-alpha |
| 1013.1.304 | `openEHR-EHR-ADMIN_ENTRY.triage.v0` | Triage | DRAFT | 2026-02-10T16:05:57+01:00 | 0.0.1-alpha |
| 1013.1.1689 | `openEHR-EHR-CLUSTER.acquisition_details_on_eye_fundus_images.v0` | Acquisition details on eye fundus images | INITIAL | 2016-07-16T19:00:31+02:00 | 0.0.1-alpha |
| 1013.1.2057 | `openEHR-EHR-CLUSTER.acquisition_details_on_ophthalmic_tomography.v0` | Acquisition details on ophthalmic tomography | INITIAL | 2015-06-19T18:47:54+02:00 | 0.0.1-alpha |
| 1013.1.2058 | `openEHR-EHR-CLUSTER.acquisition_details_on_visual_field_test.v0` | Acquisition details on visual field test | INITIAL | 2015-06-21T21:17:01+02:00 | 0.0.1-alpha |
| 1013.1.3764 | `openEHR-EHR-CLUSTER.activity-running.v0` | Running | INITIAL | 2019-04-26T19:14:53+02:00 | 0.0.1-alpha |
| 1013.1.273 | `openEHR-EHR-CLUSTER.address.v1` | Address | PUBLISHED | 2025-07-22T01:08:29+02:00 | 1.1.3 |
| 1013.1.4418 | `openEHR-EHR-CLUSTER.address_cc.v0` | Address | INITIAL | 2020-04-16T10:52:17+02:00 | 0.0.1-alpha |
| 1013.1.1742 | `openEHR-EHR-CLUSTER.address_isa.v1` | Address (ISA) | INITIAL | 2015-02-21T10:44:37+01:00 |  |
| 1013.1.1660 | `openEHR-EHR-CLUSTER.adhoc_cluster_heading.v0` | Adhoc cluster heading | DRAFT | 2025-09-01T11:34:37+02:00 | 0.0.1-alpha |
| 1013.1.5795 | `openEHR-EHR-CLUSTER.adverse_reaction_event.v1` | Adverse reaction event | PUBLISHED | 2025-07-28T11:52:36+02:00 | 1.0.2 |
| 1013.1.7339 | `openEHR-EHR-CLUSTER.airway_device.v0` | Airway Device | INITIAL | 2024-06-04T10:31:59+02:00 | 0.0.1-alpha |
| 1013.1.5615 | `openEHR-EHR-CLUSTER.alcohol_consumption.v0` | Alcohol consumption | INITIAL | 2021-07-16T09:11:38+02:00 | 0.0.1-alpha |
| 1013.1.356 | `openEHR-EHR-CLUSTER.anatomical_location-precise.v1` | Precise anatomical location | REJECTED | 2015-03-19T03:13:29+01:00 |  |
| 1013.1.587 | `openEHR-EHR-CLUSTER.anatomical_location.v1` | Anatomical location | PUBLISHED | 2026-08-17T14:47:27+02:00 | 1.5.1 |
| 1013.1.1995 | `openEHR-EHR-CLUSTER.anatomical_location_circle.v1` | Circular anatomical location | PUBLISHED | 2025-07-21T15:23:29+02:00 | 1.1.3 |
| 1013.1.3826 | `openEHR-EHR-CLUSTER.anatomical_location_gingivae.v0` | Anatomical location gingiva | INITIAL | 2019-06-11T11:04:16+02:00 | 0.0.1-alpha |
| 1013.1.1928 | `openEHR-EHR-CLUSTER.anatomical_location_precise.v0` | Precise anatomical location | DRAFT | 2019-09-24T06:19:20+02:00 | 0.0.1-alpha |
| 1013.1.1892 | `openEHR-EHR-CLUSTER.anatomical_location_relative.v1` | Relative anatomical location | DEPRECATED | 2020-07-09T10:26:16+02:00 | 1.0.2 |
| 1013.1.4863 | `openEHR-EHR-CLUSTER.anatomical_location_relative.v2` | Relative anatomical location | PUBLISHED | 2024-06-10T19:15:14+02:00 | 2.2.0 |
| 1013.1.3824 | `openEHR-EHR-CLUSTER.anatomical_location_tooth.v0` | Anatomical location tooth | INITIAL | 2019-06-11T10:53:45+02:00 | 0.0.1-alpha |
| 1013.1.2680 | `openEHR-EHR-CLUSTER.anatomical_pathology_exam.v0` | Anatomical pathology examination | TEAMREVIEW | 2019-12-17T13:17:23+01:00 | 0.0.1-alpha |
| 1013.1.7951 | `openEHR-EHR-CLUSTER.anatomical_pathology_exam_jm.v0` | Anatomical pathology examination | INITIAL | 2025-07-14T06:52:07+02:00 | 0.0.1-alpha |
| 1013.1.5884 | `openEHR-EHR-CLUSTER.art_container_details.v0` | ART container details | DRAFT | 2021-12-08T05:54:47+01:00 | 0.0.1-alpha |
| 1013.1.111 | `openEHR-EHR-CLUSTER.auscultation-chest.v1` | Auscultation of the chest | REJECTED | 2015-02-05T06:36:51+01:00 |  |
| 1013.1.110 | `openEHR-EHR-CLUSTER.auscultation.v1` | Auscultation | REJECTED | 2015-02-05T06:36:39+01:00 |  |
| 1013.1.7856 | `openEHR-EHR-CLUSTER.auscultation_bowel.v0` | Auscultation of bowel sounds | DRAFT | 2025-05-20T02:40:01+02:00 | 0.0.1-alpha |
| 1013.1.7862 | `openEHR-EHR-CLUSTER.auscultation_breath.v0` | Auscultation of breath sounds | DRAFT | 2025-05-20T02:36:58+02:00 | 0.0.1-alpha |
| 1013.1.2062 | `openEHR-EHR-CLUSTER.auscultation_lung.v0` | Auscultation of lung | REJECTED | 2019-07-15T13:14:26+02:00 | 0.0.1 |
| 1013.1.112 | `openEHR-EHR-CLUSTER.balance.v1` | Examination Findings - Balance | REJECTED | 2015-06-24T08:21:33+02:00 |  |
| 1013.1.5885 | `openEHR-EHR-CLUSTER.biobank_storage.v0` | Biobank storage | REJECTED | 2022-12-27T11:03:57+01:00 | 0.0.1 |
| 1013.1.6026 | `openEHR-EHR-CLUSTER.bioreagent.v0` | Bioreagent | DRAFT | 2025-09-01T00:44:09+02:00 | 0.0.1-alpha |
| 1013.1.6180 | `openEHR-EHR-CLUSTER.birth_detail.v0` | Birth detail | DRAFT | 2025-07-22T01:22:56+02:00 | 0.0.1-alpha |
| 1013.1.7978 | `openEHR-EHR-CLUSTER.bleeding_findings.v0` | Examination of bleeding | INITIAL | 2025-07-22T07:06:35+02:00 | 0.0.1-alpha |
| 1013.1.7542 | `openEHR-EHR-CLUSTER.blood_cell_count.v0` | Blood cell count | INITIAL | 2024-10-29T14:52:19+01:00 | 0.0.1-alpha |
| 1013.1.5077 | `openEHR-EHR-CLUSTER.boston_bowel_preparation_scale.v1` | Boston Bowel Preparation Scale | PUBLISHED | 2021-03-10T14:28:45+01:00 | 1.0.0 |
| 1013.1.8326 | `openEHR-EHR-CLUSTER.cardiff_acuity_card_test_result.v0` | Cardiff acuity test result | INITIAL | 2026-06-14T20:53:27+02:00 | 0.0.1-alpha |
| 1013.1.567 | `openEHR-EHR-CLUSTER.case_identification.v0` | Case identification | DRAFT | 2026-07-14T12:25:10+02:00 | 0.0.1-alpha |
| 1013.1.8222 | `openEHR-EHR-CLUSTER.catheter_cuff.v0` | Catheter cuff/balloon | DRAFT | 2026-03-18T12:40:20+01:00 | 0.0.1-alpha |
| 1013.1.8223 | `openEHR-EHR-CLUSTER.catheter_lumen.v0` | Catheter lumen | DRAFT | 2026-03-11T12:29:10+01:00 | 0.0.1-alpha |
| 1013.1.8221 | `openEHR-EHR-CLUSTER.catheter_tube.v0` | Catheter tube | DRAFT | 2026-03-18T12:36:18+01:00 | 0.0.1-alpha |
| 1013.1.364 | `openEHR-EHR-CLUSTER.cessation_attempts.v0` | Cessation attempts | DRAFT | 2018-05-08T08:35:31+02:00 | 0.0.1-alpha |
| 1013.1.20 | `openEHR-EHR-CLUSTER.change.v0` | Readiness for change | DRAFT | 2019-08-13T11:41:57+02:00 | 0.0.1-alpha |
| 1013.1.721 | `openEHR-EHR-CLUSTER.citation.v0` | Citation | DRAFT | 2024-11-24T11:23:14+01:00 | 0.0.1-alpha |
| 1013.1.2117 | `openEHR-EHR-CLUSTER.classification_amd.v0` | Classification of age related macular degeneration | INITIAL | 2015-06-25T11:57:59+02:00 | 0.0.1-alpha |
| 1013.1.2218 | `openEHR-EHR-CLUSTER.classification_glaucoma.v0` | Classification of glaucoma | INITIAL | 2015-09-08T14:20:29+02:00 | 0.0.1-alpha |
| 1013.1.1971 | `openEHR-EHR-CLUSTER.clinical_evidence.v1` | Clinical evidence | PUBLISHED | 2025-04-22T12:10:31+02:00 | 1.3.1 |
| 1013.1.206 | `openEHR-EHR-CLUSTER.clubbing.v1` | Clubbing | REJECTED | 2015-11-23T03:00:15+01:00 |  |
| 1013.1.6560 | `openEHR-EHR-CLUSTER.cobb_angle.v0` | Cobb angle | DRAFT | 2023-08-01T10:12:59+02:00 | 0.0.1-alpha |
| 1013.1.5792 | `openEHR-EHR-CLUSTER.condition_progress.v0` | Condition progress | INITIAL | 2022-07-20T10:03:00+02:00 | 0.0.1-alpha |
| 1013.1.2396 | `openEHR-EHR-CLUSTER.conditional_medication_rules.v0` | Conditional medication instructions | DRAFT | 2016-02-17T19:39:29+01:00 | 0.0.1-alpha |
| 1013.1.1307 | `openEHR-EHR-CLUSTER.consent_details.v0` | Informed consent details | DRAFT | 2018-09-10T08:50:10+02:00 | 0.0.1-alpha |
| 1013.1.5073 | `openEHR-EHR-CLUSTER.contact_cc.v0` | Contact | REJECTED | 2021-12-17T07:11:53+01:00 | 0.0.1 |
| 1013.1.113 | `openEHR-EHR-CLUSTER.coordination.v1` | Coordination | REJECTED | 2016-06-30T09:27:51+02:00 |  |
| 1013.1.2411 | `openEHR-EHR-CLUSTER.cormack_lehane.v1` | Cormack-Lehane classification | DEPRECATED | 2017-12-11T14:00:33+01:00 | 1.0.1 |
| 1013.1.2015 | `openEHR-EHR-CLUSTER.corneal_thickness_details.v0` | Central corneal thickness details | INITIAL | 2015-06-12T10:00:57+02:00 | 0.0.1-alpha |
| 1013.1.5483 | `openEHR-EHR-CLUSTER.coronary_anatomy.v0` | Coronary anatomy | INITIAL | 2021-06-18T03:23:33+02:00 | 0.0.1-alpha |
| 1013.1.5484 | `openEHR-EHR-CLUSTER.coronary_artery_stenosis.v0` | Coronary artery stenosis | INITIAL | 2021-06-18T03:38:00+02:00 | 0.0.1-alpha |
| 1013.1.114 | `openEHR-EHR-CLUSTER.cranial_nerves.v1` | Cranial Nerves | REJECTED | 2015-06-24T09:24:24+02:00 |  |
| 1013.1.4433 | `openEHR-EHR-CLUSTER.crowding.v0` | Household crowding | DRAFT | 2023-05-15T11:51:26+02:00 | 0.0.1-alpha |
| 1013.1.5151 | `openEHR-EHR-CLUSTER.ctcae.v1` | Common Terminology Criteria for Adverse Events (CTCAE) | PUBLISHED | 2024-11-26T07:35:07+01:00 | 1.0.3 |
| 1013.1.1861 | `openEHR-EHR-CLUSTER.death_details_parent.v1` | Death details (PARENT) | INITIAL | 2015-02-17T17:19:47+01:00 |  |
| 1013.1.6956 | `openEHR-EHR-CLUSTER.deep_tendon_reflex.v0` | Deep tendon reflex response | INITIAL | 2023-07-20T09:19:26+02:00 | 0.0.1-alpha |
| 1013.1.6363 | `openEHR-EHR-CLUSTER.delay_details.v0` | Delay details | DRAFT | 2022-08-01T07:58:13+02:00 | 0.0.1-alpha |
| 1013.1.183 | `openEHR-EHR-CLUSTER.delivery_of_infant.v1` | Delivery of infant | REJECTED | 2014-03-04T06:12:41+01:00 |  |
| 1013.1.5765 | `openEHR-EHR-CLUSTER.deporting_country.v0` | Deporting country | INITIAL | 2021-09-24T08:04:42+02:00 | 0.0.1-alpha |
| 1013.1.1867 | `openEHR-EHR-CLUSTER.dermatology_therapy_summary_detail.v1` | Dermatology  therapy summary details  | INITIAL | 2015-02-18T21:09:18+01:00 |  |
| 1013.1.17 | `openEHR-EHR-CLUSTER.device.v1` | Medical device | PUBLISHED | 2025-07-28T14:48:42+02:00 | 1.1.6 |
| 1013.1.844 | `openEHR-EHR-CLUSTER.device_details.v0` | Medical device details | DRAFT | 2026-02-19T15:48:04+01:00 | 0.0.1-alpha |
| 1013.1.2526 | `openEHR-EHR-CLUSTER.diabetic_retinopathy_classification.v0` | Classification of Diabetic Retinopathy | INITIAL | 2016-07-28T00:19:18+02:00 | 0.0.1-alpha |
| 1013.1.1691 | `openEHR-EHR-CLUSTER.diabetic_retinopathy_screening_result.v1` | Classification of diabetic retinopathy during its screening | INITIAL | 2014-05-26T14:28:22+02:00 |  |
| 1013.1.1692 | `openEHR-EHR-CLUSTER.diagnostic_criteria_dr.v0` | Diagnostic criteria DR | INITIAL | 2016-07-25T12:17:44+02:00 | 0.0.1-alpha |
| 1013.1.2745 | `openEHR-EHR-CLUSTER.dietary_nutrients.v0` | Dietary nutrients | DRAFT | 2017-05-19T09:11:52+02:00 | 0.0.1-alpha |
| 1013.1.2824 | `openEHR-EHR-CLUSTER.dietary_phytochemicals.v0` | Dietary phytochemicals | DRAFT | 2017-05-19T09:24:36+02:00 | 0.0.1-alpha |
| 1013.1.16 | `openEHR-EHR-CLUSTER.dimensions.v1` | Dimensions | REJECTED | 2017-05-11T09:21:33+02:00 |  |
| 1013.1.8265 | `openEHR-EHR-CLUSTER.disease_stage.v0` | Disease stage | INITIAL | 2026-10-01T10:25:08+02:00 | 0.0.1-alpha |
| 1013.1.1591 | `openEHR-EHR-CLUSTER.distribution.v0` | Distribution | DRAFT | 2019-09-24T04:59:55+02:00 | 0.0.1-alpha |
| 1013.1.6191 | `openEHR-EHR-CLUSTER.dob_alternative.v0` | Date of birth alternative | DRAFT | 2022-04-14T09:17:55+02:00 | 0.0.1-alpha |
| 1013.1.1678 | `openEHR-EHR-CLUSTER.document_entry_metadata.v0` | Document Entry Metadata | DRAFT | 2018-04-11T07:22:37+02:00 | 0.0.1-alpha |
| 1013.1.6310 | `openEHR-EHR-CLUSTER.dod_alternative.v0` | Date of death alternative | DRAFT | 2022-06-20T15:28:19+02:00 | 0.0.1-alpha |
| 1013.1.2751 | `openEHR-EHR-CLUSTER.dosage.v1` | Dosage | DEPRECATED | 2021-12-24T12:44:08+01:00 | 1.0.6 |
| 1013.1.5948 | `openEHR-EHR-CLUSTER.dosage.v2` | Dosage | PUBLISHED | 2025-05-22T00:56:50+02:00 | 2.0.3 |
| 1013.1.6617 | `openEHR-EHR-CLUSTER.drug_resistance_profile.v0` | Drug resistance profile | INITIAL | 2023-01-18T03:52:18+01:00 | 0.0.1-alpha |
| 1013.1.3285 | `openEHR-EHR-CLUSTER.dwelling.v0` | Dwelling | REVIEWSUSPENDED | 2022-11-28T13:35:01+01:00 | 0.0.1-alpha |
| 1013.1.1669 | `openEHR-EHR-CLUSTER.ear_cleaning.v0` | Ear Cleaning Details | DRAFT | 2019-03-13T08:09:21+01:00 | 0.0.1-alpha |
| 1013.1.8076 | `openEHR-EHR-CLUSTER.eau_nmibc_2021.v1` | EAU NMIBC risk assessment (2021) | PUBLISHED | 2026-02-12T15:08:03+01:00 | 1.0.0 |
| 1013.1.3718 | `openEHR-EHR-CLUSTER.education_record.v1` | Education record | PUBLISHED | 2025-05-21T06:05:42+02:00 | 1.0.2 |
| 1013.1.5356 | `openEHR-EHR-CLUSTER.electronic_communication.v1` | Electronic communication | PUBLISHED | 2026-07-07T17:06:08+02:00 | 1.0.4 |
| 1013.1.2592 | `openEHR-EHR-CLUSTER.electrophysiology_experiment.v0` | Electrophysiology | INITIAL | 2016-09-01T16:18:57+02:00 | 0.0.1-alpha |
| 1013.1.6042 | `openEHR-EHR-CLUSTER.embryo_specimen.v1` | Embryo specimen | PUBLISHED | 2023-11-23T11:43:48+01:00 | 1.0.0 |
| 1013.1.4500 | `openEHR-EHR-CLUSTER.employment_covid.v0` | Healthcare worker | INITIAL | 2021-02-10T04:55:16+01:00 | 0.0.1-alpha |
| 1013.1.2046 | `openEHR-EHR-CLUSTER.empower_mood.v0` | Mood Level (EMPOWER) | INITIAL | 2015-06-19T09:47:44+02:00 | 0.0.1-alpha |
| 1013.1.2047 | `openEHR-EHR-CLUSTER.empower_stress.v0` | Stress Level (EMPOWER) | INITIAL | 2015-06-19T09:47:47+02:00 | 0.0.1-alpha |
| 1013.1.6366 | `openEHR-EHR-CLUSTER.encounter_details.v0` | Encounter details | REVIEWSUSPENDED | 2026-10-19T09:39:43+02:00 | 0.0.1-alpha |
| 1013.1.5408 | `openEHR-EHR-CLUSTER.endotracheal_tube.v0` | Endotracheal tube (ETT) | INITIAL | 2021-05-05T08:42:28+02:00 | 0.0.1-alpha |
| 1013.1.165 | `openEHR-EHR-CLUSTER.environmental_conditions.v0` | Environmental conditions | DRAFT | 2024-02-05T13:25:45+01:00 | 0.0.1-alpha |
| 1013.1.219 | `openEHR-EHR-CLUSTER.exam-abdomen.v0` | Examination of the abdomen | DRAFT | 2024-11-19T12:48:59+01:00 | 0.0.1-alpha |
| 1013.1.5268 | `openEHR-EHR-CLUSTER.exam-anterior_chamber_eye.v0` | Examination of the anterior chamber of an eye | DRAFT | 2024-11-01T15:30:25+01:00 | 0.0.1-alpha |
| 1013.1.3910 | `openEHR-EHR-CLUSTER.exam-anus.v0` | Examination of the anus | DRAFT | 2024-11-01T15:57:34+01:00 | 0.0.1-alpha |
| 1013.1.3924 | `openEHR-EHR-CLUSTER.exam-aqueous_humour.v0` | Examination of the aqueous humour | DRAFT | 2024-11-01T17:06:57+01:00 | 0.0.1-alpha |
| 1013.1.3879 | `openEHR-EHR-CLUSTER.exam-auscultation-bowel_sounds.v0` | Auscultation of bowel sounds | REJECTED | 2025-05-27T02:29:23+02:00 | 0.0.1 |
| 1013.1.3880 | `openEHR-EHR-CLUSTER.exam-auscultation-breath_sounds.v0` | Auscultation of breath sounds | REJECTED | 2025-05-15T00:59:18+02:00 | 0.0.1 |
| 1013.1.3878 | `openEHR-EHR-CLUSTER.exam-auscultation.v0` | Auscultation findings | REJECTED | 2025-05-27T02:32:27+02:00 | 0.0.1 |
| 1013.1.220 | `openEHR-EHR-CLUSTER.exam-bone.v1` | Examination of bone | REJECTED | 2015-02-05T06:38:20+01:00 |  |
| 1013.1.3888 | `openEHR-EHR-CLUSTER.exam-breast.v0` | Examination of a breast | DRAFT | 2026-02-10T10:43:41+01:00 | 0.0.1-alpha |
| 1013.1.3901 | `openEHR-EHR-CLUSTER.exam-breasts.v0` | Examination of both breasts | DRAFT | 2024-10-22T16:27:15+02:00 | 0.0.1-alpha |
| 1013.1.3977 | `openEHR-EHR-CLUSTER.exam-cardiovascular_system.v0` | Examination of the cardiovascular system | DRAFT | 2024-11-12T10:05:01+01:00 | 0.0.1-alpha |
| 1013.1.221 | `openEHR-EHR-CLUSTER.exam-chest.v0` | Examination of the chest | DRAFT | 2024-11-12T14:40:54+01:00 | 0.0.1-alpha |
| 1013.1.5271 | `openEHR-EHR-CLUSTER.exam-conjunctiva.v0` | Examination of the conjunctiva | DRAFT | 2024-11-22T10:23:09+01:00 | 0.0.1-alpha |
| 1013.1.4354 | `openEHR-EHR-CLUSTER.exam-cornea.v0` | Examination of the cornea | DRAFT | 2024-11-22T10:34:29+01:00 | 0.0.1-alpha |
| 1013.1.3884 | `openEHR-EHR-CLUSTER.exam-cranial_nerves.v0` | Examination of cranial nerves | DRAFT | 2024-11-13T09:18:01+01:00 | 0.0.1-alpha |
| 1013.1.3911 | `openEHR-EHR-CLUSTER.exam-ear.v0` | Examination of an ear | DRAFT | 2024-10-21T15:54:04+02:00 | 0.0.1-alpha |
| 1013.1.222 | `openEHR-EHR-CLUSTER.exam-ears.v0` | Examination of both ears | DRAFT | 2024-10-25T10:13:51+02:00 | 0.0.1-alpha |
| 1013.1.3914 | `openEHR-EHR-CLUSTER.exam-external_auditory_canal.v0` | Examination of an external auditory canal | DRAFT | 2024-10-21T16:02:22+02:00 | 0.0.1-alpha |
| 1013.1.3786 | `openEHR-EHR-CLUSTER.exam-eye.v0` | Examination of an eye | DRAFT | 2024-10-21T16:18:06+02:00 | 0.0.1-alpha |
| 1013.1.4353 | `openEHR-EHR-CLUSTER.exam-eyelid.v0` | Examination of an eyelid | DRAFT | 2024-10-21T16:43:03+02:00 | 0.0.1-alpha |
| 1013.1.3777 | `openEHR-EHR-CLUSTER.exam-eyes.v0` | Examination of both eyes | DRAFT | 2024-10-25T10:57:32+02:00 | 0.0.1-alpha |
| 1013.1.223 | `openEHR-EHR-CLUSTER.exam-face.v0` | Examination of the face | DRAFT | 2024-11-12T15:03:39+01:00 | 0.0.1-alpha |
| 1013.1.277 | `openEHR-EHR-CLUSTER.exam-fetus.v1` | Examination of the fetus | REJECTED | 2015-02-05T06:39:24+01:00 |  |
| 1013.1.5326 | `openEHR-EHR-CLUSTER.exam-finger.v0` | Examination of a finger | DRAFT | 2024-10-19T19:11:44+02:00 | 0.0.1-alpha |
| 1013.1.5324 | `openEHR-EHR-CLUSTER.exam-fingernail.v0` | Examination of a finger nail | DRAFT | 2024-10-18T20:25:35+02:00 | 0.0.1-alpha |
| 1013.1.5330 | `openEHR-EHR-CLUSTER.exam-foot.v0` | Examination of a foot | DRAFT | 2024-10-19T21:23:47+02:00 | 0.0.1-alpha |
| 1013.1.3898 | `openEHR-EHR-CLUSTER.exam-fundus_eye.v0` | Examination of the fundus of an eye | DRAFT | 2024-11-22T10:40:49+01:00 | 0.0.1-alpha |
| 1013.1.225 | `openEHR-EHR-CLUSTER.exam-generic-joint.v1` | Examination of a named joint | REJECTED | 2015-02-06T06:32:53+01:00 |  |
| 1013.1.226 | `openEHR-EHR-CLUSTER.exam-generic-lymphnode.v1` | Examination of lymph nodes | REJECTED | 2015-02-06T06:32:59+01:00 |  |
| 1013.1.224 | `openEHR-EHR-CLUSTER.exam-generic.v1` | Examination of a named body part | REJECTED | 2015-02-05T06:37:57+01:00 |  |
| 1013.1.5329 | `openEHR-EHR-CLUSTER.exam-hand.v0` | Examination of a hand | DRAFT | 2024-10-20T20:41:04+02:00 | 0.0.1-alpha |
| 1013.1.3928 | `openEHR-EHR-CLUSTER.exam-heart.v0` | Examination of the heart | DRAFT | 2024-11-12T14:52:39+01:00 | 0.0.1-alpha |
| 1013.1.6761 | `openEHR-EHR-CLUSTER.exam-hip_joint.v0` | Physical examination of a hip joint | REVIEWSUSPENDED | 2023-11-08T09:48:51+01:00 | 0.0.1-alpha |
| 1013.1.3980 | `openEHR-EHR-CLUSTER.exam-inspection-cervix.v0` | Inspection of the cervix | REJECTED | 2025-05-13T07:14:25+02:00 | 0.0.1 |
| 1013.1.3984 | `openEHR-EHR-CLUSTER.exam-inspection-rectum.v0` | Inspection of the rectum | REJECTED | 2025-07-18T03:35:18+02:00 | 0.0.1 |
| 1013.1.3981 | `openEHR-EHR-CLUSTER.exam-inspection-vagina.v0` | Inspection of the vagina | REJECTED | 2025-07-22T00:06:30+02:00 | 0.0.1 |
| 1013.1.3642 | `openEHR-EHR-CLUSTER.exam-inspection.v0` | Inspection findings | REJECTED | 2025-07-22T00:07:49+02:00 | 0.0.1 |
| 1013.1.7847 | `openEHR-EHR-CLUSTER.exam-inspection_cervix.v0` | Inspection of the cervix | INITIAL | 2025-05-13T07:26:56+02:00 | 0.0.1-alpha |
| 1013.1.5279 | `openEHR-EHR-CLUSTER.exam-iris.v0` | Examination of an iris | DRAFT | 2024-10-22T16:13:55+02:00 | 0.0.1-alpha |
| 1013.1.3899 | `openEHR-EHR-CLUSTER.exam-lens.v0` | Examination of the lens | DRAFT | 2024-11-22T11:05:49+01:00 | 0.0.1-alpha |
| 1013.1.5331 | `openEHR-EHR-CLUSTER.exam-lower_limb.v0` | Examination of a lower limb | DRAFT | 2024-10-20T20:51:23+02:00 | 0.0.1-alpha |
| 1013.1.3872 | `openEHR-EHR-CLUSTER.exam-lung.v0` | Examination of a lung | DRAFT | 2024-10-20T21:07:20+02:00 | 0.0.1-alpha |
| 1013.1.3919 | `openEHR-EHR-CLUSTER.exam-middle_ear.v0` | Examination of a middle ear | DRAFT | 2024-10-20T21:34:29+02:00 | 0.0.1-alpha |
| 1013.1.227 | `openEHR-EHR-CLUSTER.exam-mouth.v0` | Examination of the mouth | DRAFT | 2024-11-12T15:10:31+01:00 | 0.0.1-alpha |
| 1013.1.3881 | `openEHR-EHR-CLUSTER.exam-muscle_group.v0` | Examination findings of a muscle group | DRAFT | 2025-04-22T13:23:09+02:00 | 0.0.1-alpha |
| 1013.1.3929 | `openEHR-EHR-CLUSTER.exam-neck.v0` | Examination of the neck | DRAFT | 2024-11-12T15:18:58+01:00 | 0.0.1-alpha |
| 1013.1.3972 | `openEHR-EHR-CLUSTER.exam-nervous_system.v0` | Examination of the nervous system | DRAFT | 2024-11-22T10:48:15+01:00 | 0.0.1-alpha |
| 1013.1.228 | `openEHR-EHR-CLUSTER.exam-nose.v1` | Examination of nose | REJECTED | 2015-02-05T06:38:36+01:00 |  |
| 1013.1.5269 | `openEHR-EHR-CLUSTER.exam-optic_disc.v0` | Examination of the optic disc | DRAFT | 2024-11-22T11:13:32+01:00 | 0.0.1-alpha |
| 1013.1.3987 | `openEHR-EHR-CLUSTER.exam-palpation-cervix.v0` | Palpation of the cervix | REJECTED | 2025-05-13T07:56:06+02:00 | 0.0.1 |
| 1013.1.3990 | `openEHR-EHR-CLUSTER.exam-palpation-rectum.v0` | Palpation of the rectum | REJECTED | 2025-07-18T03:36:30+02:00 | 0.0.1 |
| 1013.1.3996 | `openEHR-EHR-CLUSTER.exam-palpation-uterus.v0` | Palpation of the uterus | REJECTED | 2025-07-09T23:22:45+02:00 | 0.0.1 |
| 1013.1.3998 | `openEHR-EHR-CLUSTER.exam-palpation-vagina.v0` | Palpation of the vagina | REJECTED | 2025-07-22T00:05:42+02:00 | 0.0.1 |
| 1013.1.3643 | `openEHR-EHR-CLUSTER.exam-palpation.v0` | Palpation findings | REJECTED | 2025-07-22T00:08:32+02:00 | 0.0.1 |
| 1013.1.7848 | `openEHR-EHR-CLUSTER.exam-palpation_cervix.v0` | Palpation of the cervix | INITIAL | 2025-05-13T07:53:14+02:00 | 0.0.1-alpha |
| 1013.1.3932 | `openEHR-EHR-CLUSTER.exam-penis.v0` | Examination of the penis | DRAFT | 2024-11-12T15:29:41+01:00 | 0.0.1-alpha |
| 1013.1.3973 | `openEHR-EHR-CLUSTER.exam-peripheral_nervous_system.v0` | Examination of the peripheral nervous system | DRAFT | 2024-11-22T15:49:58+01:00 | 0.0.1-alpha |
| 1013.1.6189 | `openEHR-EHR-CLUSTER.exam-placenta.v0` | Examination of a placenta | DRAFT | 2022-05-11T12:43:49+02:00 | 0.0.1-alpha |
| 1013.1.3964 | `openEHR-EHR-CLUSTER.exam-posterior_chamber_eye.v0` | Examination of the posterior chamber of an eye | DRAFT | 2024-11-22T15:21:07+01:00 | 0.0.1-alpha |
| 1013.1.3992 | `openEHR-EHR-CLUSTER.exam-prostate.v0` | Palpation of the prostate | DRAFT | 2025-07-18T06:01:52+02:00 | 0.0.1-alpha |
| 1013.1.3882 | `openEHR-EHR-CLUSTER.exam-pupil.v0` | Examination of a pupil | DRAFT | 2024-10-21T13:15:39+02:00 | 0.0.1-alpha |
| 1013.1.7955 | `openEHR-EHR-CLUSTER.exam-rectum.v0` | Examination of the rectum | INITIAL | 2025-07-15T00:36:48+02:00 | 0.0.1-alpha |
| 1013.1.3976 | `openEHR-EHR-CLUSTER.exam-respiratory_system.v0` | Examination of the respiratory system | DRAFT | 2024-11-22T15:46:29+01:00 | 0.0.1-alpha |
| 1013.1.5270 | `openEHR-EHR-CLUSTER.exam-sclera.v0` | Examination of the sclera | DRAFT | 2024-11-22T15:29:59+01:00 | 0.0.1-alpha |
| 1013.1.3955 | `openEHR-EHR-CLUSTER.exam-scrotum.v0` | Examination of the scrotum | DRAFT | 2024-11-12T15:35:17+01:00 | 0.0.1-alpha |
| 1013.1.3933 | `openEHR-EHR-CLUSTER.exam-skin.v0` | Examination of the skin | DRAFT | 2024-11-22T15:37:32+01:00 | 0.0.1-alpha |
| 1013.1.3957 | `openEHR-EHR-CLUSTER.exam-testicle.v0` | Examination of a testicle | DRAFT | 2024-10-21T13:33:39+02:00 | 0.0.1-alpha |
| 1013.1.3960 | `openEHR-EHR-CLUSTER.exam-throat.v0` | Examination of the throat | DRAFT | 2024-11-12T15:48:30+01:00 | 0.0.1-alpha |
| 1013.1.229 | `openEHR-EHR-CLUSTER.exam-thyroid.v0` | Examination of the thyroid | DRAFT | 2024-11-22T15:41:53+01:00 | 0.0.1-alpha |
| 1013.1.5328 | `openEHR-EHR-CLUSTER.exam-toe.v0` | Examination of a toe | DRAFT | 2024-10-19T19:41:43+02:00 | 0.0.1-alpha |
| 1013.1.5325 | `openEHR-EHR-CLUSTER.exam-toenail.v0` | Examination of a toe nail | DRAFT | 2024-10-19T19:47:22+02:00 | 0.0.1-alpha |
| 1013.1.3902 | `openEHR-EHR-CLUSTER.exam-tongue.v0` | Examination of the tongue | DRAFT | 2024-11-12T15:43:35+01:00 | 0.0.1-alpha |
| 1013.1.3799 | `openEHR-EHR-CLUSTER.exam-tooth.v0` | Examination of a tooth | DRAFT | 2024-10-21T13:58:55+02:00 | 0.0.1-alpha |
| 1013.1.3913 | `openEHR-EHR-CLUSTER.exam-tympanic_membrane.v0` | Examination of a tympanic membrane | DRAFT | 2024-10-21T14:16:53+02:00 | 0.0.1-alpha |
| 1013.1.5332 | `openEHR-EHR-CLUSTER.exam-upper_limb.v0` | Examination of an upper limb | DRAFT | 2024-10-22T16:10:22+02:00 | 0.0.1-alpha |
| 1013.1.217 | `openEHR-EHR-CLUSTER.exam-uterine_cervix.v1` | Examination of the cervix | REJECTED | 2015-02-05T06:38:52+01:00 |  |
| 1013.1.7948 | `openEHR-EHR-CLUSTER.exam-uterus.v0` | Examination of the uterus | INITIAL | 2025-07-10T00:49:44+02:00 | 0.0.1-alpha |
| 1013.1.7974 | `openEHR-EHR-CLUSTER.exam-vagina.v0` | Examination of the vagina | INITIAL | 2025-07-22T05:37:44+02:00 | 0.0.1-alpha |
| 1013.1.3959 | `openEHR-EHR-CLUSTER.exam-vulva.v0` | Examination of the vulva | DRAFT | 2024-10-19T21:44:39+02:00 | 0.0.1-alpha |
| 1013.1.218 | `openEHR-EHR-CLUSTER.exam.v1` | Examination findings | DEPRECATED | 2025-07-22T00:10:17+02:00 | 1.0.10 |
| 1013.1.6153 | `openEHR-EHR-CLUSTER.exam.v2` | Physical examination findings | PUBLISHED | 2025-07-21T15:45:58+02:00 | 2.1.3 |
| 1013.1.2061 | `openEHR-EHR-CLUSTER.exam_abdomen.v0` | Examination of abdomen | REJECTED | 2019-07-20T09:08:30+02:00 | 0.0.1 |
| 1013.1.1404 | `openEHR-EHR-CLUSTER.exam_anterior_chamber.v1` | Examination Findings  -  Anterior Chamber | INITIAL | 2013-03-06T11:40:23+01:00 |  |
| 1013.1.2183 | `openEHR-EHR-CLUSTER.exam_anus.v0` | Examination of anus | REJECTED | 2019-07-21T08:55:00+02:00 | 0.0.1 |
| 1013.1.3186 | `openEHR-EHR-CLUSTER.exam_auditory_canal.v0` | Examination of external auditory canal | REJECTED | 2019-07-23T09:16:34+02:00 | 0.0.1 |
| 1013.1.5827 | `openEHR-EHR-CLUSTER.exam_blastocyst.v1` | Examination of a blastocyst | PUBLISHED | 2023-05-30T12:59:21+02:00 | 1.0.1 |
| 1013.1.2088 | `openEHR-EHR-CLUSTER.exam_breast.v0` | Examination of a breast | REJECTED | 2019-07-15T13:16:06+02:00 | 0.0.1 |
| 1013.1.2087 | `openEHR-EHR-CLUSTER.exam_breasts.v0` | Examination of both breasts | REJECTED | 2019-07-21T07:24:46+02:00 | 0.0.1 |
| 1013.1.2111 | `openEHR-EHR-CLUSTER.exam_burn.v0` | Examination of a burn | DRAFT | 2015-06-25T06:17:15+02:00 | 0.0.1-alpha |
| 1013.1.2065 | `openEHR-EHR-CLUSTER.exam_chest.v0` | Examination of chest | REJECTED | 2019-07-21T08:47:58+02:00 | 0.0.1 |
| 1013.1.2094 | `openEHR-EHR-CLUSTER.exam_cranial_nerves.v0` | Examination of cranial nerves | REJECTED | 2019-07-17T10:59:14+02:00 | 0.0.1 |
| 1013.1.3825 | `openEHR-EHR-CLUSTER.exam_dentition.v0` | Examination of dentition | INITIAL | 2019-06-11T10:59:46+02:00 | 0.0.1-alpha |
| 1013.1.2874 | `openEHR-EHR-CLUSTER.exam_ears.v0` | Examination of both ears | REJECTED | 2019-07-22T13:27:18+02:00 | 0.0.1 |
| 1013.1.5828 | `openEHR-EHR-CLUSTER.exam_embryo.v1` | Examination of a cleavage-stage embryo | PUBLISHED | 2023-05-31T09:21:30+02:00 | 1.0.1 |
| 1013.1.3478 | `openEHR-EHR-CLUSTER.exam_eom.v0` | Examination of extraocular muscles | REJECTED | 2019-07-26T09:54:31+02:00 | 0.0.2 |
| 1013.1.2171 | `openEHR-EHR-CLUSTER.exam_eye.v0` | Examination of an eye | REJECTED | 2019-07-15T13:19:37+02:00 | 0.0.1 |
| 1013.1.2164 | `openEHR-EHR-CLUSTER.exam_eye_anterior.v0` | Examination of anterior chamber | REJECTED | 2019-07-26T07:16:09+02:00 | 0.0.1 |
| 1013.1.2165 | `openEHR-EHR-CLUSTER.exam_eye_aqueous.v0` | Examination of aqueous humour | REJECTED | 2019-07-24T05:02:25+02:00 | 0.0.1 |
| 1013.1.2166 | `openEHR-EHR-CLUSTER.exam_eye_lens.v0` | Examination of lens | REJECTED | 2019-07-20T09:39:34+02:00 | 0.0.1 |
| 1013.1.2167 | `openEHR-EHR-CLUSTER.exam_eye_posterior.v0` | Examination of posterior chamber | REJECTED | 2019-07-26T07:16:46+02:00 | 0.0.1 |
| 1013.1.2168 | `openEHR-EHR-CLUSTER.exam_eye_pupil.v0` | Examination of a pupil | REJECTED | 2019-07-20T09:07:25+02:00 | 0.0.1 |
| 1013.1.2170 | `openEHR-EHR-CLUSTER.exam_eye_vitreous.v0` | Examination of vitreous humour | REJECTED | 2019-07-26T07:12:55+02:00 | 0.0.1 |
| 1013.1.2086 | `openEHR-EHR-CLUSTER.exam_eyes.v0` | Examination of both eyes | REJECTED | 2019-07-20T09:51:48+02:00 | 0.0.1 |
| 1013.1.3475 | `openEHR-EHR-CLUSTER.exam_face.v0` | Examination of face | REJECTED | 2019-07-15T09:20:45+02:00 | 0.0.2 |
| 1013.1.3543 | `openEHR-EHR-CLUSTER.exam_faeces.v0` | Examination of faeces | DRAFT | 2018-11-16T06:57:10+01:00 | 0.0.2-alpha |
| 1013.1.2338 | `openEHR-EHR-CLUSTER.exam_gait.v0` | Examination of gait | REJECTED | 2019-07-26T12:01:25+02:00 | 0.0.1 |
| 1013.1.1377 | `openEHR-EHR-CLUSTER.exam_generic.v1` | Examination Findings - Generic | REJECTED | 2015-06-23T16:02:07+02:00 |  |
| 1013.1.3827 | `openEHR-EHR-CLUSTER.exam_giginvae.v0` | Examination of giginvae | INITIAL | 2019-06-11T11:06:57+02:00 | 0.0.1-alpha |
| 1013.1.2064 | `openEHR-EHR-CLUSTER.exam_heart.v0` | Examination of heart | REJECTED | 2019-07-24T11:54:38+02:00 | 0.0.1 |
| 1013.1.207 | `openEHR-EHR-CLUSTER.exam_hydration.v0` | Hydration | DRAFT | 2019-09-24T05:27:53+02:00 | 0.0.1-alpha |
| 1013.1.1403 | `openEHR-EHR-CLUSTER.exam_lens.v1` | Examination Findings  -  Lens | INITIAL | 2013-03-06T10:56:43+01:00 |  |
| 1013.1.2109 | `openEHR-EHR-CLUSTER.exam_lesion.v0` | Examination of a lesion | REVIEWSUSPENDED | 2022-01-12T09:59:10+01:00 | 0.0.1-alpha |
| 1013.1.2184 | `openEHR-EHR-CLUSTER.exam_lip.v0` | Examination of a lip | REJECTED | 2019-07-15T13:15:16+02:00 | 0.0.1 |
| 1013.1.2185 | `openEHR-EHR-CLUSTER.exam_lips.v0` | Examination of both lips | REJECTED | 2019-07-24T04:41:55+02:00 | 0.0.1 |
| 1013.1.2063 | `openEHR-EHR-CLUSTER.exam_lung.v0` | Examination of a lung | REJECTED | 2019-07-15T13:17:17+02:00 | 0.0.1 |
| 1013.1.2149 | `openEHR-EHR-CLUSTER.exam_lungs.v0` | Examination of both lungs | REJECTED | 2019-07-24T04:41:22+02:00 | 0.0.1 |
| 1013.1.3188 | `openEHR-EHR-CLUSTER.exam_middle_ear.v0` | Examination of middle ear | REJECTED | 2019-07-24T03:37:51+02:00 | 0.0.1 |
| 1013.1.2186 | `openEHR-EHR-CLUSTER.exam_mouth.v0` | Examination of mouth | REJECTED | 2019-07-15T11:34:58+02:00 | 0.0.1 |
| 1013.1.3476 | `openEHR-EHR-CLUSTER.exam_muscle.v0` | Examination of a muscle or muscle group | REJECTED | 2019-07-20T09:06:29+02:00 | 0.0.2 |
| 1013.1.3425 | `openEHR-EHR-CLUSTER.exam_neck.v0` | Examination of neck | REJECTED | 2019-07-24T12:02:59+02:00 | 0.0.2 |
| 1013.1.3477 | `openEHR-EHR-CLUSTER.exam_nerve_sensation.v0` | Examination of nerve sensation | REJECTED | 2019-07-26T09:47:44+02:00 | 0.0.2 |
| 1013.1.2093 | `openEHR-EHR-CLUSTER.exam_nervous_system.v0` | Examination of nervous system | REJECTED | 2019-07-26T12:44:28+02:00 | 0.0.1 |
| 1013.1.3828 | `openEHR-EHR-CLUSTER.exam_occlusal.v0` | Examination of occlusal | INITIAL | 2019-06-11T11:09:33+02:00 | 0.0.1-alpha |
| 1013.1.5829 | `openEHR-EHR-CLUSTER.exam_oocyte.v1` | Examination of an oocyte | PUBLISHED | 2023-05-31T09:30:59+02:00 | 1.0.1 |
| 1013.1.3829 | `openEHR-EHR-CLUSTER.exam_oral_membrane.v0` | Examination of oral membrane | INITIAL | 2019-06-11T11:11:21+02:00 | 0.0.1-alpha |
| 1013.1.2017 | `openEHR-EHR-CLUSTER.exam_pattern.v0` | Examination pattern | REJECTED | 2019-07-15T13:20:45+02:00 | 0.0.2 |
| 1013.1.2101 | `openEHR-EHR-CLUSTER.exam_penis.v0` | Examination of penis | REJECTED | 2019-07-24T12:27:20+02:00 | 0.0.1 |
| 1013.1.2558 | `openEHR-EHR-CLUSTER.exam_perianal.v1` | Examination of perianal area | REJECTED | 2016-09-15T11:07:46+02:00 | 0.0.1 |
| 1013.1.3190 | `openEHR-EHR-CLUSTER.exam_pinna.v0` | Examination of pinna | REJECTED | 2019-07-24T05:03:29+02:00 | 0.0.1 |
| 1013.1.1610 | `openEHR-EHR-CLUSTER.exam_posterior_chamber.v1` | Examination Findings – Posterior Chamber of eye | INITIAL | 2014-05-27T16:40:50+02:00 |  |
| 1013.1.1397 | `openEHR-EHR-CLUSTER.exam_pupils.v1` | Examination Findings  - Pupils | REJECTED | 2016-02-29T02:25:07+01:00 |  |
| 1013.1.2169 | `openEHR-EHR-CLUSTER.exam_retina.v0` | Examination of retina | REJECTED | 2019-07-20T09:38:55+02:00 | 0.0.1 |
| 1013.1.2102 | `openEHR-EHR-CLUSTER.exam_scrotum.v0` | Examination of scrotum | REJECTED | 2019-07-25T10:12:13+02:00 | 0.0.1 |
| 1013.1.2096 | `openEHR-EHR-CLUSTER.exam_skin.v0` | Examination of skin | REJECTED | 2019-07-24T12:57:20+02:00 | 0.0.1 |
| 1013.1.245 | `openEHR-EHR-CLUSTER.exam_smell.v0` | Examination of smell | REJECTED | 2019-07-17T08:31:31+02:00 | 0.0.1 |
| 1013.1.3473 | `openEHR-EHR-CLUSTER.exam_taste.v0` | Examination of taste | REJECTED | 2019-07-26T07:14:14+02:00 | 0.0.2 |
| 1013.1.3831 | `openEHR-EHR-CLUSTER.exam_teeth.v0` | Examination of teeth | INITIAL | 2019-06-12T03:46:33+02:00 | 0.0.1-alpha |
| 1013.1.3599 | `openEHR-EHR-CLUSTER.exam_tendon_reflexes.v0` | Examination of deep tendon reflexes | DRAFT | 2019-01-31T08:08:23+01:00 | 0.0.2-alpha |
| 1013.1.3427 | `openEHR-EHR-CLUSTER.exam_throat.v0` | Examination of throat | REJECTED | 2019-07-25T10:54:21+02:00 | 0.0.2 |
| 1013.1.3426 | `openEHR-EHR-CLUSTER.exam_thyroid.v0` | Examination of thyroid | REJECTED | 2019-07-24T12:21:50+02:00 | 0.0.2 |
| 1013.1.2068 | `openEHR-EHR-CLUSTER.exam_tongue.v0` | Examination of tongue | REJECTED | 2019-07-21T08:48:52+02:00 | 0.0.1 |
| 1013.1.3189 | `openEHR-EHR-CLUSTER.exam_tympanic_membrane.v0` | Examination of tympanic membrane | REJECTED | 2019-07-23T08:53:45+02:00 | 0.0.1 |
| 1013.1.2097 | `openEHR-EHR-CLUSTER.exam_urethral_meatus.v0` | Examination of urethral meatus | REJECTED | 2019-07-24T12:33:21+02:00 | 0.0.1 |
| 1013.1.2098 | `openEHR-EHR-CLUSTER.exam_vagina.v1` | Examination of the vagina | REJECTED | 2015-06-25T09:49:07+02:00 | 0.0.1 |
| 1013.1.2099 | `openEHR-EHR-CLUSTER.exam_vulva.v0` | Examination of vulva | REJECTED | 2019-07-25T10:48:18+02:00 | 0.0.1 |
| 1013.1.3688 | `openEHR-EHR-CLUSTER.exam_wound.v0` | Examination of a wound | DRAFT | 2021-06-25T09:33:57+02:00 | 0.0.1-alpha |
| 1013.1.5830 | `openEHR-EHR-CLUSTER.exam_zygote.v1` | Examination of a zygote | PUBLISHED | 2023-05-31T09:26:34+02:00 | 1.0.1 |
| 1013.1.1846 | `openEHR-EHR-CLUSTER.exclusion_exam.v1` | Exclusion of examination | PUBLISHED | 2025-06-26T00:51:59+02:00 | 1.1.2 |
| 1013.1.2120 | `openEHR-EHR-CLUSTER.exclusion_symptom_sign.v0` | Exclusion of a symptom or sign | DRAFT | 2016-01-13T08:21:43+01:00 | 0.0.1-alpha |
| 1013.1.6420 | `openEHR-EHR-CLUSTER.exclusion_test.v0` | Exclusion of test | INITIAL | 2022-07-25T14:08:27+02:00 | 0.0.1-alpha |
| 1013.1.1975 | `openEHR-EHR-CLUSTER.family_prevalence.v1` | Family prevalence | PUBLISHED | 2026-06-05T10:51:46+02:00 | 1.1.0 |
| 1013.1.4003 | `openEHR-EHR-CLUSTER.fetus_abdominal.v0` | Palpation of a fetus (per abdomen) | DRAFT | 2019-07-31T09:26:57+02:00 | 0.0.1-alpha |
| 1013.1.4001 | `openEHR-EHR-CLUSTER.fetus_vaginal.v0` | Palpation of a fetus (per vagina) | DRAFT | 2019-07-31T09:25:54+02:00 | 0.0.1-alpha |
| 1013.1.7610 | `openEHR-EHR-CLUSTER.figo_staging_cancer.v1` | FIGO staging of gynaecological cancer | PUBLISHED | 2025-05-15T14:19:53+02:00 | 1.0.0 |
| 1013.1.5042 | `openEHR-EHR-CLUSTER.financial_record.v0` | Financial record | DRAFT | 2020-11-11T04:07:30+01:00 | 0.0.1-alpha |
| 1013.1.2162 | `openEHR-EHR-CLUSTER.findings_glaucoma.v0` | Findings in glaucoma | INITIAL | 2021-02-10T06:11:34+01:00 | 0.0.1-alpha |
| 1013.1.23 | `openEHR-EHR-CLUSTER.fluid.v1` | Body fluid or substance | REJECTED | 2015-09-30T09:01:00+02:00 |  |
| 1013.1.7038 | `openEHR-EHR-CLUSTER.fnclcc.v1` | FNCLCC grading system | PUBLISHED | 2024-11-18T08:47:11+01:00 | 1.0.0 |
| 1013.1.231 | `openEHR-EHR-CLUSTER.free_text.v0` | Free text | DRAFT | 2021-11-23T14:25:43+01:00 | 0.0.1-alpha |
| 1013.1.24 | `openEHR-EHR-CLUSTER.gait.v0` | Gait | DRAFT | 2024-10-11T15:04:36+02:00 | 0.0.1-alpha |
| 1013.1.2818 | `openEHR-EHR-CLUSTER.gender.v1` | Gender | DEPRECATED | 2019-04-04T09:51:05+02:00 | 1.0.2 |
| 1013.1.4446 | `openEHR-EHR-CLUSTER.genetic_variant_presence.v0` | Genetic variant presence | DRAFT | 2020-12-04T11:13:19+01:00 | 0.0.1-alpha |
| 1013.1.3749 | `openEHR-EHR-CLUSTER.genomic_conversion_variant.v1` | Genomic conversion variant | PUBLISHED | 2021-05-20T09:32:41+02:00 | 1.0.3 |
| 1013.1.3750 | `openEHR-EHR-CLUSTER.genomic_copy_number_variant.v1` | Genomic copy number variant | PUBLISHED | 2022-12-22T10:41:13+01:00 | 1.1.0 |
| 1013.1.3753 | `openEHR-EHR-CLUSTER.genomic_deletion_insertion_variant.v1` | Genomic deletion-insertion variant | PUBLISHED | 2021-05-20T09:34:45+02:00 | 1.0.3 |
| 1013.1.3751 | `openEHR-EHR-CLUSTER.genomic_deletion_variant.v1` | Genomic deletion variant | PUBLISHED | 2026-11-10T16:41:16+01:00 | 1.0.5 |
| 1013.1.3752 | `openEHR-EHR-CLUSTER.genomic_duplication_variant.v1` | Genomic duplication variant | PUBLISHED | 2021-05-20T09:35:32+02:00 | 1.0.3 |
| 1013.1.3754 | `openEHR-EHR-CLUSTER.genomic_insertion_variant.v1` | Genomic insertion variant | PUBLISHED | 2021-05-20T09:36:24+02:00 | 1.0.3 |
| 1013.1.4898 | `openEHR-EHR-CLUSTER.genomic_inversion_variant.v1` | Genomic inversion variant | PUBLISHED | 2021-06-03T15:57:19+02:00 | 1.0.4 |
| 1013.1.3756 | `openEHR-EHR-CLUSTER.genomic_repeated_sequence_variant.v1` | Genomic repeated sequence variant | PUBLISHED | 2021-05-20T09:37:22+02:00 | 1.0.3 |
| 1013.1.3757 | `openEHR-EHR-CLUSTER.genomic_substitution_variant.v1` | Genomic substitution variant | PUBLISHED | 2022-12-22T10:42:58+01:00 | 1.0.4 |
| 1013.1.3759 | `openEHR-EHR-CLUSTER.genomic_variant_result.v1` | Genomic variant result | PUBLISHED | 2026-06-25T14:41:39+02:00 | 1.1.1 |
| 1013.1.5742 | `openEHR-EHR-CLUSTER.geolocation.v0` | Geolocation | INITIAL | 2021-09-13T08:29:31+02:00 | 0.0.1-alpha |
| 1013.1.7266 | `openEHR-EHR-CLUSTER.gist_modified_nih.v1` | Modified NIH Criteria for GIST risk assessment | PUBLISHED | 2025-03-05T12:55:40+01:00 | 1.0.0 |
| 1013.1.6163 | `openEHR-EHR-CLUSTER.gleason_score.v0` | Gleason Score | DRAFT | 2026-02-10T11:28:22+01:00 | 0.0.1-alpha |
| 1013.1.26 | `openEHR-EHR-CLUSTER.health_event-poisoning.v1` | Poisoning or overdose | REJECTED | 2019-09-17T07:04:37+02:00 |  |
| 1013.1.232 | `openEHR-EHR-CLUSTER.health_event-trauma.v0` | Trauma event | REJECTED | 2019-09-17T07:04:07+02:00 | 0.0.1 |
| 1013.1.25 | `openEHR-EHR-CLUSTER.health_event.v0` | Health event | DRAFT | 2022-04-25T20:49:47+02:00 | 0.0.1-alpha |
| 1013.1.1743 | `openEHR-EHR-CLUSTER.healthcare_professional_parent.v1` | Healthcare professional (PARENT) | INITIAL | 2014-07-28T17:05:12+02:00 |  |
| 1013.1.1744 | `openEHR-EHR-CLUSTER.healthcare_provider_parent.v1` | Healthcare provider (PARENT) | INITIAL | 2015-02-21T17:30:50+01:00 |  |
| 1013.1.1748 | `openEHR-EHR-CLUSTER.hip_arthroplasty_component.v0` | Hip arthroplasty component | DRAFT | 2019-09-24T05:03:56+02:00 | 0.0.1-alpha |
| 1013.1.1872 | `openEHR-EHR-CLUSTER.hip_procedure_valdoltra.v1` | Hip procedure registry details (Valdoltra) | INITIAL | 2015-03-02T16:00:38+01:00 |  |
| 1013.1.3182 | `openEHR-EHR-CLUSTER.home_environment.v0` | Home Environment | REJECTED | 2020-02-18T09:45:39+01:00 | 0.0.1 |
| 1013.1.1634 | `openEHR-EHR-CLUSTER.household.v0` | Household | REJECTED | 2018-05-30T07:23:17+02:00 | 0.0.1 |
| 1013.1.1667 | `openEHR-EHR-CLUSTER.housing.v0` | Housing | REJECTED | 2018-05-30T08:22:15+02:00 | 0.0.1 |
| 1013.1.3283 | `openEHR-EHR-CLUSTER.housing_record.v1` | Housing record | PUBLISHED | 2024-05-13T11:58:07+02:00 | 1.0.1 |
| 1013.1.7341 | `openEHR-EHR-CLUSTER.humidification.v0` | Humidification | INITIAL | 2024-06-04T10:33:56+02:00 | 0.0.1-alpha |
| 1013.1.5485 | `openEHR-EHR-CLUSTER.image_acquisition_details.v0` | Image acquisition details | INITIAL | 2021-06-18T03:38:49+02:00 | 0.0.1-alpha |
| 1013.1.5486 | `openEHR-EHR-CLUSTER.image_reconstruction_details.v0` | Image reconstruction details | INITIAL | 2021-06-18T03:39:29+02:00 | 0.0.1-alpha |
| 1013.1.616 | `openEHR-EHR-CLUSTER.imaging.v0` | Imaging details | REJECTED | 2020-03-06T04:50:39+01:00 | 0.0.1 |
| 1013.1.5940 | `openEHR-EHR-CLUSTER.imaging_exam-bladder.v0` | Imaging examination of the bladder | DRAFT | 2022-05-30T09:07:52+02:00 | 0.0.1-alpha |
| 1013.1.5925 | `openEHR-EHR-CLUSTER.imaging_exam-cervix.v1` | Imaging examination of the cervix | PUBLISHED | 2022-10-20T08:19:53+02:00 | 1.0.0 |
| 1013.1.5997 | `openEHR-EHR-CLUSTER.imaging_exam-fallopian_tube.v1` | Imaging examination of a fallopian tube | PUBLISHED | 2025-04-22T13:10:15+02:00 | 1.0.1 |
| 1013.1.5923 | `openEHR-EHR-CLUSTER.imaging_exam-foetus.v1` | Imaging examination of a fetus | PUBLISHED | 2025-09-30T01:09:42+02:00 | 1.0.2 |
| 1013.1.5941 | `openEHR-EHR-CLUSTER.imaging_exam-gestational_sac.v1` | Imaging examination of a gestational sac | PUBLISHED | 2023-05-31T09:37:49+02:00 | 1.0.1 |
| 1013.1.7657 | `openEHR-EHR-CLUSTER.imaging_exam-heart.v0` | Imaging examination of the heart | INITIAL | 2024-12-30T02:40:40+01:00 | 0.0.1-alpha |
| 1013.1.6760 | `openEHR-EHR-CLUSTER.imaging_exam-hip_joint.v1` | Imaging examination of a hip joint | PUBLISHED | 2024-11-01T12:25:39+01:00 | 1.0.0 |
| 1013.1.5939 | `openEHR-EHR-CLUSTER.imaging_exam-lesion-adnexal_mass.v0` | Imaging examination of an adnexal mass | TEAMREVIEW | 2022-06-30T08:46:52+02:00 | 0.0.1-alpha |
| 1013.1.5908 | `openEHR-EHR-CLUSTER.imaging_exam-liver.v0` | Imaging examination of the liver | DRAFT | 2022-10-25T14:18:23+02:00 | 0.0.1-alpha |
| 1013.1.5909 | `openEHR-EHR-CLUSTER.imaging_exam-lymph_node.v0` | Imaging examination of a lymph node | DRAFT | 2022-01-11T05:21:11+01:00 | 0.0.1-alpha |
| 1013.1.5910 | `openEHR-EHR-CLUSTER.imaging_exam-lymph_node_group.v0` | Imaging examination of a lymph node group | DRAFT | 2022-01-11T05:21:12+01:00 | 0.0.1-alpha |
| 1013.1.5911 | `openEHR-EHR-CLUSTER.imaging_exam-ovary.v1` | Imaging examination of an ovary | PUBLISHED | 2024-04-15T09:47:02+02:00 | 1.0.1 |
| 1013.1.6731 | `openEHR-EHR-CLUSTER.imaging_exam-pelvis.v0` | Imaging examination of pelvis | DRAFT | 2023-05-11T11:19:21+02:00 | 0.0.1-alpha |
| 1013.1.6512 | `openEHR-EHR-CLUSTER.imaging_exam-placenta.v0` | Imaging examination of a placenta | DRAFT | 2025-10-01T23:38:46+02:00 | 0.0.1-alpha |
| 1013.1.6514 | `openEHR-EHR-CLUSTER.imaging_exam-pregnant_uterus.v0` | Imaging examination of a pregnant uterus | DRAFT | 2025-10-01T23:32:29+02:00 | 0.0.1-alpha |
| 1013.1.5942 | `openEHR-EHR-CLUSTER.imaging_exam-rectouterine_pouch.v1` | Imaging examination of the rectouterine pouch | PUBLISHED | 2022-10-20T08:25:16+02:00 | 1.0.0 |
| 1013.1.6729 | `openEHR-EHR-CLUSTER.imaging_exam-sacrum.v0` | Imaging examination of the sacrum | DRAFT | 2023-05-11T11:19:53+02:00 | 0.0.1-alpha |
| 1013.1.5912 | `openEHR-EHR-CLUSTER.imaging_exam-scrotum.v0` | Imaging examination of the scrotum | DRAFT | 2022-05-30T09:27:27+02:00 | 0.0.1-alpha |
| 1013.1.6730 | `openEHR-EHR-CLUSTER.imaging_exam-spine.v0` | Imaging examination of the entire spine | DRAFT | 2023-05-11T11:17:01+02:00 | 0.0.1-alpha |
| 1013.1.5913 | `openEHR-EHR-CLUSTER.imaging_exam-testicle.v0` | Imaging examination of a testicle | DRAFT | 2022-12-10T23:26:10+01:00 | 0.0.1-alpha |
| 1013.1.6513 | `openEHR-EHR-CLUSTER.imaging_exam-umbilical_cord.v0` | Imaging examination of an umbilical cord | DRAFT | 2022-09-23T04:30:45+02:00 | 0.0.1-alpha |
| 1013.1.5914 | `openEHR-EHR-CLUSTER.imaging_exam-uterus.v0` | Imaging examination of the uterus | TEAMREVIEW | 2022-10-24T09:16:15+02:00 | 0.0.1-alpha |
| 1013.1.5915 | `openEHR-EHR-CLUSTER.imaging_exam.v1` | Imaging examination of a body structure | PUBLISHED | 2026-04-28T16:57:03+02:00 | 1.1.4 |
| 1013.1.5907 | `openEHR-EHR-CLUSTER.imaging_exam_anomaly.v0` | Imaging examination of an anomaly | REVIEWSUSPENDED | 2025-03-03T09:21:14+01:00 | 0.0.1-alpha |
| 1013.1.4539 | `openEHR-EHR-CLUSTER.imaging_finding.v0` | Imaging finding | REJECTED | 2022-05-06T10:22:21+02:00 | 0.0.1 |
| 1013.1.6588 | `openEHR-EHR-CLUSTER.imaging_myometrial_lesion.v0` | Imaging examination of a myometrial lesion | DRAFT | 2022-11-17T01:23:01+01:00 | 0.0.1-alpha |
| 1013.1.2764 | `openEHR-EHR-CLUSTER.imaging_result.v0` | Imaging result | REJECTED | 2021-12-06T06:00:43+01:00 | 0.0.1 |
| 1013.1.6012 | `openEHR-EHR-CLUSTER.imaging_series.v0` | Imaging series | DRAFT | 2026-02-10T10:22:12+01:00 | 0.0.1-alpha |
| 1013.1.365 | `openEHR-EHR-CLUSTER.individual_personal.v0` | Individual's personal demographics | REJECTED | 2021-08-10T08:59:31+02:00 | 0.0.1 |
| 1013.1.5071 | `openEHR-EHR-CLUSTER.individual_personal_uk.v0` | Individual's personal demographics (UK) | REJECTED | 2021-12-17T07:06:03+01:00 | 0.0.1 |
| 1013.1.368 | `openEHR-EHR-CLUSTER.individual_professional.v0` | Professional Individual demographics | REJECTED | 2021-12-17T07:12:49+01:00 | 0.0.1 |
| 1013.1.4513 | `openEHR-EHR-CLUSTER.information_resource.v1` | Information resource | PUBLISHED | 2021-08-10T22:31:48+02:00 | 1.0.1 |
| 1013.1.233 | `openEHR-EHR-CLUSTER.inspection-ear_canal.v1` | Inspection of Ear Canal | REJECTED | 2012-09-07T12:22:02+02:00 |  |
| 1013.1.234 | `openEHR-EHR-CLUSTER.inspection-external_ear.v1` | Inspection of External ear | REJECTED | 2015-05-24T19:21:21+02:00 |  |
| 1013.1.235 | `openEHR-EHR-CLUSTER.inspection-joint.v1` | Inspection of joint | REJECTED | 2015-05-24T19:22:25+02:00 |  |
| 1013.1.264 | `openEHR-EHR-CLUSTER.inspection-skin-scalp.v1` | Inspection of scalp | REJECTED | 2015-11-23T02:27:02+01:00 |  |
| 1013.1.238 | `openEHR-EHR-CLUSTER.inspection-skin-wound.v1` | Inspection of open wound | REJECTED | 2015-06-25T06:09:30+02:00 |  |
| 1013.1.236 | `openEHR-EHR-CLUSTER.inspection-skin.v1` | Inspection of skin | REJECTED | 2015-11-23T02:26:20+01:00 |  |
| 1013.1.239 | `openEHR-EHR-CLUSTER.inspection-tongue.v1` | Inspection of tongue | REJECTED | 2015-06-22T05:47:49+02:00 | 0.0.1 |
| 1013.1.240 | `openEHR-EHR-CLUSTER.inspection-trauma.v1` | Inspection of blunt traumatic injury | REJECTED | 2015-05-24T19:22:54+02:00 |  |
| 1013.1.241 | `openEHR-EHR-CLUSTER.inspection-tympanic_membrane.v1` | Inspection of Tympanic Membrane | REJECTED | 2012-09-07T12:21:12+02:00 |  |
| 1013.1.242 | `openEHR-EHR-CLUSTER.inspection-tympanic_perforation.v1` | Inspection of a tympanic membrane perforation | REJECTED | 2012-09-07T12:21:44+02:00 |  |
| 1013.1.11 | `openEHR-EHR-CLUSTER.inspection.v1` | Inspection | REJECTED | 2015-06-25T06:07:51+02:00 |  |
| 1013.1.2841 | `openEHR-EHR-CLUSTER.inspection_body_fluid-sputum.v0` | Inspection of sputum | REVIEWSUSPENDED | 2020-02-03T07:40:15+01:00 | 0.0.1-alpha |
| 1013.1.4336 | `openEHR-EHR-CLUSTER.inspection_body_fluid-urine.v0` | Inspection of urine | DRAFT | 2020-02-03T07:27:47+01:00 | 0.0.1-alpha |
| 1013.1.2255 | `openEHR-EHR-CLUSTER.inspection_body_fluid.v0` | Inspection of a body fluid | REVIEWSUSPENDED | 2026-08-13T20:18:58+02:00 | 0.0.1-alpha |
| 1013.1.2095 | `openEHR-EHR-CLUSTER.inspection_cervix.v0` | Inspection of cervix | REJECTED | 2019-07-29T05:18:36+02:00 | 0.0.1 |
| 1013.1.2112 | `openEHR-EHR-CLUSTER.inspection_discharge.v1` | Inspection of discharge | REJECTED | 2015-10-02T04:10:41+02:00 | 0.0.1 |
| 1013.1.2233 | `openEHR-EHR-CLUSTER.inspection_rectum.v0` | Inspection rectum | REJECTED | 2019-07-29T05:20:02+02:00 | 0.0.1 |
| 1013.1.2114 | `openEHR-EHR-CLUSTER.inspection_vagina.v0` | Inspection of vagina | REJECTED | 2019-07-29T05:19:23+02:00 | 0.0.1 |
| 1013.1.393 | `openEHR-EHR-CLUSTER.inspired_oxygen.v1` | Inspired oxygen | PUBLISHED | 2021-01-10T01:58:53+01:00 | 1.0.2 |
| 1013.1.1972 | `openEHR-EHR-CLUSTER.interpreter_request.v1` | Interpreter request | PUBLISHED | 2019-01-21T15:15:26+01:00 | 1.0.0 |
| 1013.1.8183 | `openEHR-EHR-CLUSTER.intraocular_pressure_test_reliability_indicator.v0` | Intraocular pressure test reliability indicator | INITIAL | 2026-02-07T16:09:20+01:00 | 0.0.1-alpha |
| 1013.1.2016 | `openEHR-EHR-CLUSTER.intravitreal_injection_details.v0` | Intravitreal injection details | INITIAL | 2015-06-16T12:32:36+02:00 | 0.0.1-alpha |
| 1013.1.3755 | `openEHR-EHR-CLUSTER.inversion_variant.v1` | Genomic inversion variant | DEPRECATED | 2020-08-06T11:53:17+02:00 | 1.1.1 |
| 1013.1.115 | `openEHR-EHR-CLUSTER.issue.v0` | Issue | DRAFT | 2024-10-29T02:56:18+01:00 | 0.0.1-alpha |
| 1013.1.3850 | `openEHR-EHR-CLUSTER.item_transport.v1` | Transportation of an item | PUBLISHED | 2022-06-23T12:33:34+02:00 | 1.0.1 |
| 1013.1.3748 | `openEHR-EHR-CLUSTER.knowledge_base_reference.v1` | Knowledge base reference | PUBLISHED | 2021-05-20T09:40:14+02:00 | 1.0.1 |
| 1013.1.7640 | `openEHR-EHR-CLUSTER.lab_antibody.v0` | Antibody test finding | INITIAL | 2024-12-24T09:50:39+01:00 | 0.0.1-alpha |
| 1013.1.7643 | `openEHR-EHR-CLUSTER.lab_antigen.v0` | Antigen test finding | INITIAL | 2024-12-24T10:02:38+01:00 | 0.0.1-alpha |
| 1013.1.7644 | `openEHR-EHR-CLUSTER.lab_blood_cell_count.v0` | Blood cell count and differential finding | INITIAL | 2024-12-24T09:58:43+01:00 | 0.0.1-alpha |
| 1013.1.7645 | `openEHR-EHR-CLUSTER.lab_microscopy_culture.v0` | Microbiology culture findings | INITIAL | 2024-12-30T00:00:30+01:00 | 0.0.1-alpha |
| 1013.1.7647 | `openEHR-EHR-CLUSTER.lab_microscopy_parasitology.v0` | Microbiology parasitology findings | INITIAL | 2024-12-30T00:06:58+01:00 | 0.0.1-alpha |
| 1013.1.7649 | `openEHR-EHR-CLUSTER.lab_microscopy_stain.v0` | Microscopy stain findings | INITIAL | 2024-12-30T00:25:05+01:00 | 0.0.1-alpha |
| 1013.1.7650 | `openEHR-EHR-CLUSTER.lab_molecular_microbial.v0` | Molecular microbial test findings | INITIAL | 2026-03-09T08:52:47+01:00 | 0.0.1-alpha |
| 1013.1.568 | `openEHR-EHR-CLUSTER.lab_result_annotation.v1` | Laboratory result annotation | REJECTED | 2016-11-28T21:00:44+01:00 |  |
| 1013.1.7545 | `openEHR-EHR-CLUSTER.laboratory_stain_findings.v0` | Laboratory stain findings | INITIAL | 2024-10-29T14:56:50+01:00 | 0.0.1-alpha |
| 1013.1.2881 | `openEHR-EHR-CLUSTER.laboratory_test_analyte.v1` | Laboratory analyte result | PUBLISHED | 2026-07-21T18:14:23+02:00 | 1.2.3 |
| 1013.1.2192 | `openEHR-EHR-CLUSTER.laboratory_test_panel.v0` | Laboratory test panel | REVIEWSUSPENDED | 2019-05-16T12:16:23+02:00 | 0.0.1-alpha |
| 1013.1.7537 | `openEHR-EHR-CLUSTER.laboratory_test_serology.v0` | Laboratory serological finding | INITIAL | 2026-03-11T15:50:49+01:00 | 0.0.1-alpha |
| 1013.1.2886 | `openEHR-EHR-CLUSTER.language.v1` | Language | PUBLISHED | 2026-06-25T15:29:17+02:00 | 1.1.2 |
| 1013.1.7729 | `openEHR-EHR-CLUSTER.lens_specification.v0` | Lens_specification | INITIAL | 2025-02-07T18:00:04+01:00 | 0.0.1-alpha |
| 1013.1.7975 | `openEHR-EHR-CLUSTER.lesions.v0` | Lesions | INITIAL | 2025-07-22T06:17:19+02:00 | 0.0.1-alpha |
| 1013.1.6952 | `openEHR-EHR-CLUSTER.level_of_certainty_bc.v0` | Level of certainty (Brighton Collaboration) | INITIAL | 2023-07-20T05:17:16+02:00 | 0.0.1-alpha |
| 1013.1.297 | `openEHR-EHR-CLUSTER.level_of_exertion.v0` | Level of exertion | DRAFT | 2019-07-24T15:39:38+02:00 | 0.0.1-alpha |
| 1013.1.396 | `openEHR-EHR-CLUSTER.lymph_node_metastases.v0` | Lymph node metastases | DRAFT | 2019-11-01T10:55:47+01:00 | 0.0.1-alpha |
| 1013.1.2743 | `openEHR-EHR-CLUSTER.macronutrients.v0` | Macronutrients | DRAFT | 2017-03-21T10:52:53+01:00 | 0.0.1-alpha |
| 1013.1.423 | `openEHR-EHR-CLUSTER.macroscopy_colorectal_carcinoma.v0` | Macroscopic findings - Colorectal cancer | REVIEWSUSPENDED | 2019-11-07T07:53:18+01:00 | 0.0.1-alpha |
| 1013.1.516 | `openEHR-EHR-CLUSTER.macroscopy_lung_carcinoma.v0` | Macroscopic findings - Lung cancer | DRAFT | 2019-09-19T07:59:09+02:00 | 0.0.1-alpha |
| 1013.1.5626 | `openEHR-EHR-CLUSTER.maximal_blood_pressure.v0` | CCTA specific | INITIAL | 2022-06-15T05:01:03+02:00 | 0.0.1-alpha |
| 1013.1.1800 | `openEHR-EHR-CLUSTER.media_file.v1` | Media file | PUBLISHED | 2026-07-14T23:39:50+02:00 | 1.0.6 |
| 1013.1.2368 | `openEHR-EHR-CLUSTER.medication.v1` | Medication | DEPRECATED | 2021-12-24T12:42:11+01:00 | 1.0.6 |
| 1013.1.5947 | `openEHR-EHR-CLUSTER.medication.v2` | Medication details | PUBLISHED | 2026-07-16T11:02:55+02:00 | 2.0.6 |
| 1013.1.1449 | `openEHR-EHR-CLUSTER.medication_admin.v1` | Medication administration | REJECTED | 2015-10-29T10:04:30+01:00 |  |
| 1013.1.1447 | `openEHR-EHR-CLUSTER.medication_amount.v1` | Medication amount | REJECTED | 2015-10-29T10:00:57+01:00 |  |
| 1013.1.2300 | `openEHR-EHR-CLUSTER.medication_authorisation.v0` | Medication authorisation | DRAFT | 2017-06-19T09:18:47+02:00 | 0.0.1-alpha |
| 1013.1.1446 | `openEHR-EHR-CLUSTER.medication_ingredients.v1` | Medication ingredients | REJECTED | 2015-10-29T10:02:42+01:00 |  |
| 1013.1.2306 | `openEHR-EHR-CLUSTER.medication_order_summary.v0` | Medication order summary | DRAFT | 2025-01-28T09:05:02+01:00 | 0.0.1-alpha |
| 1013.1.2296 | `openEHR-EHR-CLUSTER.medication_preparation.v1` | Medication preparation | REJECTED | 2015-12-16T15:19:46+01:00 | 0.0.1 |
| 1013.1.2453 | `openEHR-EHR-CLUSTER.medication_supply_amount.v0` | Medication supply amount | DRAFT | 2024-10-30T22:44:57+01:00 | 0.0.1-alpha |
| 1013.1.116 | `openEHR-EHR-CLUSTER.menstrual_cycle.v1` | Menstrual cycle | REJECTED | 2015-03-16T03:44:44+01:00 |  |
| 1013.1.7544 | `openEHR-EHR-CLUSTER.microbiology_culture.v0` | Microbiology culture findings | INITIAL | 2024-10-29T14:55:12+01:00 | 0.0.1-alpha |
| 1013.1.7543 | `openEHR-EHR-CLUSTER.microbiology_parasitology.v0` | Microbiology parasitology findings | INITIAL | 2024-10-29T14:53:29+01:00 | 0.0.1-alpha |
| 1013.1.2744 | `openEHR-EHR-CLUSTER.micronutrients.v0` | Micronutrients | DRAFT | 2018-09-03T09:48:41+02:00 | 0.0.1-alpha |
| 1013.1.381 | `openEHR-EHR-CLUSTER.microscopy_breast_carcinoma.v1` | Microscopic findings - Breast cancer | REVIEWSUSPENDED | 2012-04-03T10:12:28+02:00 |  |
| 1013.1.422 | `openEHR-EHR-CLUSTER.microscopy_colorectal_carcinoma.v0` | Microscopic findings - Colorectal cancer | REVIEWSUSPENDED | 2019-11-07T02:57:10+01:00 | 0.0.1-alpha |
| 1013.1.415 | `openEHR-EHR-CLUSTER.microscopy_lung_carcinoma.v1` | Microscopic findings - Lung cancer | REJECTED | 2018-05-01T09:06:47+02:00 |  |
| 1013.1.349 | `openEHR-EHR-CLUSTER.microscopy_lymphoma.v0` | Microscopic findings - Lymphoma | REVIEWSUSPENDED | 2018-06-29T07:47:09+02:00 | 0.0.1-alpha |
| 1013.1.344 | `openEHR-EHR-CLUSTER.microscopy_melanoma.v0` | Microscopic findings - Melanoma of skin | REVIEWSUSPENDED | 2018-06-23T03:17:00+02:00 | 0.0.1-alpha |
| 1013.1.380 | `openEHR-EHR-CLUSTER.microscopy_prostate_carcinoma.v0` | Microscopic findings - Prostate cancer | REVIEWSUSPENDED | 2019-11-07T02:52:33+01:00 | 0.0.1-alpha |
| 1013.1.2859 | `openEHR-EHR-CLUSTER.microscopy_renal_biopsy_non_neoplastic.v0` | Microscopy renal biopsy non neoplastic | DRAFT | 2017-06-16T07:20:01+02:00 | 0.0.1-alpha |
| 1013.1.30 | `openEHR-EHR-CLUSTER.move-joint.v0` | Movement of a joint | REJECTED | 2018-09-10T08:53:21+02:00 | 0.0.1 |
| 1013.1.31 | `openEHR-EHR-CLUSTER.move-spine.v1` | Movement of the spine or parts thereof | REJECTED | 2018-09-10T08:53:54+02:00 |  |
| 1013.1.29 | `openEHR-EHR-CLUSTER.move.v0` | Movement | REJECTED | 2018-09-10T08:54:26+02:00 | 0.0.1 |
| 1013.1.4514 | `openEHR-EHR-CLUSTER.multimedia_source.v0` | Multimedia source | INITIAL | 2020-03-24T03:55:15+01:00 | 0.0.1-alpha |
| 1013.1.6955 | `openEHR-EHR-CLUSTER.muscle_power.v0` | Muscle power finding | INITIAL | 2026-01-22T15:47:42+01:00 | 0.0.1-alpha |
| 1013.1.6957 | `openEHR-EHR-CLUSTER.muscle_tone.v0` | Muscle tone finding | INITIAL | 2023-07-20T09:20:05+02:00 | 0.0.1-alpha |
| 1013.1.2512 | `openEHR-EHR-CLUSTER.mydriasis_application.v0` | Mydriasis application | INITIAL | 2016-07-24T11:59:22+02:00 | 0.0.1-alpha |
| 1013.1.1672 | `openEHR-EHR-CLUSTER.myringoplasty.v0` | Myringoplasty Procedure | DRAFT | 2019-03-13T08:07:13+01:00 | 0.0.1-alpha |
| 1013.1.1674 | `openEHR-EHR-CLUSTER.myringotomy.v0` | Myringotomy | DRAFT | 2019-03-13T08:08:46+01:00 | 0.0.1-alpha |
| 1013.1.628 | `openEHR-EHR-CLUSTER.notifiable_condition.v0` | Notifiable condition | DRAFT | 2018-04-25T05:05:00+02:00 | 0.0.1-alpha |
| 1013.1.2380 | `openEHR-EHR-CLUSTER.occupation_record.v1` | Occupation record | PUBLISHED | 2024-05-27T08:25:09+02:00 | 1.2.0 |
| 1013.1.117 | `openEHR-EHR-CLUSTER.oedema.v0` | Oedema | DRAFT | 2026-02-04T11:52:57+01:00 | 0.0.1-alpha |
| 1013.1.6014 | `openEHR-EHR-CLUSTER.oocyte_specimen.v1` | Oocyte specimen | PUBLISHED | 2023-11-23T11:45:38+01:00 | 1.0.0 |
| 1013.1.2900 | `openEHR-EHR-CLUSTER.operative_procedure.v0` | Operative procedure | DRAFT | 2017-07-26T05:51:10+02:00 | 0.0.1-alpha |
| 1013.1.2021 | `openEHR-EHR-CLUSTER.ophthalmic_laser_details.v0` | Ophthalmic laser procedure details | INITIAL | 2015-06-13T21:51:02+02:00 | 0.0.1-alpha |
| 1013.1.1699 | `openEHR-EHR-CLUSTER.ophthalmic_surgery_details_for_posterior_segment_of_eye.v0` | Ophthalmic surgery details for posterior segment of eye | INITIAL | 2015-06-08T22:22:01+02:00 | 0.0.1-alpha |
| 1013.1.2082 | `openEHR-EHR-CLUSTER.ophthalmic_thickness_details.v0` | Ophthalmic thickness details | INITIAL | 2015-06-22T21:49:26+02:00 | 0.0.1-alpha |
| 1013.1.371 | `openEHR-EHR-CLUSTER.organisation.v1` | Organisation | PUBLISHED | 2026-07-07T17:03:47+02:00 | 1.0.4 |
| 1013.1.4415 | `openEHR-EHR-CLUSTER.organisation_cc.v0` | Organisation | INITIAL | 2020-03-22T16:07:04+01:00 | 0.0.1-alpha |
| 1013.1.5901 | `openEHR-EHR-CLUSTER.other_significant_conditions.v0` | Other significant conditions | DRAFT | 2021-12-09T15:51:20+01:00 | 0.0.1-alpha |
| 1013.1.4402 | `openEHR-EHR-CLUSTER.outbreak_exposure.v0` | Location-based exposure | INITIAL | 2020-03-18T22:40:33+01:00 | 0.0.1-alpha |
| 1013.1.569 | `openEHR-EHR-CLUSTER.outbreak_identification.v0` | Outbreak identification | DRAFT | 2018-05-10T03:51:12+02:00 | 0.0.1-alpha |
| 1013.1.1909 | `openEHR-EHR-CLUSTER.pain.v1` | Pain | REJECTED | 2015-09-28T05:24:10+02:00 |  |
| 1013.1.243 | `openEHR-EHR-CLUSTER.palpation-external_ear.v1` | Palpation of the external ear | REJECTED | 2014-03-07T07:58:05+01:00 |  |
| 1013.1.244 | `openEHR-EHR-CLUSTER.palpation-joint.v1` | Palpation of joint | REJECTED | 2015-02-05T06:41:57+01:00 |  |
| 1013.1.12 | `openEHR-EHR-CLUSTER.palpation.v1` | Palpation | REJECTED | 2015-02-05T06:41:40+01:00 |  |
| 1013.1.3191 | `openEHR-EHR-CLUSTER.palpation_fetus.v0` | Palpation of a fetus | REJECTED | 2019-08-16T17:27:46+02:00 | 0.0.1 |
| 1013.1.1668 | `openEHR-EHR-CLUSTER.palpation_of_cervix.v0` | Palpation of Cervix | REJECTED | 2019-07-29T06:40:01+02:00 | 0.0.1 |
| 1013.1.1628 | `openEHR-EHR-CLUSTER.palpation_of_uterus.v0` | Palpation of Uterus | REJECTED | 2019-07-31T07:37:46+02:00 | 0.0.1 |
| 1013.1.2234 | `openEHR-EHR-CLUSTER.palpation_prostate.v0` | Palpation of the prostate | REJECTED | 2019-07-31T06:33:15+02:00 | 0.0.1 |
| 1013.1.2232 | `openEHR-EHR-CLUSTER.palpation_rectum.v0` | Palpation of rectum | REJECTED | 2019-07-31T06:13:16+02:00 | 0.0.1 |
| 1013.1.2100 | `openEHR-EHR-CLUSTER.palpation_testicle.v0` | Palpation of a testicle | REJECTED | 2019-07-29T05:42:38+02:00 | 0.0.1 |
| 1013.1.2113 | `openEHR-EHR-CLUSTER.palpation_vagina.v0` | Palpation of vagina | REJECTED | 2019-07-31T07:55:57+02:00 | 0.0.1 |
| 1013.1.118 | `openEHR-EHR-CLUSTER.percussion.v1` | Percussion | REJECTED | 2018-09-11T08:51:06+02:00 |  |
| 1013.1.5358 | `openEHR-EHR-CLUSTER.person.v1` | Person | PUBLISHED | 2026-07-15T11:12:43+02:00 | 1.0.5 |
| 1013.1.1745 | `openEHR-EHR-CLUSTER.person_anonymised_parent.v0` | Anonymised person (PARENT) | INITIAL | 2020-05-11T08:30:48+02:00 | 0.0.1-alpha |
| 1013.1.1746 | `openEHR-EHR-CLUSTER.person_identifiable_parent.v1` | Identifiable Person (PARENT) | INITIAL | 2015-02-21T17:58:16+01:00 |  |
| 1013.1.1988 | `openEHR-EHR-CLUSTER.person_identifier_slovenia_parent.v0` | Person identifier slovenia (PARENT) | INITIAL | 2015-05-15T18:09:05+02:00 | 0.0.1-alpha |
| 1013.1.366 | `openEHR-EHR-CLUSTER.person_name.v0` | Person name | REJECTED | 2021-08-10T09:00:37+02:00 | 0.0.1 |
| 1013.1.1747 | `openEHR-EHR-CLUSTER.person_name_isa.v1` | Person name (ISA) | INITIAL | 2014-07-28T17:05:16+02:00 |  |
| 1013.1.5148 | `openEHR-EHR-CLUSTER.pews_original.v0` | PEWS - original variables | DRAFT | 2021-01-19T05:22:04+01:00 | 0.0.1-alpha |
| 1013.1.7066 | `openEHR-EHR-CLUSTER.pharmacogenetic_test_result.v1` | Pharmacogenetic test result | PUBLISHED | 2024-11-27T13:36:54+01:00 | 1.0.1 |
| 1013.1.1764 | `openEHR-EHR-CLUSTER.photocoagulation_details.v1` | Photocoagulation details | INITIAL | 2014-08-18T23:23:39+02:00 |  |
| 1013.1.3565 | `openEHR-EHR-CLUSTER.physical_activity_calculation.v0` | Physical activity calculation | INITIAL | 2018-12-06T07:30:00+01:00 | 0.0.1-alpha |
| 1013.1.7812 | `openEHR-EHR-CLUSTER.physical_dimensions.v1` | Physical dimensions | PUBLISHED | 2026-08-18T15:36:08+02:00 | 1.0.3 |
| 1013.1.2902 | `openEHR-EHR-CLUSTER.physiological_monitoring.v0` | Physiological monitoring | DRAFT | 2019-11-21T01:46:06+01:00 | 0.0.1-alpha |
| 1013.1.7264 | `openEHR-EHR-CLUSTER.pi_rads_2_1.v1` | PI-RADS v2.1 | PUBLISHED | 2024-10-16T13:27:55+02:00 | 1.0.0 |
| 1013.1.4150 | `openEHR-EHR-CLUSTER.pp_biosample.v0` | Phenopacket biosample | INITIAL | 2019-09-26T14:19:38+02:00 | 0.0.1-alpha |
| 1013.1.4164 | `openEHR-EHR-CLUSTER.pp_diagnosis.v0` | Phenopacket diagnosis | INITIAL | 2019-09-26T14:19:40+02:00 | 0.0.1-alpha |
| 1013.1.4155 | `openEHR-EHR-CLUSTER.pp_disease.v0` | Phenopacket disease | INITIAL | 2019-09-26T14:19:42+02:00 | 0.0.1-alpha |
| 1013.1.4146 | `openEHR-EHR-CLUSTER.pp_evidence.v0` | Phenopacket evidence | INITIAL | 2019-09-26T14:19:43+02:00 | 0.0.1-alpha |
| 1013.1.4142 | `openEHR-EHR-CLUSTER.pp_external_reference.v0` | Phenopacket external reference | INITIAL | 2019-09-26T14:19:44+02:00 | 0.0.1-alpha |
| 1013.1.4162 | `openEHR-EHR-CLUSTER.pp_family_framework.v0` | Phenopacket family framework | INITIAL | 2019-09-26T02:23:06+02:00 | 0.0.1-alpha |
| 1013.1.4156 | `openEHR-EHR-CLUSTER.pp_gene.v0` | Phenopacket gene | INITIAL | 2019-09-26T02:05:18+02:00 | 0.0.1-alpha |
| 1013.1.4165 | `openEHR-EHR-CLUSTER.pp_genomic_interpretation.v0` | Phenopacket genomic interpretation | INITIAL | 2019-09-26T14:19:47+02:00 | 0.0.1-alpha |
| 1013.1.4144 | `openEHR-EHR-CLUSTER.pp_hgvsallele.v0` | Phenopacket HgvsAllele | INITIAL | 2019-09-25T07:06:59+02:00 | 0.0.1-alpha |
| 1013.1.4147 | `openEHR-EHR-CLUSTER.pp_htsfile.v0` | Phenopacket HtsFile | INITIAL | 2019-09-26T14:19:48+02:00 | 0.0.1-alpha |
| 1013.1.4154 | `openEHR-EHR-CLUSTER.pp_iscnallele.v0` | Phenopacket IscnAllele | INITIAL | 2019-09-25T07:07:19+02:00 | 0.0.1-alpha |
| 1013.1.4141 | `openEHR-EHR-CLUSTER.pp_metadata.v0` | Phenopacket MetaData | INITIAL | 2019-09-26T14:19:49+02:00 | 0.0.1-alpha |
| 1013.1.4157 | `openEHR-EHR-CLUSTER.pp_pedigree.v0` | Phenopackets pedigree | INITIAL | 2019-09-26T02:05:24+02:00 | 0.0.1-alpha |
| 1013.1.4158 | `openEHR-EHR-CLUSTER.pp_person.v0` | Phenopackets person | INITIAL | 2019-09-26T02:05:25+02:00 | 0.0.1-alpha |
| 1013.1.4159 | `openEHR-EHR-CLUSTER.pp_phenopacket_framework.v0` | Phenopacket framework | INITIAL | 2019-09-26T02:05:27+02:00 | 0.0.1-alpha |
| 1013.1.4145 | `openEHR-EHR-CLUSTER.pp_phenotypic_feature.v0` | Phenopacket phenotypic feature | INITIAL | 2019-09-26T14:19:50+02:00 | 0.0.1-alpha |
| 1013.1.4139 | `openEHR-EHR-CLUSTER.pp_procedure.v0` | Phenopacket procedure | INITIAL | 2019-09-26T14:19:51+02:00 | 0.0.1-alpha |
| 1013.1.4140 | `openEHR-EHR-CLUSTER.pp_resource.v0` | Phenopacket resource | INITIAL | 2019-09-26T14:19:51+02:00 | 0.0.1-alpha |
| 1013.1.4149 | `openEHR-EHR-CLUSTER.pp_spdiallele.v0` | Phenopacket SpdiAllele | INITIAL | 2019-09-25T07:07:10+02:00 | 0.0.1-alpha |
| 1013.1.4148 | `openEHR-EHR-CLUSTER.pp_update.v0` | Phenopacket update | INITIAL | 2019-09-26T14:19:52+02:00 | 0.0.1-alpha |
| 1013.1.4153 | `openEHR-EHR-CLUSTER.pp_variant.v0` | Phenopacket variant | INITIAL | 2019-09-26T14:19:53+02:00 | 0.0.1-alpha |
| 1013.1.4152 | `openEHR-EHR-CLUSTER.pp_vcfallele.v0` | Phenopacket VcfAllele | INITIAL | 2019-09-25T07:07:16+02:00 | 0.0.1-alpha |
| 1013.1.1451 | `openEHR-EHR-CLUSTER.problem_qualifier.v1` | Problem/Diagnosis qualifier | DEPRECATED | 2022-12-13T08:42:29+01:00 | 1.0.2 |
| 1013.1.6653 | `openEHR-EHR-CLUSTER.problem_qualifier.v2` | Problem/Diagnosis qualifier | PUBLISHED | 2026-08-27T11:38:33+02:00 | 2.1.1 |
| 1013.1.2901 | `openEHR-EHR-CLUSTER.procedure_preparation.v0` | Procedure preparation | DRAFT | 2019-11-21T02:43:30+01:00 | 0.0.1-alpha |
| 1013.1.370 | `openEHR-EHR-CLUSTER.professional_role.v0` | Professional role | REJECTED | 2021-12-17T07:13:32+01:00 | 0.0.1 |
| 1013.1.4807 | `openEHR-EHR-CLUSTER.promis_bank_v10_anxiety.v0` | PROMIS Item Bank v1.0 - Anxiety | DRAFT | 2025-09-22T09:49:38+02:00 | 0.0.1-alpha |
| 1013.1.4811 | `openEHR-EHR-CLUSTER.promis_bank_v10_depression.v0` | PROMIS Item Bank v1.0 - Depression | DRAFT | 2025-09-22T10:00:19+02:00 | 0.0.1-alpha |
| 1013.1.4812 | `openEHR-EHR-CLUSTER.promis_bank_v10_fatigue.v0` | PROMIS Item Bank v1.0 - Fatigue | DRAFT | 2025-09-22T09:25:16+02:00 | 0.0.1-alpha |
| 1013.1.4808 | `openEHR-EHR-CLUSTER.promis_bank_v10_sleep_disturbance.v0` | PROMIS Item Bank v1.0 - Sleep Disturbance | DRAFT | 2025-09-22T09:40:41+02:00 | 0.0.1-alpha |
| 1013.1.4809 | `openEHR-EHR-CLUSTER.promis_bank_v11_pain_interference.v0` | PROMIS Item Bank v1.1 - Pain Interference | DRAFT | 2025-09-22T17:06:32+02:00 | 0.0.1-alpha |
| 1013.1.4815 | `openEHR-EHR-CLUSTER.promis_bank_v20_ability_participate.v0` | PROMIS Item Bank v2.0 - Ability to Participate in Social Roles and Activities | DRAFT | 2025-09-22T10:36:03+02:00 | 0.0.1-alpha |
| 1013.1.4814 | `openEHR-EHR-CLUSTER.promis_bank_v20_physical_function.v0` | PROMIS Item Bank v2.0 - Physical Function | DRAFT | 2025-09-17T15:27:01+02:00 | 0.0.1-alpha |
| 1013.1.4813 | `openEHR-EHR-CLUSTER.promis_scale_v12_global_health.v0` | PROMIS Scale v1.2 - Global Health | DRAFT | 2025-09-22T17:59:55+02:00 | 0.0.1-alpha |
| 1013.1.209 | `openEHR-EHR-CLUSTER.pupils.v1` | Pupils | REJECTED | 2013-03-01T13:57:28+01:00 |  |
| 1013.1.5699 | `openEHR-EHR-CLUSTER.radiotherapy.v0` | Irradiation | DRAFT | 2024-08-22T10:19:42+02:00 | 0.0.1-alpha |
| 1013.1.7057 | `openEHR-EHR-CLUSTER.range_of_motion.v1` | Range of motion of a joint | PUBLISHED | 2026-06-09T15:13:33+02:00 | 1.0.2 |
| 1013.1.8340 | `openEHR-EHR-CLUSTER.reading_visual_acuity_supplementary_results.v0` | Reading visual acuity supplementary results | INITIAL | 2026-06-20T23:46:51+02:00 | 0.0.1-alpha |
| 1013.1.3762 | `openEHR-EHR-CLUSTER.reference_sequence.v1` | Reference sequence | PUBLISHED | 2026-11-10T16:29:09+01:00 | 1.1.1 |
| 1013.1.1292 | `openEHR-EHR-CLUSTER.refraction_details.v0` | Refraction Details | REVIEWSUSPENDED | 2021-02-17T14:36:32+01:00 | 0.0.1-alpha |
| 1013.1.2672 | `openEHR-EHR-CLUSTER.religion.v1` | Religious affiliation | PUBLISHED | 2023-03-21T01:03:06+01:00 | 1.1.0 |
| 1013.1.2160 | `openEHR-EHR-CLUSTER.risk_factors_in_glaucoma.v0` | Risk factors in glaucoma | INITIAL | 2015-07-05T23:41:55+02:00 | 0.0.1-alpha |
| 1013.1.1684 | `openEHR-EHR-CLUSTER.sade.v0` | Sade Classification | DRAFT | 2019-03-13T08:12:01+01:00 | 0.0.1-alpha |
| 1013.1.6960 | `openEHR-EHR-CLUSTER.sensation_finding.v0` | Sensation finding | INITIAL | 2023-07-20T09:24:42+02:00 | 0.0.1-alpha |
| 1013.1.6958 | `openEHR-EHR-CLUSTER.sensory_level.v0` | Sensory level | INITIAL | 2023-07-20T09:20:52+02:00 | 0.0.1-alpha |
| 1013.1.4256 | `openEHR-EHR-CLUSTER.sequencing_assay.v0` | Sequencing assay | TEAMREVIEW | 2025-05-21T11:02:09+02:00 | 0.0.1-alpha |
| 1013.1.3181 | `openEHR-EHR-CLUSTER.service_direction.v1` | Service direction | PUBLISHED | 2026-08-28T09:41:05+02:00 | 1.0.2 |
| 1013.1.6675 | `openEHR-EHR-CLUSTER.severity_rating_scale.v0` | Severity rating scale | REVIEWSUSPENDED | 2023-11-10T13:01:58+01:00 | 0.0.1-alpha |
| 1013.1.4858 | `openEHR-EHR-CLUSTER.simple_variant.v0` | Simple genetic variant | DRAFT | 2026-04-29T11:04:18+02:00 | 0.0.1-alpha |
| 1013.1.3967 | `openEHR-EHR-CLUSTER.skin_sensation.v0` | Skin sensation | DRAFT | 2019-07-26T11:37:31+02:00 | 0.0.1-alpha |
| 1013.1.331 | `openEHR-EHR-CLUSTER.specimen.v1` | Specimen | PUBLISHED | 2026-08-13T19:10:28+02:00 | 1.1.3 |
| 1013.1.2193 | `openEHR-EHR-CLUSTER.specimen_container.v1` | Specimen container | PUBLISHED | 2022-06-21T12:28:31+02:00 | 1.0.0 |
| 1013.1.386 | `openEHR-EHR-CLUSTER.specimen_measurements.v1` | Specimen measurements | DEPRECATED | 2025-09-03T10:02:01+02:00 | 1.0.2 |
| 1013.1.358 | `openEHR-EHR-CLUSTER.specimen_processing.v1` | Specimen processing | PUBLISHED | 2023-10-18T10:52:26+02:00 | 1.0.1 |
| 1013.1.5360 | `openEHR-EHR-CLUSTER.structured_address.v0` | Structured address | REJECTED | 2022-03-15T05:16:09+01:00 | 0.0.1 |
| 1013.1.5359 | `openEHR-EHR-CLUSTER.structured_name.v1` | Structured name of a person | PUBLISHED | 2026-07-14T23:51:34+02:00 | 1.0.3 |
| 1013.1.246 | `openEHR-EHR-CLUSTER.symptom-pain.v1` | Pain | REJECTED | 2015-03-11T07:41:08+01:00 |  |
| 1013.1.4399 | `openEHR-EHR-CLUSTER.symptom_sign-cvid.v0` | Covid-19 symptom | INITIAL | 2020-03-22T16:17:46+01:00 | 0.0.1-alpha |
| 1013.1.195 | `openEHR-EHR-CLUSTER.symptom_sign.v1` | Symptom/Sign | DEPRECATED | 2023-04-14T14:53:17+02:00 | 1.0.2 |
| 1013.1.6769 | `openEHR-EHR-CLUSTER.symptom_sign.v2` | Symptom/Sign | PUBLISHED | 2026-11-05T17:27:15+01:00 | 2.1.5 |
| 1013.1.672 | `openEHR-EHR-CLUSTER.synthesis_lymphoma.v1` | Diagnostic synthesis - lymphoma | REJECTED | 2018-05-01T09:44:59+02:00 |  |
| 1013.1.367 | `openEHR-EHR-CLUSTER.telecom_details.v0` | Communication details | REJECTED | 2021-08-24T05:27:17+02:00 | 0.0.1 |
| 1013.1.7976 | `openEHR-EHR-CLUSTER.tenderness.v0` | Tenderness findings | INITIAL | 2025-07-22T06:18:31+02:00 | 0.0.1-alpha |
| 1013.1.5722 | `openEHR-EHR-CLUSTER.test_circumstances.v0` | Testing circumstances | INITIAL | 2021-09-07T11:39:03+02:00 | 0.0.1-alpha |
| 1013.1.1761 | `openEHR-EHR-CLUSTER.therapeutic_decision_dr.v1` | Therapeutic decision DR | INITIAL | 2014-08-18T22:54:41+02:00 |  |
| 1013.1.2753 | `openEHR-EHR-CLUSTER.therapeutic_direction.v1` | Therapeutic direction | PUBLISHED | 2026-07-16T14:12:51+02:00 | 1.4.2 |
| 1013.1.1448 | `openEHR-EHR-CLUSTER.timing.v1` | Timing | REJECTED | 2016-05-04T08:33:20+02:00 |  |
| 1013.1.2245 | `openEHR-EHR-CLUSTER.timing_daily.v1` | Timing - daily | PUBLISHED | 2025-04-10T01:44:13+02:00 | 1.0.2 |
| 1013.1.2246 | `openEHR-EHR-CLUSTER.timing_nondaily.v1` | Timing - non-daily | PUBLISHED | 2023-02-07T12:20:15+01:00 | 1.1.2 |
| 1013.1.4191 | `openEHR-EHR-CLUSTER.tnm-pathological.v1` | TNM pathological classification | PUBLISHED | 2024-12-13T14:20:55+01:00 | 1.0.3 |
| 1013.1.2413 | `openEHR-EHR-CLUSTER.tnm.v1` | TNM clinical classification | PUBLISHED | 2020-06-10T13:23:48+02:00 | 1.0.0 |
| 1013.1.525 | `openEHR-EHR-CLUSTER.tnm_staging-breast.v1` | TNM staging - Breast (Sixth edition) | REJECTED | 2017-11-22T15:45:17+01:00 |  |
| 1013.1.421 | `openEHR-EHR-CLUSTER.tnm_staging-colorectal.v1` | TNM staging - Colorectal cancer (Sixth edition) | REJECTED | 2017-11-22T15:45:31+01:00 |  |
| 1013.1.417 | `openEHR-EHR-CLUSTER.tnm_staging-lung_cancer.v1` | TNM staging - Lung cancer (Sixth Edition) | REJECTED | 2017-11-22T15:46:14+01:00 |  |
| 1013.1.526 | `openEHR-EHR-CLUSTER.tnm_staging-melanoma.v1` | TNM staging - Melanoma of skin (Sixth edition) | REJECTED | 2017-11-22T15:46:37+01:00 |  |
| 1013.1.524 | `openEHR-EHR-CLUSTER.tnm_staging-prostate.v1` | TNM staging - Prostate cancer (Sixth edition) | REJECTED | 2017-11-22T15:46:52+01:00 |  |
| 1013.1.410 | `openEHR-EHR-CLUSTER.tnm_staging.v1` | Tumour - TNM Cancer staging | REJECTED | 2017-11-22T15:47:20+01:00 |  |
| 1013.1.659 | `openEHR-EHR-CLUSTER.tnm_staging_7th-breast.v1` | Tumour - Breast Cancer TNM staging 7th Edition | REJECTED | 2017-11-22T15:41:08+01:00 |  |
| 1013.1.657 | `openEHR-EHR-CLUSTER.tnm_staging_7th-colorectal.v1` | Tumour - Colorectal Cancer TNM staging 7th Edition | REJECTED | 2017-11-22T15:41:43+01:00 |  |
| 1013.1.656 | `openEHR-EHR-CLUSTER.tnm_staging_7th-lung.v1` | Tumour - Lung cancer TNM staging 7th Edition | REJECTED | 2017-11-22T15:42:29+01:00 |  |
| 1013.1.660 | `openEHR-EHR-CLUSTER.tnm_staging_7th-lymphoma.v1` | Tumour - TNM staging Lymphoma  7th Edition | REJECTED | 2017-11-22T15:47:47+01:00 |  |
| 1013.1.658 | `openEHR-EHR-CLUSTER.tnm_staging_7th-melanoma.v1` | Tumour - Primary Cutaneous Melanoma TNM staging 7th Edition | REJECTED | 2017-11-22T15:43:06+01:00 |  |
| 1013.1.655 | `openEHR-EHR-CLUSTER.tnm_staging_7th-prostate.v1` | Tumour - Prostate Cancer TNM staging 7th Edition | REJECTED | 2017-11-22T15:44:09+01:00 |  |
| 1013.1.654 | `openEHR-EHR-CLUSTER.tnm_staging_7th.v1` | Tumour - TNM Cancer staging 7th Edition | REJECTED | 2017-11-22T15:48:25+01:00 |  |
| 1013.1.1685 | `openEHR-EHR-CLUSTER.tos.v0` | Tos Classification | DRAFT | 2019-03-13T08:12:42+01:00 | 0.0.1-alpha |
| 1013.1.8147 | `openEHR-EHR-CLUSTER.transfusion_unit.v0` | Unit transfusion details | DRAFT | 2026-01-07T15:11:48+01:00 | 0.0.1-alpha |
| 1013.1.3758 | `openEHR-EHR-CLUSTER.translocation_variant.v0` | Genetic translocation variant | REVIEWSUSPENDED | 2021-02-10T05:18:27+01:00 | 0.0.1-alpha |
| 1013.1.5054 | `openEHR-EHR-CLUSTER.treatment_preferences.v0` | Treatment preferences | INITIAL | 2026-08-10T11:35:16+02:00 | 0.0.1-alpha |
| 1013.1.1626 | `openEHR-EHR-CLUSTER.triage_priority.v0` | Triage Priority | REJECTED | 2019-09-16T04:02:13+02:00 | 0.0.1 |
| 1013.1.461 | `openEHR-EHR-CLUSTER.tumour_colorectal_staging_non_tnm.v0` | Tumour - Colorectal staging (non-TNM) | DRAFT | 2018-06-14T03:23:04+02:00 | 0.0.1-alpha |
| 1013.1.411 | `openEHR-EHR-CLUSTER.tumour_invasion.v0` | Tumour - direct invasion | DRAFT | 2019-05-13T15:38:31+02:00 | 0.0.1-alpha |
| 1013.1.355 | `openEHR-EHR-CLUSTER.tumour_resection_margins.v0` | Surgical resection margins | DRAFT | 2019-11-01T11:11:36+01:00 | 0.0.1-alpha |
| 1013.1.247 | `openEHR-EHR-CLUSTER.tympanic_ventilation_tube.v1` | Tympanic Ventilation tube | REJECTED | 2012-09-07T12:22:23+02:00 |  |
| 1013.1.6587 | `openEHR-EHR-CLUSTER.vascularisation_ultrasound.v0` | Vascularisation findings on ultrasound | DRAFT | 2022-11-17T01:06:31+01:00 | 0.0.1-alpha |
| 1013.1.2601 | `openEHR-EHR-CLUSTER.ventilator_settings2.v0` | Ventilator settings/findings | INITIAL | 2016-09-17T11:46:50+02:00 | 0.0.1-alpha |
| 1013.1.6959 | `openEHR-EHR-CLUSTER.vibration_finding.v0` | Vibration finding | INITIAL | 2023-07-20T09:22:40+02:00 | 0.0.1-alpha |
| 1013.1.585 | `openEHR-EHR-CLUSTER.waveform.v0` | Waveform | DRAFT | 2018-06-14T08:07:26+02:00 | 0.0.1-alpha |
| 1013.1.7265 | `openEHR-EHR-CLUSTER.who_grade_bone_sarcoma.v1` | WHO histological grade of bone sarcoma | PUBLISHED | 2024-11-18T08:55:21+01:00 | 1.0.0 |
| 1013.1.8075 | `openEHR-EHR-CLUSTER.who_grade_urothelial_neoplasms_1973.v1` | WHO histological grade of urothelial neoplasms (1973) | PUBLISHED | 2026-02-19T12:35:29+01:00 | 1.0.0 |
| 1013.1.8074 | `openEHR-EHR-CLUSTER.who_grade_urothelial_neoplasms_2004.v1` | WHO histological grade of urothelial neoplasms (2004/2016) | PUBLISHED | 2026-02-19T12:36:07+01:00 | 1.0.0 |
| 1013.1.3577 | `openEHR-EHR-CLUSTER.wound_details.v0` | Wound assertion details | INITIAL | 2019-03-25T05:28:36+01:00 | 0.0.1-alpha |
| 1013.1.5349 | `openEHR-EHR-COMPOSITION.advance_care.v0` | Advance care | DRAFT | 2026-08-10T11:36:05+02:00 | 0.0.1-alpha |
| 1013.1.1425 | `openEHR-EHR-COMPOSITION.adverse_reaction_list.v1` | Adverse reaction list | PUBLISHED | 2026-07-28T14:09:27+02:00 | 1.1.5 |
| 1013.1.1656 | `openEHR-EHR-COMPOSITION.care_plan.v0` | Care plan | DRAFT | 2024-01-24T13:51:26+01:00 | 0.0.1-alpha |
| 1013.1.3512 | `openEHR-EHR-COMPOSITION.data_collection.v0` | Data collection | INITIAL | 2021-07-30T07:33:38+02:00 | 0.0.1-alpha |
| 1013.1.5723 | `openEHR-EHR-COMPOSITION.disease_surveillance.v0` | Disease surveillance | INITIAL | 2021-09-07T11:40:13+02:00 | 0.0.1-alpha |
| 1013.1.2048 | `openEHR-EHR-COMPOSITION.empower_odl.v0` | ODL (EMPOWER) | INITIAL | 2015-06-19T09:47:48+02:00 | 0.0.1-alpha |
| 1013.1.120 | `openEHR-EHR-COMPOSITION.encounter.v1` | Encounter | PUBLISHED | 2025-11-21T10:23:03+01:00 | 1.0.12 |
| 1013.1.1968 | `openEHR-EHR-COMPOSITION.event_summary.v0` | Event summary | REVIEWSUSPENDED | 2024-10-01T13:39:06+02:00 | 0.0.1-alpha |
| 1013.1.1679 | `openEHR-EHR-COMPOSITION.family_history.v0` | Family history | DRAFT | 2019-07-24T11:12:57+02:00 | 0.0.1-alpha |
| 1013.1.5797 | `openEHR-EHR-COMPOSITION.health_certificate.v1` | Health certificate | PUBLISHED | 2023-06-16T10:56:48+02:00 | 1.0.0 |
| 1013.1.1969 | `openEHR-EHR-COMPOSITION.health_summary.v1` | Health summary | PUBLISHED | 2025-05-26T02:25:02+02:00 | 1.0.3 |
| 1013.1.1648 | `openEHR-EHR-COMPOSITION.lifestyle_factors.v0` | Lifestyle risk factors | DRAFT | 2024-06-05T12:47:58+02:00 | 0.0.1-alpha |
| 1013.1.286 | `openEHR-EHR-COMPOSITION.medication_list.v1` | Medication list | PUBLISHED | 2025-06-26T03:01:32+02:00 | 1.0.4 |
| 1013.1.2201 | `openEHR-EHR-COMPOSITION.notification.v0` | Notification | DRAFT | 2018-09-03T09:21:59+02:00 | 0.0.1-alpha |
| 1013.1.1630 | `openEHR-EHR-COMPOSITION.obstetric_history.v0` | Obstetric history | DRAFT | 2021-02-10T04:37:21+01:00 | 0.0.1-alpha |
| 1013.1.1657 | `openEHR-EHR-COMPOSITION.pregnancy_summary.v0` | Pregnancy summary | DRAFT | 2022-04-12T07:34:13+02:00 | 0.0.1-alpha |
| 1013.1.121 | `openEHR-EHR-COMPOSITION.prescription.v0` | Prescription | DRAFT | 2016-05-23T23:00:10+02:00 | 0.0.1-alpha |
| 1013.1.287 | `openEHR-EHR-COMPOSITION.problem_list.v1` | Problem list | DEPRECATED | 2020-05-06T10:17:03+02:00 | 1.0.3 |
| 1013.1.4726 | `openEHR-EHR-COMPOSITION.problem_list.v2` | Problem list | PUBLISHED | 2024-11-24T12:13:12+01:00 | 2.0.2 |
| 1013.1.1640 | `openEHR-EHR-COMPOSITION.progress_note.v0` | Progress Note | DRAFT | 2021-09-27T09:52:51+02:00 | 0.0.1-alpha |
| 1013.1.6616 | `openEHR-EHR-COMPOSITION.report-case_classification.v0` | Case classification report | INITIAL | 2022-11-28T05:53:00+01:00 | 0.0.1-alpha |
| 1013.1.6386 | `openEHR-EHR-COMPOSITION.report-clinical_investigation.v0` | Clinical investigation report | DRAFT | 2022-11-28T09:44:12+01:00 | 0.0.1-alpha |
| 1013.1.6367 | `openEHR-EHR-COMPOSITION.report-post_mortem.v0` | Post mortem report | DRAFT | 2022-08-19T06:53:07+02:00 | 0.0.2-alpha |
| 1013.1.1322 | `openEHR-EHR-COMPOSITION.report-procedure.v1` | Procedure report | PUBLISHED | 2026-06-25T13:13:24+02:00 | 1.1.0 |
| 1013.1.1324 | `openEHR-EHR-COMPOSITION.report-result.v1` | Result report | PUBLISHED | 2026-06-25T13:14:48+02:00 | 1.1.0 |
| 1013.1.677 | `openEHR-EHR-COMPOSITION.report.v1` | Report | PUBLISHED | 2026-07-07T16:27:11+02:00 | 1.3.1 |
| 1013.1.18 | `openEHR-EHR-COMPOSITION.request.v1` | Request for service | PUBLISHED | 2026-09-12T14:48:51+02:00 | 1.1.4 |
| 1013.1.1325 | `openEHR-EHR-COMPOSITION.review.v0` | Review | DRAFT | 2026-06-18T11:38:23+02:00 | 0.0.1-alpha |
| 1013.1.2430 | `openEHR-EHR-COMPOSITION.self_monitoring.v0` | Self monitoring | REJECTED | 2022-08-04T08:40:02+02:00 | 0.0.1 |
| 1013.1.6343 | `openEHR-EHR-COMPOSITION.self_reported_data.v1` | Self-reported data | PUBLISHED | 2026-09-17T11:17:31+02:00 | 1.1.6 |
| 1013.1.1646 | `openEHR-EHR-COMPOSITION.social_summary.v0` | Social summary | DRAFT | 2021-02-10T05:12:35+01:00 | 0.0.1-alpha |
| 1013.1.1906 | `openEHR-EHR-COMPOSITION.therapeutic_precautions.v0` | Therapeutic precautions | DRAFT | 2021-11-30T02:09:22+01:00 | 0.0.1-alpha |
| 1013.1.1970 | `openEHR-EHR-COMPOSITION.transfer_summary.v1` | Transfer of care summary | PUBLISHED | 2025-05-26T03:37:38+02:00 | 1.0.2 |
| 1013.1.1424 | `openEHR-EHR-COMPOSITION.vaccination_list.v0` | Vaccination list | DRAFT | 2021-07-05T03:51:46+02:00 | 0.0.1-alpha |
| 1013.1.281 | `openEHR-EHR-ELEMENT.last_normal_menstrual_period.v1` | Last Normal Menstrual Period | REJECTED | 2015-10-29T07:42:19+01:00 |  |
| 1013.1.282 | `openEHR-EHR-ELEMENT.menstrual_cycle_day.v1` | Current Day of Menstrual cycle | REJECTED | 2015-10-29T07:43:37+01:00 |  |
| 1013.1.1427 | `openEHR-EHR-EVALUATION.absence.v1` | Absence of Information | DEPRECATED | 2019-10-28T06:05:43+01:00 | 1.0.2 |
| 1013.1.4213 | `openEHR-EHR-EVALUATION.absence.v2` | Absence of information | PUBLISHED | 2024-11-24T09:49:29+01:00 | 2.0.2 |
| 1013.1.4274 | `openEHR-EHR-EVALUATION.advance_care_directive.v1` | Advance care directive | DEPRECATED | 2022-09-09T08:44:30+02:00 | 1.2.1 |
| 1013.1.6499 | `openEHR-EHR-EVALUATION.advance_care_directive.v2` | Advance care directive | PUBLISHED | 2026-05-04T15:45:47+02:00 | 2.0.4 |
| 1013.1.1752 | `openEHR-EHR-EVALUATION.advance_decision_refuse_treatment_uk.v0` | Advance decision to refuse treatment | REJECTED | 2019-11-28T13:14:20+01:00 | 0.0.1 |
| 1013.1.4902 | `openEHR-EHR-EVALUATION.advance_intervention_decisions.v1` | Advance intervention decisions | PUBLISHED | 2021-04-21T13:10:12+02:00 | 1.0.0 |
| 1013.1.197 | `openEHR-EHR-EVALUATION.adverse_reaction.v1` | Adverse Reaction | REJECTED | 2015-11-19T10:30:35+01:00 |  |
| 1013.1.1713 | `openEHR-EHR-EVALUATION.adverse_reaction_risk.v1` | Adverse reaction risk | DEPRECATED | 2023-09-11T14:44:11+02:00 | 1.2.3 |
| 1013.1.7022 | `openEHR-EHR-EVALUATION.adverse_reaction_risk.v2` | Adverse reaction risk | PUBLISHED | 2026-08-12T14:46:35+02:00 | 2.0.2 |
| 1013.1.1521 | `openEHR-EHR-EVALUATION.alcohol_consumption_summary.v1` | Alcohol consumption summary | PUBLISHED | 2026-04-23T15:20:31+02:00 | 1.1.0 |
| 1013.1.190 | `openEHR-EHR-EVALUATION.alert.v1` | Alert | REJECTED | 2016-11-07T08:25:27+01:00 |  |
| 1013.1.5972 | `openEHR-EHR-EVALUATION.art_cycle_summary.v1` | Assisted reproduction treatment cycle summary | PUBLISHED | 2022-05-05T09:30:39+02:00 | 1.0.0 |
| 1013.1.6043 | `openEHR-EHR-EVALUATION.biospecimen_summary-semen.v0` | Biospecimen summary - semen | REJECTED | 2022-12-07T14:36:59+01:00 | 0.0.1 |
| 1013.1.5604 | `openEHR-EHR-EVALUATION.birth_summary.v0` | Birth summary | DRAFT | 2025-04-09T12:22:07+02:00 | 0.0.1-alpha |
| 1013.1.6354 | `openEHR-EHR-EVALUATION.blood_group.v0` | Blood group | DRAFT | 2026-08-13T20:01:41+02:00 | 0.0.1-alpha |
| 1013.1.4889 | `openEHR-EHR-EVALUATION.breast_feeding_summary.v0` | Breast feeding summary | DRAFT | 2022-04-08T06:37:56+02:00 | 0.0.1-alpha |
| 1013.1.5606 | `openEHR-EHR-EVALUATION.cause_of_death.v1` | Cause of death | PUBLISHED | 2024-11-26T07:31:06+01:00 | 1.0.2 |
| 1013.1.191 | `openEHR-EHR-EVALUATION.check_list-medication.v1` | A check list for medications | REJECTED | 2015-04-16T09:21:46+02:00 |  |
| 1013.1.203 | `openEHR-EHR-EVALUATION.check_list.v1` | A health oriented check list | REJECTED | 2015-04-16T09:37:05+02:00 |  |
| 1013.1.720 | `openEHR-EHR-EVALUATION.citation.v0` | Citation | REJECTED | 2020-05-12T09:11:59+02:00 | 0.0.1 |
| 1013.1.409 | `openEHR-EHR-EVALUATION.clinical_synopsis.v1` | Clinical synopsis | PUBLISHED | 2026-11-04T22:05:14+01:00 | 1.0.6 |
| 1013.1.3155 | `openEHR-EHR-EVALUATION.communication_capability.v1` | Communication capability | PUBLISHED | 2025-10-09T09:01:04+02:00 | 1.0.3 |
| 1013.1.4419 | `openEHR-EHR-EVALUATION.comorbidity_summary_covid.v0` | Condition summary | INITIAL | 2020-03-18T09:57:23+01:00 | 0.0.1-alpha |
| 1013.1.2432 | `openEHR-EHR-EVALUATION.consumer_note.v0` | Consumer note | DRAFT | 2016-05-04T08:48:13+02:00 | 0.0.1-alpha |
| 1013.1.2175 | `openEHR-EHR-EVALUATION.container.v0` | Container | DRAFT | 2016-08-31T08:04:59+02:00 | 0.0.1-alpha |
| 1013.1.3273 | `openEHR-EHR-EVALUATION.contraceptive_summary.v1` | Contraceptive use summary | PUBLISHED | 2021-01-20T14:23:02+01:00 | 1.0.0 |
| 1013.1.2106 | `openEHR-EHR-EVALUATION.contraindication-intravitreal_antiVEGF.v0` | Contraindication of intravitreal anti-VEGF injections | INITIAL | 2015-06-25T01:42:16+02:00 | 0.0.1-alpha |
| 1013.1.1388 | `openEHR-EHR-EVALUATION.contraindication.v1` | Contraindication | PUBLISHED | 2025-05-26T04:07:35+02:00 | 1.1.2 |
| 1013.1.5460 | `openEHR-EHR-EVALUATION.death_details_old.v0` | Death details | REJECTED | 2022-02-18T10:13:15+01:00 | 0.0.3 |
| 1013.1.5605 | `openEHR-EHR-EVALUATION.death_summary.v1` | Death summary | PUBLISHED | 2023-05-31T08:50:09+02:00 | 1.0.0 |
| 1013.1.2381 | `openEHR-EHR-EVALUATION.device_summary.v0` | Medical device summary | DRAFT | 2021-02-10T08:29:38+01:00 | 0.0.1-alpha |
| 1013.1.3562 | `openEHR-EHR-EVALUATION.dietary_habit_screening.v0` | Kostvanor, screening | INITIAL | 2018-12-12T11:11:27+01:00 | 0.0.1-alpha |
| 1013.1.1670 | `openEHR-EHR-EVALUATION.differential_diagnoses.v1` | Differential diagnoses | PUBLISHED | 2021-01-12T10:34:16+01:00 | 1.0.0 |
| 1013.1.4890 | `openEHR-EHR-EVALUATION.dnacpr_decision_uk.v0` | DNACPR decision | REJECTED | 2021-04-13T04:19:39+02:00 | 0.0.1 |
| 1013.1.7953 | `openEHR-EHR-EVALUATION.donation_summary.v0` | Biological donation summary | INITIAL | 2025-07-14T08:10:08+02:00 | 0.0.1-alpha |
| 1013.1.1755 | `openEHR-EHR-EVALUATION.dr_screening_convenient.v0` | DR screening convenient | INITIAL | 2016-07-25T11:34:05+02:00 | 0.0.1-alpha |
| 1013.1.3184 | `openEHR-EHR-EVALUATION.education_summary.v1` | Education summary | PUBLISHED | 2022-09-26T10:53:36+02:00 | 1.0.2 |
| 1013.1.6942 | `openEHR-EHR-EVALUATION.environmental_survey.v0` | Environmental survey | INITIAL | 2024-10-28T15:04:33+01:00 | 0.0.1-alpha |
| 1013.1.4340 | `openEHR-EHR-EVALUATION.estimated_date_delivery.v0` | Estimated date of delivery (EDD) | REVIEWSUSPENDED | 2025-10-23T09:41:10+02:00 | 0.0.1-alpha |
| 1013.1.5162 | `openEHR-EHR-EVALUATION.ethnicity.v1` | Ethnic identity | PUBLISHED | 2025-08-04T15:41:26+02:00 | 1.1.5 |
| 1013.1.7549 | `openEHR-EHR-EVALUATION.event_investigation_classification.v0` | Health event investigation classification | INITIAL | 2024-12-30T03:25:34+01:00 | 0.0.1-alpha |
| 1013.1.301 | `openEHR-EHR-EVALUATION.excluded-adverse.v1` | Exclusion of adverse reaction caused by specific agent | REJECTED | 2009-07-28T16:13:45+02:00 |  |
| 1013.1.302 | `openEHR-EHR-EVALUATION.excluded-condition.v1` | Statement of exclusion of specific condition | REJECTED | 2009-07-28T16:13:48+02:00 |  |
| 1013.1.303 | `openEHR-EHR-EVALUATION.excluded-intervention.v1` | Statement of exclusions regarding interventions | REJECTED | 2013-09-03T21:17:16+02:00 |  |
| 1013.1.300 | `openEHR-EHR-EVALUATION.excluded.v1` | General statement of exclusions or states | REJECTED | 2009-07-28T16:13:54+02:00 |  |
| 1013.1.618 | `openEHR-EHR-EVALUATION.exclusion-adverse.v1` | Exclusion statement - Adverse Reaction | REJECTED | 2013-09-03T17:31:35+02:00 |  |
| 1013.1.1480 | `openEHR-EHR-EVALUATION.exclusion-adverse_reaction.v1` | Exclusion of an Adverse Reaction | REJECTED | 2017-03-15T08:14:08+01:00 |  |
| 1013.1.625 | `openEHR-EHR-EVALUATION.exclusion-family_history.v1` | Exclusion of Family History | REJECTED | 2017-03-15T08:14:34+01:00 |  |
| 1013.1.619 | `openEHR-EHR-EVALUATION.exclusion-medication.v1` | Exclusion of a Medication | REJECTED | 2017-03-15T08:12:33+01:00 |  |
| 1013.1.620 | `openEHR-EHR-EVALUATION.exclusion-problem_diagnosis.v1` | Exclusion of a Problem/Diagnosis | REJECTED | 2017-03-15T08:13:10+01:00 | 0.0.1 |
| 1013.1.1485 | `openEHR-EHR-EVALUATION.exclusion-procedure.v1` | Exclusion of a Procedure | REJECTED | 2017-03-15T08:13:34+01:00 |  |
| 1013.1.617 | `openEHR-EHR-EVALUATION.exclusion.v1` | Exclusion statement | DEPRECATED | 2017-03-15T08:15:14+01:00 | 1.0.1 |
| 1013.1.2733 | `openEHR-EHR-EVALUATION.exclusion_global.v1` | Exclusion - global | PUBLISHED | 2020-05-11T08:31:51+02:00 | 1.1.4 |
| 1013.1.2737 | `openEHR-EHR-EVALUATION.exclusion_specific.v1` | Exclusion - specific | PUBLISHED | 2019-11-08T06:19:36+01:00 | 1.0.2 |
| 1013.1.1649 | `openEHR-EHR-EVALUATION.exposure.v0` | Exposure | DRAFT | 2017-09-29T09:19:26+02:00 | 0.0.1-alpha |
| 1013.1.1900 | `openEHR-EHR-EVALUATION.family_history.v1` | Family history | DEPRECATED | 2016-06-29T10:54:17+02:00 | 1.0.1 |
| 1013.1.2469 | `openEHR-EHR-EVALUATION.family_history.v2` | Family history summary | REASSESS_DRAFT | 2026-03-31T13:25:48+02:00 | 2.0.5-alpha |
| 1013.1.2989 | `openEHR-EHR-EVALUATION.financial_summary.v1` | Financial summary | PUBLISHED | 2024-07-01T14:07:24+02:00 | 1.0.2 |
| 1013.1.2755 | `openEHR-EHR-EVALUATION.food_nutrition_summary.v0` | Food and nutrition summary | DRAFT | 2023-04-21T15:18:54+02:00 | 0.0.1-alpha |
| 1013.1.8348 | `openEHR-EHR-EVALUATION.functional_support_requirements.v0` | Functional support requirements | TEAMREVIEW | 2026-11-10T07:20:54+01:00 | 0.0.1-alpha |
| 1013.1.4349 | `openEHR-EHR-EVALUATION.gambling_summary.v0` | Gambling summary | DRAFT | 2020-02-13T01:17:11+01:00 | 0.0.1-alpha |
| 1013.1.3715 | `openEHR-EHR-EVALUATION.gender.v1` | Gender | PUBLISHED | 2026-08-17T15:49:52+02:00 | 1.1.5 |
| 1013.1.124 | `openEHR-EHR-EVALUATION.goal.v1` | Goal | PUBLISHED | 2023-01-18T14:51:09+01:00 | 1.1.2 |
| 1013.1.8191 | `openEHR-EHR-EVALUATION.hand_dominance.v1` | Hand dominance | PUBLISHED | 2026-09-17T10:03:04+02:00 | 1.0.0 |
| 1013.1.4396 | `openEHR-EHR-EVALUATION.health_risk-covid.v0` | Covid-19 infection risk assessment | INITIAL | 2020-03-22T16:22:49+01:00 | 0.0.1-alpha |
| 1013.1.176 | `openEHR-EHR-EVALUATION.health_risk.v1` | Health risk assessment | PUBLISHED | 2025-07-28T14:46:21+02:00 | 1.2.2 |
| 1013.1.3287 | `openEHR-EHR-EVALUATION.housing_summary.v1` | Housing summary | PUBLISHED | 2025-05-26T04:19:44+02:00 | 1.1.0 |
| 1013.1.8345 | `openEHR-EHR-EVALUATION.impairment_summary.v0` | Impairment summary | TEAMREVIEW | 2026-11-10T07:17:38+01:00 | 0.0.1-alpha |
| 1013.1.7294 | `openEHR-EHR-EVALUATION.implanted_device_summary.v0` | Implanted medical device summary | INITIAL | 2025-10-03T03:59:07+02:00 | 0.0.1-alpha |
| 1013.1.1675 | `openEHR-EHR-EVALUATION.infant_feeding.v0` | Infant feeding summary | DRAFT | 2022-07-20T10:02:00+02:00 | 0.0.1-alpha |
| 1013.1.6428 | `openEHR-EHR-EVALUATION.infectious_disease_investigation_classification.v0` | Infectious disease investigation classification | DRAFT | 2023-10-29T09:34:04+01:00 | 0.0.1-alpha |
| 1013.1.1918 | `openEHR-EHR-EVALUATION.infectious_disease_summary.v0` | Infectious disease summary | DRAFT | 2019-09-24T10:18:02+02:00 | 0.0.1-alpha |
| 1013.1.4449 | `openEHR-EHR-EVALUATION.infectious_exposure.v0` | Infectious exposure investigation | REJECTED | 2022-06-25T06:49:15+02:00 | 0.0.1 |
| 1013.1.7523 | `openEHR-EHR-EVALUATION.infectious_investigation_classification.v0` | Infectious disease investigation classification | INITIAL | 2025-12-05T06:38:14+01:00 | 0.0.1-alpha |
| 1013.1.6607 | `openEHR-EHR-EVALUATION.intervention_summary.v1` | Intervention summary | PUBLISHED | 2026-06-24T14:49:15+02:00 | 1.0.2 |
| 1013.1.4801 | `openEHR-EHR-EVALUATION.issue.v0` | Issue | INITIAL | 2020-06-07T07:46:14+02:00 | 0.0.1-alpha |
| 1013.1.5655 | `openEHR-EHR-EVALUATION.last_menstrual_period.v1` | Last menstrual period | PUBLISHED | 2022-03-21T15:32:13+01:00 | 1.0.0 |
| 1013.1.3280 | `openEHR-EHR-EVALUATION.living_arrangement.v0` | Living arrangement | REVIEWSUSPENDED | 2023-05-08T15:54:21+02:00 | 0.0.1-alpha |
| 1013.1.7534 | `openEHR-EHR-EVALUATION.living_arrangement_hl.v0` | Living arrangement | INITIAL | 2024-10-28T16:34:08+01:00 | 0.0.1-alpha |
| 1013.1.2105 | `openEHR-EHR-EVALUATION.long_term_process_enrollment-antiVEGF_AMD.v0` | Enrollment in intravitreal anti-VEGF therapy for wet AMD | INITIAL | 2015-06-25T01:03:23+02:00 | 0.0.1-alpha |
| 1013.1.2104 | `openEHR-EHR-EVALUATION.long_term_process_enrollment.v0` | Enrollment in a long-term healthcare process | INITIAL | 2026-10-27T15:26:56+01:00 | 0.0.1-alpha |
| 1013.1.7550 | `openEHR-EHR-EVALUATION.management_summary.v0` | Management summary | INITIAL | 2024-10-29T18:37:41+01:00 | 0.0.1-alpha |
| 1013.1.6429 | `openEHR-EHR-EVALUATION.maternal_mortality_classification.v0` | Maternal mortality classification | DRAFT | 2022-08-01T08:09:40+02:00 | 0.0.1-alpha |
| 1013.1.1866 | `openEHR-EHR-EVALUATION.medication_safety_event.v1` | Medication safety event | INITIAL | 2015-02-18T21:08:15+01:00 |  |
| 1013.1.2825 | `openEHR-EHR-EVALUATION.medication_summary.v1` | Medication summary | PUBLISHED | 2023-03-20T19:49:59+01:00 | 1.0.1 |
| 1013.1.1662 | `openEHR-EHR-EVALUATION.menstruation_summary.v1` | Menstruation summary | PUBLISHED | 2023-08-09T10:16:04+02:00 | 1.1.1 |
| 1013.1.4824 | `openEHR-EHR-EVALUATION.mental_capacity.v0` | Mental capacity | INITIAL | 2020-06-11T10:51:43+02:00 | 0.0.1-alpha |
| 1013.1.7654 | `openEHR-EHR-EVALUATION.obstetric_summary-JM.v0` | Obstetric summary | INITIAL | 2024-12-24T11:57:07+01:00 | 0.0.1-alpha |
| 1013.1.1093 | `openEHR-EHR-EVALUATION.obstetric_summary.v1` | Obstetric summary | PUBLISHED | 2021-02-10T07:59:20+01:00 | 1.0.1 |
| 1013.1.2965 | `openEHR-EHR-EVALUATION.occupation_summary.v1` | Occupation summary | PUBLISHED | 2025-05-26T04:27:44+02:00 | 1.0.3 |
| 1013.1.8425 | `openEHR-EHR-EVALUATION.peak_intraocular_pressure_assertion.v0` | Peak intraocular pressure assertion | INITIAL | 2026-07-31T13:19:18+02:00 | 0.0.1-alpha |
| 1013.1.4876 | `openEHR-EHR-EVALUATION.personal_safety_summary.v0` | Personal safety summary | INITIAL | 2020-12-17T03:34:33+01:00 | 0.0.1-alpha |
| 1013.1.7084 | `openEHR-EHR-EVALUATION.pharmacogenetic_gene_profile.v0` | Pharmacogenetic gene profile | DRAFT | 2023-12-01T15:31:33+01:00 | 0.0.1-alpha |
| 1013.1.2877 | `openEHR-EHR-EVALUATION.physical_activity_summary.v0` | Physical activity summary | DRAFT | 2020-10-01T05:50:36+02:00 | 0.0.1-alpha |
| 1013.1.5973 | `openEHR-EHR-EVALUATION.physical_appearance.v0` | Physical appearance of an individual | REVIEWSUSPENDED | 2022-05-19T12:07:47+02:00 | 0.0.1-alpha |
| 1013.1.7548 | `openEHR-EHR-EVALUATION.poisoning_summary.v0` | Poisoning event summary | INITIAL | 2024-12-26T06:22:00+01:00 | 0.0.1-alpha |
| 1013.1.4161 | `openEHR-EHR-EVALUATION.pp_cohort.v0` | Phenopacket cohort | INITIAL | 2019-09-26T02:16:08+02:00 | 0.0.1-alpha |
| 1013.1.4166 | `openEHR-EHR-EVALUATION.pp_interpretation.v0` | Phenopacket interpretation | INITIAL | 2019-09-26T14:19:55+02:00 | 0.0.1-alpha |
| 1013.1.2593 | `openEHR-EHR-EVALUATION.precaution.v1` | Precaution | PUBLISHED | 2024-01-23T13:44:27+01:00 | 1.1.0 |
| 1013.1.1300 | `openEHR-EHR-EVALUATION.pregnancy_bf_status.v0` | Pregnancy/breast feeding status | REJECTED | 2020-09-16T14:20:53+02:00 | 0.0.1 |
| 1013.1.6329 | `openEHR-EHR-EVALUATION.pregnancy_care_summary.v0` | Pregnancy care summary | DRAFT | 2024-12-24T12:12:37+01:00 | 0.0.1-alpha |
| 1013.1.8224 | `openEHR-EHR-EVALUATION.pregnancy_precautions.v0` | Pregnancy precautions | TEAMREVIEW | 2026-10-15T04:02:14+02:00 | 0.0.1-alpha |
| 1013.1.6192 | `openEHR-EHR-EVALUATION.pregnancy_status.v0` | Current pregnancy status | INITIAL | 2024-03-08T03:12:34+01:00 | 0.0.1-alpha |
| 1013.1.177 | `openEHR-EHR-EVALUATION.pregnancy_summary.v0` | Pregnancy summary | DRAFT | 2025-07-22T01:51:22+02:00 | 0.0.1-alpha |
| 1013.1.168 | `openEHR-EHR-EVALUATION.problem-diagnosis.v1` | Diagnosis | REJECTED | 2013-03-06T21:21:15+01:00 |  |
| 1013.1.169 | `openEHR-EHR-EVALUATION.problem_diagnosis.v1` | Problem/Diagnosis | PUBLISHED | 2026-07-16T11:11:28+02:00 | 1.7.4 |
| 1013.1.4420 | `openEHR-EHR-EVALUATION.procedure_summary_covid.v0` | COVID - procedure summary | INITIAL | 2020-04-21T06:47:15+02:00 | 0.0.1-alpha |
| 1013.1.290 | `openEHR-EHR-EVALUATION.reason_for_encounter.v1` | Reason for encounter | PUBLISHED | 2025-09-01T00:23:04+02:00 | 1.0.3 |
| 1013.1.1822 | `openEHR-EHR-EVALUATION.recommendation-DR_treatment.v1` | Recommendation of treatment for diabetic retinopathy | INITIAL | 2014-12-05T17:41:44+01:00 |  |
| 1013.1.1823 | `openEHR-EHR-EVALUATION.recommendation-amd_treatment.v0` | Recommendation on the treatment of AMD | INITIAL | 2015-07-06T00:03:20+02:00 | 0.0.1-alpha |
| 1013.1.2159 | `openEHR-EHR-EVALUATION.recommendation-glaucoma_treatment.v0` | Recommended treatment for glaucoma | INITIAL | 2015-09-08T19:27:11+02:00 | 0.0.1-alpha |
| 1013.1.1380 | `openEHR-EHR-EVALUATION.recommendation.v1` | Recommendation | DEPRECATED | 2021-09-16T09:27:12+02:00 | 1.1.3 |
| 1013.1.5755 | `openEHR-EHR-EVALUATION.recommendation.v2` | Recommendation | PUBLISHED | 2026-11-04T22:10:25+01:00 | 2.0.5 |
| 1013.1.125 | `openEHR-EHR-EVALUATION.risk-family_history.v1` | Risk of condition based on family history | REJECTED | 2015-03-04T04:48:10+01:00 |  |
| 1013.1.5804 | `openEHR-EHR-EVALUATION.sdoh_assessment.v0` | Social determinants of health (SDOH) self-assessment | INITIAL | 2021-10-07T02:24:48+02:00 | 0.0.1-alpha |
| 1013.1.4351 | `openEHR-EHR-EVALUATION.sexual_health_summary.v0` | Sexual health summary | DRAFT | 2020-02-13T07:26:29+01:00 | 0.0.1-alpha |
| 1013.1.2817 | `openEHR-EHR-EVALUATION.smokeless_tobacco_summary.v1` | Smokeless tobacco summary | PUBLISHED | 2026-04-24T10:18:33+02:00 | 1.1.1 |
| 1013.1.7662 | `openEHR-EHR-EVALUATION.social_network-JM.v0` | Social network | INITIAL | 2024-12-26T06:52:48+01:00 | 0.0.1-alpha |
| 1013.1.3288 | `openEHR-EHR-EVALUATION.social_network.v1` | Social network | PUBLISHED | 2022-10-13T10:48:17+02:00 | 1.0.1 |
| 1013.1.2378 | `openEHR-EHR-EVALUATION.social_summary.v1` | Social summary | PUBLISHED | 2020-04-23T07:14:26+02:00 | 1.1.1 |
| 1013.1.1418 | `openEHR-EHR-EVALUATION.source.v0` | Source information | DRAFT | 2024-07-01T19:48:02+02:00 | 0.0.1-alpha |
| 1013.1.6013 | `openEHR-EHR-EVALUATION.specimen_summary.v1` | Specimen summary | PUBLISHED | 2023-07-13T10:06:27+02:00 | 1.1.0 |
| 1013.1.362 | `openEHR-EHR-EVALUATION.substance_use_summary-alcohol.v1` | Alcohol Use Summary | REJECTED | 2015-12-07T14:48:55+01:00 |  |
| 1013.1.363 | `openEHR-EHR-EVALUATION.substance_use_summary-tobacco.v1` | Tobacco Use Summary | REJECTED | 2015-12-07T14:48:44+01:00 |  |
| 1013.1.354 | `openEHR-EHR-EVALUATION.substance_use_summary.v1` | Substance use summary | PUBLISHED | 2026-03-25T14:23:55+01:00 | 1.0.2 |
| 1013.1.2466 | `openEHR-EHR-EVALUATION.tobacco_smoking_summary.v1` | Tobacco smoking summary | DEPRECATED | 2026-04-23T08:18:42+02:00 | 1.1.1 |
| 1013.1.8284 | `openEHR-EHR-EVALUATION.tobacco_smoking_summary.v2` | Tobacco smoking summary | PUBLISHED | 2026-04-24T10:56:15+02:00 | 2.0.1 |
| 1013.1.1520 | `openEHR-EHR-EVALUATION.tobacco_use_summary.v1` | Tobacco Use Summary | REJECTED | 2016-06-06T15:44:06+02:00 | 0.0.1 |
| 1013.1.5725 | `openEHR-EHR-EVALUATION.transfusion_summary.v0` | Transfusion summary | INITIAL | 2021-09-07T11:55:31+02:00 | 0.0.1-alpha |
| 1013.1.4873 | `openEHR-EHR-EVALUATION.transport_access_summary.v0` | Transport access summary | INITIAL | 2020-12-17T03:39:42+01:00 | 0.0.1-alpha |
| 1013.1.4427 | `openEHR-EHR-EVALUATION.travel_summary.v0` | Travel summary | REJECTED | 2022-08-29T03:08:53+02:00 | 0.0.1 |
| 1013.1.1389 | `openEHR-EHR-EVALUATION.vaccination_summary.v0` | Vaccination summary | DRAFT | 2024-02-19T11:01:33+01:00 | 0.0.1-alpha |
| 1013.1.4823 | `openEHR-EHR-EVALUATION.vaping_summary.v0` | Vaping summary | INITIAL | 2026-02-05T04:51:12+01:00 | 0.0.1-alpha |
| 1013.1.7340 | `openEHR-EHR-INSTRUCTION.assisted_ventilation.v0` | Assisted ventilation order | INITIAL | 2024-06-04T10:33:08+02:00 | 0.0.1-alpha |
| 1013.1.7253 | `openEHR-EHR-INSTRUCTION.care_pathway_order.v1` | Care pathway order | PUBLISHED | 2026-09-21T13:29:30+02:00 | 1.0.0 |
| 1013.1.1653 | `openEHR-EHR-INSTRUCTION.care_plan_request.v0` | Care plan request | DRAFT | 2018-07-23T09:56:10+02:00 | 0.0.1-alpha |
| 1013.1.39 | `openEHR-EHR-INSTRUCTION.follow_up.v1` | Follow up instruction | REJECTED | 2015-01-22T09:50:20+01:00 |  |
| 1013.1.2742 | `openEHR-EHR-INSTRUCTION.health_education_request.v0` | Health education request | DRAFT | 2019-04-28T04:59:41+02:00 | 0.0.1-alpha |
| 1013.1.126 | `openEHR-EHR-INSTRUCTION.imaging.v1` | Imaging request | REJECTED | 2013-01-18T08:38:04+01:00 |  |
| 1013.1.1302 | `openEHR-EHR-INSTRUCTION.informed_consent_request.v0` | Informed consent request | DRAFT | 2023-02-16T16:53:30+01:00 | 0.0.1-alpha |
| 1013.1.180 | `openEHR-EHR-INSTRUCTION.intravenous_fluid_order.v1` | Intravenous fluid order | REJECTED | 2015-01-22T09:51:23+01:00 |  |
| 1013.1.127 | `openEHR-EHR-INSTRUCTION.medication.v1` | Medication order | REJECTED | 2013-07-05T19:34:44+02:00 |  |
| 1013.1.1445 | `openEHR-EHR-INSTRUCTION.medication_order.v1` | Medication order | DEPRECATED | 2018-02-16T12:55:07+01:00 | 1.0.2 |
| 1013.1.3124 | `openEHR-EHR-INSTRUCTION.medication_order.v2` | Medication order | DEPRECATED | 2021-12-24T12:38:03+01:00 | 2.0.5 |
| 1013.1.5946 | `openEHR-EHR-INSTRUCTION.medication_order.v3` | Medication order | PUBLISHED | 2026-07-16T11:07:48+02:00 | 3.2.1 |
| 1013.1.205 | `openEHR-EHR-INSTRUCTION.non_drug_therapy.v1` | Non-drug therapy | REJECTED | 2015-01-22T09:51:56+01:00 |  |
| 1013.1.2431 | `openEHR-EHR-INSTRUCTION.notification.v0` | Notification | DRAFT | 2016-05-04T08:34:52+02:00 | 0.0.1-alpha |
| 1013.1.8 | `openEHR-EHR-INSTRUCTION.procedure.v1` | Procedure instruction | REJECTED | 2009-03-13T17:16:28+01:00 |  |
| 1013.1.1365 | `openEHR-EHR-INSTRUCTION.request-imaging_exam.v1` | Imaging Examination Request | REJECTED | 2018-06-26T09:03:06+02:00 |  |
| 1013.1.678 | `openEHR-EHR-INSTRUCTION.request-lab_test.v0` | Laboratory Test request | REJECTED | 2018-06-26T09:04:11+02:00 | 0.0.1 |
| 1013.1.722 | `openEHR-EHR-INSTRUCTION.request-procedure.v0` | Procedure request | REJECTED | 2018-06-26T09:03:40+02:00 | 0.0.1 |
| 1013.1.613 | `openEHR-EHR-INSTRUCTION.request-referral.v1` | Referral request | REJECTED | 2016-08-31T08:15:20+02:00 |  |
| 1013.1.1757 | `openEHR-EHR-INSTRUCTION.request-report.v1` | Diagnostic report request | INITIAL | 2014-08-18T20:47:56+02:00 |  |
| 1013.1.7240 | `openEHR-EHR-INSTRUCTION.service_request-imaging_examination.v0` | Imaging examination request | INITIAL | 2024-04-19T04:29:10+02:00 | 0.0.1-alpha |
| 1013.1.7230 | `openEHR-EHR-INSTRUCTION.service_request-laboratory_test.v0` | Laboratory test request | INITIAL | 2024-04-19T05:08:43+02:00 | 0.0.1-alpha |
| 1013.1.614 | `openEHR-EHR-INSTRUCTION.service_request.v1` | Service request | REASSESS_DRAFT | 2026-07-15T00:01:58+02:00 | 1.1.3-alpha |
| 1013.1.5692 | `openEHR-EHR-INSTRUCTION.supplemental_oxygen_order.v0` | Supplemental oxygen order | DRAFT | 2023-06-16T12:45:16+02:00 | 0.0.1-alpha |
| 1013.1.4588 | `openEHR-EHR-INSTRUCTION.therapeutic_activity_order.v0` | Therapeutic activity | INITIAL | 2020-04-05T14:08:31+02:00 | 0.0.1-alpha |
| 1013.1.6811 | `openEHR-EHR-INSTRUCTION.therapeutic_item_order.v1` | Therapeutic item order | PUBLISHED | 2023-11-20T12:56:29+01:00 | 1.0.0 |
| 1013.1.202 | `openEHR-EHR-INSTRUCTION.transfusion_order.v0` | Transfusion order | DRAFT | 2018-07-23T10:11:43+02:00 | 0.0.1-alpha |
| 1013.1.38 | `openEHR-EHR-ITEM_TREE.follow_up.v1` | Follow up | REJECTED | 2015-02-05T06:54:50+01:00 |  |
| 1013.1.201 | `openEHR-EHR-ITEM_TREE.gas_administration.v1` | Gas administration | REJECTED | 2015-02-05T06:54:59+01:00 |  |
| 1013.1.155 | `openEHR-EHR-ITEM_TREE.imaging.v1` | Imaging data | REJECTED | 2013-01-18T08:38:15+01:00 |  |
| 1013.1.181 | `openEHR-EHR-ITEM_TREE.intravenous_fluids.v1` | Intravenous fluids | REJECTED | 2015-02-05T06:55:08+01:00 |  |
| 1013.1.283 | `openEHR-EHR-ITEM_TREE.medication-formulation.v1` | Medication description involving a formulation | REJECTED | 2013-09-03T21:19:33+02:00 |  |
| 1013.1.288 | `openEHR-EHR-ITEM_TREE.medication-vaccine.v1` | Vaccine description | REJECTED | 2013-09-03T21:18:33+02:00 |  |
| 1013.1.156 | `openEHR-EHR-ITEM_TREE.medication.v1` | Medication description | REJECTED | 2013-09-03T21:19:52+02:00 |  |
| 1013.1.157 | `openEHR-EHR-ITEM_TREE.procedure.v1` | Procedure | REJECTED | 2009-03-13T17:16:45+01:00 |  |
| 1013.1.3659 | `openEHR-EHR-OBSERVATION.abbey_pain_scale.v0` | Abbey pain scale | DRAFT | 2020-11-03T03:53:50+01:00 | 0.0.1-alpha |
| 1013.1.3660 | `openEHR-EHR-OBSERVATION.abc_score_massive_transfusion.v0` | Assessment of Blood Consumption (ABC) Score | DRAFT | 2019-07-11T10:00:48+02:00 | 0.0.1-alpha |
| 1013.1.3662 | `openEHR-EHR-OBSERVATION.abc_stroke_risk_score.v0` | ABC-stroke risk score | DRAFT | 2019-03-05T06:14:06+01:00 | 0.0.1-alpha |
| 1013.1.3664 | `openEHR-EHR-OBSERVATION.abcd2_score.v0` | ABCD2 score | DRAFT | 2019-03-06T07:34:28+01:00 | 0.0.1-alpha |
| 1013.1.1641 | `openEHR-EHR-OBSERVATION.acoustic_reflex_result.v0` | Acoustic reflex test result | DRAFT | 2018-10-05T10:28:03+02:00 | 0.0.1-alpha |
| 1013.1.3317 | `openEHR-EHR-OBSERVATION.acvpu.v1` | ACVPU scale | PUBLISHED | 2024-11-24T09:57:08+01:00 | 1.0.1 |
| 1013.1.5658 | `openEHR-EHR-OBSERVATION.adverse_reaction_monitoring.v1` | Adverse reaction monitoring | PUBLISHED | 2025-09-01T02:39:57+02:00 | 1.0.2 |
| 1013.1.6902 | `openEHR-EHR-OBSERVATION.adverse_reaction_screening.v1` | Adverse reaction screening questionnaire | PUBLISHED | 2025-07-03T16:56:38+02:00 | 1.0.0 |
| 1013.1.6941 | `openEHR-EHR-OBSERVATION.aedes_indices.v0` | Aedes indices | INITIAL | 2024-12-29T06:32:57+01:00 | 0.0.1-alpha |
| 1013.1.5140 | `openEHR-EHR-OBSERVATION.affected_body_surface_area-burn.v0` | Burn-affected body surface area | DRAFT | 2021-07-16T07:01:54+02:00 | 0.0.1-alpha |
| 1013.1.4996 | `openEHR-EHR-OBSERVATION.affected_body_surface_area.v0` | Affected body surface area | DRAFT | 2021-07-16T06:58:48+02:00 | 0.0.1-alpha |
| 1013.1.5617 | `openEHR-EHR-OBSERVATION.agatston_score.v0` | Agatston score | INITIAL | 2021-07-27T09:34:19+02:00 | 0.0.1-alpha |
| 1013.1.3361 | `openEHR-EHR-OBSERVATION.age_assertion.v1` | Age assertion | PUBLISHED | 2025-07-28T12:13:05+02:00 | 1.0.3 |
| 1013.1.3782 | `openEHR-EHR-OBSERVATION.air_score.v0` | Appendicitis Inflammatory Response (AIR) Score  | DRAFT | 2019-11-08T07:24:18+01:00 | 0.0.1-alpha |
| 1013.1.1644 | `openEHR-EHR-OBSERVATION.alcohol_audit.v0` | Alcohol Use Disorders Identification Test (AUDIT) | DRAFT | 2025-09-01T02:43:04+02:00 | 0.0.1-alpha |
| 1013.1.1631 | `openEHR-EHR-OBSERVATION.alcohol_intake.v0` | Alcohol intake | DRAFT | 2019-08-12T15:34:16+02:00 | 0.0.1-alpha |
| 1013.1.5250 | `openEHR-EHR-OBSERVATION.aldrete_score.v0` | Aldrete score | DRAFT | 2021-03-03T06:14:06+01:00 | 0.0.1-alpha |
| 1013.1.5399 | `openEHR-EHR-OBSERVATION.alsfrs_r.v0` | Revised Amyotrophic Lateral Sclerosis Functional Rating Scale (ALSFRS-R) | DRAFT | 2021-04-29T09:48:16+02:00 | 0.0.1-alpha |
| 1013.1.3783 | `openEHR-EHR-OBSERVATION.alvarado_score.v0` | Alvarado score | DRAFT | 2019-05-10T07:39:56+02:00 | 0.0.1-alpha |
| 1013.1.3318 | `openEHR-EHR-OBSERVATION.aofas.v0` | AOFAS | DRAFT | 2021-03-01T01:56:46+01:00 | 0.0.1-alpha |
| 1013.1.3316 | `openEHR-EHR-OBSERVATION.aos_score.v0` | AOS | DRAFT | 2021-03-01T01:56:39+01:00 | 0.0.1-alpha |
| 1013.1.172 | `openEHR-EHR-OBSERVATION.apgar.v1` | Apgar score | DEPRECATED | 2020-07-09T11:41:24+02:00 | 1.0.1 |
| 1013.1.4866 | `openEHR-EHR-OBSERVATION.apgar.v2` | Apgar score | PUBLISHED | 2024-11-29T15:26:25+01:00 | 2.0.5 |
| 1013.1.1336 | `openEHR-EHR-OBSERVATION.asa_status.v1` | ASA physical status classification system | PUBLISHED | 2026-11-01T02:29:47+01:00 | 1.1.1 |
| 1013.1.4332 | `openEHR-EHR-OBSERVATION.atria_bleeding_risk.v0` | ATRIA bleeding risk score | DRAFT | 2020-02-03T05:18:20+01:00 | 0.0.1-alpha |
| 1013.1.4333 | `openEHR-EHR-OBSERVATION.atria_stroke_risk.v0` | ATRIA stroke risk score | DRAFT | 2021-02-10T05:05:20+01:00 | 0.0.1-alpha |
| 1013.1.3327 | `openEHR-EHR-OBSERVATION.atrs.v0` | ATRS | DRAFT | 2021-03-01T01:56:57+01:00 | 0.0.1-alpha |
| 1013.1.4367 | `openEHR-EHR-OBSERVATION.au_absolute_cvd_risk.v0` | Australian absolute cardiovascular disease risk calculator | INITIAL | 2020-02-18T01:55:57+01:00 | 0.0.1-alpha |
| 1013.1.44 | `openEHR-EHR-OBSERVATION.audiogram.v1` | Audiogram Result | REJECTED | 2018-04-25T04:49:19+02:00 |  |
| 1013.1.1677 | `openEHR-EHR-OBSERVATION.audiogram_result.v0` | Audiogram test result | DRAFT | 2018-03-27T05:07:05+02:00 | 0.0.1-alpha |
| 1013.1.1651 | `openEHR-EHR-OBSERVATION.audiology_speech_test_result.v0` | Audiology speech test result | DRAFT | 2019-07-11T10:10:22+02:00 | 0.0.1-alpha |
| 1013.1.8125 | `openEHR-EHR-OBSERVATION.audiology_speech_test_result_local.v0` | Speech intelligibility test result | DRAFT | 2025-12-01T18:35:48+01:00 | 0.0.1-alpha |
| 1013.1.3171 | `openEHR-EHR-OBSERVATION.auditory_brainstem_response_result.v0` | Auditory brainstem response (ABR) result | DRAFT | 2018-03-27T08:35:20+02:00 | 0.0.1-alpha |
| 1013.1.187 | `openEHR-EHR-OBSERVATION.autopsy.v1` | Autopsy examination | REJECTED | 2018-05-01T05:06:13+02:00 |  |
| 1013.1.1378 | `openEHR-EHR-OBSERVATION.avpu.v0` | AVPU | DRAFT | 2017-07-07T09:02:10+02:00 | 0.0.1-alpha |
| 1013.1.8325 | `openEHR-EHR-OBSERVATION.axial_ocular_biometry.v0` | Axial ocular biometry | INITIAL | 2026-06-14T18:33:05+02:00 | 0.0.1-alpha |
| 1013.1.2349 | `openEHR-EHR-OBSERVATION.behavioural_observation_audiometry_result.v0` | Behavioural observation audiometry (BOA) result | DRAFT | 2018-03-27T05:20:00+02:00 | 0.0.1-alpha |
| 1013.1.3331 | `openEHR-EHR-OBSERVATION.beighton_score.v0` | Beighton hypermobility score | DRAFT | 2018-06-30T09:21:11+02:00 | 0.0.1-alpha |
| 1013.1.8319 | `openEHR-EHR-OBSERVATION.berg_balance_scale.v1` | Berg Balance Scale (BBS) | PUBLISHED | 2026-09-16T08:32:08+02:00 | 1.0.0 |
| 1013.1.3716 | `openEHR-EHR-OBSERVATION.bishop_score.v0` | Bishop score | DRAFT | 2019-07-12T03:10:08+02:00 | 0.0.1-alpha |
| 1013.1.129 | `openEHR-EHR-OBSERVATION.blood_gases.v1` | Blood gas assessment | REJECTED | 2008-07-15T15:10:06+02:00 |  |
| 1013.1.47 | `openEHR-EHR-OBSERVATION.blood_match.v1` | Blood matching | REJECTED | 2018-09-11T08:47:59+02:00 |  |
| 1013.1.130 | `openEHR-EHR-OBSERVATION.blood_pressure.v1` | Blood Pressure | DEPRECATED | 2018-12-14T06:15:43+01:00 | 1.1.3 |
| 1013.1.3574 | `openEHR-EHR-OBSERVATION.blood_pressure.v2` | Blood pressure | PUBLISHED | 2026-11-04T21:43:38+01:00 | 2.0.17 |
| 1013.1.697 | `openEHR-EHR-OBSERVATION.bodily_output-defaecation.v1` | Defaecation | REJECTED | 2015-11-09T00:27:37+01:00 |  |
| 1013.1.696 | `openEHR-EHR-OBSERVATION.bodily_output-urination.v1` | Urination | REJECTED | 2015-11-09T00:27:00+01:00 |  |
| 1013.1.695 | `openEHR-EHR-OBSERVATION.bodily_output.v1` | Bodily output | REJECTED | 2015-11-09T00:24:31+01:00 |  |
| 1013.1.2842 | `openEHR-EHR-OBSERVATION.body_composition.v0` | Body composition | DRAFT | 2017-10-06T16:23:25+02:00 | 0.0.1-alpha |
| 1013.1.131 | `openEHR-EHR-OBSERVATION.body_mass_index.v1` | Body mass index | DEPRECATED | 2017-07-07T09:30:47+02:00 | 1.0.1 |
| 1013.1.2893 | `openEHR-EHR-OBSERVATION.body_mass_index.v2` | Body Mass Index (BMI) | PUBLISHED | 2025-08-20T09:08:32+02:00 | 2.1.0 |
| 1013.1.1749 | `openEHR-EHR-OBSERVATION.body_segment.v0` | Measurement of Body Segment | REJECTED | 2019-03-07T07:41:57+01:00 | 0.0.1 |
| 1013.1.3670 | `openEHR-EHR-OBSERVATION.body_segment_area.v1` | Body segment area | PUBLISHED | 2020-10-16T10:52:13+02:00 | 1.0.2 |
| 1013.1.3790 | `openEHR-EHR-OBSERVATION.body_segment_circumference.v1` | Body segment circumference | PUBLISHED | 2020-08-13T10:54:13+02:00 | 1.0.1 |
| 1013.1.6534 | `openEHR-EHR-OBSERVATION.body_segment_discrepancy.v1` | Body segment discrepancy | PUBLISHED | 2023-01-03T13:12:43+01:00 | 1.0.0 |
| 1013.1.3669 | `openEHR-EHR-OBSERVATION.body_segment_length.v1` | Body segment length | PUBLISHED | 2023-07-11T15:26:57+02:00 | 1.1.1 |
| 1013.1.1318 | `openEHR-EHR-OBSERVATION.body_surface_area.v1` | Body surface area | PUBLISHED | 2025-05-26T04:43:15+02:00 | 1.1.4 |
| 1013.1.49 | `openEHR-EHR-OBSERVATION.body_temperature.v1` | Body temperature | DEPRECATED | 2017-05-23T11:22:08+02:00 | 1.0.3 |
| 1013.1.2796 | `openEHR-EHR-OBSERVATION.body_temperature.v2` | Body temperature | PUBLISHED | 2026-11-04T21:59:50+01:00 | 2.1.11 |
| 1013.1.352 | `openEHR-EHR-OBSERVATION.body_weight-adjusted.v1` | Adjusted Body weight | REJECTED | 2019-09-24T10:27:15+02:00 |  |
| 1013.1.178 | `openEHR-EHR-OBSERVATION.body_weight-birth.v1` | Body weight at birth | REJECTED | 2017-06-20T15:13:37+02:00 |  |
| 1013.1.132 | `openEHR-EHR-OBSERVATION.body_weight.v1` | Body weight | DEPRECATED | 2017-09-26T10:42:31+02:00 | 1.0.1 |
| 1013.1.2960 | `openEHR-EHR-OBSERVATION.body_weight.v2` | Body weight | PUBLISHED | 2026-02-09T14:54:57+01:00 | 2.1.12 |
| 1013.1.5239 | `openEHR-EHR-OBSERVATION.boston_carpal_tunnel.v0` | Boston Carpal Tunnel Questionnaire Score (BOSTON) | DRAFT | 2021-03-01T01:56:49+01:00 | 0.0.1-alpha |
| 1013.1.1034 | `openEHR-EHR-OBSERVATION.braden_scale-q.v0` | Modified Braden Q Scale | REJECTED | 2019-10-08T07:55:55+02:00 | 0.0.1 |
| 1013.1.1014 | `openEHR-EHR-OBSERVATION.braden_scale.v1` | Braden scale | PUBLISHED | 2025-09-22T17:15:09+02:00 | 1.2.2 |
| 1013.1.4190 | `openEHR-EHR-OBSERVATION.braden_scale_q.v0` | Modified Braden Q scale | DRAFT | 2019-10-08T07:55:10+02:00 | 0.0.1-alpha |
| 1013.1.3339 | `openEHR-EHR-OBSERVATION.briganti_risk_score.v0` | Briganti Risk Score | DRAFT | 2018-06-30T09:23:11+02:00 | 0.0.1-alpha |
| 1013.1.1455 | `openEHR-EHR-OBSERVATION.bristol_stool_scale.v0` | Bristol stool scale | DRAFT | 2020-09-28T14:11:46+02:00 | 0.0.1-alpha |
| 1013.1.5150 | `openEHR-EHR-OBSERVATION.bvc.v1` | Brøset Violence Checklist (BVC) | PUBLISHED | 2021-03-16T14:29:15+01:00 | 1.0.0 |
| 1013.1.3858 | `openEHR-EHR-OBSERVATION.cage.v0` | CAGE questionnaire | DRAFT | 2019-07-11T14:12:47+02:00 | 0.0.1-alpha |
| 1013.1.4685 | `openEHR-EHR-OBSERVATION.capillary_refill.v1` | Capillary refill time (CRT) | PUBLISHED | 2025-09-01T00:35:13+02:00 | 1.0.1 |
| 1013.1.4995 | `openEHR-EHR-OBSERVATION.caprini_score.v0` | Caprini score | DRAFT | 2020-10-02T10:12:12+02:00 | 0.0.1-alpha |
| 1013.1.7511 | `openEHR-EHR-OBSERVATION.categorical_loudness_scaling.v1` | Categorical loudness scaling | PUBLISHED | 2025-12-19T15:53:12+01:00 | 1.0.0 |
| 1013.1.1415 | `openEHR-EHR-OBSERVATION.ccs_angina_status.v0` | Angina symptom classification (CCS) | DRAFT | 2025-09-01T02:46:53+02:00 | 0.0.1-alpha |
| 1013.1.2422 | `openEHR-EHR-OBSERVATION.cgas.v1` | Children's Global Assessment Scale | PUBLISHED | 2019-03-25T08:59:30+01:00 | 1.0.2 |
| 1013.1.2288 | `openEHR-EHR-OBSERVATION.chadsvas_score.v0` | CHA₂DS₂-VASc score | DRAFT | 2025-08-04T14:57:46+02:00 | 0.0.1-alpha |
| 1013.1.3323 | `openEHR-EHR-OBSERVATION.chaq.v0` | Childhood Health Assessment Questionnaire | DRAFT | 2018-06-30T09:19:12+02:00 | 0.0.1-alpha |
| 1013.1.6797 | `openEHR-EHR-OBSERVATION.charlson_comorbidity_index.v1` | Charlson Comorbidity Index (CCI) | DEPRECATED | 2024-03-21T08:39:34+01:00 | 1.0.2 |
| 1013.1.7218 | `openEHR-EHR-OBSERVATION.charlson_comorbidity_index.v2` | Charlson Comorbidity Index (CCI) | PUBLISHED | 2024-03-21T08:34:40+01:00 | 2.0.0 |
| 1013.1.4985 | `openEHR-EHR-OBSERVATION.cheop_scale.v0` | Children's Hospital of Eastern Ontario Pain Scale (CHEOPS) | DRAFT | 2020-10-01T05:42:26+02:00 | 0.0.1-alpha |
| 1013.1.3601 | `openEHR-EHR-OBSERVATION.chest_circumference.v0` | Chest circumference | DRAFT | 2019-01-31T08:20:43+01:00 | 0.0.1-alpha |
| 1013.1.141 | `openEHR-EHR-OBSERVATION.chest_expansion.v1` | Measurement of chest and expansion | REJECTED | 2015-12-05T07:56:37+01:00 |  |
| 1013.1.2741 | `openEHR-EHR-OBSERVATION.child_growth_indicator.v0` | Child growth indicator | DRAFT | 2024-10-27T23:42:10+01:00 | 0.0.1-alpha |
| 1013.1.3808 | `openEHR-EHR-OBSERVATION.child_pugh_score.v0` | Child-Pugh score | DRAFT | 2020-02-16T15:48:01+01:00 | 0.0.1-alpha |
| 1013.1.6355 | `openEHR-EHR-OBSERVATION.child_snapshot.v0` | Child snapshot | INITIAL | 2022-08-01T07:49:32+02:00 | 0.0.1-alpha |
| 1013.1.4691 | `openEHR-EHR-OBSERVATION.clinical_frailty_scale.v1` | Clinical Frailty Scale (CFS) | PUBLISHED | 2021-07-25T00:15:53+02:00 | 1.0.3 |
| 1013.1.6020 | `openEHR-EHR-OBSERVATION.clinical_frailty_scale2.v1` | Clinical Frailty Scale (CFS 2.0) | PUBLISHED | 2026-08-17T14:15:38+02:00 | 1.0.1 |
| 1013.1.3320 | `openEHR-EHR-OBSERVATION.cmas_score.v0` | Childhood Myositis Assessment Scale | DRAFT | 2018-06-30T09:18:28+02:00 | 0.0.1-alpha |
| 1013.1.2607 | `openEHR-EHR-OBSERVATION.comfort_behaviour_scale.v0` | Comfort behaviour scale | DRAFT | 2016-09-23T07:28:01+02:00 | 0.0.1-alpha |
| 1013.1.1391 | `openEHR-EHR-OBSERVATION.conference.v0` | Conference | DRAFT | 2019-09-24T10:46:04+02:00 | 0.0.1-alpha |
| 1013.1.2174 | `openEHR-EHR-OBSERVATION.container.v0` | Container | DRAFT | 2023-04-26T13:37:52+02:00 | 0.0.1-alpha |
| 1013.1.7638 | `openEHR-EHR-OBSERVATION.contrast_sensitivity_test.v0` | Contrast sensitivity test | INITIAL | 2024-12-19T15:08:49+01:00 | 0.0.1-alpha |
| 1013.1.3054 | `openEHR-EHR-OBSERVATION.cormack_lehane.v1` | Cormack-Lehane classification | PUBLISHED | 2025-05-26T04:46:36+02:00 | 1.0.2 |
| 1013.1.4986 | `openEHR-EHR-OBSERVATION.cow_score.v0` | Clinical Opiate Withdrawal Scale (COWS) | DRAFT | 2020-10-01T06:15:54+02:00 | 0.0.1-alpha |
| 1013.1.6782 | `openEHR-EHR-OBSERVATION.cpax.v1` | Chelsea Critical Care Physical Assessment (CPAx) tool | PUBLISHED | 2023-09-22T12:46:37+02:00 | 1.0.0 |
| 1013.1.4694 | `openEHR-EHR-OBSERVATION.crb_65.v1` | CRB-65 score | PUBLISHED | 2020-06-30T13:28:53+02:00 | 1.0.0 |
| 1013.1.2605 | `openEHR-EHR-OBSERVATION.critical_pain_observation_tool.v0` | Critical care pain observation tool (CPOT) | DRAFT | 2016-09-28T07:21:26+02:00 | 0.0.1-alpha |
| 1013.1.4358 | `openEHR-EHR-OBSERVATION.crusade_bleeding.v0` | CRUSADE bleeding score | DRAFT | 2020-02-17T05:11:10+01:00 | 0.0.1-alpha |
| 1013.1.5237 | `openEHR-EHR-OBSERVATION.crusade_bleeding_risk_score.v0` | Crusade Bleeding Risk Score | DRAFT | 2022-04-05T03:34:31+02:00 | 0.0.1-alpha |
| 1013.1.4695 | `openEHR-EHR-OBSERVATION.curb_65.v1` | CURB-65 score | PUBLISHED | 2020-06-30T13:35:01+02:00 | 1.0.0 |
| 1013.1.5726 | `openEHR-EHR-OBSERVATION.current_pregnancy_screening.v0` | Current pregnancy screening questionnaire | INITIAL | 2023-07-19T07:35:54+02:00 | 0.0.1-alpha |
| 1013.1.4990 | `openEHR-EHR-OBSERVATION.das28-CRP.v0` | Disease Activity Score-28 with CRP (DAS28‐CRP) | DRAFT | 2020-10-01T07:49:54+02:00 | 0.0.1-alpha |
| 1013.1.3338 | `openEHR-EHR-OBSERVATION.das28.v0` | Disease Activity Score-28 (DAS28) | DRAFT | 2022-05-23T04:48:01+02:00 | 0.0.1-alpha |
| 1013.1.4988 | `openEHR-EHR-OBSERVATION.dash_score.v0` | DASH prediction score | DRAFT | 2020-10-01T06:52:20+02:00 | 0.0.1-alpha |
| 1013.1.1385 | `openEHR-EHR-OBSERVATION.demo.v1` | Demonstration | PUBLISHED | 2018-05-14T08:30:58+02:00 | 1.0.0 |
| 1013.1.1863 | `openEHR-EHR-OBSERVATION.dermatology_therapy_summary.v1` | Dermatology therapy summary | INITIAL | 2015-02-18T21:02:05+01:00 |  |
| 1013.1.5244 | `openEHR-EHR-OBSERVATION.developmental_milestone.v0` | Developmental milestone | INITIAL | 2026-11-12T05:05:42+01:00 | 0.0.1-alpha |
| 1013.1.5844 | `openEHR-EHR-OBSERVATION.device_screening.v0` | Medical device screening questionnaire | DRAFT | 2025-07-07T18:57:12+02:00 | 0.0.1-alpha |
| 1013.1.1815 | `openEHR-EHR-OBSERVATION.diabetic_wound_wagner.v0` | Diabetic wound classification (Wagner) | DRAFT | 2019-09-24T10:56:02+02:00 | 0.0.1-alpha |
| 1013.1.7434 | `openEHR-EHR-OBSERVATION.digit_span.v0` | Digit span (DS) | DRAFT | 2024-09-12T07:42:04+02:00 | 0.0.1-alpha |
| 1013.1.53 | `openEHR-EHR-OBSERVATION.distraction_hearing_test.v1` | Distraction Hearing Test | REJECTED | 2014-03-16T04:58:29+01:00 |  |
| 1013.1.5144 | `openEHR-EHR-OBSERVATION.downton_fall_risk_index.v0` | Downton Fall Risk Index (DFRI) | DRAFT | 2025-08-18T02:28:37+02:00 | 0.0.1-alpha |
| 1013.1.1871 | `openEHR-EHR-OBSERVATION.easi_score.v0` | EASI score | DRAFT | 2019-09-24T11:01:39+02:00 | 0.0.1-alpha |
| 1013.1.276 | `openEHR-EHR-OBSERVATION.ecg_result.v1` | ECG result | PUBLISHED | 2022-01-13T14:26:36+01:00 | 1.0.4 |
| 1013.1.1317 | `openEHR-EHR-OBSERVATION.ecog.v1` | ECOG performance status | DEPRECATED | 2025-10-08T15:56:48+02:00 | 1.0.5 |
| 1013.1.8091 | `openEHR-EHR-OBSERVATION.ecog.v2` | ECOG performance status | PUBLISHED | 2025-10-08T15:52:57+02:00 | 2.0.0 |
| 1013.1.1281 | `openEHR-EHR-OBSERVATION.edinburgh_pnd_scale.v0` | Edinburgh postnatal depression scale | DRAFT | 2019-09-24T11:08:20+02:00 | 0.0.1-alpha |
| 1013.1.3334 | `openEHR-EHR-OBSERVATION.edmonton_frail_scale.v0` | Edmonton frail scale | DRAFT | 2018-06-30T09:21:56+02:00 | 0.0.1-alpha |
| 1013.1.5176 | `openEHR-EHR-OBSERVATION.ejection_fraction-left_ventricle.v0` | Left ventricular ejection fraction | INITIAL | 2021-01-29T02:28:31+01:00 | 0.0.1-alpha |
| 1013.1.5174 | `openEHR-EHR-OBSERVATION.ejection_fraction.v0` | Ejection fraction | DRAFT | 2023-08-09T10:49:33+02:00 | 0.0.1-alpha |
| 1013.1.54 | `openEHR-EHR-OBSERVATION.electroacoustic_hearing_test.v1` | Electroacoustic Hearing Test | REJECTED | 2014-03-16T05:05:37+01:00 |  |
| 1013.1.5854 | `openEHR-EHR-OBSERVATION.embryo_assessment.v1` | Oocyte and embryo assessment | PUBLISHED | 2022-03-18T15:27:52+01:00 | 1.0.0 |
| 1013.1.2049 | `openEHR-EHR-OBSERVATION.empower_meal.v0` | Meal (EMPOWER) | INITIAL | 2015-06-19T09:47:49+02:00 | 0.0.1-alpha |
| 1013.1.2050 | `openEHR-EHR-OBSERVATION.empower_mood.v0` | Mood (EMPOWER) | INITIAL | 2015-06-19T09:47:50+02:00 | 0.0.1-alpha |
| 1013.1.2051 | `openEHR-EHR-OBSERVATION.empower_physical_exercises.v0` | Physical Activity (EMPOWER) | INITIAL | 2024-07-03T11:20:02+02:00 | 0.0.1-alpha |
| 1013.1.2052 | `openEHR-EHR-OBSERVATION.empower_sleep.v0` | Sleep (EMPOWER) | INITIAL | 2015-06-19T09:47:53+02:00 | 0.0.1-alpha |
| 1013.1.2053 | `openEHR-EHR-OBSERVATION.empower_stress.v0` | Stress (EMPOWER) | INITIAL | 2015-06-19T09:47:54+02:00 | 0.0.1-alpha |
| 1013.1.4672 | `openEHR-EHR-OBSERVATION.eortc_qlq_c30.v0` | EORTC QLQ-C30 | REJECTED | 2020-08-18T08:43:48+02:00 | 0.0.1 |
| 1013.1.4717 | `openEHR-EHR-OBSERVATION.eortc_qlq_cr29.v0` | EORTC QLQ-CR29 | REJECTED | 2020-08-18T08:44:08+02:00 | 0.0.1 |
| 1013.1.6133 | `openEHR-EHR-OBSERVATION.epic_cp.v1` | Expanded Prostate Cancer Index Composite for Clinical Practice (EPIC-CP) | PUBLISHED | 2026-09-25T14:40:38+02:00 | 1.0.0 |
| 1013.1.8137 | `openEHR-EHR-OBSERVATION.eq_5d_5l.v0` | EQ-5D-5L | INITIAL | 2025-12-14T01:50:54+01:00 | 0.0.1-alpha |
| 1013.1.3819 | `openEHR-EHR-OBSERVATION.esas_r.v1` | Edmonton Symptom Assessment System Revised (ESAS-r) | PUBLISHED | 2025-05-26T05:05:46+02:00 | 1.0.1 |
| 1013.1.4991 | `openEHR-EHR-OBSERVATION.estimated_glomerular_filtration_rate.v0` | Estimated glomerular filtration rate (eGFR) | DRAFT | 2020-10-02T05:31:05+02:00 | 0.0.1-alpha |
| 1013.1.271 | `openEHR-EHR-OBSERVATION.exam.v1` | Physical examination findings | PUBLISHED | 2025-07-16T14:04:51+02:00 | 1.1.5 |
| 1013.1.3830 | `openEHR-EHR-OBSERVATION.exam_oral.v0` | Exam oral | INITIAL | 2019-06-12T03:44:07+02:00 | 0.0.1-alpha |
| 1013.1.5155 | `openEHR-EHR-OBSERVATION.exclusion-adverse_reactions.v0` | Exclusion of adverse reactions | INITIAL | 2021-01-29T02:45:36+01:00 | 0.0.1-alpha |
| 1013.1.5156 | `openEHR-EHR-OBSERVATION.exclusion-pregnancy.v0` | Exclusion of pregnancy | INITIAL | 2021-01-29T02:47:25+01:00 | 0.0.1-alpha |
| 1013.1.5154 | `openEHR-EHR-OBSERVATION.exclusion.v0` | Exclusion | INITIAL | 2021-01-21T09:44:14+01:00 | 0.0.1-alpha |
| 1013.1.4906 | `openEHR-EHR-OBSERVATION.exclusion_pregnancy.v0` | Exclusion of pregnancy | REJECTED | 2022-04-08T06:43:09+02:00 | 0.0.1 |
| 1013.1.7915 | `openEHR-EHR-OBSERVATION.exophthalmometry.v0` | Exophthalmometry | INITIAL | 2025-06-12T00:52:37+02:00 | 0.0.1-alpha |
| 1013.1.7439 | `openEHR-EHR-OBSERVATION.expanded_prostate_cancer_index_composite.v1` | Expanded Prostate Cancer Index Composite (EPIC) | PUBLISHED | 2026-08-31T22:06:52+02:00 | 1.0.2 |
| 1013.1.2591 | `openEHR-EHR-OBSERVATION.experiment.v0` | Experiment | INITIAL | 2016-09-01T16:10:58+02:00 | 0.0.1-alpha |
| 1013.1.4437 | `openEHR-EHR-OBSERVATION.exposure_screening.v1` | Exposure screening questionnaire | PUBLISHED | 2025-01-24T10:11:43+01:00 | 1.0.3 |
| 1013.1.4734 | `openEHR-EHR-OBSERVATION.fact_g-Hep.v0` | FACT-Hep | DRAFT | 2020-05-08T16:44:58+02:00 | 0.0.1-alpha |
| 1013.1.4732 | `openEHR-EHR-OBSERVATION.fact_g.v0` | FACT-G | DRAFT | 2020-05-08T16:45:34+02:00 | 0.0.1-alpha |
| 1013.1.3541 | `openEHR-EHR-OBSERVATION.faecal_output.v0` | Faecal output | DRAFT | 2020-09-28T14:11:09+02:00 | 0.0.1-alpha |
| 1013.1.248 | `openEHR-EHR-OBSERVATION.faeces.v0` | Faeces | REJECTED | 2018-11-16T06:39:15+01:00 | 0.0.1 |
| 1013.1.2609 | `openEHR-EHR-OBSERVATION.fagerstrom.v0` | Fagerström test for nicotine dependence | DRAFT | 2022-06-29T14:56:49+02:00 | 0.0.1-alpha |
| 1013.1.5152 | `openEHR-EHR-OBSERVATION.family_history_screening_questionnaire.v1` | Family history screening questionnaire | PUBLISHED | 2025-10-20T12:36:00+02:00 | 1.0.0 |
| 1013.1.5922 | `openEHR-EHR-OBSERVATION.fetal_biometry.v1` | Fetal biometry | PUBLISHED | 2025-08-05T06:35:35+02:00 | 1.0.3 |
| 1013.1.5396 | `openEHR-EHR-OBSERVATION.fetal_growth.v0` | Fetal growth indicators | DRAFT | 2025-08-05T06:46:26+02:00 | 0.0.1-alpha |
| 1013.1.1198 | `openEHR-EHR-OBSERVATION.fetal_heart-monitoring.v0` | Fetal heart monitoring | DRAFT | 2026-08-13T13:24:36+02:00 | 0.0.1-alpha |
| 1013.1.1197 | `openEHR-EHR-OBSERVATION.fetal_heart.v0` | Fetal heart rate | DRAFT | 2026-08-13T13:59:47+02:00 | 0.0.1-alpha |
| 1013.1.216 | `openEHR-EHR-OBSERVATION.fetal_movement.v0` | Fetal movement | DRAFT | 2025-08-05T07:08:16+02:00 | 0.0.1-alpha |
| 1013.1.1865 | `openEHR-EHR-OBSERVATION.fitzpatrick_skin_type.v0` | Fitzpatrick skin type | DRAFT | 2019-10-02T08:34:00+02:00 | 0.0.1-alpha |
| 1013.1.1682 | `openEHR-EHR-OBSERVATION.fluid_balance.v1` | Fluid balance | PUBLISHED | 2020-04-06T13:03:15+02:00 | 1.1.1 |
| 1013.1.1671 | `openEHR-EHR-OBSERVATION.fluid_input.v1` | Fluid input | PUBLISHED | 2020-04-06T12:59:54+02:00 | 1.0.2 |
| 1013.1.4239 | `openEHR-EHR-OBSERVATION.fluid_output-blood.v0` | Blood loss | DRAFT | 2019-11-15T05:28:31+01:00 | 0.0.1-alpha |
| 1013.1.1683 | `openEHR-EHR-OBSERVATION.fluid_output.v1` | Fluid output | PUBLISHED | 2020-04-06T13:01:55+02:00 | 1.0.2 |
| 1013.1.2884 | `openEHR-EHR-OBSERVATION.food_item.v0` | Food item | DRAFT | 2017-06-29T08:56:02+02:00 | 0.0.1-alpha |
| 1013.1.4304 | `openEHR-EHR-OBSERVATION.four_a_test.v1` | 4AT | PUBLISHED | 2023-04-27T10:35:28+02:00 | 1.0.1 |
| 1013.1.5249 | `openEHR-EHR-OBSERVATION.four_score.v0` | Full Outline of UnResponsiveness (FOUR) score | DRAFT | 2021-03-03T05:12:41+01:00 | 0.0.1-alpha |
| 1013.1.5017 | `openEHR-EHR-OBSERVATION.functional_ability.v0` | Functional ability | TEAMREVIEW | 2026-06-24T08:33:09+02:00 | 0.0.1-alpha |
| 1013.1.208 | `openEHR-EHR-OBSERVATION.fundoscopic_examination.v0` | Fundoscopic examination of eyes | REJECTED | 2026-05-07T08:54:59+02:00 | 0.0.1 |
| 1013.1.8024 | `openEHR-EHR-OBSERVATION.g8_screening_tool.v0` | G-8 screening tool | DRAFT | 2025-08-26T14:06:37+02:00 | 0.0.1-alpha |
| 1013.1.3335 | `openEHR-EHR-OBSERVATION.gad_7_score.v0` | GAD-7 score | DRAFT | 2019-03-28T05:56:32+01:00 | 0.0.1-alpha |
| 1013.1.4763 | `openEHR-EHR-OBSERVATION.gestation_assertion.v0` | Gestation assertion | DRAFT | 2024-10-28T14:24:59+01:00 | 0.0.1-alpha |
| 1013.1.1947 | `openEHR-EHR-OBSERVATION.glasgow_coma-paed.v0` | Paediatric Glasgow Coma Scale | REJECTED | 2019-10-08T06:26:27+02:00 | 0.0.1 |
| 1013.1.137 | `openEHR-EHR-OBSERVATION.glasgow_coma_scale.v1` | Glasgow Coma Scale (GCS) | PUBLISHED | 2020-04-05T11:57:15+02:00 | 1.2.0 |
| 1013.1.4188 | `openEHR-EHR-OBSERVATION.glasgow_coma_scale_paediatric.v0` | Paediatric Glasgow Coma Scale (pGCS) | DRAFT | 2025-12-12T15:25:34+01:00 | 0.0.1-alpha |
| 1013.1.6671 | `openEHR-EHR-OBSERVATION.glasgow_outcome_scale_extended.v1` | Glasgow Outcome Scale - Extended (GOSE) | PUBLISHED | 2023-02-15T16:42:52+01:00 | 1.0.0 |
| 1013.1.249 | `openEHR-EHR-OBSERVATION.global.v1` | Global assessment | REJECTED | 2015-11-23T00:56:43+01:00 |  |
| 1013.1.4253 | `openEHR-EHR-OBSERVATION.gpaq.v0` | Global Physical Activity Questionnaire (GPAQ) | DRAFT | 2019-11-21T09:11:43+01:00 | 0.0.1-alpha |
| 1013.1.3340 | `openEHR-EHR-OBSERVATION.gpcog_screening_test.v0` | GPCOG screening test | DRAFT | 2018-06-30T09:23:25+02:00 | 0.0.1-alpha |
| 1013.1.4359 | `openEHR-EHR-OBSERVATION.grace_admission.v0` | GRACE score (admission) | DRAFT | 2020-02-17T03:35:40+01:00 | 0.0.1-alpha |
| 1013.1.4362 | `openEHR-EHR-OBSERVATION.grace_discharge.v0` | GRACE score (discharge) | DRAFT | 2020-02-17T03:43:16+01:00 | 0.0.1-alpha |
| 1013.1.3428 | `openEHR-EHR-OBSERVATION.growth_velocity.v0` | Growth velocity | DRAFT | 2018-08-22T05:43:00+02:00 | 0.0.1-alpha |
| 1013.1.7751 | `openEHR-EHR-OBSERVATION.guss.v1` | Gugging Swallowing Screen (GUSS) | PUBLISHED | 2026-04-16T13:57:58+02:00 | 1.0.0 |
| 1013.1.7752 | `openEHR-EHR-OBSERVATION.guss_icu.v1` | Gugging Swallowing Screen for Intensive Care Units (GUSS - ICU) | PUBLISHED | 2026-04-16T14:00:32+02:00 | 1.0.0 |
| 1013.1.8139 | `openEHR-EHR-OBSERVATION.hand_grip_strength.v1` | Handgrip strength | PUBLISHED | 2026-07-07T10:55:57+02:00 | 1.0.0 |
| 1013.1.2617 | `openEHR-EHR-OBSERVATION.hannallah_pain_scale.v0` | Hannallah Objective Pain Scale (OPS) | DRAFT | 2016-09-26T07:32:20+02:00 | 0.0.1-alpha |
| 1013.1.3337 | `openEHR-EHR-OBSERVATION.haq.v0` | Health Assessment Questionnaire | DRAFT | 2018-06-30T09:22:41+02:00 | 0.0.1-alpha |
| 1013.1.1873 | `openEHR-EHR-OBSERVATION.harris_hip.v0` | Harris Hip Score (HHS) | DRAFT | 2025-11-08T08:10:11+01:00 | 0.0.1-alpha |
| 1013.1.4345 | `openEHR-EHR-OBSERVATION.has_bled.v0` | HAS-BLED score | DRAFT | 2025-08-04T15:03:49+02:00 | 0.0.1-alpha |
| 1013.1.2555 | `openEHR-EHR-OBSERVATION.head_circumference.v1` | Head circumference | PUBLISHED | 2025-05-26T05:08:11+02:00 | 1.0.4 |
| 1013.1.2350 | `openEHR-EHR-OBSERVATION.hearing_screening_result.v0` | Hearing screening test result | DRAFT | 2018-09-03T09:20:25+02:00 | 0.0.1-alpha |
| 1013.1.1416 | `openEHR-EHR-OBSERVATION.heart_failure_symptom_questionnaire.v1` | Heart failure symptom questionnaire | INITIAL | 2013-03-08T14:04:07+01:00 |  |
| 1013.1.171 | `openEHR-EHR-OBSERVATION.heart_rate-pulse.v1` | Pulse | REJECTED | 2013-02-20T10:42:09+01:00 |  |
| 1013.1.4357 | `openEHR-EHR-OBSERVATION.heart_score.v0` | HEART score | DRAFT | 2020-02-17T04:12:09+01:00 | 0.0.1-alpha |
| 1013.1.7153 | `openEHR-EHR-OBSERVATION.heartbeat-pulse.v0` | Pulse | INITIAL | 2024-04-11T09:41:57+02:00 | 0.0.1-alpha |
| 1013.1.7154 | `openEHR-EHR-OBSERVATION.heartbeat.v0` | Heartbeat | INITIAL | 2024-04-11T09:31:59+02:00 | 0.0.1-alpha |
| 1013.1.2892 | `openEHR-EHR-OBSERVATION.height-adjusted.v0` | Adjusted height/length | REJECTED | 2018-04-26T08:49:20+02:00 | 0.0.1 |
| 1013.1.138 | `openEHR-EHR-OBSERVATION.height.v1` | Height/Length | DEPRECATED | 2018-04-26T08:52:11+02:00 | 1.0.1 |
| 1013.1.3210 | `openEHR-EHR-OBSERVATION.height.v2` | Height/Length | PUBLISHED | 2026-02-09T15:47:16+01:00 | 2.1.1 |
| 1013.1.2815 | `openEHR-EHR-OBSERVATION.hip_circumference.v1` | Hip circumference | PUBLISHED | 2025-05-26T05:25:05+02:00 | 1.0.1 |
| 1013.1.5800 | `openEHR-EHR-OBSERVATION.hirsutism_scales.v1` | Hirsutism scales | PUBLISHED | 2021-12-22T13:39:23+01:00 | 1.0.0 |
| 1013.1.2320 | `openEHR-EHR-OBSERVATION.honos.v0` | Health of the Nation Outcome Scale | DRAFT | 2018-09-05T08:53:34+02:00 | 0.0.1-alpha |
| 1013.1.7850 | `openEHR-EHR-OBSERVATION.hoos.v0` | Hip Disability and Osteoarthritis Outcome Score (HOOS) | INITIAL | 2025-05-14T08:53:39+02:00 | 0.0.1-alpha |
| 1013.1.3336 | `openEHR-EHR-OBSERVATION.howru.v1` | howRU score | DRAFT | 2018-06-30T09:22:25+02:00 |  |
| 1013.1.4548 | `openEHR-EHR-OBSERVATION.hscore.v0` | HScore | DRAFT | 2020-04-01T14:09:36+02:00 | 0.0.1-alpha |
| 1013.1.2608 | `openEHR-EHR-OBSERVATION.humpty_dumpty_falls_risk_assessment_tool.v0` | Humpty dumpty falls scale | DRAFT | 2018-05-09T04:53:52+02:00 | 0.0.1-alpha |
| 1013.1.3325 | `openEHR-EHR-OBSERVATION.iciq_ui_short.v0` | ICIQ-UI Short Form | DRAFT | 2018-06-30T09:19:43+02:00 | 0.0.1-alpha |
| 1013.1.1869 | `openEHR-EHR-OBSERVATION.iga_eczema_treat.v0` | IGA eczema (TREAT) | DRAFT | 2019-10-01T07:34:30+02:00 | 0.0.1-alpha |
| 1013.1.3326 | `openEHR-EHR-OBSERVATION.iief_5.v0` | IIEF-5-Score | DRAFT | 2018-06-30T09:19:58+02:00 | 0.0.1-alpha |
| 1013.1.5234 | `openEHR-EHR-OBSERVATION.ikdc.v0` | IKDC subjective knee evaluation | DRAFT | 2021-03-01T01:56:41+01:00 | 0.0.1-alpha |
| 1013.1.134 | `openEHR-EHR-OBSERVATION.imaging.v1` | Imaging test | REJECTED | 2013-09-16T12:26:13+02:00 |  |
| 1013.1.1494 | `openEHR-EHR-OBSERVATION.imaging_exam_result.v1` | Imaging examination result | PUBLISHED | 2026-08-13T17:49:34+02:00 | 1.1.3 |
| 1013.1.188 | `openEHR-EHR-OBSERVATION.indirect_oximetry.v1` | Pulse oximetry | DEPRECATED | 2018-01-26T10:48:33+01:00 | 1.0.1 |
| 1013.1.250 | `openEHR-EHR-OBSERVATION.infant_feeding.v0` | Feeding | DRAFT | 2019-11-06T06:49:48+01:00 | 0.0.1-alpha |
| 1013.1.3698 | `openEHR-EHR-OBSERVATION.intermacs_profile.v0` | INTERMACS profile | DRAFT | 2019-04-28T04:50:28+02:00 | 0.0.1-alpha |
| 1013.1.1369 | `openEHR-EHR-OBSERVATION.intraocular_pressure.v1` | Intraocular pressure test | PUBLISHED | 2026-09-21T13:25:09+02:00 | 1.0.0 |
| 1013.1.766 | `openEHR-EHR-OBSERVATION.intravascular_pressure-cvp.v1` | Central venous pressure | REJECTED | 2018-11-27T04:11:23+01:00 |  |
| 1013.1.60 | `openEHR-EHR-OBSERVATION.intravascular_pressure-jvp.v0` | Jugular venous pressure | REJECTED | 2018-11-27T06:18:04+01:00 | 0.0.1 |
| 1013.1.140 | `openEHR-EHR-OBSERVATION.intravascular_pressure.v0` | Intravascular pressure | DRAFT | 2025-11-23T04:17:34+01:00 | 0.0.1-alpha |
| 1013.1.7671 | `openEHR-EHR-OBSERVATION.investigation_screening-JM.v0` | Diagnostic investigation screening questionnaire | INITIAL | 2024-12-29T04:55:16+01:00 | 0.0.1-alpha |
| 1013.1.6599 | `openEHR-EHR-OBSERVATION.investigation_screening.v1` | Diagnostic investigation screening questionnaire | PUBLISHED | 2025-05-08T10:45:55+02:00 | 1.3.0 |
| 1013.1.2361 | `openEHR-EHR-OBSERVATION.ipss.v1` | International prostate symptom score (IPSS) | PUBLISHED | 2026-07-08T12:43:28+02:00 | 1.0.2 |
| 1013.1.5456 | `openEHR-EHR-OBSERVATION.iss-revised.v0` | Revised International Staging System for Multiple Myeloma (R-ISS) | DRAFT | 2021-06-10T02:41:29+02:00 | 0.0.1-alpha |
| 1013.1.5455 | `openEHR-EHR-OBSERVATION.iss.v0` | International Staging System for Multiple Myeloma (ISS) | DRAFT | 2021-05-28T09:05:19+02:00 | 0.0.1-alpha |
| 1013.1.4794 | `openEHR-EHR-OBSERVATION.issue_screening.v0` | Issue screening questionnaire | REJECTED | 2022-06-24T09:00:52+02:00 | 0.0.1 |
| 1013.1.3556 | `openEHR-EHR-OBSERVATION.jugular_venous_pressure.v0` | Jugular venous pressure | DRAFT | 2018-11-27T06:35:20+01:00 | 0.0.1-alpha |
| 1013.1.5015 | `openEHR-EHR-OBSERVATION.kads.v0` | Kutcher Adolescent Depression Scale (KADS) | DRAFT | 2020-10-26T07:53:27+01:00 | 0.0.1-alpha |
| 1013.1.5145 | `openEHR-EHR-OBSERVATION.karnofsky_performance_status_scale.v1` | Karnofsky Performance Status (KPS) scale | PUBLISHED | 2023-03-31T11:03:18+02:00 | 1.0.2 |
| 1013.1.4984 | `openEHR-EHR-OBSERVATION.kessler_k10_scale.v0` | Kessler Psychological Distress Scale (K10) | DRAFT | 2020-10-01T04:37:21+02:00 | 0.0.1-alpha |
| 1013.1.563 | `openEHR-EHR-OBSERVATION.lab_test-blood_gases.v1` | Blood gas assessment | REJECTED | 2016-11-14T12:38:43+01:00 |  |
| 1013.1.564 | `openEHR-EHR-OBSERVATION.lab_test-blood_glucose.v1` | Blood glucose | REJECTED | 2016-11-14T12:38:36+01:00 |  |
| 1013.1.565 | `openEHR-EHR-OBSERVATION.lab_test-blood_match.v1` | Blood matching | REJECTED | 2016-11-14T12:38:31+01:00 |  |
| 1013.1.630 | `openEHR-EHR-OBSERVATION.lab_test-esr.v1` | Erythroctye sedimentation rate | REJECTED | 2016-11-14T12:38:25+01:00 |  |
| 1013.1.566 | `openEHR-EHR-OBSERVATION.lab_test-full_blood_count.v1` | Full blood count | REJECTED | 2016-11-14T12:38:20+01:00 |  |
| 1013.1.551 | `openEHR-EHR-OBSERVATION.lab_test-hba1c.v1` | Haemoglobin A1c | REJECTED | 2016-11-14T12:38:15+01:00 |  |
| 1013.1.351 | `openEHR-EHR-OBSERVATION.lab_test-histopathology.v1` | Histopathology | REJECTED | 2016-11-14T12:38:10+01:00 |  |
| 1013.1.554 | `openEHR-EHR-OBSERVATION.lab_test-immunology-ANA.v1` | Antinuclear antibody (ANA) | REJECTED | 2016-11-14T12:34:29+01:00 |  |
| 1013.1.552 | `openEHR-EHR-OBSERVATION.lab_test-immunology.v1` | Immunology titres | REJECTED | 2016-11-14T12:38:02+01:00 |  |
| 1013.1.553 | `openEHR-EHR-OBSERVATION.lab_test-immunology_titres.v1` | Immunology titre test | REJECTED | 2009-12-14T22:22:45+01:00 |  |
| 1013.1.555 | `openEHR-EHR-OBSERVATION.lab_test-lipids.v1` | Lipid studies | REJECTED | 2016-11-14T12:37:56+01:00 |  |
| 1013.1.556 | `openEHR-EHR-OBSERVATION.lab_test-liver_function.v1` | Liver function tests | REJECTED | 2016-11-14T12:37:46+01:00 |  |
| 1013.1.557 | `openEHR-EHR-OBSERVATION.lab_test-microalbumin.v1` | Urine microalbumin | REJECTED | 2016-11-14T12:36:06+01:00 |  |
| 1013.1.687 | `openEHR-EHR-OBSERVATION.lab_test-microbiology-csf.v1` | Microbiolology of cerebro-spinal fluid | REJECTED | 2016-11-14T12:34:46+01:00 |  |
| 1013.1.559 | `openEHR-EHR-OBSERVATION.lab_test-microbiology-urine.v1` | Microbiolology of urine | REJECTED | 2016-11-14T12:35:53+01:00 |  |
| 1013.1.558 | `openEHR-EHR-OBSERVATION.lab_test-microbiology.v1` | Microbiolology | REJECTED | 2016-11-14T12:36:31+01:00 |  |
| 1013.1.560 | `openEHR-EHR-OBSERVATION.lab_test-thyroid.v1` | Thyroid function tests | REJECTED | 2016-11-14T12:36:22+01:00 |  |
| 1013.1.561 | `openEHR-EHR-OBSERVATION.lab_test-urea_and_electrolytes.v1` | Urea and electrolytes | REJECTED | 2016-11-14T12:36:12+01:00 |  |
| 1013.1.562 | `openEHR-EHR-OBSERVATION.lab_test-urine_protein.v1` | Urine protein | REJECTED | 2016-11-14T12:35:59+01:00 |  |
| 1013.1.350 | `openEHR-EHR-OBSERVATION.lab_test.v1` | Laboratory test | REJECTED | 2016-11-14T12:38:50+01:00 |  |
| 1013.1.330 | `openEHR-EHR-OBSERVATION.laboratory_result-histology-haematopoietic_tumour.v1` | Histology result haematopoietic tumours | REJECTED | 2009-05-06T08:52:01+02:00 |  |
| 1013.1.329 | `openEHR-EHR-OBSERVATION.laboratory_result-histology.v1` | Laboratory histology | REJECTED | 2009-05-06T08:48:30+02:00 |  |
| 1013.1.328 | `openEHR-EHR-OBSERVATION.laboratory_result.v1` | Laboratory result | REJECTED | 2009-05-06T08:47:25+02:00 |  |
| 1013.1.2195 | `openEHR-EHR-OBSERVATION.laboratory_test-histopathology.v1` | Histopathology test | REJECTED | 2016-11-29T17:57:40+01:00 | 0.0.1 |
| 1013.1.2191 | `openEHR-EHR-OBSERVATION.laboratory_test_result.v1` | Laboratory test result | PUBLISHED | 2026-07-16T15:50:39+02:00 | 1.2.8 |
| 1013.1.6128 | `openEHR-EHR-OBSERVATION.leg_length_discrepancy.v0` | Leg length discrepancy | REJECTED | 2022-09-29T10:50:12+02:00 | 0.0.1 |
| 1013.1.6527 | `openEHR-EHR-OBSERVATION.lenke_classification.v1` | Lenke classification system | PUBLISHED | 2022-12-16T12:49:20+01:00 | 1.0.0 |
| 1013.1.8236 | `openEHR-EHR-OBSERVATION.light_projection_test.v0` | Light projection test | TEAMREVIEW | 2026-04-03T09:18:57+02:00 | 0.0.1-alpha |
| 1013.1.2804 | `openEHR-EHR-OBSERVATION.malinas_score.v0` | Malinas score | DRAFT | 2017-05-05T08:08:39+02:00 | 0.0.1-alpha |
| 1013.1.3053 | `openEHR-EHR-OBSERVATION.mallampati_classification.v1` | Modified Mallampati classification | PUBLISHED | 2017-12-11T13:52:00+01:00 | 1.0.0 |
| 1013.1.2816 | `openEHR-EHR-OBSERVATION.malnutrition_screening_tool.v1` | Malnutrition Screening Tool (MST) | PUBLISHED | 2023-06-26T13:00:54+02:00 | 1.0.0 |
| 1013.1.4448 | `openEHR-EHR-OBSERVATION.management_screening.v1` | Management screening questionnaire | DEPRECATED | 2025-01-23T14:16:30+01:00 | 1.0.3 |
| 1013.1.7702 | `openEHR-EHR-OBSERVATION.management_screening.v2` | Management screening questionnaire | PUBLISHED | 2025-07-28T13:29:02+02:00 | 2.0.4 |
| 1013.1.1508 | `openEHR-EHR-OBSERVATION.mantoux_test_result.v0` | Mantoux test result | DRAFT | 2022-10-26T05:01:05+02:00 | 0.0.1-alpha |
| 1013.1.7636 | `openEHR-EHR-OBSERVATION.map_hand.v1` | Measure of activity performance of the hand (MAP-Hand) | PUBLISHED | 2025-03-05T12:18:35+01:00 | 1.0.0 |
| 1013.1.5078 | `openEHR-EHR-OBSERVATION.mayo_score.v1` | Mayo score | PUBLISHED | 2021-04-21T13:39:29+02:00 | 1.0.0 |
| 1013.1.4677 | `openEHR-EHR-OBSERVATION.medication_screening.v1` | Medication screening questionnaire | PUBLISHED | 2025-01-23T14:28:34+01:00 | 1.0.3 |
| 1013.1.7652 | `openEHR-EHR-OBSERVATION.medication_statement-JM.v0` | Medication use statement - JM | INITIAL | 2024-12-24T11:45:56+01:00 | 0.0.1-alpha |
| 1013.1.4949 | `openEHR-EHR-OBSERVATION.medication_statement.v0` | Medication use statement | REVIEWSUSPENDED | 2024-02-22T11:59:21+01:00 | 0.0.1-alpha |
| 1013.1.3546 | `openEHR-EHR-OBSERVATION.medication_use.v0` | Medication screening questionnaire | REJECTED | 2020-07-16T05:04:03+02:00 | 0.0.1 |
| 1013.1.5285 | `openEHR-EHR-OBSERVATION.menstrual_cycles.v0` | Menstrual cycles | REJECTED | 2021-12-16T09:41:25+01:00 | 0.0.1 |
| 1013.1.5657 | `openEHR-EHR-OBSERVATION.menstrual_diary.v1` | Menstrual diary | PUBLISHED | 2022-02-01T15:13:23+01:00 | 1.0.1 |
| 1013.1.1922 | `openEHR-EHR-OBSERVATION.menstruation.v1` | Menstrual cycle | PUBLISHED | 2022-02-01T15:12:19+01:00 | 1.0.1 |
| 1013.1.142 | `openEHR-EHR-OBSERVATION.menstruation_old.v0` | Menstruation | REJECTED | 2019-08-22T05:36:22+02:00 | 0.0.1 |
| 1013.1.7931 | `openEHR-EHR-OBSERVATION.mini_bestest.v1` | Mini-Balance Evaluation Systems Test (Mini-BESTest) | PUBLISHED | 2025-09-19T11:41:26+02:00 | 1.0.0 |
| 1013.1.2840 | `openEHR-EHR-OBSERVATION.mna_sf.v0` | Mini Nutritional Assessment - Short Form (MNA-SF) | INITIAL | 2026-11-10T21:02:00+01:00 | 0.0.1-alpha |
| 1013.1.5254 | `openEHR-EHR-OBSERVATION.modified_aldrete_score.v0` | Modified Aldrete score | DRAFT | 2021-03-03T06:23:06+01:00 | 0.0.1-alpha |
| 1013.1.128 | `openEHR-EHR-OBSERVATION.modified_barthel_index.v0` | Modified Barthel index | DRAFT | 2020-04-16T16:22:43+02:00 | 0.0.1-alpha |
| 1013.1.4671 | `openEHR-EHR-OBSERVATION.modified_rankin_scale.v1` | Modified Rankin Scale (mRS) | PUBLISHED | 2025-03-04T10:56:43+01:00 | 1.0.1 |
| 1013.1.3328 | `openEHR-EHR-OBSERVATION.moxfq.v0` | MOXFQ | DRAFT | 2021-03-01T01:56:58+01:00 | 0.0.1-alpha |
| 1013.1.1368 | `openEHR-EHR-OBSERVATION.msfc_score.v1` | MSFC score | DEPRECATED | 2025-05-27T02:11:55+02:00 | 1.0.1 |
| 1013.1.7894 | `openEHR-EHR-OBSERVATION.msfc_score.v2` | Multiple Sclerosis Functional Composite (MSFC) | PUBLISHED | 2025-05-27T02:09:19+02:00 | 2.1.0 |
| 1013.1.4755 | `openEHR-EHR-OBSERVATION.mskcc_bowel_function_instrument.v0` | MSKCC Bowel Function Instrument | DRAFT | 2020-05-11T16:14:14+02:00 | 0.0.1-alpha |
| 1013.1.5398 | `openEHR-EHR-OBSERVATION.mskcc_motzer.v0` | Memorial Sloan-Kettering Cancer Center (MSKCC/Motzer) score | DRAFT | 2021-04-29T09:44:34+02:00 | 0.0.1-alpha |
| 1013.1.4704 | `openEHR-EHR-OBSERVATION.murray_score.v0` | Murray score | DRAFT | 2020-05-15T12:58:02+02:00 | 0.0.1-alpha |
| 1013.1.2805 | `openEHR-EHR-OBSERVATION.must.v0` | Malnutrition Universal Screening Tool (MUST) | DRAFT | 2018-05-14T09:56:56+02:00 | 0.0.1-alpha |
| 1013.1.1035 | `openEHR-EHR-OBSERVATION.neonatal_skin_risk_assessment.v0` | Neonatal Skin Risk Assessment Scale (NSRAS) | DRAFT | 2019-10-08T07:02:15+02:00 | 0.0.1-alpha |
| 1013.1.7006 | `openEHR-EHR-OBSERVATION.neurologic_assessment_in_neuro_oncology_scale.v0` | Neurologic Assessment in Neuro-Oncology (NANO) scale | REVIEWSUSPENDED | 2025-04-25T14:16:17+02:00 | 0.0.1-alpha |
| 1013.1.3342 | `openEHR-EHR-OBSERVATION.news2.v1` | National Early Warning Score 2 (NEWS2) | PUBLISHED | 2023-07-11T12:57:47+02:00 | 1.0.5 |
| 1013.1.2423 | `openEHR-EHR-OBSERVATION.news_uk_rcp.v1` | National Early Warning Score (NEWS) | PUBLISHED | 2023-07-11T13:10:03+02:00 | 1.2.2 |
| 1013.1.2041 | `openEHR-EHR-OBSERVATION.nihss.v0` | NIHSS | DRAFT | 2025-10-01T08:39:20+02:00 | 0.0.1-alpha |
| 1013.1.1202 | `openEHR-EHR-OBSERVATION.nine_hole_peg_test.v1` | Nine Hole Peg Test | PUBLISHED | 2025-07-07T20:22:34+02:00 | 1.0.0 |
| 1013.1.3564 | `openEHR-EHR-OBSERVATION.nutrition_intake.v0` | Nutrition intake | DRAFT | 2018-12-06T08:31:14+01:00 | 0.0.1-alpha |
| 1013.1.2836 | `openEHR-EHR-OBSERVATION.nutritional_risk_screening.v1` | Nutritional Risk Screening (NRS 2002) | PUBLISHED | 2019-03-05T10:51:02+01:00 | 1.1.1 |
| 1013.1.1493 | `openEHR-EHR-OBSERVATION.nyha_heart_failure.v1` | New York Heart Association functional classification | PUBLISHED | 2025-12-12T15:22:31+01:00 | 1.0.3 |
| 1013.1.5798 | `openEHR-EHR-OBSERVATION.onews_se.v0` | Obstetric National Early Warning Score (ONEWS) - Sweden | INITIAL | 2021-10-05T09:33:24+02:00 | 0.0.1-alpha |
| 1013.1.63 | `openEHR-EHR-OBSERVATION.operation_record.v1` | Operation Record | REJECTED | 2017-01-07T02:31:43+01:00 |  |
| 1013.1.1464 | `openEHR-EHR-OBSERVATION.ophthalmic_tomography_examination.v0` | Ophthalmic tomography examination | INITIAL | 2015-06-25T22:29:00+02:00 | 0.0.1-alpha |
| 1013.1.267 | `openEHR-EHR-OBSERVATION.oral_fluid_intake.v1` | Oral fluid intake | REJECTED | 2015-11-23T01:02:11+01:00 |  |
| 1013.1.2616 | `openEHR-EHR-OBSERVATION.oucher_pain_scale.v0` | Oucher pain scale | DRAFT | 2016-09-26T03:16:20+02:00 | 0.0.1-alpha |
| 1013.1.5242 | `openEHR-EHR-OBSERVATION.oxford_elbow.v0` | Oxford Elbow Questionnaire Score (OSE) | DRAFT | 2021-03-01T01:57:00+01:00 | 0.0.1-alpha |
| 1013.1.5240 | `openEHR-EHR-OBSERVATION.oxford_hip.v0` | Oxford Hip Questionnaire Score (OHS) | DRAFT | 2021-03-01T01:56:54+01:00 | 0.0.1-alpha |
| 1013.1.5235 | `openEHR-EHR-OBSERVATION.oxford_knee.v0` | Oxford Knee Questionnaire Score (OKS) | DRAFT | 2021-03-01T01:56:42+01:00 | 0.0.1-alpha |
| 1013.1.5236 | `openEHR-EHR-OBSERVATION.oxford_shoulder.v0` | Oxford Shoulder Questionnaire Score (OSS) | DRAFT | 2021-03-01T01:56:43+01:00 | 0.0.1-alpha |
| 1013.1.5243 | `openEHR-EHR-OBSERVATION.oxford_shoulder_instability.v0` | Oxford Shoulder Instability Questionnaire Score (OSI) | DRAFT | 2021-03-01T01:57:01+01:00 | 0.0.1-alpha |
| 1013.1.1296 | `openEHR-EHR-OBSERVATION.paced_auditory_serial_addition_test.v1` | Paced Auditory Serial Addition Test | PUBLISHED | 2025-05-26T05:37:54+02:00 | 1.0.0 |
| 1013.1.5256 | `openEHR-EHR-OBSERVATION.padss.v0` | Post Anaesthesia Discharge Scoring System (PADSS) | DRAFT | 2021-03-03T07:05:04+01:00 | 0.0.1-alpha |
| 1013.1.5395 | `openEHR-EHR-OBSERVATION.pasi_score.v1` | Psoriasis Area Severity Index (PASI) | PUBLISHED | 2023-01-03T10:47:39+01:00 | 1.0.0 |
| 1013.1.1402 | `openEHR-EHR-OBSERVATION.pathology_test-blood_glucose.v1` | Blood Glucose Test Result | REJECTED | 2017-12-03T23:58:22+01:00 |  |
| 1013.1.1428 | `openEHR-EHR-OBSERVATION.pathology_test-lipids.v1` | Lipid Studies Test Result | REJECTED | 2017-12-03T23:58:39+01:00 |  |
| 1013.1.1401 | `openEHR-EHR-OBSERVATION.pathology_test.v1` | Pathology Test Result | REJECTED | 2017-12-03T23:57:29+01:00 |  |
| 1013.1.3710 | `openEHR-EHR-OBSERVATION.pefr_result.v0` | Peak expiratory flow result | REJECTED | 2022-10-24T08:44:05+02:00 | 0.0.1 |
| 1013.1.1331 | `openEHR-EHR-OBSERVATION.penetration_aspiration_scale.v0` | Penetration-aspiration scale | DRAFT | 2019-10-01T04:25:59+02:00 | 0.0.1-alpha |
| 1013.1.5149 | `openEHR-EHR-OBSERVATION.pews.v0` | Paediatric Early Warning Score (PEWS) | DRAFT | 2021-01-28T02:03:12+01:00 | 0.0.1-alpha |
| 1013.1.4537 | `openEHR-EHR-OBSERVATION.pf_ratio.v1` | PaO₂/FiO₂ ratio | PUBLISHED | 2020-09-02T09:28:11+02:00 | 1.0.0 |
| 1013.1.1864 | `openEHR-EHR-OBSERVATION.pga_eczema_treat.v0` | PGA eczema (TREAT) | DRAFT | 2019-10-01T07:30:01+02:00 | 0.0.1-alpha |
| 1013.1.4306 | `openEHR-EHR-OBSERVATION.phfrat1.v0` | Falls risk assessment screening tool (PHFRAT - part 1) | DRAFT | 2019-12-26T07:26:29+01:00 | 0.0.1-alpha |
| 1013.1.1645 | `openEHR-EHR-OBSERVATION.phq_9.v0` | Patient health questionnaire-9 (PHQ-9) | DRAFT | 2026-10-20T15:31:41+02:00 | 0.0.1-alpha |
| 1013.1.2876 | `openEHR-EHR-OBSERVATION.physical_activity.v0` | Physical activity | DRAFT | 2017-06-20T09:41:04+02:00 | 0.0.1-alpha |
| 1013.1.8270 | `openEHR-EHR-OBSERVATION.physical_activity_screening.v1` | Physical activity screening questionnaire | PUBLISHED | 2026-11-09T15:27:45+01:00 | 1.0.1 |
| 1013.1.5628 | `openEHR-EHR-OBSERVATION.physical_activity_screening_questionnaire.v0` | Physical activity screening questionnaire | INITIAL | 2021-07-27T09:37:29+02:00 | 0.0.1-alpha |
| 1013.1.7547 | `openEHR-EHR-OBSERVATION.physical_environment_screening.v0` | Physical environment screening | INITIAL | 2024-12-26T06:20:07+01:00 | 0.0.1-alpha |
| 1013.1.1870 | `openEHR-EHR-OBSERVATION.poem_score.v0` | POEM score | DRAFT | 2016-05-31T10:26:43+02:00 | 0.0.1-alpha |
| 1013.1.253 | `openEHR-EHR-OBSERVATION.postural_oedema.v1` | Postural oedema | REJECTED | 2017-03-14T04:36:59+01:00 |  |
| 1013.1.4720 | `openEHR-EHR-OBSERVATION.pregnancy_assertion.v0` | Pregnancy assertion | REVIEWSUSPENDED | 2026-04-14T09:43:04+02:00 | 0.0.1-alpha |
| 1013.1.254 | `openEHR-EHR-OBSERVATION.pregnancy_test.v0` | Pregnancy test result | DRAFT | 2019-03-25T08:48:25+01:00 | 0.0.1-alpha |
| 1013.1.4442 | `openEHR-EHR-OBSERVATION.problem_screening.v1` | Problem/Diagnosis screening questionnaire | PUBLISHED | 2025-08-31T17:09:30+02:00 | 1.0.8 |
| 1013.1.4439 | `openEHR-EHR-OBSERVATION.procedure_screening.v1` | Procedure screening questionnaire | PUBLISHED | 2025-07-28T13:26:35+02:00 | 1.0.6 |
| 1013.1.1647 | `openEHR-EHR-OBSERVATION.progress_note.v1` | Progress note | PUBLISHED | 2026-11-04T22:07:55+01:00 | 1.1.2 |
| 1013.1.4810 | `openEHR-EHR-OBSERVATION.promis.v0` | PROMIS | DRAFT | 2026-02-26T10:22:22+01:00 | 0.0.1-alpha |
| 1013.1.170 | `openEHR-EHR-OBSERVATION.pulse.v1` | Pulse/Heart beat | DEPRECATED | 2019-12-17T14:58:57+01:00 | 1.0.3 |
| 1013.1.4295 | `openEHR-EHR-OBSERVATION.pulse.v2` | Pulse/Heart beat | PUBLISHED | 2026-04-27T14:21:15+02:00 | 2.0.9 |
| 1013.1.2337 | `openEHR-EHR-OBSERVATION.pulse_deficit.v0` | Pulse deficit | DRAFT | 2015-11-23T00:21:38+01:00 | 0.0.1-alpha |
| 1013.1.3084 | `openEHR-EHR-OBSERVATION.pulse_oximetry.v1` | Pulse oximetry | PUBLISHED | 2026-04-29T15:31:53+02:00 | 1.1.6 |
| 1013.1.3813 | `openEHR-EHR-OBSERVATION.qsofa_score.v1` | qSOFA score | PUBLISHED | 2020-04-02T12:17:28+02:00 | 1.0.1 |
| 1013.1.7753 | `openEHR-EHR-OBSERVATION.rass.v0` | Richmond Agitation-Sedation Scale (RASS) | DRAFT | 2025-12-10T06:18:07+01:00 | 0.0.1-alpha |
| 1013.1.5397 | `openEHR-EHR-OBSERVATION.reach_b.v0` | REACH-B score | DRAFT | 2021-04-29T09:36:03+02:00 | 0.0.1-alpha |
| 1013.1.5629 | `openEHR-EHR-OBSERVATION.reaction_screening.v0` | Reaction screening | INITIAL | 2021-07-27T10:23:21+02:00 | 0.0.1-alpha |
| 1013.1.1399 | `openEHR-EHR-OBSERVATION.refraction.v0` | Refraction assessment | DRAFT | 2021-02-17T14:40:43+01:00 | 0.0.1-alpha |
| 1013.1.8517 | `openEHR-EHR-OBSERVATION.refractive_error_measurement.v0` | Refractive error measurement | INITIAL | 2026-10-25T23:27:48+01:00 | 0.0.1-alpha |
| 1013.1.2209 | `openEHR-EHR-OBSERVATION.registro_periodontal_simplificado.v0` | Periodontal screening and recording | INITIAL | 2015-08-06T09:32:21+02:00 | 0.0.1-alpha |
| 1013.1.145 | `openEHR-EHR-OBSERVATION.respiration.v1` | Respirations | DEPRECATED | 2019-11-22T14:00:40+01:00 | 1.2.1 |
| 1013.1.4218 | `openEHR-EHR-OBSERVATION.respiration.v2` | Respiration | PUBLISHED | 2026-01-15T16:11:29+01:00 | 2.0.12 |
| 1013.1.7178 | `openEHR-EHR-OBSERVATION.revised_cardiac_risk_index.v1` | Revised cardiac risk index | PUBLISHED | 2024-04-22T15:31:07+02:00 | 1.0.0 |
| 1013.1.2606 | `openEHR-EHR-OBSERVATION.richmond_agitation_sedation_scale.v0` | Richmond agitation sedation scale (RASS) | REJECTED | 2025-12-10T06:20:43+01:00 | 0.0.1 |
| 1013.1.1625 | `openEHR-EHR-OBSERVATION.rinne_weber_result.v0` | Rinne and Weber test results | DRAFT | 2018-03-26T06:27:12+02:00 | 0.0.1-alpha |
| 1013.1.3324 | `openEHR-EHR-OBSERVATION.safas.v0` | SAFAS | DRAFT | 2021-03-01T01:56:53+01:00 | 0.0.1-alpha |
| 1013.1.2661 | `openEHR-EHR-OBSERVATION.sara_scale.v0` | SARA ataxia scale | DRAFT | 2016-11-08T12:55:08+01:00 | 0.0.1-alpha |
| 1013.1.3719 | `openEHR-EHR-OBSERVATION.scoff_questionnaire.v0` | Eating disorder screening (SCOFF) | DRAFT | 2019-04-04T08:58:32+02:00 | 0.0.1-alpha |
| 1013.1.1868 | `openEHR-EHR-OBSERVATION.scorad_index.v0` | SCORAD index | DRAFT | 2021-02-10T08:15:46+01:00 | 0.0.1-alpha |
| 1013.1.6184 | `openEHR-EHR-OBSERVATION.self_test_result-pregnancy.v0` | Pregnancy test result | INITIAL | 2022-04-12T06:36:37+02:00 | 0.0.1-alpha |
| 1013.1.6183 | `openEHR-EHR-OBSERVATION.self_test_result.v0` | Self-test result | INITIAL | 2022-04-12T06:35:20+02:00 | 0.0.1-alpha |
| 1013.1.5727 | `openEHR-EHR-OBSERVATION.sexual_health_screening.v0` | Sexual health screening questionnaire | INITIAL | 2022-07-01T13:41:38+02:00 | 0.0.1-alpha |
| 1013.1.6526 | `openEHR-EHR-OBSERVATION.simplified_tanner_whitehouse_3.v1` | Simplified Tanner-Whitehouse III assessment | PUBLISHED | 2022-12-16T12:55:49+01:00 | 1.0.0 |
| 1013.1.3341 | `openEHR-EHR-OBSERVATION.six_cit.v0` | 6 Item Cognitive Impairment Test (6CIT) | DRAFT | 2021-07-28T04:09:38+02:00 | 0.0.1-alpha |
| 1013.1.2861 | `openEHR-EHR-OBSERVATION.skeletal_age.v0` | Skeletal age | DRAFT | 2020-04-17T15:47:30+02:00 | 0.0.1-alpha |
| 1013.1.5370 | `openEHR-EHR-OBSERVATION.soas_r.v0` | Staff Observation Aggression Scale - Revised (SOAS-R) | REVIEWSUSPENDED | 2021-04-28T08:43:41+02:00 | 0.0.1-alpha |
| 1013.1.5364 | `openEHR-EHR-OBSERVATION.soas_re.v0` | The Staff Observation Aggression Scale – Revised Emergency (SOAS-RE) | DRAFT | 2021-04-19T15:33:04+02:00 | 0.0.1-alpha |
| 1013.1.4668 | `openEHR-EHR-OBSERVATION.social_context_screening.v1` | Social context screening questionnaire | PUBLISHED | 2026-06-19T09:34:27+02:00 | 1.1.2 |
| 1013.1.7541 | `openEHR-EHR-OBSERVATION.social_context_screening_hl.v0` | Social context screening questionnaire JM | INITIAL | 2024-10-29T13:26:41+01:00 | 0.0.1-alpha |
| 1013.1.4696 | `openEHR-EHR-OBSERVATION.sofa_score.v0` | SOFA score | DRAFT | 2021-03-17T16:20:44+01:00 | 0.0.1-alpha |
| 1013.1.8294 | `openEHR-EHR-OBSERVATION.specular_microscopy.v0` | Corneal specular microscopy | INITIAL | 2026-04-29T23:18:18+02:00 | 0.0.1-alpha |
| 1013.1.255 | `openEHR-EHR-OBSERVATION.speech.v0` | Speech | DRAFT | 2018-08-31T09:27:28+02:00 | 0.0.1-alpha |
| 1013.1.1414 | `openEHR-EHR-OBSERVATION.spirometry_result.v1` | Spirometry result | DEPRECATED | 2022-12-13T09:29:42+01:00 | 1.0.1 |
| 1013.1.6655 | `openEHR-EHR-OBSERVATION.spirometry_result.v2` | Spirometry result | PUBLISHED | 2024-02-05T11:59:57+01:00 | 2.0.1 |
| 1013.1.3332 | `openEHR-EHR-OBSERVATION.sprs.v0` | Spastic Paraplegia Rating Scale (SPRS) | DRAFT | 2018-06-30T09:21:26+02:00 | 0.0.1-alpha |
| 1013.1.68 | `openEHR-EHR-OBSERVATION.story.v1` | Story/History | PUBLISHED | 2026-06-18T12:26:08+02:00 | 1.3.3 |
| 1013.1.2290 | `openEHR-EHR-OBSERVATION.stratify_no.v1` | STRATIFY Falls Risk Assessment Tool | PUBLISHED | 2018-05-29T17:04:38+02:00 | 1.0.1 |
| 1013.1.147 | `openEHR-EHR-OBSERVATION.substance_use-alcohol.v1` | Alcohol Consumption | REJECTED | 2018-08-31T09:33:48+02:00 |  |
| 1013.1.148 | `openEHR-EHR-OBSERVATION.substance_use-caffeine.v0` | Caffeine consumption | REJECTED | 2018-08-31T09:34:29+02:00 | 0.0.1 |
| 1013.1.149 | `openEHR-EHR-OBSERVATION.substance_use-tobacco.v0` | Tobacco Use | REJECTED | 2018-08-31T09:32:34+02:00 | 0.0.1 |
| 1013.1.146 | `openEHR-EHR-OBSERVATION.substance_use.v0` | Substance use | DRAFT | 2024-06-14T15:25:20+02:00 | 0.0.1-alpha |
| 1013.1.4903 | `openEHR-EHR-OBSERVATION.substance_use_screening.v1` | Substance use screening questionnaire | PUBLISHED | 2025-01-27T08:34:08+01:00 | 1.0.3 |
| 1013.1.4802 | `openEHR-EHR-OBSERVATION.symptom_sign.v0` | Symptom/Sign | INITIAL | 2020-06-07T07:47:23+02:00 | 0.0.1-alpha |
| 1013.1.4432 | `openEHR-EHR-OBSERVATION.symptom_sign_screening.v1` | Symptom/sign screening questionnaire | PUBLISHED | 2025-08-05T10:58:32+02:00 | 1.0.7 |
| 1013.1.2798 | `openEHR-EHR-OBSERVATION.tanner.v1` | Tanner stages | PUBLISHED | 2017-10-06T12:48:34+02:00 | 1.0.0 |
| 1013.1.5238 | `openEHR-EHR-OBSERVATION.tegner_activity_level_scale.v0` | Tegner Activity Level Scale | DRAFT | 2021-03-01T01:56:48+01:00 | 0.0.1-alpha |
| 1013.1.3183 | `openEHR-EHR-OBSERVATION.telecommunication.v0` | Telecommunication Record | DRAFT | 2018-04-11T07:36:54+02:00 | 0.0.1-alpha |
| 1013.1.285 | `openEHR-EHR-OBSERVATION.temperature.v0` | Temperature | DRAFT | 2024-05-15T12:43:05+02:00 | 0.0.1-alpha |
| 1013.1.256 | `openEHR-EHR-OBSERVATION.tendon_babinski_reflexes.v0` | Tendon and Babinski reflexes | REJECTED | 2018-09-10T09:10:05+02:00 | 0.0.1 |
| 1013.1.2863 | `openEHR-EHR-OBSERVATION.testicular_volume.v1` | Testicular volume | PUBLISHED | 2018-06-21T09:03:21+02:00 | 1.0.0 |
| 1013.1.266 | `openEHR-EHR-OBSERVATION.third_party_observation-hearing.v1` | Carer observation about hearing of subject | REJECTED | 2014-03-16T06:39:51+01:00 |  |
| 1013.1.265 | `openEHR-EHR-OBSERVATION.third_party_observation.v0` | Carer observation | DRAFT | 2019-07-12T03:21:15+02:00 | 0.0.1-alpha |
| 1013.1.1200 | `openEHR-EHR-OBSERVATION.timed_25_foot_walk.v1` | Timed 25-Foot Walk | PUBLISHED | 2025-05-21T23:46:37+02:00 | 1.0.0 |
| 1013.1.2412 | `openEHR-EHR-OBSERVATION.tnm_stage_pathological.v0` | TNM stage - pathological | REJECTED | 2019-10-08T04:10:50+02:00 | 0.0.1 |
| 1013.1.1629 | `openEHR-EHR-OBSERVATION.tobacco_use.v0` | Tobacco Use | INITIAL | 2018-11-21T08:16:56+01:00 | 0.0.1-alpha |
| 1013.1.6601 | `openEHR-EHR-OBSERVATION.transmission_screening.v0` | Infection transmission screening questionnaire | INITIAL | 2023-07-19T06:30:21+02:00 | 0.0.1-alpha |
| 1013.1.4400 | `openEHR-EHR-OBSERVATION.travel_history.v0` | Travel trip history | INITIAL | 2020-03-22T15:55:39+01:00 | 0.0.1-alpha |
| 1013.1.4431 | `openEHR-EHR-OBSERVATION.travel_screening.v1` | Travel screening questionnaire | PUBLISHED | 2025-08-25T13:37:15+02:00 | 1.0.0 |
| 1013.1.5461 | `openEHR-EHR-OBSERVATION.trunk_impairment_scale.v0` | Trunk Impairment Scale (TIS) | DRAFT | 2021-06-04T03:33:18+02:00 | 0.0.1-alpha |
| 1013.1.73 | `openEHR-EHR-OBSERVATION.tympanogram.v1` | Tympanogram Result | REJECTED | 2015-02-05T06:52:03+01:00 |  |
| 1013.1.1845 | `openEHR-EHR-OBSERVATION.tympanogram_226hz.v0` | Tympanogram test result - 226Hz | DRAFT | 2018-03-27T06:48:30+02:00 | 0.0.1-alpha |
| 1013.1.1844 | `openEHR-EHR-OBSERVATION.tympanogram_hf.v0` | Tympanogram test result - high frequency | DRAFT | 2018-03-27T07:09:29+02:00 | 0.0.1-alpha |
| 1013.1.150 | `openEHR-EHR-OBSERVATION.urinalysis.v1` | Urinalysis | PUBLISHED | 2019-07-12T03:34:07+02:00 | 1.1.0 |
| 1013.1.262 | `openEHR-EHR-OBSERVATION.urine_output.v1` | Urine output | REJECTED | 2017-03-14T05:26:10+01:00 |  |
| 1013.1.261 | `openEHR-EHR-OBSERVATION.uterine_contractions.v0` | Uterine contractions | DRAFT | 2019-07-12T04:11:46+02:00 | 0.0.1-alpha |
| 1013.1.8301 | `openEHR-EHR-OBSERVATION.v_risk_y.v1` | Violence Risk Assessment Checklist for Youth aged 12-18 (V-RISK-Y) | PUBLISHED | 2026-09-16T14:45:56+02:00 | 1.0.0 |
| 1013.1.6555 | `openEHR-EHR-OBSERVATION.vaccination_screening.v0` | Vaccination screening questionnaire | INITIAL | 2023-07-19T07:32:26+02:00 | 0.0.1-alpha |
| 1013.1.6581 | `openEHR-EHR-OBSERVATION.vaccination_status.v0` | Vaccination status | INITIAL | 2022-11-17T10:08:41+01:00 | 0.0.1-alpha |
| 1013.1.7342 | `openEHR-EHR-OBSERVATION.ventilator_record.v0` | Ventilator record | INITIAL | 2024-06-04T10:34:42+02:00 | 0.0.1-alpha |
| 1013.1.3322 | `openEHR-EHR-OBSERVATION.visaa.v0` | VISA-A | DRAFT | 2021-03-01T01:56:51+01:00 | 0.0.1-alpha |
| 1013.1.1291 | `openEHR-EHR-OBSERVATION.visual_acuity.v0` | Visual acuity test | TEAMREVIEW | 2026-08-20T16:45:46+02:00 | 0.0.1-alpha |
| 1013.1.1370 | `openEHR-EHR-OBSERVATION.visual_field_measurement.v0` | Visual field measurement | DRAFT | 2025-04-24T08:55:46+02:00 | 0.0.1-alpha |
| 1013.1.6504 | `openEHR-EHR-OBSERVATION.vital_status.v0` | Vital status | INITIAL | 2024-12-29T01:07:41+01:00 | 0.0.1-alpha |
| 1013.1.3810 | `openEHR-EHR-OBSERVATION.vte_risk_uk_nice.v0` | VTE risk (UK NICE) | INITIAL | 2019-05-23T13:59:38+02:00 | 0.0.1-alpha |
| 1013.1.2814 | `openEHR-EHR-OBSERVATION.waist_circumference.v1` | Waist circumference | PUBLISHED | 2025-10-08T02:40:07+02:00 | 1.0.5 |
| 1013.1.2916 | `openEHR-EHR-OBSERVATION.waist_height_ratio.v0` | Waist-height ratio | DRAFT | 2017-08-15T08:36:38+02:00 | 0.0.1-alpha |
| 1013.1.332 | `openEHR-EHR-OBSERVATION.waist_hip_ratio.v0` | Waist-hip ratio | DRAFT | 2020-05-15T13:56:30+02:00 | 0.0.1-alpha |
| 1013.1.260 | `openEHR-EHR-OBSERVATION.warble_tones_hearing_test.v1` | Warble Tones Hearing Test | REJECTED | 2014-03-16T06:49:41+01:00 |  |
| 1013.1.1036 | `openEHR-EHR-OBSERVATION.waterlow_score.v0` | Waterlow score | DRAFT | 2019-06-17T17:48:26+02:00 | 0.0.1-alpha |
| 1013.1.259 | `openEHR-EHR-OBSERVATION.word_list_hearing_test.v1` | Word List Hearing Test | REJECTED | 2014-03-16T06:50:52+01:00 |  |
| 1013.1.5066 | `openEHR-EHR-OBSERVATION.ygtss_revised.v1` | Yale Global Tic Severity Scale - Revised (YGTSS-R) | PUBLISHED | 2022-01-28T08:56:20+01:00 | 1.0.0 |
| 1013.1.3781 | `openEHR-EHR-OBSERVATION.ymrs.v0` | Young Mania Rating Scale (YMRS) | DRAFT | 2019-05-10T06:39:55+02:00 | 0.0.1-alpha |
| 1013.1.631 | `openEHR-EHR-SECTION.adhoc.v1` | Ad hoc heading | PUBLISHED | 2026-02-25T20:01:10+01:00 | 1.0.12 |
| 1013.1.5350 | `openEHR-EHR-SECTION.advance_care.v0` | Advance care | DRAFT | 2021-08-31T09:42:18+02:00 | 0.0.1-alpha |
| 1013.1.607 | `openEHR-EHR-SECTION.adverse_reaction_list.v0` | Adverse reaction list | DRAFT | 2020-10-28T09:11:05+01:00 | 0.0.1-alpha |
| 1013.1.2182 | `openEHR-EHR-SECTION.analyze_encounter.v0` | Analysis of clinical encounter | INITIAL | 2016-07-15T23:33:39+02:00 | 0.0.1-alpha |
| 1013.1.1707 | `openEHR-EHR-SECTION.clinical_decision.v0` | Clinical decision | INITIAL | 2016-07-15T23:34:37+02:00 | 0.0.1-alpha |
| 1013.1.1705 | `openEHR-EHR-SECTION.clinical_image_acquisition.v0` | Clinical image acquisition and validation | INITIAL | 2016-07-15T23:36:47+02:00 | 0.0.1-alpha |
| 1013.1.152 | `openEHR-EHR-SECTION.conclusion.v0` | Conclusion | DRAFT | 2020-10-29T02:25:20+01:00 | 0.0.1-alpha |
| 1013.1.2261 | `openEHR-EHR-SECTION.diagnostic_model.v0` | Diagnostic model | INITIAL | 2015-10-01T16:30:37+02:00 | 0.0.1-alpha |
| 1013.1.608 | `openEHR-EHR-SECTION.diagnostic_reports.v0` | Diagnostic test results | DRAFT | 2019-04-12T09:44:37+02:00 | 0.0.1-alpha |
| 1013.1.1703 | `openEHR-EHR-SECTION.diagnostic_test_planning.v0` | Diagnostic test planning | INITIAL | 2016-07-21T13:45:39+02:00 | 0.0.1-alpha |
| 1013.1.153 | `openEHR-EHR-SECTION.discharge_summary_outline.v1` | Discharge Summary Outline | REJECTED | 2009-03-13T17:18:05+01:00 |  |
| 1013.1.2121 | `openEHR-EHR-SECTION.eye_fundus_acquisition.v0` | Eye fundus image acquisition and validation | INITIAL | 2016-07-15T23:47:41+02:00 | 0.0.1-alpha |
| 1013.1.3737 | `openEHR-EHR-SECTION.family_history.v0` | Family history | DRAFT | 2020-10-28T09:05:40+01:00 | 0.0.1-alpha |
| 1013.1.1706 | `openEHR-EHR-SECTION.image_test_analysis.v0` | Image test analysis | INITIAL | 2016-07-15T23:48:28+02:00 | 0.0.1-alpha |
| 1013.1.3727 | `openEHR-EHR-SECTION.immunisation_list.v0` | Vaccination list | DRAFT | 2020-10-28T08:52:51+01:00 | 0.0.1-alpha |
| 1013.1.2509 | `openEHR-EHR-SECTION.intraocular_injection.v0` | Intraocular injection | INITIAL | 2016-07-16T17:22:11+02:00 | 0.0.1-alpha |
| 1013.1.1704 | `openEHR-EHR-SECTION.intraocular_pressure_study.v0` | Intraocular pressure study | INITIAL | 2016-07-15T23:50:03+02:00 | 0.0.1-alpha |
| 1013.1.3537 | `openEHR-EHR-SECTION.lab_test_report.v0` | Lab report | INITIAL | 2018-11-12T07:02:37+01:00 | 0.0.1-alpha |
| 1013.1.1709 | `openEHR-EHR-SECTION.laser.v0` | Ophthalmic laser procedure | INITIAL | 2016-07-15T23:50:39+02:00 | 0.0.1-alpha |
| 1013.1.5019 | `openEHR-EHR-SECTION.lifestyle_risk_factors.v0` | Lifestyle risk factors | DRAFT | 2020-10-28T01:22:36+01:00 | 0.0.1-alpha |
| 1013.1.2155 | `openEHR-EHR-SECTION.medication_administration.v0` | Medication administration procedure | INITIAL | 2016-07-15T23:51:21+02:00 | 0.0.1-alpha |
| 1013.1.609 | `openEHR-EHR-SECTION.medication_list.v0` | Medication list | DRAFT | 2020-10-28T09:01:20+01:00 | 0.0.1-alpha |
| 1013.1.1760 | `openEHR-EHR-SECTION.next_step_planning.v0` | Next step planning | INITIAL | 2016-07-24T16:38:41+02:00 | 0.0.1-alpha |
| 1013.1.1756 | `openEHR-EHR-SECTION.patients_admittance.v0` | Patient's admittance | INITIAL | 2016-07-15T23:52:20+02:00 | 0.0.1-alpha |
| 1013.1.1701 | `openEHR-EHR-SECTION.patients_background.v0` | Patients background | INITIAL | 2016-07-15T23:53:02+02:00 | 0.0.1-alpha |
| 1013.1.610 | `openEHR-EHR-SECTION.problem_list.v0` | Problem list | DRAFT | 2021-04-12T12:51:30+02:00 | 0.0.1-alpha |
| 1013.1.611 | `openEHR-EHR-SECTION.referral_details.v0` | Referral details | DRAFT | 2020-10-28T06:08:37+01:00 | 0.0.1-alpha |
| 1013.1.3538 | `openEHR-EHR-SECTION.result_details.v0` | Lab result details | INITIAL | 2018-11-12T07:33:52+01:00 | 0.0.1-alpha |
| 1013.1.339 | `openEHR-EHR-SECTION.soap.v0` | SOAP headings | DRAFT | 2020-10-28T03:40:02+01:00 | 0.0.1-alpha |
| 1013.1.1710 | `openEHR-EHR-SECTION.surgery_procedure.v0` | Ophthalmic surgical procedure | INITIAL | 2016-07-24T21:59:24+02:00 | 0.0.1-alpha |
| 1013.1.2143 | `openEHR-EHR-SECTION.visual_acuity_study.v0` | Visual acuity study | INITIAL | 2016-07-15T23:56:50+02:00 | 0.0.1-alpha |
| 1013.1.278 | `openEHR-EHR-SECTION.vital_signs.v0` | Vital signs | DRAFT | 2020-10-28T14:10:33+01:00 | 0.0.1-alpha |

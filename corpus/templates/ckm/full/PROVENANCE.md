# CKM template library (full pack) — provenance

Every template the official openEHR CKM (`https://ckm.openehr.org/ckm/rest/v1`) publishes, exported
by CKM itself as an Operational Template and vendored verbatim by
`scripts/vendor/ckm-templates.sh` on 2026-11-02T00:03:32Z.

This is the BREADTH pack: real-world OPT 1.4 shapes for the reader /
WebTemplate builder gates. The curated hospital-simulation journey
pack is the parent directory (its own `PROVENANCE.md`); the slugs here
are derived from CKM display names and are NOT a naming contract.

## Licensing

CKM publishes no repository-level license; each OPT embeds its source
archetypes' `licence` metadata, and the pack is **mixed**. Counted by
this script over the vendored files at the stamp above: **113** name
CC-BY-SA 4.0, **10** name CC-BY-SA 3.0, **0** name both, and
**182** name no Creative Commons licence. Read the individual
file; its own metadata is the authority. Vendored verbatim, so
authorship and licence metadata ride along; root reference copies:
`LICENSE-CC-BY-SA-3.0` and `LICENSE-CC-BY-SA-4.0`, and `REUSE.toml`
declares this tree as `CC-BY-SA-3.0 AND CC-BY-SA-4.0`.

- published by CKM: **306**
- vendored: **305**
- unreachable: **1**

## Unreachable (recorded, not skipped)

CKM answers 404 for resources held in a private incubator; they are
only exportable by a signed-in account with access.

| cid | display name | status |
|---|---|---|
| 1013.26.14 | Heart Failure Clinic First Visit Summary | DRAFT |

## Vendored

| cid | file | display name | status | modified | asset version |
|---|---|---|---|---|---|
| 1013.26.1 | `demo-with-hide-on-form.opt` | Demo with hide-on-form | DRAFT | 2010-01-19T08:19:03+01:00 | 1 |
| 1013.26.1005 | `additional-notes-jm.opt` | Additional notes - JM | INITIAL | 2025-12-14T09:42:23+01:00 | 3 |
| 1013.26.101 | `classification-and-treatment-of-amd.opt` | Classification and treatment of AMD | INITIAL | 2016-07-29T02:53:26+02:00 | 4 |
| 1013.26.1010 | `bpg-administration-jm.opt` | BPG administration - JM | INITIAL | 2024-12-27T00:52:58+01:00 | 2 |
| 1013.26.1011 | `non-treponemal-screening-jm.opt` | Non-treponemal screening - JM | INITIAL | 2024-12-24T12:15:03+01:00 | 1 |
| 1013.26.1012 | `treponemal-confirmation-test-jm.opt` | Treponemal confirmation test- JM | INITIAL | 2024-12-24T12:15:55+01:00 | 1 |
| 1013.26.1013 | `obstetric-summary-jm.opt` | Obstetric summary - JM | INITIAL | 2024-12-29T08:00:36+01:00 | 2 |
| 1013.26.1014 | `bpg-medication-screening-jm.opt` | BPG Medication screening - JM | INITIAL | 2024-12-24T12:25:47+01:00 | 1 |
| 1013.26.1029 | `microbiology-culture-findings-jm.opt` | Microbiology culture findings - JM | INITIAL | 2024-12-30T00:02:36+01:00 | 2 |
| 1013.26.1030 | `molecular-microbial-test-findings-jm.opt` | Molecular microbial test findings - JM | INITIAL | 2024-12-24T13:05:45+01:00 | 1 |
| 1013.26.1032 | `white-cell-count-and-differential.opt` | White cell count and differential | INITIAL | 2024-12-24T13:10:57+01:00 | 1 |
| 1013.26.1033 | `csf-blood-cell-count-and-differential.opt` | CSF blood cell count and differential | INITIAL | 2024-12-24T13:11:31+01:00 | 1 |
| 1013.26.1034 | `india-ink-stain-findings.opt` | India ink stain findings | INITIAL | 2024-12-30T00:44:30+01:00 | 2 |
| 1013.26.1035 | `microbiology-parasitology-findings.opt` | Microbiology parasitology findings | INITIAL | 2024-12-30T00:09:25+01:00 | 2 |
| 1013.26.1036 | `rheumatic-fever-heart-disease-case-investigation-form.opt` | Rheumatic fever_heart disease case investigation form | INITIAL | 2024-12-30T02:51:59+01:00 | 6 |
| 1013.26.1037 | `ophthalmia-neonatorum-case-investigation-form.opt` | Ophthalmia neonatorum case investigation form | INITIAL | 2024-12-30T02:07:37+01:00 | 4 |
| 1013.26.1038 | `pertussis-case-investigation-form.opt` | Pertussis case investigation form | INITIAL | 2024-12-30T02:11:40+01:00 | 4 |
| 1013.26.1040 | `management-notes-jm.opt` | Management notes - JM | INITIAL | 2025-12-12T04:59:32+01:00 | 2 |
| 1013.26.1047 | `final-classification-jm.opt` | Final classification - JM | INITIAL | 2025-12-19T03:27:46+01:00 | 7 |
| 1013.26.1056 | `birth-detail-jm.opt` | Birth detail - JM | INITIAL | 2024-12-27T00:50:30+01:00 | 2 |
| 1013.26.1063 | `antibiotic-prophylaxis.opt` | Antibiotic prophylaxis | INITIAL | 2024-12-27T01:15:30+01:00 | 1 |
| 1013.26.1092 | `person-jm-mother.opt` | Person - JM Mother | INITIAL | 2025-12-12T06:33:51+01:00 | 4 |
| 1013.26.1094 | `father-demographic-information-jm.opt` | Father demographic information - JM | INITIAL | 2024-12-29T02:43:19+01:00 | 1 |
| 1013.26.1097 | `specimen-collection-details-jm.opt` | Specimen collection details - JM | INITIAL | 2024-12-29T04:31:57+01:00 | 4 |
| 1013.26.1101 | `gram-stain-findings.opt` | Gram stain findings | INITIAL | 2024-12-30T00:31:28+01:00 | 3 |
| 1013.26.1102 | `ziehl-neelson-stain-findings.opt` | Ziehl-Neelson stain findings | INITIAL | 2024-12-30T00:46:42+01:00 | 2 |
| 1013.26.111 | `patient-admission-into-the-long-term-treatment-for-wet-amd.opt` | Patient admission into the long-term treatment for wet AMD | INITIAL | 2016-07-29T02:55:45+02:00 | 2 |
| 1013.26.1114 | `laboratory-test-screening-jm.opt` | Laboratory test screening - JM | INITIAL | 2025-07-13T06:26:21+02:00 | 5 |
| 1013.26.1117 | `other-investigation-screening-jm.opt` | Other investigation screening - JM | INITIAL | 2025-07-12T11:50:30+02:00 | 3 |
| 1013.26.1118 | `imaging-examination-screening-jm.opt` | Imaging examination screening - JM | INITIAL | 2025-07-12T11:19:58+02:00 | 2 |
| 1013.26.112 | `intraocular-injection.opt` | Intraocular injection | INITIAL | 2016-07-29T00:58:27+02:00 | 3 |
| 1013.26.1129 | `immunoglobulin-test-finding-jm.opt` | Immunoglobulin test finding - JM | INITIAL | 2024-12-29T23:09:37+01:00 | 2 |
| 1013.26.113 | `amd-treatment.opt` | AMD treatment | INITIAL | 2016-07-29T01:03:12+02:00 | 2 |
| 1013.26.114 | `follow-up-schedule-for-the-treatment-of-wet-amd.opt` | Follow-up schedule for the treatment of wet AMD | INITIAL | 2016-07-29T01:41:10+02:00 | 2 |
| 1013.26.115 | `visual-acuity-study.opt` | Visual acuity study | INITIAL | 2016-07-24T12:38:15+02:00 | 2 |
| 1013.26.116 | `clinical-image-acquisition-and-validation-nmr.opt` | Clinical image acquisition and validation NMR | INITIAL | 2016-07-25T16:26:47+02:00 | 4 |
| 1013.26.1166 | `echocardiography-findings-jm.opt` | Echocardiography findings - JM | INITIAL | 2024-12-30T02:46:46+01:00 | 2 |
| 1013.26.117 | `clinical-image-acquisition-and-validation-oct.opt` | Clinical image acquisition and validation OCT | INITIAL | 2016-07-17T19:49:52+02:00 | 2 |
| 1013.26.118 | `next-step-remote-assessment-or-schedule-additional-diagnostic-tests.opt` | Next step: remote assessment or schedule additional diagnostic tests | INITIAL | 2016-07-17T15:11:54+02:00 | 1 |
| 1013.26.119 | `diagnostic-tests-for-monitoring-the-treatment-of-wet-amd.opt` | Diagnostic tests for monitoring the treatment of wet AMD | INITIAL | 2016-07-29T03:02:45+02:00 | 2 |
| 1013.26.12 | `epsos-active-problems-section-1-3-6-1-4-1-19376-1-5-3-1-3-6.opt` | epSOS Active Problems Section 1.3.6.1.4.1.19376.1.5.3.1.3.6 | DRAFT | 2013-05-30T01:39:51+02:00 | 1 |
| 1013.26.1218 | `intervention-summary-jm.opt` | Intervention summary - JM | INITIAL | 2025-07-13T03:45:24+02:00 | 1 |
| 1013.26.1219 | `2025-master-infectious-disease-case-investigation-form-jm.opt` | 2025 MASTER Infectious disease case investigation form - JM | INITIAL | 2025-12-19T03:31:04+01:00 | 5 |
| 1013.26.1222 | `typhoid-case-investigation-form-jm.opt` | Typhoid case investigation form - JM | INITIAL | 2025-12-19T03:44:33+01:00 | 7 |
| 1013.26.123 | `image-test-analysis-for-amd.opt` | Image test analysis for AMD | INITIAL | 2016-07-17T21:10:56+02:00 | 1 |
| 1013.26.1230 | `tetanus-case-investigation-form-jm.opt` | Tetanus case investigation form - JM | INITIAL | 2025-12-19T03:49:57+01:00 | 4 |
| 1013.26.1235 | `hansen-s-disease-case-investigation-form-jm.opt` | Hansen's disease case investigation form - JM | INITIAL | 2025-12-19T03:58:12+01:00 | 4 |
| 1013.26.1236 | `inpatient-episode-details-jm.opt` | Inpatient episode details - JM | INITIAL | 2025-07-14T06:56:21+02:00 | 1 |
| 1013.26.1238 | `hepatitis-b-c-case-investigation-form-jm.opt` | Hepatitis B_C case investigation form - JM | INITIAL | 2025-12-19T03:38:38+01:00 | 4 |
| 1013.26.124 | `evaluation-of-the-remote-assessment-model-for-wet-amd.opt` | Evaluation of the remote assessment model for wet AMD | INITIAL | 2016-07-17T21:14:18+02:00 | 1 |
| 1013.26.1240 | `class-1-notification-form-jm.opt` | Class 1 notification form - JM | INITIAL | 2025-07-14T13:23:15+02:00 | 1 |
| 1013.26.125 | `amd-assessment.opt` | AMD assessment | INITIAL | 2016-07-29T03:08:12+02:00 | 2 |
| 1013.26.128 | `patient-s-enrolment-in-cg-follow-up.opt` | Patient’s enrolment in CG follow-up | INITIAL | 2016-07-24T11:20:22+02:00 | 3 |
| 1013.26.13 | `epsos-vital-signs-observations-1-3-6-1-4-1-19376-1-5-3-1-1-5-3-2.opt` | epSOS Vital Signs Observations 1.3.6.1.4.1.19376.1.5.3.1.1.5.3.2 | DRAFT | 2013-05-30T02:37:01+02:00 | 1 |
| 1013.26.131 | `patient-s-background-and-diagnosis-of-glaucoma.opt` | Patient's background and diagnosis of glaucoma | INITIAL | 2016-07-18T17:06:35+02:00 | 1 |
| 1013.26.132 | `classification-and-treatment-of-glaucoma.opt` | Classification and treatment of glaucoma | INITIAL | 2016-07-24T11:18:27+02:00 | 2 |
| 1013.26.133 | `patient-admission-into-the-follow-up-service-for-cg.opt` | Patient admission into the follow-up service for CG | INITIAL | 2016-07-24T11:27:11+02:00 | 2 |
| 1013.26.1335 | `yellow-fever-case-investigation-form-jm.opt` | Yellow fever case investigation form - JM | INITIAL | 2025-12-19T04:02:18+01:00 | 4 |
| 1013.26.1336 | `cholera-case-investigation-form-jm.opt` | Cholera case investigation form - JM | INITIAL | 2025-12-17T04:29:29+01:00 | 2 |
| 1013.26.1339 | `blood-donation-summary-jm.opt` | Blood donation summary - JM | INITIAL | 2025-12-15T05:30:17+01:00 | 1 |
| 1013.26.134 | `follow-up-schedule-for-cg.opt` | Follow-up schedule for CG | INITIAL | 2016-07-24T11:49:33+02:00 | 2 |
| 1013.26.1351 | `travel-event-jm.opt` | Travel event - JM | INITIAL | 2025-12-19T03:19:12+01:00 | 2 |
| 1013.26.1363 | `ips-problem-list.opt` | IPS Problem List | INITIAL | 2026-08-19T11:33:13+02:00 | 3 |
| 1013.26.1365 | `ips-allergies-and-intolerances.opt` | IPS Allergies and Intolerances | INITIAL | 2026-08-19T11:24:49+02:00 | 2 |
| 1013.26.138 | `planning-the-diagnostic-tests-to-follow-up-the-progression-of-cg.opt` | Planning the diagnostic tests to follow-up the progression of CG | INITIAL | 2016-07-24T11:46:08+02:00 | 1 |
| 1013.26.141 | `intraocular-pressure-study.opt` | Intraocular pressure study | INITIAL | 2016-07-25T16:26:59+02:00 | 2 |
| 1013.26.142 | `acquisition-and-validation-of-vf-test.opt` | Acquisition and validation of VF test | INITIAL | 2016-07-24T16:01:56+02:00 | 1 |
| 1013.26.143 | `next-step-in-cg-follow-up-service.opt` | Next step in CG follow-up service | INITIAL | 2016-07-24T17:02:18+02:00 | 1 |
| 1013.26.144 | `diagnostic-tests-in-the-follow-up-of-cg.opt` | Diagnostic tests in the follow-up of CG | INITIAL | 2016-07-24T17:05:04+02:00 | 1 |
| 1013.26.145 | `image-test-analysis-for-cg.opt` | Image test analysis for CG | INITIAL | 2016-07-24T18:36:41+02:00 | 1 |
| 1013.26.146 | `cg-assessment.opt` | CG assessment | INITIAL | 2016-07-24T18:39:43+02:00 | 1 |
| 1013.26.147 | `administration-of-antiglaucoma-drugs.opt` | Administration of antiglaucoma drugs | INITIAL | 2016-07-24T22:59:02+02:00 | 1 |
| 1013.26.148 | `laser-procedures-to-treat-cg.opt` | Laser procedures to treat CG | INITIAL | 2016-07-24T23:00:35+02:00 | 1 |
| 1013.26.149 | `incisional-glaucoma-surgery.opt` | Incisional glaucoma surgery | INITIAL | 2016-07-24T23:02:22+02:00 | 1 |
| 1013.26.150 | `modify-cg-treatment.opt` | Modify CG treatment | INITIAL | 2016-07-24T23:03:35+02:00 | 1 |
| 1013.26.156 | `patient-s-enrolment-in-dr-screening.opt` | Patient’s enrolment in DR screening | INITIAL | 2016-07-25T12:57:01+02:00 | 1 |
| 1013.26.157 | `patient-s-background-leading-to-suspicion-of-dr.opt` | Patient's background leading to suspicion of DR | INITIAL | 2016-07-25T12:58:22+02:00 | 1 |
| 1013.26.158 | `patient-admission-into-the-dr-screening-service.opt` | Patient admission into the DR screening service | INITIAL | 2016-07-25T12:59:19+02:00 | 1 |
| 1013.26.16 | `ear-primary-hip-arthroplasty-report.opt` | EAR Primary Hip Arthroplasty Report | DRAFT | 2015-06-29T15:52:31+02:00 | 3 |
| 1013.26.160 | `planning-the-diagnostic-tests-to-detect-and-classify-dr.opt` | Planning the diagnostic tests to detect and classify DR | INITIAL | 2016-07-25T15:10:40+02:00 | 1 |
| 1013.26.164 | `next-step-in-dr-screening-service.opt` | Next step in DR screening service | INITIAL | 2016-07-25T16:27:32+02:00 | 1 |
| 1013.26.165 | `diagnostic-tests-in-the-dr-screening-service.opt` | Diagnostic tests in the DR screening service | INITIAL | 2016-07-25T16:29:04+02:00 | 1 |
| 1013.26.167 | `image-test-analysis-for-dr.opt` | Image test analysis for DR | INITIAL | 2016-07-26T18:08:35+02:00 | 1 |
| 1013.26.169 | `identification-of-patients-with-dr.opt` | Identification of patients with DR | INITIAL | 2016-07-28T00:24:11+02:00 | 1 |
| 1013.26.17 | `ear-revision-hip-arthroplasty-report.opt` | EAR Revision Hip Arthroplasty Report | DRAFT | 2015-06-29T15:53:50+02:00 | 3 |
| 1013.26.170 | `dr-screening.opt` | DR screening | INITIAL | 2016-07-28T00:27:30+02:00 | 1 |
| 1013.26.171 | `classification-and-treatment-of-dr-and-dme.opt` | Classification and treatment of DR and DME | INITIAL | 2016-07-28T01:03:08+02:00 | 1 |
| 1013.26.172 | `dr-assessment.opt` | DR assessment | INITIAL | 2016-07-28T01:07:02+02:00 | 1 |
| 1013.26.174 | `laser-procedures-to-treat-dr.opt` | Laser procedures to treat DR | INITIAL | 2016-07-28T03:28:24+02:00 | 1 |
| 1013.26.175 | `surgical-procedures-to-treat-dr.opt` | Surgical procedures to treat DR | INITIAL | 2016-07-28T03:29:57+02:00 | 1 |
| 1013.26.176 | `dr-treatment.opt` | DR treatment | INITIAL | 2016-07-28T03:34:01+02:00 | 1 |
| 1013.26.180 | `next-step-in-amd-monitoring-service.opt` | Next step in AMD monitoring service | INITIAL | 2016-07-29T02:49:54+02:00 | 1 |
| 1013.26.189 | `season-s-greetings.opt` | Season's Greetings! | INITIAL | 2017-12-24T13:38:36+01:00 | 1 |
| 1013.26.191 | `gp-data-set.opt` | GP data set | INITIAL | 2018-10-15T02:11:04+02:00 | 3 |
| 1013.26.197 | `rcpa-breast-cancer-histopathology-1st-edition-2010.opt` | RCPA Breast cancer histopathology 1st Edition 2010 | INITIAL | 2018-11-12T06:56:37+01:00 | 1 |
| 1013.26.198 | `rcpa-breast-cancer-structured-report-1st-edition-2010.opt` | RCPA Breast Cancer Structured Report 1st Edition 2010 | INITIAL | 2018-11-12T07:51:23+01:00 | 1 |
| 1013.26.199 | `british-columbia-cancer-agency-breast-cancer-synoptic-report.opt` | British Columbia Cancer agency Breast Cancer Synoptic Report | INITIAL | 2018-11-12T12:27:58+01:00 | 1 |
| 1013.26.2 | `ereferral.opt` | eReferral | DRAFT | 2010-03-25T07:26:15+01:00 | 1 |
| 1013.26.20 | `patient-s-admittance-in-dr-screening-service.opt` | Patient's admittance in DR screening service | INITIAL | 2015-10-19T12:18:13+02:00 | 2 |
| 1013.26.21 | `patient-s-background-for-dr-screening.opt` | Patient's background for DR screening | INITIAL | 2015-10-19T12:26:18+02:00 | 2 |
| 1013.26.214 | `wound-presence-assertion.opt` | Wound Presence Assertion | INITIAL | 2019-03-19T02:21:21+01:00 | 1 |
| 1013.26.215 | `wound-assessment-panel.opt` | Wound Assessment Panel | INITIAL | 2019-03-19T02:24:46+01:00 | 1 |
| 1013.26.216 | `wound-assert.opt` | Wound Assert | INITIAL | 2019-03-25T05:45:10+01:00 | 2 |
| 1013.26.217 | `wound-related-observations-panel.opt` | Wound related observations Panel | INITIAL | 2019-04-25T03:16:23+02:00 | 2 |
| 1013.26.218 | `wound-assert-report.opt` | Wound Assert Report | INITIAL | 2019-03-25T05:54:18+01:00 | 3 |
| 1013.26.22 | `diagnostic-test-planning.opt` | Diagnostic test planning | INITIAL | 2014-08-18T14:43:24+02:00 | 1 |
| 1013.26.223 | `genomics-example-template.opt` | Genomics example template | DRAFT | 2020-10-15T11:30:20+02:00 | 2 |
| 1013.26.23 | `intraocular-pressure-study-2.opt` | Intraocular pressure study | INITIAL | 2014-08-18T19:24:03+02:00 | 1 |
| 1013.26.234 | `phenopacket.opt` | Phenopacket | INITIAL | 2019-09-30T12:02:58+02:00 | 3 |
| 1013.26.235 | `family.opt` | Family | INITIAL | 2019-09-30T12:06:57+02:00 | 2 |
| 1013.26.236 | `cohort.opt` | Cohort | INITIAL | 2019-09-30T12:05:12+02:00 | 2 |
| 1013.26.238 | `interpretation.opt` | Interpretation | INITIAL | 2019-09-30T11:55:18+02:00 | 5 |
| 1013.26.249 | `molecular-pathology-report.opt` | Molecular Pathology Report | DRAFT | 2020-02-10T13:56:57+01:00 | 2 |
| 1013.26.25 | `next-step-planning.opt` | Next step planning | INITIAL | 2014-08-18T21:11:49+02:00 | 1 |
| 1013.26.252 | `family-history-summary-item-r2.opt` | Family history summary item R2 | INITIAL | 2020-07-13T09:11:28+02:00 | 2 |
| 1013.26.253 | `family-history-summary-item-r1.opt` | Family history summary item R1 | INITIAL | 2020-07-13T09:09:03+02:00 | 2 |
| 1013.26.254 | `aboriginal-and-torres-strait-islander-health-check-master.opt` | Aboriginal and Torres Strait Islander health check - MASTER | INITIAL | 2020-07-14T07:46:21+02:00 | 8 |
| 1013.26.258 | `tobacco-smoking-summary-item-r1.opt` | Tobacco smoking summary item R1 | INITIAL | 2020-07-13T09:33:14+02:00 | 2 |
| 1013.26.259 | `tobacco-smoking-summary-item-r2.opt` | Tobacco smoking summary item R2 | INITIAL | 2020-07-13T09:31:10+02:00 | 2 |
| 1013.26.260 | `occupation-summary-item-r2.opt` | Occupation summary item R2 | INITIAL | 2020-07-13T09:18:20+02:00 | 3 |
| 1013.26.261 | `education-summary-item-r2.opt` | Education summary item R2 | INITIAL | 2020-07-13T09:04:36+02:00 | 2 |
| 1013.26.266 | `imaging-examination-result.opt` | Imaging examination result | DRAFT | 2020-03-06T04:42:17+01:00 | 1 |
| 1013.26.267 | `openehr-suspected-covid-19-assessment-v0.opt` | openEHR-Suspected Covid-19 assessment.v0 | INITIAL | 2020-03-09T23:42:04+01:00 | 4 |
| 1013.26.27 | `laser-procedure.opt` | Laser procedure | INITIAL | 2014-08-18T23:24:20+02:00 | 1 |
| 1013.26.271 | `openehr-confirmed-covid-19-infection-report-v0.opt` | openEHR-Confirmed Covid-19 infection report.v0 | INITIAL | 2020-03-10T04:56:28+01:00 | 2 |
| 1013.26.273 | `au-covid-19-likelihood-assessment.opt` | AU COVID-19 Likelihood Assessment | DRAFT | 2020-03-15T02:01:20+01:00 | 2 |
| 1013.26.28 | `surgical-procedure.opt` | Surgical procedure | INITIAL | 2014-08-18T23:28:02+02:00 | 1 |
| 1013.26.280 | `suspected-covid-19-assessment-v0-1.opt` | Suspected Covid-19 Assessment.v0.1 | INITIAL | 2020-03-22T16:27:03+01:00 | 2 |
| 1013.26.282 | `openehr-confirmed-covid-19-infection-report-v0-2.opt` | openEHR confirmed COVID-19 infection report.v0 | DRAFT | 2020-08-07T07:19:11+02:00 | 2 |
| 1013.26.286 | `openehr-suspected-covid-19-risk-assessment-v0.opt` | openEHR suspected COVID-19 risk assessment.v0 | DRAFT | 2020-08-07T07:25:07+02:00 | 3 |
| 1013.26.288 | `symptom-sign-screening.opt` | Symptom/sign screening | DRAFT | 2020-03-24T07:19:21+01:00 | 1 |
| 1013.26.291 | `covid-19-pneumonia-diagnosis-and-treatment-7th-edition.opt` | COVID-19 Pneumonia Diagnosis and Treatment (7th edition) | DRAFT | 2020-04-11T07:39:38+02:00 | 4 |
| 1013.26.298 | `openehr-suspected-covid-19-risk-assessment-nephrology-v0.opt` | openEHR suspected COVID-19 risk assessment nephrology.v0 | INITIAL | 2020-07-07T03:05:36+02:00 | 4 |
| 1013.26.302 | `promis-29.opt` | PROMIS-29 | DRAFT | 2020-06-10T11:03:27+02:00 | 2 |
| 1013.26.305 | `obstetric-summary-item-r1.opt` | Obstetric summary item R1 | INITIAL | 2020-07-13T09:35:49+02:00 | 3 |
| 1013.26.313 | `health-issue-item-r2.opt` | Health issue item R2 | INITIAL | 2020-07-13T09:16:15+02:00 | 2 |
| 1013.26.314 | `food-and-nutrition-summary-item-r2.opt` | Food and nutrition summary item R2 | INITIAL | 2020-07-13T09:12:17+02:00 | 2 |
| 1013.26.315 | `substance-use-summary-item-r2.opt` | Substance use summary item R2 | INITIAL | 2020-07-13T06:13:47+02:00 | 1 |
| 1013.26.316 | `absolute-cardiovascular-risk-assessment-item-r2.opt` | Absolute cardiovascular risk assessment item R2 | INITIAL | 2020-07-13T08:46:54+02:00 | 1 |
| 1013.26.317 | `alcohol-consumption-summary-item-r1.opt` | Alcohol consumption summary item R1 | INITIAL | 2020-07-13T08:49:56+02:00 | 2 |
| 1013.26.319 | `alcohol-use-disorders-identification-test-audit-item-r2.opt` | Alcohol Use Disorders Identification Test (AUDIT) item R2 | INITIAL | 2020-07-13T08:52:45+02:00 | 1 |
| 1013.26.320 | `alcohol-consumption-summary-item-r2.opt` | Alcohol consumption summary item R2 | INITIAL | 2020-07-13T08:58:57+02:00 | 1 |
| 1013.26.321 | `blood-pressure-item-r2.opt` | Blood pressure item R2 | INITIAL | 2020-07-13T09:00:19+02:00 | 1 |
| 1013.26.322 | `body-mass-index-item-r2.opt` | Body mass index item R2 | INITIAL | 2020-07-13T09:00:51+02:00 | 1 |
| 1013.26.323 | `body-temperature-item-r2.opt` | Body temperature item R2 | INITIAL | 2020-07-13T09:02:03+02:00 | 1 |
| 1013.26.324 | `body-weight-item-r2.opt` | Body weight item R2 | INITIAL | 2020-07-13T09:02:26+02:00 | 1 |
| 1013.26.326 | `examination-of-an-ear-item-r2.opt` | Examination of an ear item R2 | INITIAL | 2020-07-14T07:41:24+02:00 | 2 |
| 1013.26.327 | `examination-of-an-eye-item-r2.opt` | Examination of an eye item R2 | INITIAL | 2020-07-13T09:05:19+02:00 | 1 |
| 1013.26.328 | `examination-of-both-eyes-item-r2.opt` | Examination of both eyes item R2 | INITIAL | 2020-07-13T09:06:52+02:00 | 1 |
| 1013.26.329 | `examination-of-the-mouth-item-r2.opt` | Examination of the mouth item R2 | INITIAL | 2020-07-14T07:42:14+02:00 | 2 |
| 1013.26.33 | `slovenia-res-primary-hip-arthroplasty-report.opt` | Slovenia RES Primary Hip Arthroplasty Report | REVIEWSUSPENDED | 2016-01-19T18:42:53+01:00 | 15 |
| 1013.26.330 | `examination-of-the-heart-item-r2.opt` | Examination of the heart item R2 | INITIAL | 2020-07-13T09:08:19+02:00 | 1 |
| 1013.26.334 | `gambling-summary-item-r2.opt` | Gambling summary item R2 | INITIAL | 2020-07-13T09:12:40+02:00 | 1 |
| 1013.26.335 | `goal-item-r2.opt` | Goal item R2 | INITIAL | 2020-07-13T09:13:39+02:00 | 1 |
| 1013.26.336 | `gender-item-r2.opt` | Gender item R2 | INITIAL | 2020-07-13T09:14:02+02:00 | 1 |
| 1013.26.337 | `gender-item-r1.opt` | Gender item R1 | INITIAL | 2020-07-13T09:14:54+02:00 | 1 |
| 1013.26.34 | `slovenia-res-revision-hip-arthroplasty-report.opt` | Slovenia RES Revision Hip Arthroplasty Report | REVIEWSUSPENDED | 2016-01-19T18:45:04+01:00 | 12 |
| 1013.26.340 | `living-arrangement-summary-item-r2.opt` | Living arrangement summary item R2 | INITIAL | 2020-07-13T09:18:51+02:00 | 1 |
| 1013.26.341 | `medication-order-item-r1.opt` | Medication order item R1 | INITIAL | 2020-07-13T09:19:26+02:00 | 1 |
| 1013.26.343 | `problem-diagnosis-item-r1.opt` | Problem/Diagnosis item R1 | INITIAL | 2020-07-13T09:22:32+02:00 | 1 |
| 1013.26.344 | `physical-activity-summary-item-r2.opt` | Physical activity summary item R2 | INITIAL | 2020-07-13T09:22:58+02:00 | 1 |
| 1013.26.345 | `personal-safety-summary-item-r2.opt` | Personal safety summary item R2 | INITIAL | 2020-07-13T09:25:11+02:00 | 1 |
| 1013.26.346 | `pulse-item-r2.opt` | Pulse item R2 | INITIAL | 2020-07-13T09:28:15+02:00 | 1 |
| 1013.26.347 | `respiration-item-r2.opt` | Respiration item R2 | INITIAL | 2020-07-13T09:28:45+02:00 | 1 |
| 1013.26.350 | `transport-access-summary-item-r2.opt` | Transport access summary item R2 | INITIAL | 2020-07-13T09:32:02+02:00 | 1 |
| 1013.26.351 | `waist-circumference-item-r2.opt` | Waist circumference item R2 | INITIAL | 2020-07-13T09:32:27+02:00 | 1 |
| 1013.26.353 | `height-length-item-r2.opt` | Height/length item R2 | INITIAL | 2020-07-14T07:40:05+02:00 | 1 |
| 1013.26.356 | `clinical-synopsis-item-r1.opt` | Clinical synopsis item R1 | INITIAL | 2020-07-22T09:12:24+02:00 | 1 |
| 1013.26.357 | `medicines-list-item-r1.opt` | Medicines list item R1 | INITIAL | 2020-07-22T09:13:37+02:00 | 1 |
| 1013.26.358 | `medicine-item-r1.opt` | Medicine item R1 | INITIAL | 2020-07-22T09:14:22+02:00 | 1 |
| 1013.26.359 | `adverse-reaction-risk-item-r1.opt` | Adverse reaction risk item R1 | INITIAL | 2020-07-22T09:15:03+02:00 | 1 |
| 1013.26.360 | `problem-diagnosis-list-item-r1.opt` | Problem/Diagnosis list item R1 | INITIAL | 2020-07-22T09:15:48+02:00 | 1 |
| 1013.26.361 | `procedure-item-r1.opt` | Procedure item R1 | INITIAL | 2020-07-22T09:18:08+02:00 | 1 |
| 1013.26.362 | `vaccination-list-item-r1.opt` | Vaccination list item R1 | INITIAL | 2020-07-22T09:18:53+02:00 | 1 |
| 1013.26.363 | `social-summary-item-r1.opt` | Social summary item R1 | INITIAL | 2020-07-22T09:19:28+02:00 | 1 |
| 1013.26.364 | `pregnancy-detail-a-item-r1.opt` | Pregnancy detail A item R1 | INITIAL | 2020-07-22T09:22:32+02:00 | 2 |
| 1013.26.366 | `pregnancy-detail-b-item-r1.opt` | Pregnancy detail B item R1 | INITIAL | 2020-07-22T09:23:03+02:00 | 1 |
| 1013.26.367 | `pregnancy-detail-c-item-r1.opt` | Pregnancy detail C item R1 | INITIAL | 2020-07-22T09:25:25+02:00 | 1 |
| 1013.26.368 | `vaccination-item-r1.opt` | Vaccination item R1 | INITIAL | 2020-07-22T09:25:59+02:00 | 1 |
| 1013.26.369 | `follow-up-activity-item-r1.opt` | Follow up activity item R1 | INITIAL | 2020-07-22T09:26:33+02:00 | 1 |
| 1013.26.370 | `reason-for-encounter-item-r1.opt` | Reason for encounter item R1 | INITIAL | 2020-07-22T09:27:06+02:00 | 1 |
| 1013.26.371 | `pregnancy-detail-b2-item-r1.opt` | Pregnancy detail B2 item R1 | INITIAL | 2020-07-22T09:31:04+02:00 | 1 |
| 1013.26.372 | `gecco-core.opt` | GECCO core | DRAFT | 2021-04-07T07:19:15+02:00 | 3 |
| 1013.26.376 | `international-patient-summary.opt` | International Patient Summary | DRAFT | 2020-08-18T04:28:14+02:00 | 1 |
| 1013.26.377 | `sars-event-notification.opt` | SARS event notification | DRAFT | 2020-10-13T06:36:16+02:00 | 1 |
| 1013.26.379 | `pathological-breast-tumour-size.opt` | Pathological breast tumour size | INITIAL | 2021-03-04T12:03:42+01:00 | 1 |
| 1013.26.380 | `vital-signs.opt` | Vital signs | DRAFT | 2021-03-08T13:30:44+01:00 | 1 |
| 1013.26.383 | `glucose-test-result-example.opt` | Glucose test result example | DRAFT | 2021-05-18T16:21:12+02:00 | 1 |
| 1013.26.386 | `ccta-report.opt` | CCTA report | INITIAL | 2021-08-02T07:29:55+02:00 | 1 |
| 1013.26.389 | `cause-of-death.opt` | Cause of death | DRAFT | 2021-08-05T13:32:01+02:00 | 1 |
| 1013.26.390 | `ett.opt` | ETT | INITIAL | 2021-08-11T10:29:51+02:00 | 1 |
| 1013.26.391 | `follow-up.opt` | Follow up | INITIAL | 2021-08-21T04:21:35+02:00 | 1 |
| 1013.26.396 | `ehr-person-with-unstructured-name-and-address.opt` | EHR Person with unstructured name and address | DRAFT | 2021-09-13T05:10:33+02:00 | 2 |
| 1013.26.398 | `ehr-person-with-structured-name-and-address.opt` | EHR Person with structured name and address | DRAFT | 2021-09-13T05:43:43+02:00 | 2 |
| 1013.26.399 | `ehr-organisation-with-contact-person.opt` | EHR Organisation with contact person | DRAFT | 2021-09-13T05:37:15+02:00 | 2 |
| 1013.26.40 | `treat-registry-report.opt` | TREAT Registry report | DRAFT | 2015-02-18T21:32:34+01:00 | 1 |
| 1013.26.402 | `ehr-person-with-structured-name-and-address2.opt` | EHR Person with structured name and address2 | INITIAL | 2021-09-13T07:06:19+02:00 | 1 |
| 1013.26.408 | `generic-lab-test-result-example-simple.opt` | Generic lab test result example simple | DRAFT | 2021-10-18T11:28:46+02:00 | 3 |
| 1013.26.419 | `oocyte-and-embryo-assessment.opt` | Oocyte and Embryo assessment | DRAFT | 2021-11-30T05:34:39+01:00 | 1 |
| 1013.26.420 | `embryo-storage.opt` | Embryo storage | DRAFT | 2021-11-30T07:29:31+01:00 | 1 |
| 1013.26.421 | `imaging.opt` | Imaging | DRAFT | 2022-01-25T10:11:52+01:00 | 3 |
| 1013.26.424 | `pelvic-ultrasound-simple.opt` | Pelvic ultrasound simple | DRAFT | 2022-02-10T21:40:54+01:00 | 3 |
| 1013.26.426 | `folliculometry.opt` | Folliculometry | DRAFT | 2022-02-10T21:30:05+01:00 | 2 |
| 1013.26.429 | `middle-name-and-nickname.opt` | Middle name and nickname | DRAFT | 2025-09-16T00:53:18+02:00 | 2 |
| 1013.26.430 | `pregnancy-ecosystem-testing.opt` | Pregnancy ecosystem testing | INITIAL | 2022-04-14T10:44:33+02:00 | 1 |
| 1013.26.56 | `health-risk-assessment.opt` | Health risk assessment | DRAFT | 2015-05-05T09:02:01+02:00 | 1 |
| 1013.26.6 | `epsos-medication-summary-section-1-3-6-1-4-1-12559-11-10-1-3-1-2-3.opt` | epSOS Medication Summary Section 1.3.6.1.4.1.12559.11.10.1.3.1.2.3 | DRAFT | 2016-05-24T18:09:42+02:00 | 3 |
| 1013.26.61 | `odl-report-vital-signs.opt` | ODL Report Vital Signs | INITIAL | 2015-06-26T12:30:12+02:00 | 3 |
| 1013.26.62 | `examination-archetypes.opt` | Examination archetypes | DRAFT | 2015-06-24T10:45:24+02:00 | 1 |
| 1013.26.65 | `odl-report-empower.opt` | ODL Report EMPOWER | INITIAL | 2015-06-26T12:50:28+02:00 | 2 |
| 1013.26.67 | `odl-report-issues.opt` | ODL Report Issues | INITIAL | 2015-06-26T13:03:05+02:00 | 1 |
| 1013.26.71 | `consultation-for-amd-assessment.opt` | Consultation for AMD assessment | INITIAL | 2015-10-05T21:12:02+02:00 | 3 |
| 1013.26.76 | `consultation-for-patient-admission-in-dr-screening.opt` | Consultation for patient admission in DR screening | INITIAL | 2015-10-19T12:36:04+02:00 | 1 |
| 1013.26.79 | `follow-up-schedule-for-patients-with-suspected-dr.opt` | Follow-up schedule for patients with suspected DR | INITIAL | 2016-07-25T15:15:27+02:00 | 2 |
| 1013.26.8 | `epsos-allergy-and-other-adverse-reactions-section-1-3-6-1-4-1-19376-1-5-3-1-3-13.opt` | epSOS Allergy and Other Adverse Reactions Section 1.3.6.1.4.1.19376.1.5.3.1.3.13 | DRAFT | 2013-05-29T21:42:20+02:00 | 1 |
| 1013.26.80 | `eprescription-fhir.opt` | ePrescription (FHIR) | DRAFT | 2016-05-23T23:01:02+02:00 | 11 |
| 1013.26.891 | `pharmacogenetic-test-result.opt` | Pharmacogenetic test result | DRAFT | 2024-04-08T13:20:45+02:00 | 2 |
| 1013.26.892 | `pgx-cumulative-pharmacogenetic-profile.opt` | PGX - Cumulative pharmacogenetic profile | DRAFT | 2024-04-08T17:20:03+02:00 | 2 |
| 1013.26.896 | `aedes-indices-jm.opt` | Aedes indices - JM | INITIAL | 2024-12-29T06:35:17+01:00 | 2 |
| 1013.26.897 | `age-assertion-jm.opt` | Age assertion - JM | INITIAL | 2025-12-12T04:40:59+01:00 | 3 |
| 1013.26.898 | `age-assertion-jm-infant.opt` | Age assertion - JM infant | INITIAL | 2025-12-17T04:05:02+01:00 | 4 |
| 1013.26.899 | `structured-name-jm-full.opt` | Structured name - JM Full | INITIAL | 2025-12-14T09:04:27+01:00 | 3 |
| 1013.26.9 | `epsos-history-of-past-illness-section-1-3-6-1-4-1-19376-1-5-3-1-3-8.opt` | epSOS History of Past Illness Section   1.3.6.1.4.1.19376.1.5.3.1.3.8 | DRAFT | 2013-05-30T01:16:06+02:00 | 3 |
| 1013.26.900 | `address-jm-local.opt` | Address - JM Local | INITIAL | 2025-12-10T08:54:26+01:00 | 3 |
| 1013.26.901 | `address-jm-international.opt` | Address - JM International | INITIAL | 2025-12-12T03:47:52+01:00 | 3 |
| 1013.26.902 | `phone-jm-unspecified.opt` | Phone - JM Unspecified | INITIAL | 2025-07-12T11:06:32+02:00 | 2 |
| 1013.26.903 | `person-jm-father.opt` | Person - JM Father | INITIAL | 2025-07-14T13:21:44+02:00 | 6 |
| 1013.26.904 | `phone-jm-mobile.opt` | Phone - JM Mobile | INITIAL | 2025-07-12T11:24:49+02:00 | 2 |
| 1013.26.905 | `phone-jm-home.opt` | Phone - JM Home | INITIAL | 2025-07-12T11:22:44+02:00 | 2 |
| 1013.26.906 | `email-jm.opt` | Email - JM | INITIAL | 2025-07-12T11:31:38+02:00 | 2 |
| 1013.26.907 | `phone-jm-work.opt` | Phone - JM Work | INITIAL | 2025-07-12T11:24:18+02:00 | 2 |
| 1013.26.908 | `address-jm-local-gis.opt` | Address - JM Local_GIS | INITIAL | 2025-12-12T04:43:16+01:00 | 3 |
| 1013.26.909 | `person-jm-adult-patient.opt` | Person - JM Adult patient | INITIAL | 2025-12-12T04:52:19+01:00 | 5 |
| 1013.26.910 | `person-jm-child-patient.opt` | Person - JM Child patient | INITIAL | 2025-12-12T04:54:08+01:00 | 5 |
| 1013.26.911 | `structured-name-jm-full-maiden.opt` | Structured name - JM Full_Maiden | INITIAL | 2024-10-27T16:26:11+01:00 | 1 |
| 1013.26.912 | `person-jm-nameless-infant-patient.opt` | Person - JM Nameless infant patient | INITIAL | 2025-12-12T04:55:00+01:00 | 4 |
| 1013.26.913 | `date-of-birth-jm.opt` | Date of birth - JM | INITIAL | 2025-07-12T11:37:43+02:00 | 2 |
| 1013.26.914 | `sex-and-gender-jm-newborn.opt` | Sex and gender - JM newborn | INITIAL | 2025-12-12T04:56:04+01:00 | 4 |
| 1013.26.915 | `sex-and-gender-jm.opt` | Sex and gender - JM | INITIAL | 2025-12-12T04:45:07+01:00 | 3 |
| 1013.26.916 | `case-demographic-information-jm.opt` | Case demographic information - JM | INITIAL | 2025-12-14T10:26:37+01:00 | 7 |
| 1013.26.917 | `structured-name-jm-core.opt` | Structured name - JM Core | INITIAL | 2024-12-27T10:42:59+01:00 | 2 |
| 1013.26.918 | `person-jm-notifying-individual.opt` | Person - JM Notifying individual | INITIAL | 2024-12-29T02:03:13+01:00 | 2 |
| 1013.26.919 | `organisation-healthcare-facility-jm.opt` | Organisation - Healthcare facility JM | INITIAL | 2025-12-14T09:41:02+01:00 | 4 |
| 1013.26.920 | `person-jm-investigating-officer.opt` | Person - JM Investigating officer | INITIAL | 2025-12-14T09:44:02+01:00 | 4 |
| 1013.26.921 | `person-jm-clinician.opt` | Person - JM Clinician | INITIAL | 2025-12-14T09:39:02+01:00 | 3 |
| 1013.26.922 | `person-jm-public-health-official.opt` | Person - JM Public health official | INITIAL | 2025-12-14T09:45:01+01:00 | 4 |
| 1013.26.923 | `infectious-disease-investigation-metadata-jm.opt` | Infectious disease investigation metadata - JM | INITIAL | 2025-12-19T03:03:36+01:00 | 6 |
| 1013.26.924 | `investigator-initial-infectious-disease-investigation-classification-jm.opt` | Investigator initial infectious disease investigation classification - JM | INITIAL | 2025-12-12T04:35:53+01:00 | 3 |
| 1013.26.925 | `investigator-classification-initial-jm.opt` | Investigator classification - initial JM | INITIAL | 2025-12-14T10:34:55+01:00 | 4 |
| 1013.26.926 | `other-diagnosis-jm.opt` | Other diagnosis - JM | INITIAL | 2024-10-27T18:25:51+01:00 | 1 |
| 1013.26.927 | `investigator-diagnosis-jm.opt` | Investigator diagnosis - JM | INITIAL | 2024-10-27T18:26:27+01:00 | 1 |
| 1013.26.928 | `local-final-infectious-disease-investigation-classification-jm.opt` | Local final infectious disease investigation classification - JM | INITIAL | 2024-10-27T18:28:22+01:00 | 1 |
| 1013.26.929 | `cause-of-death-jm.opt` | Cause of death - JM | INITIAL | 2024-12-24T12:30:04+01:00 | 2 |
| 1013.26.930 | `field-investigator-classification-jm.opt` | Field investigator classification - JM | INITIAL | 2025-12-14T08:57:13+01:00 | 7 |
| 1013.26.931 | `death-summary-jm.opt` | Death summary - JM | INITIAL | 2024-10-27T18:32:29+01:00 | 1 |
| 1013.26.932 | `outcome-jm.opt` | Outcome - JM | INITIAL | 2025-12-14T08:41:00+01:00 | 5 |
| 1013.26.933 | `symptom-sign-screening-jm.opt` | Symptom/sign screening - JM | INITIAL | 2025-12-12T04:19:25+01:00 | 2 |
| 1013.26.934 | `body-temperature-jm.opt` | Body temperature - JM | INITIAL | 2025-12-12T04:20:19+01:00 | 2 |
| 1013.26.935 | `neuro-examination-findings-jm.opt` | Neuro examination findings - JM | INITIAL | 2024-10-27T19:04:40+01:00 | 1 |
| 1013.26.936 | `clinical-screening-jm.opt` | Clinical screening - JM | INITIAL | 2025-12-12T04:21:08+01:00 | 2 |
| 1013.26.937 | `infant-measurements-jm.opt` | Infant measurements - JM | INITIAL | 2025-12-12T04:13:20+01:00 | 4 |
| 1013.26.938 | `birth-summary-jm.opt` | Birth summary - JM | INITIAL | 2025-12-12T04:02:26+01:00 | 4 |
| 1013.26.939 | `infant-clinical-context-jm.opt` | Infant clinical context - JM | INITIAL | 2025-12-14T09:46:25+01:00 | 8 |
| 1013.26.94 | `eprescription-epsos-contsys.opt` | ePrescription (epSoS/Contsys) | DRAFT | 2016-05-23T23:16:04+02:00 | 1 |
| 1013.26.940 | `food-and-nutrition-summary-jm.opt` | Food and nutrition summary - JM | INITIAL | 2024-10-28T00:20:01+01:00 | 1 |
| 1013.26.941 | `vaccinations-jm.opt` | Vaccinations - JM | INITIAL | 2025-12-12T04:00:51+01:00 | 2 |
| 1013.26.942 | `gestation-assertion.opt` | Gestation assertion | INITIAL | 2024-10-28T00:52:06+01:00 | 1 |
| 1013.26.943 | `gestation-assertion-jm.opt` | Gestation assertion - JM | INITIAL | 2025-07-13T04:45:55+02:00 | 2 |
| 1013.26.944 | `pregnancy-screening-jm.opt` | Pregnancy screening - JM | INITIAL | 2024-10-28T01:00:47+01:00 | 1 |
| 1013.26.945 | `problem-diagnosis-screening-jm.opt` | Problem/Diagnosis screening - JM | INITIAL | 2025-12-12T03:59:43+01:00 | 4 |
| 1013.26.946 | `clinical-context-jm.opt` | Clinical context - JM | INITIAL | 2025-12-15T05:32:20+01:00 | 5 |
| 1013.26.947 | `travel-event-visitor-jm.opt` | Travel event - visitor JM | INITIAL | 2024-10-28T02:33:45+01:00 | 1 |
| 1013.26.948 | `travel-event-international-jm.opt` | Travel event - international JM | INITIAL | 2024-10-28T02:41:00+01:00 | 1 |
| 1013.26.949 | `travel-screening-jm.opt` | Travel screening - JM | INITIAL | 2025-12-14T10:20:11+01:00 | 9 |
| 1013.26.950 | `travel-profile-jm.opt` | Travel profile - JM | INITIAL | 2025-12-14T10:23:53+01:00 | 6 |
| 1013.26.951 | `person-jm-contact-for-tracing.opt` | Person - JM Contact for tracing | INITIAL | 2025-12-12T04:43:52+01:00 | 6 |
| 1013.26.952 | `contact-for-tracing-jm.opt` | Contact for tracing - JM | INITIAL | 2025-12-14T09:49:40+01:00 | 9 |
| 1013.26.953 | `environmental-survey-jm.opt` | Environmental survey - JM | INITIAL | 2024-10-28T15:08:26+01:00 | 1 |
| 1013.26.954 | `exposure-screening-jm.opt` | Exposure screening - JM | INITIAL | 2025-07-12T11:12:56+02:00 | 2 |
| 1013.26.955 | `exposure-profile-jm.opt` | Exposure profile - JM | INITIAL | 2025-12-19T03:20:40+01:00 | 10 |
| 1013.26.956 | `person-jm-next-of-kin.opt` | Person - JM Next of kin | INITIAL | 2025-12-14T09:06:27+01:00 | 6 |
| 1013.26.957 | `social-network-jm.opt` | Social network - JM | INITIAL | 2025-12-14T09:16:48+01:00 | 6 |
| 1013.26.958 | `education-summary-jm.opt` | Education summary - JM | INITIAL | 2024-10-28T16:10:18+01:00 | 1 |
| 1013.26.959 | `clinical-notes-jm.opt` | Clinical notes - JM | INITIAL | 2025-12-12T04:17:47+01:00 | 2 |
| 1013.26.960 | `home-environment-jm.opt` | Home environment - JM | INITIAL | 2025-12-14T09:30:38+01:00 | 4 |
| 1013.26.961 | `occupation-summary-jm.opt` | Occupation summary - JM | INITIAL | 2025-12-14T09:32:31+01:00 | 5 |
| 1013.26.962 | `social-profile-jm.opt` | Social profile - JM | INITIAL | 2025-12-14T09:35:31+01:00 | 6 |
| 1013.26.963 | `receiving-laboratory-private-jm.opt` | Receiving laboratory - private - JM | INITIAL | 2025-12-12T05:09:30+01:00 | 3 |
| 1013.26.964 | `receiving-laboratory-public-jm.opt` | Receiving laboratory - public - JM | INITIAL | 2025-12-12T05:05:32+01:00 | 2 |
| 1013.26.965 | `laboratory-analyte-result-quantitative-jm.opt` | Laboratory analyte result - quantitative - JM | INITIAL | 2025-12-12T05:06:21+01:00 | 3 |
| 1013.26.966 | `laboratory-analyte-result-interpretative-jm.opt` | Laboratory analyte result - interpretative - JM | INITIAL | 2025-12-12T05:08:02+01:00 | 2 |
| 1013.26.969 | `management-screening-jm.opt` | Management screening - JM | INITIAL | 2025-12-12T05:13:24+01:00 | 4 |
| 1013.26.970 | `treatment-summary-jm.opt` | Treatment summary - JM | INITIAL | 2025-12-14T10:31:31+01:00 | 5 |
| 1013.26.972 | `imaging-examination-result-jm.opt` | Imaging examination result - JM | INITIAL | 2025-12-12T05:00:51+01:00 | 2 |
| 1013.26.973 | `laboratory-test-result-jm.opt` | Laboratory test result - JM | INITIAL | 2025-12-12T05:10:11+01:00 | 4 |
| 1013.26.974 | `clinical-management-jm.opt` | Clinical management - JM | INITIAL | 2025-12-19T03:22:36+01:00 | 8 |
| 1013.26.975 | `2024-master-infectious-disease-case-investigation-form-jm.opt` | 2024 MASTER Infectious disease case investigation form - JM | INITIAL | 2024-12-30T01:35:47+01:00 | 8 |
| 1013.26.976 | `mother-demographic-information-jm.opt` | Mother demographic information - JM | INITIAL | 2024-12-29T02:41:49+01:00 | 3 |
| 1013.26.977 | `congenital-syphilis-case-investigation-form.opt` | Congenital syphilis case investigation form | INITIAL | 2024-12-30T01:49:57+01:00 | 6 |
| 1013.26.979 | `meningitis-encephalitis-case-investigation-form.opt` | Meningitis_Encephalitis case investigation form | INITIAL | 2024-12-30T02:03:40+01:00 | 8 |
| 1013.26.98 | `patient-s-enrolment-in-anti-vegf-therapy.opt` | Patient’s enrolment in anti-VEGF therapy | INITIAL | 2016-07-16T13:24:39+02:00 | 3 |
| 1013.26.980 | `diphtheria-case-investigation-form.opt` | Diphtheria case investigation form | INITIAL | 2024-12-30T01:55:25+01:00 | 7 |
| 1013.26.981 | `person-jm-guardian.opt` | Person - JM Guardian | INITIAL | 2024-12-29T01:56:15+01:00 | 3 |
| 1013.26.988 | `accidental-poisoning-case-investigation-form.opt` | Accidental poisoning case investigation form | INITIAL | 2024-12-30T03:35:42+01:00 | 12 |
| 1013.26.99 | `patient-s-background-leading-to-the-diagnosis-of-amd.opt` | Patient's background leading to the diagnosis of AMD | INITIAL | 2016-07-16T13:25:25+02:00 | 2 |

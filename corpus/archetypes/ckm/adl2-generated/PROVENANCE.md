# CKM generated ADL 2 export: provenance

Fetched by `scripts/vendor/ckm-adl2-export.sh` on 2026-11-12T14:39:05Z from
`https://ckm.openehr.org/ckm/retrieveArchetype?cid-archetype=<cid>&format=ADL2`, one request per archetype
the REST list (`https://ckm.openehr.org/ckm/rest/v1/archetypes`, every `resource-state` its
contract `https://ckm.openehr.org/ckm/rest/v1/openapi.json` enumerates) names: 1263 listed,
1205 exported. CKM ran Release 1.22.0 with Archie's `adl_2.4_support`
converter (announced on https://discourse.openehr.org/t/adl2-generation-appears-to-be-incorrect/17448,
post 13, 2026-11-04).

## What this is

Every file is CKM's GENERATED 1.4->2 conversion: its header carries
`generated`. It is vendored verbatim to measure that conversion and the
CKM sources (#4512, #4973). It is not authored ADL 2: the ADL 2 corpus is
`scripts/vendor/adl2-archetypes.sh`, and these files are never an oracle
for FerroEHR's own converter.

## Pin

The servlet serves the current conversion and takes no version, so the
pin is the fetch date above and the sha256 of every file in
`SHA256SUMS`. A re-run that changes a digest is a change in CKM's
conversion or in the archetype.

## Exercised

`crates/openehr-adl/tests/it/ckm_adl2_generated.rs` parses every file and
validates it as an ADL 2 upload is validated, parents first; each refusal
is adjudicated there with its rule code and whether the conversion or the
source causes it.

## Licensing

CKM publishes no repository-level licence; each archetype carries its own
`description` licence metadata (predominantly CC-BY-SA 3.0), which rides
along verbatim; reference copy: `LICENSE-CC-BY-SA-3.0` at the repository root.

## Not exported

NOTFOUND is a 404 (a resource CKM does not make public, such as one in
INITIAL or TEAMREVIEW state); EMPTY is a 200 with no body, CKM's
conversion producing nothing; BAD is a body that is not generated ADL 2;
FAIL is no answer after three tries.

| cid | file | outcome |
|---|---|---|
| 1013.1.125 | openEHR-EHR-EVALUATION.risk-family_history.v1.adls | NOTFOUND |
| 1013.1.1365 | openEHR-EHR-INSTRUCTION.request-imaging_exam.v1.adls | NOTFOUND |
| 1013.1.147 | openEHR-EHR-OBSERVATION.substance_use-alcohol.v1.adls | EMPTY |
| 1013.1.148 | openEHR-EHR-OBSERVATION.substance_use-caffeine.v0.adls | EMPTY |
| 1013.1.149 | openEHR-EHR-OBSERVATION.substance_use-tobacco.v0.adls | EMPTY |
| 1013.1.168 | openEHR-EHR-EVALUATION.problem-diagnosis.v1.adls | NOTFOUND |
| 1013.1.171 | openEHR-EHR-OBSERVATION.heart_rate-pulse.v1.adls | NOTFOUND |
| 1013.1.1757 | openEHR-EHR-INSTRUCTION.request-report.v1.adls | NOTFOUND |
| 1013.1.1858 | openEHR-EHR-ACTION.procedure-Hip_arthroplasty_previous_procedure_res.v1.adls | EMPTY |
| 1013.1.1859 | openEHR-EHR-ACTION.procedure-primary_hip_arthroplasty_res.v1.adls | EMPTY |
| 1013.1.1860 | openEHR-EHR-ACTION.procedure-revision_hip_arthroplasty_res.v1.adls | EMPTY |
| 1013.1.188 | openEHR-EHR-OBSERVATION.indirect_oximetry.v1.adls | EMPTY |
| 1013.1.1889 | openEHR-EHR-ACTION.procedure-hip_arthroplasty_revision_previous_procedure_res.v1.adls | EMPTY |
| 1013.1.1947 | openEHR-EHR-OBSERVATION.glasgow_coma-paed.v0.adls | NOTFOUND |
| 1013.1.197 | openEHR-EHR-EVALUATION.adverse_reaction.v1.adls | EMPTY |
| 1013.1.2195 | openEHR-EHR-OBSERVATION.laboratory_test-histopathology.v1.adls | NOTFOUND |
| 1013.1.232 | openEHR-EHR-CLUSTER.health_event-trauma.v0.adls | EMPTY |
| 1013.1.246 | openEHR-EHR-CLUSTER.symptom-pain.v1.adls | NOTFOUND |
| 1013.1.26 | openEHR-EHR-CLUSTER.health_event-poisoning.v1.adls | EMPTY |
| 1013.1.2859 | openEHR-EHR-CLUSTER.microscopy_renal_biopsy_non_neoplastic.v0.adls | EMPTY |
| 1013.1.30 | openEHR-EHR-CLUSTER.move-joint.v0.adls | EMPTY |
| 1013.1.31 | openEHR-EHR-CLUSTER.move-spine.v1.adls | EMPTY |
| 1013.1.3183 | openEHR-EHR-OBSERVATION.telecommunication.v0.adls | EMPTY |
| 1013.1.328 | openEHR-EHR-OBSERVATION.laboratory_result.v1.adls | EMPTY |
| 1013.1.329 | openEHR-EHR-OBSERVATION.laboratory_result-histology.v1.adls | EMPTY |
| 1013.1.330 | openEHR-EHR-OBSERVATION.laboratory_result-histology-haematopoietic_tumour.v1.adls | EMPTY |
| 1013.1.350 | openEHR-EHR-OBSERVATION.lab_test.v1.adls | EMPTY |
| 1013.1.351 | openEHR-EHR-OBSERVATION.lab_test-histopathology.v1.adls | EMPTY |
| 1013.1.356 | openEHR-EHR-CLUSTER.anatomical_location-precise.v1.adls | EMPTY |
| 1013.1.4512 | openEHR-EHR-OBSERVATION.mental_state.v0.adls | NOTFOUND |
| 1013.1.485 | openEHR-DEMOGRAPHIC-ADDRESS.address_iso-health_provider.v1.adls | NOTFOUND |
| 1013.1.551 | openEHR-EHR-OBSERVATION.lab_test-hba1c.v1.adls | EMPTY |
| 1013.1.552 | openEHR-EHR-OBSERVATION.lab_test-immunology.v1.adls | EMPTY |
| 1013.1.553 | openEHR-EHR-OBSERVATION.lab_test-immunology_titres.v1.adls | EMPTY |
| 1013.1.554 | openEHR-EHR-OBSERVATION.lab_test-immunology-ANA.v1.adls | EMPTY |
| 1013.1.555 | openEHR-EHR-OBSERVATION.lab_test-lipids.v1.adls | EMPTY |
| 1013.1.556 | openEHR-EHR-OBSERVATION.lab_test-liver_function.v1.adls | EMPTY |
| 1013.1.557 | openEHR-EHR-OBSERVATION.lab_test-microalbumin.v1.adls | EMPTY |
| 1013.1.558 | openEHR-EHR-OBSERVATION.lab_test-microbiology.v1.adls | EMPTY |
| 1013.1.559 | openEHR-EHR-OBSERVATION.lab_test-microbiology-urine.v1.adls | EMPTY |
| 1013.1.560 | openEHR-EHR-OBSERVATION.lab_test-thyroid.v1.adls | EMPTY |
| 1013.1.561 | openEHR-EHR-OBSERVATION.lab_test-urea_and_electrolytes.v1.adls | EMPTY |
| 1013.1.562 | openEHR-EHR-OBSERVATION.lab_test-urine_protein.v1.adls | EMPTY |
| 1013.1.563 | openEHR-EHR-OBSERVATION.lab_test-blood_gases.v1.adls | EMPTY |
| 1013.1.564 | openEHR-EHR-OBSERVATION.lab_test-blood_glucose.v1.adls | EMPTY |
| 1013.1.565 | openEHR-EHR-OBSERVATION.lab_test-blood_match.v1.adls | EMPTY |
| 1013.1.566 | openEHR-EHR-OBSERVATION.lab_test-full_blood_count.v1.adls | EMPTY |
| 1013.1.585 | openEHR-EHR-CLUSTER.waveform.v0.adls | EMPTY |
| 1013.1.5939 | openEHR-EHR-CLUSTER.imaging_exam-lesion-adnexal_mass.v0.adls | NOTFOUND |
| 1013.1.60 | openEHR-EHR-OBSERVATION.intravascular_pressure-jvp.v0.adls | EMPTY |
| 1013.1.6043 | openEHR-EHR-EVALUATION.biospecimen_summary-semen.v0.adls | NOTFOUND |
| 1013.1.613 | openEHR-EHR-INSTRUCTION.request-referral.v1.adls | NOTFOUND |
| 1013.1.616 | openEHR-EHR-CLUSTER.imaging.v0.adls | EMPTY |
| 1013.1.630 | openEHR-EHR-OBSERVATION.lab_test-esr.v1.adls | EMPTY |
| 1013.1.678 | openEHR-EHR-INSTRUCTION.request-lab_test.v0.adls | NOTFOUND |
| 1013.1.687 | openEHR-EHR-OBSERVATION.lab_test-microbiology-csf.v1.adls | EMPTY |
| 1013.1.722 | openEHR-EHR-INSTRUCTION.request-procedure.v0.adls | NOTFOUND |
| 1013.1.766 | openEHR-EHR-OBSERVATION.intravascular_pressure-cvp.v1.adls | EMPTY |

# CKM generated ADL 2 export: provenance

Fetched by `scripts/vendor/ckm-adl2-export.sh` on 2026-11-04T18:41:09Z from
`https://ckm.openehr.org/ckm/retrieveArchetype?cid-archetype=<cid>&format=ADL2`, one request per archetype
the REST list (`https://ckm.openehr.org/ckm/rest/v1/archetypes`) names: 946 listed,
936 exported. CKM ran Release 1.22.0 with Archie's `adl_2.4_support`
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
| 1013.1.1757 | openEHR-EHR-INSTRUCTION.request-report.v1.adls | NOTFOUND |
| 1013.1.1858 | openEHR-EHR-ACTION.procedure-Hip_arthroplasty_previous_procedure_res.v1.adls | EMPTY |
| 1013.1.1859 | openEHR-EHR-ACTION.procedure-primary_hip_arthroplasty_res.v1.adls | EMPTY |
| 1013.1.1860 | openEHR-EHR-ACTION.procedure-revision_hip_arthroplasty_res.v1.adls | EMPTY |
| 1013.1.1889 | openEHR-EHR-ACTION.procedure-hip_arthroplasty_revision_previous_procedure_res.v1.adls | EMPTY |
| 1013.1.2859 | openEHR-EHR-CLUSTER.microscopy_renal_biopsy_non_neoplastic.v0.adls | EMPTY |
| 1013.1.3183 | openEHR-EHR-OBSERVATION.telecommunication.v0.adls | EMPTY |
| 1013.1.4512 | openEHR-EHR-OBSERVATION.mental_state.v0.adls | NOTFOUND |
| 1013.1.585 | openEHR-EHR-CLUSTER.waveform.v0.adls | EMPTY |
| 1013.1.5939 | openEHR-EHR-CLUSTER.imaging_exam-lesion-adnexal_mass.v0.adls | NOTFOUND |

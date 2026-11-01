---
name: adl2-rest-wire-contract-location
description: Where the ITS-REST ADL2 template operations wire contract lives (paths, params, schemas, responses) + the OperationalTemplateV2 opaque-object fact
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# ITS-REST ADL2 template operations — wire-contract spec locations

Base path `/definition/template/adl2` (server `https://{baseUrl}/v1`, `definition.openapi.yaml` L18). Path->op wiring: `docs/specs/openehr/ITS-REST/specifications/definition.openapi.yaml` L36-49.

The 5 operation YAMLs: `docs/specs/openehr/ITS-REST/specifications/operations/definition_template_adl2_{list,upload,get,example_get,version_get}.yaml`.
- list: GET /adl2 ; upload: POST /adl2 ; get: GET /adl2/{template_id} ; example: GET /adl2/{template_id}/example ; version_get: GET /adl2/{template_id}/{version} (op file has `deprecated: true` L3).

**LOAD-BEARING: `schemas/aom/OperationalTemplateV2.yaml` is `{title: OPERATIONAL_TEMPLATE_V2, type: object}` — an opaque object, NO declared properties.** OPT body is treated as free-form; the actual serialization is ADL2 source text (text/plain, `oneOf: [OperationalTemplateV2, string]`). So the OAS declares no JSON structure for the OPT.

Component YAMLs (all under `.../specifications/`):
- Accept headers: `parameters/header/Accept_Template_adl2.yaml` = {application/json, application/xml, text/plain}; `Accept_JSON.yaml` = {application/json} (used by list); `Accept_LOCATABLE.yaml` = {application/json, application/xml, application/openehr.wt.flat+json, application/openehr.wt.structured+json} (used by example_get).
- `parameters/header/Prefer.yaml` enum {return=representation, return=minimal, return=identifier}, default return=minimal.
- TemplateMetadata/list-item: `schemas/definition/TemplateMetadata.yaml` (props template_id, version[deprecated], concept, archetype_id, created_timestamp; required all except version). TemplateList = array of TemplateMetadata.
- 201 upload identifier body: `schemas/others/TemplateIdentifier.yaml` = {title: Identifier, required template_id} — NOTE discrepancy: overview prose (Requests_and_responses.md L311-319) says return=identifier body is a `{uid}` object, but the adl2 upload 201 refs TemplateIdentifier which uses `template_id`.
- Responses: `responses/{200_TemplateList_adl2,201_Template_adl2_upload,200_Template_adl2_retrieved,200_Template_example_retrieved,400,409_template_already_exists,404_unknown_template_id,404_unknown_template_id_or_version,406}.yaml`.

**Prefer semantics for upload:** overview `docs/overview/Requests_and_responses.md` L282-322 (minimal->204/empty, identifier->body w/ id never 204, representation->full body). 201 response desc restates it.

**CNF coverage GAP:** NO `I_DEFINITION_ADL2` Robot suite exists (only `I_DEFINITION_ADL14` + `I_DEFINITION_QUERY` under CNF/tests/platform/robot/). Schedule `CNF/docs/platform_test_schedule/master04-func_tc_definition_adl.adoc` references the I_DEFINITION_ADL2 SM interface abstractly; `CNF/scripts/openehr_platform/tc_def-adl2_prov.txt` is an SM abstract test (provenance upload), not a REST wire test. No concrete REST status-code/payload conformance case for ADL2.

**Spec silences to flag:** no If-Match/ETag precondition on upload (upload uses `at_version` query param, deprecated, name="version"); example_get is the only ADL2 op declaring 406; upload declares NO 400-content beyond generic; 409 body is empty (description only).

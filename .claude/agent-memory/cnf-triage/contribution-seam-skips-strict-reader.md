---
name: contribution-seam-skips-strict-reader
description: APP bin (confirmed 2026-08-05) — both CONTRIBUTION commit lanes never typed-decode versions[].data, so parse-class refusals surface as 422 where every direct route answers 400
metadata:
  type: project
---

**The defect.** `POST /ehr/{id}/contribution` and `POST /demographic/contribution`
parse the request body as a bare `serde_json::Value`
(`app/ferroehr-rest/src/api/ehr/contribution.rs` — `negotiate::json_value`) and
never run each member's `versions[i].data` through
`openehr_its::json::from_canonical_value`. A payload the strict reader CANNOT
CONSTRUCT (empty `1..*` container, and every other parse-class shape) therefore
reaches semantic validation and comes back **422**, while the identical bytes on
the direct route (`decode_composition_body` → `negotiate::rm_value::<Composition>`)
come back **400**.

**Why 400 is the required class** (all read first-hand):
- ITS-REST overview `Requests_and_responses.md` §HTTP status codes: 400 =
  "syntactically invalid content"; 422 = "well-formed but … semantic errors".
- Released OAS `responses/422.yaml` narrows 422 to content that "could be
  converted to a resource" — an empty `1..*` list cannot (BMM
  `cardinality.lower == 1`), so the 422 antecedent is false.
- `operations/contribution_create.yaml` + `demographic_contribution_create.yaml`
  declare **201/400/404/409 only — no 422 at all** (confirmed in the split tree
  AND the bundled `ehr-codegen.openapi.yaml`).
- The register already fixes the line for this route in two halves:
  **AMB-193** ("an empty mandatory list is unrepresentable and refused at
  parse") + **AMB-194** ("400 is reserved on this route for the SHAPE class …
  and a complete-but-incomplete body parses"). Parses ⇒ 422; does not parse ⇒ 400.

**Reproduced 2026-08-05** (image `ghcr.io/ferrohealth/ferroehr:local`, plain
`docker-compose.yml`, no SMART overlay — the SMART overlay 403s the admin
template upload under Basic auth):
`minimal_event.cluster_no_items` → direct composition **400**
("invalid canonical JSON body: a container with a cardinality lower bound of 1
must have at least one member"), same bytes inside a CONTRIBUTION member
**422**. Same split for `paragraph_no_items` and for
`demographic/person.cluster_no_items` on `/demographic/contribution`
("body does not validate as PERSON: …").

**Fix direction:** typed-decode `data` by its `Kind` at the commit seam
(`app/ferroehr/src/versioning/contribution.rs` Action::Create ~:608-619,
Action::Modify ~:646-659) and map the decode refusal to `ServiceError::BadRequest`.
The demographic lane already decodes inside
`service/demographic/validate.rs::party_check` but maps it to `Unprocessable` —
that mapping is the same defect, one line up.

Related: [[parse-vs-semantic-400-422-split]], [[nonempty-1star-containers]].

---
name: definition-artefact-delete-ownership
description: Where the spec assigns archetype/template/OPT DELETION (SM Definitions component, never Admin), and where the total authorization/privilege silence for it lives — plus the CNF sentence + EHRbase placeholder that are the only "admin" hooks
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# Who owns archetype / template / OPT deletion — file map

Companion to [[admin-api-location]], [[adl14-aom14-validity-location]],
[[adl2-rest-wire-contract-location]], [[extension-surface-spec-silence]].

## Released ITS-REST 1.1.0 defines NO artefact/template DELETE at all
`ITS-REST/specifications/definition.openapi.yaml` — 11 path+method combos,
**zero DELETE**, and the only path families are `/definition/template/adl1.4`,
`/definition/template/adl2`, `/definition/query/{qualified_query_name}`.
**No `/definition/archetype/**` and no `/definition/artefact/**` path exists**
(released bundles agree: `crates/openehr-its/vendor/rest-oas/definition-*.openapi.yaml`).
`admin.openapi.yaml` = exactly 2 paths, both EHR-scoped deletes. So any
archetype-delete / artefact-delete / admin-template-delete route is OWN EXTENSION.
Lifecycle: Definition API `x-status: STABLE`, Admin API `x-status: DEVELOPMENT`.

## SM assigns ALL of them to the Definitions component (reference text only)
`SM/docs/openehr_platform/master04-definition_package.adoc` §Archetypes and
Templates — the sentence it states on ownership, which grounds nothing (the
route ownership is our own design): the ADL14/ADL2 interfaces
"enable upload, updating and **removal** of archetypes and templates". Also
states the ADL2 kind-collapse ("archetypes and 'templates' are all instances of
archetypes, formally speaking") and that ADL1.4 templates are "distinct
artefacts" — distinct in ID/FORM (UUID + XML vs ARCHETYPE_ID), never in authority.
- `SM/docs/UML/classes/i_definition_adl14.adoc` — `delete_archetype` (L74-87)
  AND `delete_opt` (L145-158) live in the SAME interface.
- `SM/docs/UML/classes/i_definition_adl2.adoc` L100-110 — one `delete_artefact`
  covers archetype/template/OPT (cf. `upload_artefact` L28-42 wording).
- `SM/docs/UML/classes/i_admin_service.adoc` — 6 ops, only `physical_ehr_delete`
  + `physical_party_delete`. **The strings template/archetype/artefact appear
  NOWHERE in master15-admin_service.adoc or the 3 i_admin_* class files.**
- Component one-liners: `master02-overview.adoc` L31 (Definitions = "upload and
  querying of definition artefacts, including archetypes, templates and
  queries") vs L40 (Admin = "administrative facilities on all services in the
  installed environment, such as back-up").

## Authorization is spec-SILENT everywhere (both components)
- `ITS-REST/.../docs/overview/Requests_and_responses.md` §Authentication and
  authorization (L28-35) — SHOULD implement, "does not mandate a specific
  authentication scheme", only the 401/403/407 split is MUST. **No per-op,
  per-API or per-role statement; `administrat*` appears NOWHERE in ITS-REST
  docs/ operations/ responses/.**
- All 7 `*.openapi.yaml` declare `security: []` identically and there are ZERO
  `securitySchemes` — the OAS does not differentiate Admin from Definition.
- `docs/admin/Description.md` is a 29-line stub (Purpose/Related/Status) that
  does NOT characterize the Admin API as destructive or privileged.
- `SM/docs/openehr_platform/master02-overview.adoc` §Global Style: L90 lists
  "approach to access control and authorisation" as an implementation-choice
  dimension; L109 puts authn/authz out of band ("assumed to have been dealt
  with before any particular call"); the L75 `Auth_error` exception is generic
  to every call.

## The only two "admin" hooks in the whole tree (corroboration at most)
1. `CNF/docs/platform_test_schedule/master04-func_tc_definition_adl.adoc` **L325**
   — the ONLY privilege sentence anywhere: "only users with admin permissions
   should be able to physically delete OPTs". It is labelled *Implementation
   recommendations*, and its axis is **logical-vs-PHYSICAL delete, not
   template-vs-archetype**. The chapter itself is titled "DEFINITION Service /
   I_DEFINITION_ADL2 and I_DEFINITION_ADL14" and its §delete_opt() has 4 cases;
   there is NO delete_archetype and NO ADL2 case (5 sections, all ADL14 OPT).
2. `CNF/tests/platform/robot/I_DEFINITION_ADL14/delete_opt/*.robot` — EHRbase
   (Vitasystems/HMS) copyright, `Force Tags … TODO`, body is
   `Log NOT IMPLEMENTED` + "THIS IS JUST A PLACEHOLDER!", and carries a
   `*** comments ***` line **"This will probably be available via ADMIN API
   only!"** — speculative, unimplemented, EHRbase-authored. Almost certainly the
   historical origin of the `/admin/template/{id}` route; NOT spec.
   `CNF/docs/profiles/master03-profiles.adoc` L53-58 backs the opposite: the
   Admin capability set is entirely EHR/Demographic (no template/archetype
   capability), DEFINITION API is CORE/STANDARD, ADMIN API is OPTIONS-only.

## AM: no ownership/lifecycle KIND distinction
`AM/docs/Overview/master03-the_specifications.adoc` L17 — "Since in ADL 2 a
template is just a kind of archetype". `master02-formalism_overview.adoc` L21
(use-case/business-event vs topic; "Templates are the technical means of using
archetypes in runtime systems") + `master04-semantic_overview.adoc` L104 +
`master05-artefacts.adoc` §The Development Process are FUNCTIONAL/semantic only.
Nothing calls a template deployment configuration under separate administrative
authority; grep for site/deployment/local-specific ownership in AM = nothing.

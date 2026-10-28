- Nothing ITS-REST 1.1.0 does not define is served under the openEHR base path
  any more. FerroEHR serves three namespaces beside each other under
  `/ferroehr/rest/`: `/openehr/v1/` holds only the ITS-REST routes,
  `/fhir/r4/` the FHIR and IHE interfaces, and `/extensions/v1/` FerroEHR's own
  extensions. The ITI-81 retrieval moved from
  `/ferroehr/rest/openehr/v1/fhir/r4/AuditEvent` to
  `/ferroehr/rest/fhir/r4/AuditEvent`. These routes moved from
  `/ferroehr/rest/openehr/v1/...` to `/ferroehr/rest/extensions/v1/...` with
  the same relative path: `/terminology/...`, `/admin/event_subscription...`,
  `/demographic/party_relationship...`,
  `/demographic/versioned_party_relationship/...`,
  `/definition/archetype/...`, `/definition/artefact/...`, `/message/export...`,
  `/message/import...`, `/query/cohort`, and the admin extensions
  `/admin/config`, `/admin/template/{template_id}`,
  `/admin/query/{qualified_query_name}/{version}`, `/admin/party/...`,
  `/admin/archive/...`, `/admin/report/...`, `/admin/dump`, `/admin/load`,
  `/admin/integrity/...`, `/admin/restriction...`,
  `/admin/access-restriction...`, `/admin/research-objection` and
  `/admin/retention/...`. The old paths answer 404, with no redirect. The
  served OpenAPI document and its family documents name the new paths; the
  admin extensions, the archetype library and the cohort query have family
  documents of their own (`admin-extensions`, `archetypes`, `cohort`).

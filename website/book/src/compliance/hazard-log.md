# Hazard log: the logging component

This page is the clinical safety hazard log of FerroEHR's European logging
software component, the part of the EHR system that "provides logging
information related to access by health professionals or other individuals to
priority categories of personal electronic health data" (EHDS Art. 2(2)(o),
Regulation (EU) 2025/327). EHDS Annex II 1.1 asks that the harmonised software
components "achieve the performance intended by its manufacturer" and be
designed so that, "during normal conditions of use, they are suitable for their
intended purpose and their use does not put at risk patient safety".

The Regulation names no method or standard for showing that, so the form of
this log is FerroEHR's own design. For each hazard it states the cause, what
could happen to a patient, the control, the test that shows the control holds,
and what remains. "Normal conditions of use" are those of the
[intended-purpose statement](intended-purpose.md). The log is the EHDS-side
companion of the [CRA risk assessment](cra-risk-assessment.md), and it is
versioned with the release like every page of this book.

<!-- toc -->

> [!WARNING]
> The quotations are from the Official Journal text vendored in the repository
> at
> [`docs/law/eu/ehds/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/ehds/text.html)
> ([EUR-Lex](https://eur-lex.europa.eu/eli/reg/2025/327/oj)). A control and a
> passing test show what the software does; they do not show that the
> component meets Annex II, which no conformity assessment has examined.

## The component and the surface it records

The component is, in the code: `ferroehr::system_log` (the access-event model,
the audit store, the DICOM and FHIR renderings and the sinks),
`ferroehr-rest::system_log` (the middleware that records every API access and
classifies it), the ITI-81 `GET /fhir/r4/AuditEvent` retrieval, and the
retention register in `ferroehr::storage::marks`. The middleware is installed
outermost on the API router, around the authentication layer, so it sees every
response of the openEHR REST surface, refusals included. What each record
holds, field by field against Annex II 3.2, is on the
[audit trail](../audit.md#the-ehds-logging-elements-mapped) page.

Test paths below are relative to the repository root. Each was read for this
log; "no test" means none exists.

## H1: an access is not recorded

**Cause:** a route the middleware does not classify; a record dropped when
the emission queue is full or the store is unreachable; a domain-level
access (resolving a subject to an EHR, an identifier resolution, an EHR
Extract export) that skips the middleware; reads made straight against the
database.

**Effect:** a patient, or the organisation investigating a breach, cannot
learn that their record was read. Under EHDS Art. 9 a patient has a right to
that information.

**Control:** every generated ITS-REST operation has an explicit audit
classification, and an unknown operation falls back to the audited default.
A rejected access (`401`, `403`) is always recorded. Under
`[audit] fail_mode = "closed"` a request whose record cannot be taken is
answered `503` and the domain-level operations withhold their result. An AQL
execution emits one record per EHR it served. `deployment_profile =
"production"` refuses audit switched off, audit with no durable sink and
audit failing open, unless each is accepted by name.

**Shown by:**

- `app/ferroehr-rest/src/system_log/classify.rs`:
  `every_generated_operation_is_explicit`,
  `every_read_operation_produces_an_access_record`,
  `unrecognised_operation_fails_closed_to_default`;
- `app/ferroehr-rest/tests/it/audit_e2e.rs`: one test per resource family
  (for example `composition_get_emits_read_record`,
  `aql_execute_emits_execute_record`), `unauthenticated_request_emits_401_record`,
  `fail_closed_returns_503_when_channel_full`,
  `fail_closed_503_carries_openehr_error_body_and_retry_after`,
  `aql_execute_emits_one_access_record_per_served_ehr`;
- `app/ferroehr/tests/it/access_event_fail_closed.rs`:
  `a_linkage_resolution_is_withheld_when_its_record_is_rejected`,
  `a_subject_lookup_is_withheld_when_its_record_is_rejected`,
  `an_identifier_resolution_is_withheld_when_its_record_is_rejected`,
  `an_extract_export_is_withheld_when_its_record_is_rejected`;
- `app/ferroehr/src/config/deployment.rs`:
  `production_refuses_each_open_gap_and_names_the_remedy`,
  `production_refuses_the_open_access_default_and_the_open_audit_fail_mode`;
- `app/ferroehr/tests/it/legal_marks.rs`:
  `setting_and_lifting_a_mark_is_recorded_in_the_access_trail`;
- `app/ferroehr-rest/tests/it/audit_route_coverage.rs`:
  `every_route_writes_exactly_one_access_record` enumerates every route the
  assembled router mounts, the extension routes included, drives each one
  once, and asserts exactly one access record carrying its operation id. The
  routes it exempts (the health probes, the public status and discovery
  documents, the OpenAPI document and Swagger UI, the `OPTIONS` manifest and
  the management surface) are listed in the test with the reason.

**Remains:** the shipped `fail_mode` is `open`, so under the `sandbox`
profile a full queue drops a record and the request succeeds
(`fail_open_serves_request_when_channel_full` pins that behaviour). Database
access outside the server is recorded nowhere.

## H2: a record names the wrong person, organisation or patient

**Cause:** the actor taken from something other than the verified
credential; an organisation guessed when the token carries none; the EHR
taken from the wrong part of the request; one record written for an AQL
page that served several patients.

**Effect:** an access is attributed to a professional who did not make it,
or recorded against the wrong patient, so a patient is told of an access to
someone else's record and the real one goes unseen.

**Control:** the principal comes from the authenticated credential; a `401`
records the caller as unknown and never fabricates one. The organisation is
read from the same token claim the ABAC layer reads, and stays empty when
there is no claim or the caller used Basic. The EHR comes from the resource
the dispatch resolved, or from the request path where none exists. An AQL
execution records each served EHR separately.

**Shown by:**

- `app/ferroehr/tests/it/audit_ehds_mapping.rs`:
  `element_a_the_accessing_organisation_is_recorded_and_rendered_in_fhir`,
  `element_b_the_person_who_accessed_is_recorded_and_rendered`,
  `nen_7513_the_actor_roles_are_recorded_and_rendered_in_both_formats`;
- `app/ferroehr/tests/it/audit_store.rs`:
  `access_fields_round_trip_on_both_write_paths`,
  `the_accessing_organisation_round_trips_on_both_write_paths`,
  `the_actor_roles_round_trip_on_both_write_paths`;
- `app/ferroehr-rest/src/system_log/middleware.rs`:
  `ehr_id_from_various_paths`, `object_id_from_template_paths`,
  `object_id_from_query_paths`, `object_id_from_demographic_paths`;
- `app/ferroehr-rest/tests/it/audit_e2e.rs`:
  `unauthenticated_request_emits_401_record`,
  `aql_execute_emits_one_access_record_per_served_ehr`.

**Remains:** a shared account or an application's service credential names
the application, not the professional behind it; whether a natural person
stands behind a token, and at which authentication assurance level, is not
recorded (planned, #3622).

## H3: the category of the data is not recorded

**Cause:** a template or archetype missing from the deployment's
`[audit.categories]` map; a record type the classifier does not reach.

**Effect:** a record cannot say whether the access reached a patient
summary, a prescription or a test result, which Annex II 3.2(c) asks for
and the patient's information under EHDS Art. 9(2)(c) depends on.

**Control:** each access is classified when it happens, by template id, then
root archetype id, then the query's positive constraints. A logical delete is
classified by the version it deleted, a `VERSIONED_COMPOSITION` read by the
composition it holds, a `CONTRIBUTION` read (with or without
`Prefer: resolve_refs`) by the versions it committed, and an EHR Extract
export or import by the versions it carried or landed. `EHR`, `EHR_STATUS`,
`EHR_ACCESS` and the directory are recorded `none` by resource kind. An
access the map cannot classify is recorded `unclassified`, with the ids as
evidence, and is never refused. Each record carries the digest of the map
that classified it.

**Shown by:**

- `app/ferroehr-rest/tests/it/audit_categories.rs`:
  `an_unmapped_template_is_unclassified_with_its_ids_as_evidence`,
  `a_multi_category_query_records_every_category_it_served`,
  `a_leaf_only_query_is_classified_from_its_constraints`,
  `a_zero_row_query_is_classified_from_its_constraints`,
  `a_not_contains_operand_never_counts`,
  `a_flat_write_is_classified_by_its_template`,
  `a_logical_delete_is_classified_by_the_version_it_deletes`,
  `a_versioned_composition_read_is_classified_by_its_composition`,
  `a_contribution_read_is_classified_by_its_versions`;
- `app/ferroehr/tests/it/service_message_audit.rs`:
  `the_extract_records_carry_the_categories_of_the_data`;
- `app/ferroehr/tests/it/audit_ehds_mapping.rs`:
  `element_c_the_priority_category_is_recorded_and_rendered_in_fhir_only`.

**Remains:** the map is the deploying organisation's, and FerroEHR ships
none, so an incomplete map yields `unclassified` records. Records in the
demographic domain carry no classification, because no priority category
lives there. The Helm chart carries the map as `config.audit.categories`, empty
by default as in the binary.

## H4: a record misstates where the data came from

**Cause:** a body's `FEEDER_AUDIT` provenance ignored; an AQL page's
origins taken from one row only.

**Effect:** a patient or a reviewer is told the wrong system created the
data that was read (Annex II 3.2(e)).

**Control:** each committed version records the distinct originating systems
of its body, or this server when it has none; a read records the origins of
what it served, an AQL page the union, with the true count beside a capped
set. The FHIR `AuditEvent` of a read or a query carries the origins of the
data it served.

**Shown by:** `app/ferroehr/tests/it/access_origins.rs`:
`a_body_without_feeder_audit_has_this_server_as_its_origin`,
`feeder_audits_anywhere_in_the_body_count_once_each`,
`an_aql_page_records_the_union_of_the_origins_it_served`,
`an_unstamped_row_is_assessed_from_the_body_at_read`;
`app/ferroehr-rest/tests/it/audit_e2e.rs`:
`composition_get_records_the_origins_of_the_served_data`;
`app/ferroehr/src/system_log/fhir.rs`:
`query_record_carries_the_origins_of_the_served_data`.

**Remains:** the DICOM rendering carries no origins, because PS3.15 defines no
element for them; the local store and the FHIR export carry them.

## H5: the log is lost, or reaped too early

**Cause:** a retention period shorter than the law requires; a reaper that
deletes more than it should; the database that holds the local store lost;
a forwarding sink down.

**Effect:** the information a patient has a right to for "at least three
years from each date of access" (EHDS Art. 9(2)) no longer exists.

**Control:** each jurisdiction has a registered retention floor (three years
in every EU Member State, longer where national law sets more), and a
configured period below it stops the boot. The reaper deletes only records
past the horizon, in calendar years where the period is given in years, and
the chain stays verifiable after it runs. The forwarding outbox keeps
records until the sink takes them. `production` refuses audit with no
durable sink unless accepted by name.

**Shown by:**

- `app/ferroehr/src/config/mod.rs`:
  `a_retention_below_the_jurisdiction_floor_is_refused`,
  `the_ehds_floor_holds_in_every_member_state_and_a_longer_national_floor_wins`,
  `retention_years_meets_a_floor_in_years_exactly`,
  `a_ceiling_below_a_floor_is_refused`;
- `app/ferroehr/src/system_log/config.rs`:
  `the_ehds_floor_reaches_every_member_state_and_a_longer_national_floor_wins`;
- `app/ferroehr/tests/it/audit_store.rs`:
  `reap_deletes_only_rows_past_the_horizon`,
  `reap_years_deletes_only_rows_past_the_calendar_horizon`;
- `app/ferroehr/tests/it/audit_chain.rs`:
  `retention_reaping_leaves_a_verifiable_trail`;
- `app/ferroehr/tests/it/audit_feed.rs`:
  `arr_outage_leaves_rows_pending_then_delivers_on_recovery`.

**Remains:** a local store shares the fate of its database, so backups and
an off-box sink are the deploying organisation's
([B7](../threat-model.md#b7--the-audit-trail)).

## H6: a patient cannot read their log

**Cause:** the log reachable only with an administrator credential; no
retrieval scoped to one patient.

**Effect:** the information EHDS Art. 9 gives a patient cannot be produced.

**Control:** ITI-81 `GET /fhir/r4/AuditEvent` searches the log by patient,
agent, action and date, and a configured subject audit role reads one
subject's records and nothing else. EHDS Art. 9(2) has the information
provided "through electronic health data access services", which the Member
States establish (Art. 4(1)); FerroEHR serves the log to such a service and
has no patient-facing interface of its own.

**Shown by:** `app/ferroehr-rest/tests/it/audit_iti81.rs`:
`searchset_bundle_with_filters_and_paging`,
`the_subject_audit_role_reads_one_subjects_log_and_nothing_else`,
`rbac_gates_the_audit_surface_to_admins`.

**Remains:** the subject audit role is unset by default, so the log is
admin-only until the deployment configures it.

## H7: the access log is altered

**Cause:** a record changed or deleted in the database, by a compromised
application role or by someone with direct access.

**Effect:** an access disappears, or appears to have been made by someone
else, and the record a patient or a court relies on is false.

**Control:** records are hash-chained in the database; `UPDATE`, `DELETE`
and `TRUNCATE` on the log are refused; the application role cannot run DDL
or rewrite the log; `audit.verify_audit_chain()` names any record that
changed or went missing; the server runs that verification one minute after
boot and then every `[audit.store] verify_interval_seconds` (default one day),
logs a finding at `ERROR`, counts it in `atna_audit_chain_findings` and shows
it on the `audit_chain` readiness indicator; forwarding puts copies off the
box.

**Shown by:**

- `app/ferroehr/tests/it/audit_chain.rs`:
  `a_modified_audit_record_is_detected_and_named`,
  `a_deleted_audit_record_is_detected_in_the_middle_and_at_the_end`,
  `the_audit_table_refuses_mutation_deletion_and_truncation`,
  `the_application_role_cannot_run_ddl_or_rewrite_the_audit_trail`,
  `the_batched_write_path_is_chained_too`,
  `the_chain_check_reports_a_tampered_record_on_the_health_surface`,
  `the_audit_subsystem_schedules_the_chain_check`;
- `app/ferroehr/tests/it/audit_store.rs`:
  `the_access_trail_refuses_rewriting_deletion_and_truncation`.

**Remains:** the chain gives evidence of tampering and does not prevent it.
A deployment that sets `verify_interval_seconds = 0` finds a broken chain only
when an operator runs the check. The indicator never flips readiness, so a
finding reaches someone only through an alert on the counter or the
indicator, which the deploying organisation sets up.

## H8: the log discloses more than it should

**Cause:** the category classification or identifying content copied into
request logs, spans or metrics; the log served to a caller with no right to
it.

**Effect:** the log of who read a patient's record becomes a disclosure of
that record.

**Control:** the classification stays in the access log; telemetry carries
no identified data; the ITI-81 surface is authorised like any other.

**Shown by:** `app/ferroehr-rest/tests/it/audit_categories.rs`:
`the_classification_never_reaches_spans_logs_or_metrics`;
`app/ferroehr/tests/it/privacy_leak.rs`:
`the_atna_records_carry_the_opaque_pseudonym_and_nothing_else`;
`app/ferroehr-rest/tests/it/audit_iti81.rs`:
`rbac_gates_the_audit_surface_to_admins`.

**Remains:** the log names patients and actions by design, so whoever holds
ITI-81 access reads that.

## H9: an emergency access to restricted data is not marked

**Cause:** a professional overrides a restriction in an emergency, and the
record looks like any other access.

**Effect:** EHDS Art. 9(1) includes "access provided in accordance with
Article 11(5)" in what a patient is told about; an unmarked override hides
that it happened.

**Control:** none yet; no test.

**Remains:** the override and its marking are planned (#3624).

## When the log is reviewed

The log is reviewed in the release that changes any module of the component
listed above, with the hazards, controls and test names checked against that
release's tree. It is also reviewed when an incident, a complaint or a
vulnerability shows a hazard the log does not name. The
[technical documentation](technical-documentation.md) cites it as the
EHDS-side risk assessment of the component.

| Version | Date | Change |
|---|---|---|
| 1 | 2026-10-06 | First log, nine hazards |

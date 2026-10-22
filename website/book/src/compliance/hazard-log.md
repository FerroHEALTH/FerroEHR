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

The component is, in the code: the FerroHEALTH products' shared crates
`ehds-logging` 0.2.4 (the Annex II 3.2 access record, the Art. 14(1)
categories and their classification, the Art. 9(2) retention floor and the
emergency mark) and `ihe-atna` 0.2.4 (the BALP and DICOM renderings and the
ATNA sender), from FerroAUDIT; `ferroehr::system_log` (the access-event model,
the audit store, what each rendering says and the sinks),
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
The extension routes that change a record carry one too: the physical delete
of a demographic party is recorded as a delete of that party, as the physical
EHR delete is of the EHR. A rejected access (`401`, `403`) is always recorded. Under
`[audit] fail_mode = "closed"` a request whose record cannot be taken is
answered `503` and the domain-level operations withhold their result. An AQL
execution emits one record per EHR it served. `deployment_profile =
"production"` refuses audit switched off, audit with no durable sink and
audit failing open, unless each is accepted by name.

**Shown by:**

- `app/ferroehr-rest/src/system_log/classify.rs`:
  `every_generated_operation_is_explicit`,
  `every_read_operation_produces_an_access_record`,
  `unrecognised_operation_fails_closed_to_default`,
  `representative_mappings`;
- `app/ferroehr-rest/tests/it/admin_party_delete_http.rs`:
  `an_admin_deletes_the_party_and_the_access_record_names_it`;
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
execution records each served EHR separately. With
`[authz.professional]` configured, a token that names no natural person is
refused on patient data unless it is a client token acting for a named
professional, and the record carries the acting mode (`person` or `client`)
and that professional; with `[authz.assurance]` configured, it carries the
assurance level the token maps to, and a token below the configured minimum is
refused with a step-up challenge. The DICOM message and the FHIR
`AuditEvent` give the client and this server the same data-flow role (DICOM
PS3.15 §A.5.2.7, IHE BALP): on a read the server is the source and the client
the destination, on a write or a query the client is the source. A FHIR
record of a delete carries the agent types the BALP `Delete` profile fixes
(the client as DCM `110150` Application, the server as `custodian`).

**Shown by:**

- `app/ferroehr/tests/it/audit_ehds_mapping.rs`:
  `element_a_the_accessing_organisation_is_recorded_and_rendered_in_fhir`,
  `element_b_the_person_who_accessed_is_recorded_and_rendered`,
  `element_b_the_professional_behind_a_client_is_recorded_and_rendered`,
  `the_assurance_level_is_recorded_and_reaches_the_fhir_rendering_as_a_level`,
  `nen_7513_the_actor_roles_are_recorded_and_rendered_in_both_formats`;
- `app/ferroehr/tests/it/audit_store.rs`:
  `access_fields_round_trip_on_both_write_paths`,
  `the_accessing_organisation_round_trips_on_both_write_paths`,
  `the_actor_roles_round_trip_on_both_write_paths`,
  `both_inserts_persist_the_authentication_columns`;
- `app/ferroehr-rest/tests/it/patient_data_assurance.rs`:
  `a_token_below_the_minimum_gets_the_step_up_challenge`,
  `a_client_token_without_acting_for_is_refused`,
  `a_client_acting_for_a_professional_passes_and_is_recorded`;
- `app/ferroehr-rest/src/system_log/middleware.rs`:
  `ehr_id_from_various_paths`, `object_id_from_template_paths`,
  `object_id_from_query_paths`, `object_id_from_demographic_paths`;
- `app/ferroehr-rest/tests/it/audit_e2e.rs`:
  `unauthenticated_request_emits_401_record`,
  `aql_execute_emits_one_access_record_per_served_ehr`;
- `app/ferroehr/src/system_log/fhir.rs`:
  `dicom_and_fhir_agree_on_the_data_flow_direction`,
  `delete_carries_the_agent_types_balp_delete_fixes`.

**Remains:** both blocks are off by default, and until the deployment
configures `[authz.professional]` a client application's credential is
recorded as that application, not as the professional behind it. A shared
Basic account names the account; with either block configured, Basic is
refused on patient data. The DICOM format has no element for the assurance
level, so the syslog feed carries none; the FHIR record carries the level as
the BALP `ihe-assuranceLevel` extension, and the raw claim value stays in the
local store's column.

## H3: the category of the data is not recorded

**Cause:** a template or archetype missing from the deployment's
`[audit.categories]` map; a record type the classifier does not reach.

**Effect:** a record cannot say whether the access reached a patient
summary, a prescription or a test result, which Annex II 3.2(c) asks for
and the patient's information under EHDS Art. 9(2)(c) depends on.

**Control:** each access is classified when it happens by `ehds-logging`, by
template id, then root archetype id, then the query's positive constraints,
every one of which must be mapped. A logical delete is
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

**Control:** records are hash-chained in the database, and a record's digest
covers every column but the digest and the two forwarding stamps (digest
version 2, audit migration 0011); `UPDATE`, `DELETE`
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
  `the_audit_subsystem_schedules_the_chain_check`,
  `the_current_digest_covers_every_column`,
  `every_hashed_column_is_tamper_evident`,
  `records_under_the_original_digest_still_verify`;
- `app/ferroehr/tests/it/audit_store.rs`:
  `the_access_trail_refuses_rewriting_deletion_and_truncation`.

**Remains:** the chain gives evidence of tampering and does not prevent it.
A record written before the upgrade to 4.3.9 keeps its version 1 digest,
which leaves out the columns added after the chain was introduced (the
purpose, legal basis, categories, authentication fields and the emergency
mark among them); on those records the append-only trigger alone protects
those columns, and the table owner can disable it.
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

## H9: an emergency access to data restricted under EHDS Art. 8 is not marked

**Cause:** a natural person restricts the access of health professionals to
some of their data (EHDS Art. 8), and a professional reaches that data
anyway because the person's vital interests require it (Art. 11(5)). Without
a mark, the access record of that emergency looks like any other.

**Effect:** EHDS Art. 9(1) includes "access provided in accordance with
Article 11(5)" in what a patient is told about, and Art. 11(5) asks that such
cases be "logged in a clear and understandable format" and be "easily
accessible for the data subject". An unmarked emergency access hides that it
happened.

**Control:** a deployment names the purpose-of-use codes its callers declare
for an emergency (`[audit] emergency_purpose_codes`, each also on
`purpose_codes`). An access declaring one carries the emergency mark in its
access record: the `emergency_access` column, and in the FHIR `AuditEvent` the
`ehds-logging` entity `ehds-emergency-access`, which states the mark in plain
words and names the purposes that marked it.
The person's own subject-scoped ITI-81 retrieval serves that document. The
mark lifts no restriction: an object restricted under GDPR Art. 18 is refused
with an emergency purpose exactly as without one, and the refused attempt is
logged with the mark.

**Shown by:** `app/ferroehr-rest/tests/it/audit_emergency_access.rs`:
`an_emergency_access_is_marked_and_shown_in_the_subjects_own_access_log`,
`an_emergency_purpose_does_not_lift_a_gdpr_restriction`;
`app/ferroehr/tests/it/audit_ehds_mapping.rs`:
`art_11_5_the_emergency_mark_reaches_the_fhir_rendering_only`;
`app/ferroehr/tests/it/audit_store.rs`:
`both_inserts_persist_the_emergency_mark_and_it_cannot_be_rewritten`.

**Remains:** the Art. 8 restriction hides restricted data from every
professional route, query and export, and only a caller holding
`authz.rbac.emergency_access_role` that declares an emergency purpose reads it
(`app/ferroehr-rest/tests/it/access_restriction_http.rs`:
`only_an_emergency_access_reads_restricted_data_and_is_marked`). The mark rests
on the purpose the caller declares, and an emergency the caller does not
declare is not marked. The DICOM syslog rendering has no purpose element and
does not carry the mark.

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
| 2 | 2026-10-07 | H9 names the EHDS Art. 8 restriction; the emergency mark and the subject-scoped retrieval are its control |
| 3 | 2026-10-08 | Reviewed for release 4.3.6 (changes since 4.3.5: the assurance level and the natural person, the emergency mark, the FHIR connector removed beside the ITI-81 route); every named test exists and asserts its control; no hazard added |
| 4 | 2026-10-15 | Reviewed for release 4.3.9 (changes since 4.3.6: digest version 2 of the audit chain over every column, audit migration 0011 (#3686); one data-flow direction for the DICOM and FHIR renderings (#3687) and the BALP `Delete` agent types (#3750); the physical party delete classified and its party named (#3683); every audit-store and retention-register checkout through the typed database acquire (#3749)); every named test exists and asserts its control; H1, H2 and H7 name the new tests, H7 the version 1 records; no hazard added |
| 5 | 2026-10-20 | Reviewed for release 5.1.0 (changes since 5.0.0: the audit FHIR feed authenticates with an OAuth 2.0 client-credentials token, and a token endpoint that refuses or cannot be reached is a transport error, so the records stay pending and are delivered on recovery as H5 states (#3829); the assurance and professional rules the access record carries move under `[authz]` (#3872)); every named test exists and asserts its control; no hazard added |
| 6 | 2026-10-20 | Reviewed for release 5.2.0 (changes since 5.1.0: the EHDS Art. 8 restriction of access lands, so H9's control is the restriction with its emergency override and the mark on that access record (#3682); a read of a party's protected identifiers is recorded per scheme (#3776); the access context carries whether a request enforces the restriction); every named test exists and asserts its control; no hazard added |
| 7 | 2026-10-20 | Reviewed for the move onto FerroAUDIT (#3887): the access record, its categories and their classification, the retention floor and the emergency mark come from `ehds-logging` 0.1.1, the BALP and DICOM renderings and the ATNA sender from `ihe-atna` 0.1.1; the syslog feed sends over TLS only; H2, H3 and H9 name the moved tests; every named test exists and asserts its control; no hazard added |
| 8 | 2026-10-22 | Reviewed for release 5.3.2 (changes since 5.2.0: the move onto FerroAUDIT of review 7 (#3887), then `ehds-logging` and `ihe-atna` 0.2.4 (#3902), so the record carries the token's `jti`, the operation subtype, `outcomeDesc` and the optional DICOM elements again, a Basic user is recorded with no issuer, a retention without end or under three years is stated as such, and archetype ids are compared as openEHR BASE 1.3.0 asks with each id kept as the access named it (#3908); the ITI-81 search form-decodes its query string (#3891)); every named test exists and asserts its control; no hazard added |

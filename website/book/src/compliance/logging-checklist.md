<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The EHDS logging checklist

EHDS Art. 25 asks every EHR system for a European logging software component,
and Annex II point 3 sets what it records and offers (Regulation (EU)
2025/327, [EUR-Lex](https://eur-lex.europa.eu/eli/reg/2025/327/oj)). The
draft implementing act on the European digital testing environment (Art.
40(4), Have your say initiative 17513, 2026-11-19) checks the logging
component against assertions. The Xt-EHR Joint Action's deliverable D8.2,
"EHR Conformity Assessment Scheme: Assertion Document and Checklists"
(2026-05-19, [PDF](https://www.xt-ehr.eu/wp-content/uploads/2026/05/Xt-EHR-D8.2.pdf)),
§5.1.8 to §5.1.13, states those assertions and lists the items the automated
testing environment leaves to a checklist.

This page is FerroEHR's evidence for each item: the article it rests on, and
the test, configuration or document that shows FerroEHR's side of it, or the
gap. Test paths are relative to the repository root. Several items are met by
the EHR system FerroEHR forms with FerroSMART, which authenticates the
person, and FerroAUDIT, whose shared crates render the record and run the
Audit Record Repository; the row then names the component.

<!-- toc -->

> [!WARNING]
> D8.2 is a project deliverable and the testing-environment act a draft. A
> passing test shows what the software does; it does not show that the
> component meets Annex II, which no conformity assessment has examined.
> The [hazard log](hazard-log.md) states, per hazard, what remains.

## Assertions the testing environment checks

| Item | EHDS | Evidence | State |
|---|---|---|---|
| `log-UserAccess`: per access by a health professional or other user, the healthcare provider, the natural person, the categories of data, the time and date and the origin of the data | Annex II 3.2(a) to (e), Art. 9(1) | One access record per access, written before the response leaves; the five elements field by field on [the EHDS logging elements, mapped](../ehds-logging.md#the-ehds-logging-elements-mapped). Tests: `app/ferroehr/tests/it/audit_ehds_mapping.rs` (`element_a_the_accessing_organisation_is_recorded_and_rendered_in_fhir`, `element_b_the_person_who_accessed_is_recorded_and_rendered`, `element_c_the_priority_category_is_recorded_and_rendered_in_fhir_only`, `element_d_the_time_of_access_is_recorded_and_rendered`, `element_e_the_origin_of_the_data_is_recorded_and_rendered_in_fhir`); `app/ferroehr-rest/tests/it/audit_route_coverage.rs` (`every_route_writes_exactly_one_access_record`); `app/ferroehr-rest/tests/it/audit_e2e.rs` (`a_request_whose_record_cannot_be_written_is_refused`) | Evidenced |
| `log-APIAccess`: per access through the API, the accessing EHR system, the categories, the time and date and the origin | Annex II 3.2 | The client application is the record's `principal` in `client` mode and its own agent in the FHIR `AuditEvent`, beside the person it acts for. Tests: `app/ferroehr-rest/tests/it/caller_kinds_http.rs` (`a_system_callers_records_carry_its_client_and_provider`, `a_system_clients_write_and_read_are_recorded_under_one_client_identity`); `app/ferroehr/tests/it/audit_ehds_mapping.rs` (`element_b_the_professional_behind_a_client_is_recorded_and_rendered`); `app/ferroehr-rest/tests/it/exchanged_patient_token_http.rs` (`the_access_record_names_the_patient_the_client_and_the_level`); categories, time and origin by the tests of the row above | Evidenced |
| `log-UserID`: unambiguous electronic identification of each end user | Annex II 3.1, 3.2(b), Art. 16(1) | FerroSMART signs the person in at an eIDAS-recognised eID and carries the level of assurance in the token (FerroHEALTH/FerroSMART#784). FerroEHR checks the level on every request for patient data against `[authz.assurance]` and records it with the person ([assurance level and the natural person](../security.md#assurance-level-and-the-natural-person-on-patient-data)). Tests: `app/ferroehr-rest/tests/it/patient_data_assurance.rs` (`a_token_below_the_minimum_gets_the_step_up_challenge`, `a_delegate_acting_for_a_professional_passes_and_is_recorded`); `app/ferroehr/tests/it/audit_ehds_mapping.rs` (`the_assurance_level_is_recorded_and_reaches_the_fhir_rendering_as_a_level`) | Evidenced |
| `log-Review`: tools to review and analyse the log, or support for external ones | Annex II 3.3, Art. 9(2) | ITI-81 `GET /fhir/r4/AuditEvent` searches the local store by patient, agent, action and date, and a `read_subject_audit` grant reads one subject's records ([retrieving audit records](../audit-integrity.md#retrieving-audit-records-iti-81)); the access log is also served over ITS-REST and AQL ([reading the access log](../access-log.md#reading-the-access-log-over-its-rest)). The family's Audit Record Repository with the Retrieve Audit Message option shipped in FerroHEALTH/FerroAUDIT#278. Tests: `app/ferroehr-rest/tests/it/audit_iti81.rs` (`searchset_bundle_with_filters_and_paging`, `a_subject_audit_grant_reads_one_subjects_log_and_nothing_else`); `app/ferroehr-rest/tests/it/access_log_http.rs` (`a_read_of_the_log_writes_an_entry_naming_the_log`) | Evidenced |
| `log-interoperability` (should): a standard format and exchange for the log | Annex II 3.3 | Every record renders as an IHE BALP FHIR `AuditEvent` and as a DICOM PS3.15 audit message, sent over the ITI-20 syslog and FHIR feeds ([sinks](../audit.md#sinks)); the check of the BALP mapping against the D8.2 and D5.2 fields shipped in FerroHEALTH/FerroAUDIT#279. Tests: `app/ferroehr/src/system_log/fhir/tests.rs` (`dicom_and_fhir_agree_on_the_data_flow_direction`, `query_record_carries_the_origins_of_the_served_data`); `app/ferroehr/tests/it/audit_feed.rs` (`arr_outage_leaves_rows_pending_then_delivers_on_recovery`) | Evidenced |

## Checklist items outside automated testing

| Item | EHDS | Evidence | State |
|---|---|---|---|
| `log-Retention`: retention periods and access rights that consider the origins and categories of the data | Annex II 3.4, Art. 9(2) | The retention register keys a period on a priority category or an origin, and an access directive's data scope names categories and origins ([retention, restriction and objection](retention.md)). A period under the three years of Art. 9(2) is refused. Tests: `app/ferroehr/tests/it/legal_marks.rs` (`a_category_keyed_period_lists_its_objects_and_unclassified_takes_the_longest`, `an_origin_keyed_period_lists_the_objects_of_its_origin`); `app/ferroehr/tests/it/access_eval_steps/steps.rs` (`step_2_data_scope_attributes`); `app/ferroehr-rest/tests/it/access_evaluator_http.rs` (`a_category_restriction_with_an_exception`); `app/ferroehr-rest/tests/it/access_policy_http.rs` (`a_log_retention_that_breaks_a_bound_is_refused_and_one_that_holds_applies`) | Evidenced |
| `Gen-Installation`: supplied and installed per the manufacturer's instructions without affecting the system's characteristics and performance | Annex II 1.2 | The [installation](../installation/index.md) paths and the [go-live checklist](../security/go-live-checklist.md). The release pipeline installs the quickstart on fresh volumes and measures the logging component on it, and a claim not met refuses the release ([intended performance](logging-performance.md)); `scripts/deploy-probe.sh` checks each integration of a deployment at the far end | Evidenced |
| `Gen-Documentation`: detailed installation and configuration documentation for the operator | Annex II 1.2, Art. 37, Art. 38 | This book: [installation](../installation/index.md), the [configuration reference](../installation/configuration.md), [operations](../operations.md) and [decommissioning](../operations-decommissioning.md); the [instructions for use](https://ferrohealth.eu/docs/ehds/instructions-for-use.html) and the [information sheet](https://ferrohealth.eu/docs/ehds/information-sheet.html) in the FerroHEALTH book | Evidenced |
| `gen-performanceSafety`: in normal use, the component does not prevent the intended purpose, impact performance meaningfully or put patient safety at risk | Annex II 1.1 | The [hazard log](hazard-log.md) states each hazard with its control and test, and the [intended performance](logging-performance.md) states the latency, throughput and completeness claims that `scripts/perf/audit-verdict.sh` checks on every release candidate | Evidenced |
| `gen-noUndueBurden`: no feature that prohibits, restricts or burdens authorised access, sharing or use | Annex II 2.5 | The openEHR ITS-REST API in full, measured by the [conformance](../conformance.md) record of every release. The licence gates no feature, endpoint or limit (`app/ferroehr/src/licence/mod.rs`, [licensing](../licensing.md)). The Annex II 2.5 re-assessment is on the [claims review](claims-review.md) | Evidenced |
| `gen-SystemReplacement`: authorised export of the stored data for replacing the system, in reasonable time and without impediment | Annex II 2.6 | The whole repository leaves as an archive in canonical JSON or canonical XML, standalone parties included, and loads back ([dump and load](../storage-admin-apis.md#dump-and-load)); one EHR leaves as an openEHR EHR Extract ([exporting an EHR](../beyond-core/messaging.md#exporting-an-ehr)), and the capacity run measures the EHR export ([capacity](../capacity.md)). Tests: `app/ferroehr/tests/it/service_dump_load.rs` (`export_then_load_into_fresh_db_round_trips_byte_equal`, `the_canonical_xml_archive_holds_original_version_documents_under_the_published_root`) | Evidenced |

Each item names its evidence, and no item is a gap. The checklist is read
again with the hazard log, in the release that changes any module of the
logging component.

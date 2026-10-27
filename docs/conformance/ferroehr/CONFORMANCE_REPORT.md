# Conformance Report

SUT: FerroEHR 4.3.0 (sut `ferroehr`) · schedule cnf-2.0-w2 · ITS its-rest
Runner: veredictum 0.1.5 · verification pack: passed

## Summary

| Status | Count |
| --- | --- |
| passed | 1104 |
| failed | 0 |
| errored | 0 |
| skipped | 0 |
| not_applicable | 41 |
| total | 1145 |

## By chapter

Grouping is the published per-chapter chart's taxonomy: chapters with their bands; `cited n/a` counts the not-executed outcomes that carry a citation (`not_applicable` + `skipped`).

| Chapter / band | passed | failed | errored | cited n/a |
| --- | --- | --- | --- | --- |
| **EHR** | 488 | 0 | 0 | 18 |
| — EHR resource | 25 | 0 | 0 | 2 |
| — EHR_STATUS | 50 | 0 | 0 | 5 |
| — COMPOSITION | 145 | 0 | 0 | 0 |
| — DIRECTORY | 84 | 0 | 0 | 6 |
| — CONTRIBUTION | 116 | 0 | 0 | 5 |
| — Item tags | 62 | 0 | 0 | 0 |
| — Revision history | 6 | 0 | 0 | 0 |
| **Definitions** | 103 | 0 | 0 | 10 |
| — ADL 1.4 templates | 39 | 0 | 0 | 5 |
| — ADL 2 artefacts | 36 | 0 | 0 | 2 |
| — Stored queries | 28 | 0 | 0 | 3 |
| **Query** | 75 | 0 | 0 | 2 |
| — Ad-hoc AQL | 64 | 0 | 0 | 2 |
| — Stored query execution | 11 | 0 | 0 | 0 |
| **Demographic** | 111 | 0 | 0 | 4 |
| — Parties | 86 | 0 | 0 | 3 |
| — Party relationships | 19 | 0 | 0 | 0 |
| — Versioned party | 6 | 0 | 0 | 1 |
| **Messaging** | 50 | 0 | 0 | 0 |
| — EHR Extract | 34 | 0 | 0 | 0 |
| — TDD | 16 | 0 | 0 | 0 |
| **Admin** | 54 | 0 | 0 | 2 |
| — Admin service | 24 | 0 | 0 | 2 |
| — Archive | 15 | 0 | 0 | 0 |
| — Dump & load | 15 | 0 | 0 | 0 |
| **System** | 1 | 0 | 0 | 0 |
| — Conformance manifest | 1 | 0 | 0 | 0 |
| **Content validation** | 133 | 0 | 0 | 1 |
| — Data types | 53 | 0 | 0 | 0 |
| — Interval data types | 30 | 0 | 0 | 0 |
| — Structure & cardinality | 50 | 0 | 0 | 1 |
| **Simplified formats** | 69 | 0 | 0 | 2 |
| — FLAT & STRUCTURED | 22 | 0 | 0 | 0 |
| — Web Template | 5 | 0 | 0 | 0 |
| — Path mapping | 31 | 0 | 0 | 0 |
| — Scope & legacy media | 11 | 0 | 0 | 2 |
| **Security & privacy** | 6 | 0 | 0 | 0 |
| — Authenticated access | 2 | 0 | 0 | 0 |
| — Authorization separation | 1 | 0 | 0 | 0 |
| — Audit accountability | 1 | 0 | 0 | 0 |
| — Anonymous EHRs | 1 | 0 | 0 | 0 |
| — EHR/demographic separation | 1 | 0 | 0 | 0 |
| **Signing** | 11 | 0 | 0 | 2 |
| — Version signing | 11 | 0 | 0 | 2 |
| **SMART App Launch** | 3 | 0 | 0 | 0 |
| — Discovery | 1 | 0 | 0 | 0 |
| — Resource scopes | 2 | 0 | 0 | 0 |
| **Performance** | 0 | 0 | 0 | 0 |

## By capability

Selected gating cases per claimed capability. `inconclusive` counts cases whose exchange did not conclude (transport fault, unmapped status, step resolution) — they block a `passed` evidence token and are triaged, never absorbed.

| Capability | Evidence | passed | failed | inconclusive | unevidenced |
| --- | --- | --- | --- | --- | --- |
| Adl14ArchetypeProvisioning | pass | 13 | 0 | 0 | 0 |
| Adl14OptProvisioning | pass | 23 | 0 | 0 | 5 |
| Adl2ArchetypeProvisioning | pass | 10 | 0 | 0 | 0 |
| Adl2OptProvisioning | pass | 32 | 0 | 0 | 2 |
| TemplateExamples | pass | 3 | 0 | 0 | 0 |
| QueryProvisioning | pass | 27 | 0 | 0 | 0 |
| EhrOperations | pass | 22 | 0 | 0 | 2 |
| EhrStatus | pass | 48 | 0 | 0 | 5 |
| CompositionOps | pass | 63 | 0 | 0 | 0 |
| DirectoryOps | pass | 88 | 0 | 0 | 8 |
| ChangeSets | pass | 108 | 0 | 0 | 5 |
| Versioning | pass | 75 | 0 | 0 | 0 |
| ArchetypeValidation | pass | 134 | 0 | 0 | 2 |
| PartyOperations | pass | 89 | 0 | 0 | 4 |
| PartyRelationshipOperations | pass | 19 | 0 | 0 | 0 |
| DemographicArchetypeValidation | pass | 11 | 0 | 0 | 0 |
| AqlBasic | pass | 50 | 0 | 0 | 2 |
| AqlAdvanced | pass | 4 | 0 | 0 | 0 |
| AqlTerminology | pass | 6 | 0 | 0 | 0 |
| ActivityReport | pass | 15 | 0 | 0 | 0 |
| PhysicalDeletion | pass | 9 | 0 | 0 | 2 |
| EhrDumpLoad | pass | 15 | 0 | 0 | 0 |
| BulkEhrLoad | pass | 2 | 0 | 0 | 0 |
| EhrArchive | pass | 7 | 0 | 0 | 0 |
| DemographicArchive | pass | 8 | 0 | 0 | 0 |
| EhrExtract | pass | 34 | 0 | 0 | 0 |
| DefinitionApi | pass | 1 | 0 | 0 | 0 |
| EhrApi | pass | 2 | 0 | 0 | 0 |
| DemographicApi | pass | 64 | 0 | 0 | 1 |
| QueryApi | pass | 22 | 0 | 0 | 0 |
| AdminApi | pass | 8 | 0 | 0 | 0 |
| MessageApi | pass | 4 | 0 | 0 | 0 |
| SystemApi | pass | 1 | 0 | 0 | 0 |
| ItemTags | pass | 66 | 0 | 0 | 0 |
| Signing | pass | 11 | 0 | 0 | 2 |
| SimplifiedFormats | pass | 73 | 0 | 0 | 2 |
| SmartAppLaunch | pass | 3 | 0 | 0 | 0 |
| EhrDemographicSeparation | pass | 1 | 0 | 0 | 0 |
| AuthenticatedAccess | pass | 2 | 0 | 0 | 0 |
| AuthorizationSeparation | pass | 1 | 0 | 0 | 0 |
| AuditAccountability | pass | 1 | 0 | 0 | 0 |
| AnonymousEhrs | pass | 1 | 0 | 0 | 0 |

## Performance measurements

### PERF-hospital_sim-class_POC — class POC · EARNED

Offered load sustained: 2.04/s over 3600 s (after 300 s warmup) · environment: consumer-laptop (8 cores, 16 GB, nvme, single-node docker compose (8-CPU/8GB Docker VM) on Apple M2, the SMART resource-server posture (docker/sut-smart.yml overlays the base stack) with the external-terminology profile composed beside it (a seeded HAPI FHIR R4 server, docker compose --profile terminology + docker/sut-terminology.yml, fail-open); alongside it a second deployment of the same image in the openPGP version-signing posture (project ferroehr-cnf-pgp, docker/sut-signing-pgp.yml + docker/sut-terminology-failclosed.yml + docker/sut-pgp-parallel.yml, host port 8081) declared as the sut_pgp instance, which carries the fail-closed terminology posture — the measured-performance stage drives the primary deployment alone)

| Operation | Requests | Errors | p50 (ms) | p90 (ms) | p99 (ms) |
| --- | --- | --- | --- | --- | --- |
| adhoc_query | 792 | 0 | 29.1 | 37.6 | 63.6 |
| admin_contribution_report | 60 | 0 | 158.6 | 176.3 | 268.8 |
| analytics_query | 23 | 0 | 61.6 | 75.6 | 196.6 |
| archetype_adl2_list | 60 | 0 | 12.6 | 17.7 | 28.4 |
| composition_commit | 280 | 0 | 50.3 | 106.2 | 213.6 |
| composition_commit_flat | 7 | 0 | 41.7 | 52.5 | 52.5 |
| composition_delete | 4 | 0 | 27.7 | 35.2 | 35.2 |
| composition_read | 1583 | 0 | 24.2 | 33.2 | 65.8 |
| composition_read_current | 896 | 0 | 26.5 | 35.9 | 57.3 |
| composition_read_flat | 7 | 0 | 16.7 | 24.6 | 24.6 |
| composition_revision_history | 889 | 0 | 14.0 | 20.6 | 31.2 |
| composition_update | 68 | 0 | 46.3 | 67.3 | 239.2 |
| composition_version_read | 30 | 0 | 28.1 | 35.1 | 48.7 |
| contribution_commit | 48 | 0 | 53.8 | 138.0 | 187.1 |
| contribution_read | 82 | 0 | 19.6 | 28.0 | 59.8 |
| directory_create | 12 | 0 | 25.5 | 39.6 | 86.5 |
| directory_read | 803 | 0 | 17.0 | 24.1 | 40.0 |
| directory_update | 12 | 0 | 28.5 | 34.1 | 41.9 |
| ehr_create | 12 | 0 | 28.0 | 32.3 | 33.2 |
| ehr_extract_export | 792 | 0 | 148.6 | 181.6 | 267.5 |
| ehr_read | 72 | 0 | 17.5 | 25.1 | 36.5 |
| ehr_status_read | 24 | 0 | 16.0 | 23.1 | 25.1 |
| ehr_status_update | 24 | 0 | 21.2 | 36.1 | 55.0 |
| party_create | 6 | 0 | 21.5 | 28.0 | 28.0 |
| party_read | 6 | 0 | 15.9 | 24.2 | 24.2 |
| party_relationship_create | 6 | 0 | 25.5 | 30.0 | 30.0 |
| party_relationship_read | 6 | 0 | 10.8 | 17.0 | 17.0 |
| party_update | 6 | 0 | 15.3 | 26.8 | 26.8 |
| readonly_write_denied | 7 | 0 | 29.9 | 45.6 | 45.6 |
| smart_configuration_read | 7 | 0 | 14.9 | 24.0 | 24.0 |
| stored_query_execute | 180 | 0 | 41.0 | 55.4 | 102.7 |
| system_options | 7 | 0 | 10.4 | 17.6 | 17.6 |
| tags_put | 30 | 0 | 23.5 | 32.7 | 40.6 |
| tags_read | 30 | 0 | 17.4 | 26.0 | 38.1 |
| tdd_import | 7 | 0 | 34.8 | 60.3 | 60.3 |
| template_adl2_list | 61 | 0 | 13.1 | 18.6 | 54.1 |
| template_example | 60 | 0 | 29.7 | 101.9 | 261.0 |
| template_get | 60 | 0 | 64.2 | 134.4 | 239.6 |
| template_list | 60 | 0 | 53.1 | 80.4 | 90.3 |
| terminology_query | 23 | 0 | 19.0 | 25.9 | 72.8 |
| unauthenticated_probe | 7 | 0 | 17.9 | 22.8 | 22.8 |
| ward_query | 180 | 0 | 27.7 | 37.6 | 71.4 |

Percentiles re-derive from the embedded HDR V2 histograms; the class verdict is recomputed from them by the verdict pipeline, never trusted from this table.

## Honesty

Coverage: 1087 of 1129 selected cases driven.

Not-executed verdicts (each cited):

| Case | Citation |
| --- | --- |
| CONT-ITEM-unmatched_member_refused | option unmatched-member-refused: the ICS declares unmatched-member-accepted for register AMB-229's `unmatched-member` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_ADMIN_SERVICE.physical_party_delete-delete_existing | I_ADMIN_SERVICE.physical_party_delete: AMB-33 |
| I_ADMIN_SERVICE.physical_party_delete-delete_non_existing | I_ADMIN_SERVICE.physical_party_delete: AMB-33 |
| I_DEFINITION_ADL14.delete_opt-delete_existing | I_DEFINITION_ADL14.delete_opt: AMB-17 |
| I_DEFINITION_ADL14.delete_opt-delete_latest_version | I_DEFINITION_ADL14.delete_opt: AMB-17 |
| I_DEFINITION_ADL14.delete_opt-delete_non_existing | I_DEFINITION_ADL14.delete_opt: AMB-17 |
| I_DEFINITION_ADL14.delete_opt-delete_specific_version | I_DEFINITION_ADL14.delete_opt: AMB-17 |
| I_DEFINITION_ADL14.get_opt-retrieve_latest_version | option adl14-partial-id-latest: the ICS declares adl14-partial-id-exact for register AMB-104's `adl14-partial-id` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_DEFINITION_ADL2.opts_count-unrealized | I_DEFINITION_ADL2.opts_count: AMB-37 |
| I_DEFINITION_ADL2.templates_count-unrealized | I_DEFINITION_ADL2.templates_count: AMB-37 |
| I_DEFINITION_QUERY.delete_query-delete_existing | I_DEFINITION_QUERY.delete_query: AMB-127 |
| I_DEFINITION_QUERY.list_matching_queries-id_pattern | I_DEFINITION_QUERY.list_matching_queries: AMB-121 |
| I_DEFINITION_QUERY.queries_count-count | I_DEFINITION_QUERY.queries_count: AMB-127 |
| I_DEMOGRAPHIC_SERVICE.create_party-xml | option party-xml-write-refused: the ICS declares party-xml-write-accepted for register AMB-167's `party-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_DEMOGRAPHIC_SERVICE.get_party-xml_not_acceptable | option party-xml-unsupported: the ICS declares party-xml-supported for register AMB-167's `party-xml` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_DEMOGRAPHIC_SERVICE.update_party-xml | option party-xml-write-refused: the ICS declares party-xml-write-accepted for register AMB-167's `party-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_CONTRIBUTION.list_contributions-ehr_containing_directory | I_EHR_CONTRIBUTION.list_contributions: AMB-22 |
| I_EHR_CONTRIBUTION.list_contributions-ehr_containing_ehr_status | I_EHR_CONTRIBUTION.list_contributions: AMB-22 |
| I_EHR_CONTRIBUTION.list_contributions-empty | I_EHR_CONTRIBUTION.list_contributions: AMB-22 |
| I_EHR_CONTRIBUTION.list_contributions-non_existing_ehr | I_EHR_CONTRIBUTION.list_contributions: AMB-22 |
| I_EHR_CONTRIBUTION.list_contributions-post_commit | I_EHR_CONTRIBUTION.list_contributions: AMB-22 |
| I_EHR_DIRECTORY.create_directory-xml | option directory-xml-write-refused: the ICS declares directory-xml-write-accepted for register AMB-167's `directory-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_DIRECTORY.get_directory-xml_not_acceptable | option directory-xml-unsupported: the ICS declares directory-xml-supported for register AMB-167's `directory-xml` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_DIRECTORY.get_versioned_directory-bad_ehr | I_EHR_DIRECTORY.get_versioned_directory: AMB-24 |
| I_EHR_DIRECTORY.get_versioned_directory-directory_with_two_versions | I_EHR_DIRECTORY.get_versioned_directory: AMB-24 |
| I_EHR_DIRECTORY.get_versioned_directory-empty_ehr | I_EHR_DIRECTORY.get_versioned_directory: AMB-24 |
| I_EHR_DIRECTORY.update_directory-xml | option directory-xml-write-refused: the ICS declares directory-xml-write-accepted for register AMB-167's `directory-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_SERVICE.create_ehr-xml | option ehr-xml-write-refused: the ICS declares ehr-xml-write-accepted for register AMB-167's `ehr-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_SERVICE.get_ehr-xml_not_acceptable | option ehr-xml-unsupported: the ICS declares ehr-xml-supported for register AMB-167's `ehr-xml` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_STATUS.clear_ehr_modifiable-xml_body | option ehr-status-xml-write-refused: the ICS declares ehr-status-xml-write-accepted for register AMB-167's `ehr-status-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_STATUS.clear_ehr_queryable-xml_body | option ehr-status-xml-write-refused: the ICS declares ehr-status-xml-write-accepted for register AMB-167's `ehr-status-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_STATUS.get_ehr_status-xml_not_acceptable | option ehr-status-xml-unsupported: the ICS declares ehr-status-xml-supported for register AMB-167's `ehr-status-xml` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_STATUS.set_ehr_modifiable-xml_body | option ehr-status-xml-write-refused: the ICS declares ehr-status-xml-write-accepted for register AMB-167's `ehr-status-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_EHR_STATUS.set_ehr_queryable-xml_body | option ehr-status-xml-write-refused: the ICS declares ehr-status-xml-write-accepted for register AMB-167's `ehr-status-xml-write` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_ITS_REST_VERSIONED_PARTY.versioned_party_get-xml | option versioned-party-xml-supported: the ICS declares versioned-party-xml-unsupported for register AMB-167's `versioned-party-xml` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_QUERY_SERVICE.execute_ad_hoc_query-aggregates_non_numeric_executed | option aql-nonnumeric-aggregate-executed: the ICS declares aql-nonnumeric-aggregate-refused for register AMB-227's `aql-nonnumeric-aggregate` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| I_QUERY_SERVICE.execute_ad_hoc_query-undefined_containment_empty | option undefined-containment-empty: the ICS declares undefined-containment-refused for register AMB-220's `undefined-containment` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| SF-DEPRECATED-media_supported | option sf-deprecated-types-supported: the ICS declares sf-deprecated-types-unsupported for register AMB-39's `sf-deprecated-types` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| SF-LEGACY-nc_flat_media_supported | option legacy-alt-formats-supported: the ICS declares legacy-alt-formats-unsupported for register AMB-61's `legacy-alt-formats` option family (statement.options), and the arms are mutually exclusive — ISO/IEC 9646 test selection |
| SIG-VERSION-directory_signature_present | I_EHR_DIRECTORY.get_versioned_directory: AMB-24 |
| SIG-VERSION-directory_verifiable | I_EHR_DIRECTORY.get_versioned_directory: AMB-24 |

# openEHR CDR conformance comparison (generated)

> **Measured, not asserted.** Every cell below is derived from the two
> committed `results.json`/`verdicts.json` sets produced by the CNF 2.0
> instrument (`veredictum`) executing the SAME committed
> catalogue against each system, each with its own committed party statement
> (`docs/conformance/party/<sut>/`). Nothing here is hand-entered
> (`scripts/render/comparison.sh`; CI: `scripts/checks/conformance-numbers.sh`).
>
> - A capability a party's statement does not claim reads **not claimed** and
>   never gates its verdicts; a ground unrealizable on a party's topology or
>   technology profile reads **N/A with a machine citation**, never fail.
> - This comparison makes **no certification claim on behalf of any other
>   vendor** -- each column is computed from that SUT's own run.
> - Where the comparison SUT out-performs ferroehr, its cell reads pass
>   while ours reads fail -- stated plainly, not hidden.

## Systems under test

| | ferroehr | EHRbase |
|---|---|---|
| Product | ferroehr 4.3.0 | ehrbase 2.34.0 |
| Run date | 2026-09-15 | 2026-08-14 |
| Party statement | committed with the runner (declares ITS-REST pin, signing, terminology posture) | committed with the runner |
| Stack | the project's own compose stack, built from the current sources | a dedicated compose stack of the official EHRbase images |

## Methodology

Both systems execute the **same committed CNF 2.0 catalogue** (1145 case-by-format
executions) through the same instrument (`veredictum`), each on
fresh volumes with its own committed party set: the ixit names the reachable
instances (EHRbase declares no readonly principal), and the statement (the
ICS) declares the claimed capabilities, spec versions, and ambiguity-register
options — ISO/IEC 9646-style test selection excuses undeclared option
branches, unclaimed capabilities, and release-dated behaviour outside the
declared versions as N/A with a citation, never as silent skips. Verdicts are
pure functions of (statement, results, catalogue, capability matrix).

**The declared-version delta matters and is stated, not hidden:** ferroehr
declares ITS-REST **1.1.0**
while EHRbase declares ITS-REST
**1.0.3** —
the catalogue realizes 1.1.0, so every Release-1.1.0-dated behaviour (the
Demographic API, ITEM_TAGs, Simplified Formats on the wire, the admin EHR
delete, the weak-`ETag`/`Location` header forms, …) is cited N/A for the
1.0.3 declaration rather than driven against a release EHRbase never
claimed. The verdict-bearing comparison below is therefore each party's
**in-scope subset**, never the raw record.

## Profile verdicts

| Profile | ferroehr | EHRbase |
|---|---|---|
| CORE | pass | fail |
| STANDARD | pass | fail |
| OPTIONS | pass | not claimed |
| SEC-BASIC | pass | not claimed |

## In-scope outcomes

Runs compared: **ferroehr** (run of 2026-09-15) vs **EHRbase
2.34.0** (run of 2026-08-14) — the SAME catalogue through the same
runner, each with its own committed party statement. Per the presentation
rule, the headline is each party's VERDICT SCOPE (the cases its own
declarations select), never the raw record: a raw count would book
release-dated and unclaimed surfaces against a party that never claimed
them.

| | verdict scope (selected) | driven | in-scope passed | in-scope failed | in-scope inconclusive |
|---|---|---|---|---|---|
| **ferroehr** | 1129 | 1087 | 1087 | 0 | 0 |
| **EHRbase** | 692 | 571 | 148 | 148 | 275 |

An **inconclusive** row's wire answered outside the operation's bound outcome
map, or its required ground could not be established (e.g. a refused
provisioning exchange) — never counted as a failure of the behaviour under
test. Every not-run row in the full committed record
(`docs/conformance/<sut>/results.json`) carries a machine-readable
citation: an undeclared option branch, an unclaimed capability, a
release-dated behaviour outside the declared spec versions, or a ground the
party's topology cannot establish.

## Capability-by-capability

Evidence tokens from each party's computed verdicts: **passed** (every
gating case green), **failed** (at least one gating case red),
**inconclusive** (a gating case neither passed nor failed cleanly),
**not_evidenced** (claimed, but no gating case produced a verdict — there
is no excused state: a required capability without passing evidence fails
its tier, whichever party claims it), or **not_claimed** (absent from that
party's ICS, so no case naming it was ever selected — it can never count
for or against that party). The whole capability matrix is listed for both
columns, because the matrix is the profiles book as data, not a claim list.

| Capability | ferroehr | EHRbase |
|---|---|---|
| ActivityReport | passed | not_claimed |
| Adl14ArchetypeProvisioning | passed | failed |
| Adl14OptProvisioning | passed | failed |
| Adl2ArchetypeProvisioning | passed | not_claimed |
| Adl2OptProvisioning | passed | not_claimed |
| AdminApi | passed | not_evidenced |
| AnonymousEhrs | passed | not_claimed |
| AqlAdvanced | passed | inconclusive |
| AqlBasic | passed | failed |
| AqlTerminology | passed | not_claimed |
| ArchetypeValidation | passed | failed |
| AuditAccountability | passed | not_claimed |
| AuthenticatedAccess | passed | passed |
| AuthorizationSeparation | passed | not_evidenced |
| BulkEhrLoad | passed | not_claimed |
| ChangeSets | passed | failed |
| CompositionOps | passed | inconclusive |
| DefinitionApi | passed | failed |
| DemographicApi | passed | not_claimed |
| DemographicArchetypeValidation | passed | not_claimed |
| DemographicArchive | passed | not_claimed |
| DirectoryOps | passed | failed |
| EhrApi | passed | failed |
| EhrArchive | passed | not_claimed |
| EhrDemographicSeparation | passed | passed |
| EhrDumpLoad | passed | not_claimed |
| EhrExtract | passed | not_claimed |
| EhrOperations | passed | failed |
| EhrStatus | passed | failed |
| ItemTags | passed | not_claimed |
| MessageApi | passed | not_claimed |
| PartyOperations | passed | not_claimed |
| PartyRelationshipOperations | passed | not_claimed |
| PhysicalDeletion | passed | not_evidenced |
| QueryApi | passed | failed |
| QueryProvisioning | passed | failed |
| Signing | passed | not_claimed |
| SimplifiedFormats | passed | not_claimed |
| SmartAppLaunch | passed | not_claimed |
| SystemApi | passed | not_claimed |
| Tds | not_claimed | not_claimed |
| TemplateExamples | passed | not_evidenced |
| Versioning | passed | failed |

## Failures — both directions

### ferroehr failures (with the EHRbase outcome on the identical case)

| Case | Format | Failure | EHRbase outcome |
|---|---|---|---|
| — | — | *none — zero failing cases* | — |

### EHRbase failures by schedule chapter

| Chapter | failed cases |
|---|---|
| CONT | 67 |
| I_EHR_STATUS | 27 |
| I_EHR_CONTRIBUTION | 13 |
| I_EHR_DIRECTORY | 13 |
| I_DEFINITION_QUERY | 10 |
| I_DEFINITION_ADL14 | 8 |
| I_EHR_SERVICE | 4 |
| I_QUERY_SERVICE | 4 |
| I_EHR_COMPOSITION | 1 |
| I_ITS_REST_REVISION_HISTORY | 1 |

<details><summary>Every EHRbase-failed case, with the ferroehr outcome on the identical case</summary>

| Case | Format | EHRbase failure | ferroehr outcome |
|---|---|---|---|
| CONT-COMP-content_card_1plus-context_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_1plus-context_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_3plus-context_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_3plus-context_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_3to5-context_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_3to5-context_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_any-context_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_any-context_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_mand-context_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_mand-context_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_opt-context_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMP-content_card_opt-context_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMPOSITION-content_cardinality_count6 | — | expected `created`, observed `validation_failed` | passed |
| CONT-COMPOSITION-context_existence | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_CODED_TEXT-validate_open | — | expected `bad_request`, observed `validation_failed` | passed |
| CONT-DV_DATE-validate_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_DATE-validate_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_DATE_TIME-validate_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_DATE_TIME-validate_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_DURATION-validate_fields | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_DURATION-validate_fields_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_DURATION-validate_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_IDENTIFIER-validate_all_list | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_IDENTIFIER-validate_all_pattern | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_DATE-validate_lower_upper_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_DATE-validate_lower_upper_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_DATE_TIME-validate_lower_upper_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_DATE_TIME-validate_lower_upper_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_DURATION-validate_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_DURATION-validate_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_ORDINAL-validate_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_PROPORTION-validate_ratio_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_SCALE-validate_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_TIME-validate_lower_upper_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_INTERVAL_DV_TIME-validate_lower_upper_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_MULTIMEDIA-validate_media_type | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_PARSABLE-validate_value_formalism | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_TEXT-validate_open | — | expected `bad_request`, observed `validation_failed` | passed |
| CONT-DV_TIME-validate_constraint | — | expected `created`, observed `validation_failed` | passed |
| CONT-DV_TIME-validate_range | — | expected `created`, observed `validation_failed` | passed |
| CONT-EVENT-state_ex_mand | — | expected `bad_request`, observed `validation_failed` | passed |
| CONT-EVENT-state_ex_opt | — | expected `bad_request`, observed `validation_failed` | passed |
| CONT-EVENT-type_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-EVENT-type_interval_event | — | expected `created`, observed `validation_failed` | passed |
| CONT-EVENT-type_point_event | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_1plus-summary_ex_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_1plus-summary_ex_opt | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_3plus-summary_ex_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_3plus-summary_ex_opt | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_3to5-summary_ex_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_3to5-summary_ex_opt | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_any-summary_ex_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_any-summary_ex_opt | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_mand-summary_ex_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_mand-summary_ex_opt | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_opt-summary_ex_mand | — | expected `created`, observed `validation_failed` | passed |
| CONT-HIST-events_card_opt-summary_ex_opt | — | expected `created`, observed `validation_failed` | passed |
| CONT-HISTORY-events_cardinality_count6 | — | expected `created`, observed `validation_failed` | passed |
| CONT-ITEM_STR-type_any | — | expected `created`, observed `validation_failed` | passed |
| CONT-ITEM_STR-type_item_list | — | expected `created`, observed `validation_failed` | passed |
| CONT-ITEM_STR-type_item_single | — | expected `created`, observed `validation_failed` | passed |
| CONT-ITEM_STR-type_item_table | — | expected `created`, observed `validation_failed` | passed |
| CONT-ITEM_STR-type_item_tree | — | expected `created`, observed `validation_failed` | passed |
| CONT-OBS-state_ex_mand-protocol_ex_mand | — | expected `bad_request`, observed `validation_failed` | passed |
| CONT-OBS-state_ex_mand-protocol_ex_opt | — | expected `bad_request`, observed `validation_failed` | passed |
| CONT-OBS-state_ex_opt-protocol_ex_mand | — | expected `bad_request`, observed `validation_failed` | passed |
| CONT-OBS-state_ex_opt-protocol_ex_opt | — | expected `bad_request`, observed `validation_failed` | passed |
| I_DEFINITION_ADL14.delete_archetype-clinical_forbidden | — | expected `forbidden`, observed `not_found` | passed |
| I_DEFINITION_ADL14.upload_opt-invalid_opt | — | expected `validation_failed`, observed `not_acceptable` | passed |
| I_DEFINITION_ADL14.upload_opt-largest_published | — | expected `created`, observed `not_acceptable` | passed |
| I_DEFINITION_ADL14.upload_opt-valid_opt | — | expected `created`, observed `not_acceptable` | passed |
| I_DEFINITION_ADL14.upload_opt-valid_opt_twice_conflict | — | expected `created`, observed `not_acceptable` | passed |
| I_DEFINITION_ADL14.upload_opt-valid_opt_twice_no_conflict | — | expected `created`, observed `not_acceptable` | passed |
| I_DEFINITION_ADL14.validate_opt-invalid_opt | — | expected `validation_failed`, observed `not_acceptable` | passed |
| I_DEFINITION_ADL14.validate_opt-valid_opt | — | expected `created`, observed `not_acceptable` | passed |
| I_DEFINITION_QUERY.list_queries-prefix_all_versions | — | [0]/name: path resolves to nothing | passed |
| I_DEFINITION_QUERY.list_queries-version_get_xml_not_acceptable | — | expected `not_acceptable`, observed `ok` | passed |
| I_DEFINITION_QUERY.list_queries-xml_not_acceptable | — | expected `not_acceptable`, observed `ok` | passed |
| I_DEFINITION_QUERY.store_query-default_slot_with_higher_version | — | header Location: value "http://localhost:8091/ehrbase/rest/openehr/v1/definition/query/org | passed |
| I_DEFINITION_QUERY.store_query-dotted_name | — | expected `stored`, observed `bad_request` | passed |
| I_DEFINITION_QUERY.store_query-unqualified_name | — | expected `stored`, observed `bad_request` | passed |
| I_DEFINITION_QUERY.store_query-update_in_place | — | header Location: value "http://localhost:8091/ehrbase/rest/openehr/v1/definition/query/org | passed |
| I_DEFINITION_QUERY.store_query-version_duplicate_case_variant_name | — | expected `conflict`, observed `stored` | passed |
| I_DEFINITION_QUERY.store_query-version_prefix_rejected | — | expected `bad_request`, observed `stored` | passed |
| I_DEFINITION_QUERY.store_query-version_prerelease_rejected | — | expected `bad_request`, observed `stored` | passed |
| I_EHR_COMPOSITION.get_versioned_composition-malformed_uid | — | expected `bad_request`, observed `not_found` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-delete_directory | — | expected `created`, observed `not_found` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-deleted_member_with_data | — | expected `validation_failed`, observed `created` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-ehr_status_incomplete_lifecycle | — | expected `validation_failed`, observed `created` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-ehr_status_invalid_change_type | — | expected `conflict`, observed `validation_failed` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-ehr_status_invalid_change_type_deleted | — | expected `conflict`, observed `not_found` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-ehr_status_valid_combinations | — | header Last-Modified: expected present, got none | passed |
| I_EHR_CONTRIBUTION.commit_contribution-fail_modify_non_existing_directory | — | expected `validation_failed`, observed `precondition_failed` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-full_ehr_status | — | header Last-Modified: expected present, got none | passed |
| I_EHR_CONTRIBUTION.commit_contribution-minimal_ehr_status | — | header Last-Modified: expected present, got none | passed |
| I_EHR_CONTRIBUTION.commit_contribution-non_exiting_opt | — | expected `template_not_found`, observed `validation_failed` | passed |
| I_EHR_CONTRIBUTION.commit_contribution-update_existing_directory | — | header Last-Modified: expected present, got none | passed |
| I_EHR_CONTRIBUTION.commit_contribution-valid_directory | — | header Last-Modified: expected present, got none | passed |
| I_EHR_CONTRIBUTION.get_contribution-version_ref_shape | — | header Last-Modified: expected present, got none | passed |
| I_EHR_DIRECTORY.create_directory-ehr_not_modifiable | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_DIRECTORY.create_directory-root_archetype_id_mismatch | — | expected `validation_failed`, observed `created` | passed |
| I_EHR_DIRECTORY.create_directory-versioned_id_items | — | expected `created`, observed `bad_request` | passed |
| I_EHR_DIRECTORY.create_directory-wide_id_items | — | expected `created`, observed `bad_request` | passed |
| I_EHR_DIRECTORY.delete_directory-ehr_with_directory | — | header Last-Modified: expected present, got none | passed |
| I_EHR_DIRECTORY.delete_directory-empty_ehr | — | expected `not_found`, observed `precondition_failed` | passed |
| I_EHR_DIRECTORY.delete_directory-etag_names_new_version | — | header Last-Modified: expected present, got none | passed |
| I_EHR_DIRECTORY.get_directory-deleted_head | — | header Last-Modified: expected present, got none | passed |
| I_EHR_DIRECTORY.get_directory_at_time-deleted_at_time | — | header Last-Modified: expected present, got none | passed |
| I_EHR_DIRECTORY.get_directory_at_version-deleted_version | — | header Last-Modified: expected present, got none | passed |
| I_EHR_DIRECTORY.update_directory-empty_ehr | — | expected `not_found`, observed `precondition_failed` | passed |
| I_EHR_DIRECTORY.update_directory-root_archetype_id_mismatch | — | expected `validation_failed`, observed `updated` | passed |
| I_EHR_DIRECTORY.update_directory-stale_if_match | — | header ETag: expected the latest version uid, got none | passed |
| I_EHR_SERVICE.create_ehr-committal_headers | — | commit_audit/description/value: path resolves to nothing | passed |
| I_EHR_SERVICE.create_ehr-invalid_status | — | expected `validation_failed`, observed `created` | passed |
| I_EHR_SERVICE.create_ehr-wrong_method | — | header Allow: expected a value matching ".*(GET.*POST\\|POST.*GET).*", got none | passed |
| I_EHR_SERVICE.get_ehr-malformed_ehr_id | — | expected `bad_request`, observed `not_found` | passed |
| I_EHR_STATUS.clear_ehr_modifiable-bad_ehr | — | expected `not_found`, observed `precondition_missing` | passed |
| I_EHR_STATUS.clear_ehr_modifiable-existing_ehr | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.clear_ehr_modifiable-invalid_body | — | expected `validation_failed`, observed `bad_request` | passed |
| I_EHR_STATUS.clear_ehr_modifiable-stale_if_match | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.clear_ehr_queryable-bad_ehr | — | expected `not_found`, observed `precondition_missing` | passed |
| I_EHR_STATUS.clear_ehr_queryable-existing_ehr | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.clear_ehr_queryable-invalid_body | — | expected `validation_failed`, observed `bad_request` | passed |
| I_EHR_STATUS.clear_ehr_queryable-stale_if_match | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.get_ehr_status-at_time_future | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.get_ehr_status-at_time_omitted | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.get_ehr_status_at_version-addressed_version | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.get_versioned_ehr_status-at_time_future | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.get_versioned_ehr_status-at_time_omitted | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.get_versioned_ehr_status-contained_uid_form | — | header Last-Modified: expected present, got none | passed |
| I_EHR_STATUS.get_versioned_ehr_status-container_shape | — | owner_id/type: "ehr" != expected "EHR" | passed |
| I_EHR_STATUS.get_versioned_ehr_status-xml | canonical-xml | header Last-Modified: expected present, got none | passed |
| I_EHR_STATUS.set_ehr_modifiable-bad_ehr | — | expected `not_found`, observed `precondition_missing` | passed |
| I_EHR_STATUS.set_ehr_modifiable-existing_ehr | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.set_ehr_modifiable-invalid_body | — | expected `validation_failed`, observed `bad_request` | passed |
| I_EHR_STATUS.set_ehr_modifiable-missing_if_match | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.set_ehr_modifiable-stale_if_match | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.set_ehr_queryable-bad_ehr | — | expected `not_found`, observed `precondition_missing` | passed |
| I_EHR_STATUS.set_ehr_queryable-existing_ehr | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.set_ehr_queryable-invalid_body | — | expected `validation_failed`, observed `bad_request` | passed |
| I_EHR_STATUS.set_ehr_queryable-missing_if_match | — | expected `updated`, observed `precondition_missing` | passed |
| I_EHR_STATUS.set_ehr_queryable-rm_version_empty | — | expected `validation_failed`, observed `bad_request` | passed |
| I_EHR_STATUS.set_ehr_queryable-stale_if_match | — | expected `updated`, observed `precondition_missing` | passed |
| I_ITS_REST_REVISION_HISTORY.versioned_ehr_status_revision_history-two_versions | canonical-json | expected `updated`, observed `precondition_missing` | passed |
| I_QUERY_SERVICE.execute_ad_hoc_query-bare_ehr_scope_header | — | row count 100 != expected 1 | passed |
| I_QUERY_SERVICE.execute_ad_hoc_query-bare_ehr_scope_param | — | row count 100 != expected 1 | passed |
| I_QUERY_SERVICE.execute_ad_hoc_query-unknown_ehr_scope | — | expected `not_found`, observed `ok` | passed |
| I_QUERY_SERVICE.execute_stored_query-fetch_with_top | — | expected `stored`, observed `bad_request` | passed |

</details>

## EHRbase measured-window error classes (adjudicated)

The EHRbase measured record is honest as measured — errors are observations.
Every class below was reproduced against a freshly composed EHRbase and
adjudicated three-way against the RELEASED ITS-REST docs text before any
narrative: the driver payloads are spec-correct (the identical exchanges
succeed 0-error against ferroehr in the committed record), and each class
attributes to the EHRbase implementation. No expectation was bent either way.

| Operation | errors/requests (measured window) |
|---|---|
| `composition_update` | 77/77 |
| `ehr_status_update` | 26/26 |
| `tags_put` | 34/34 |
| `tags_read` | 34/34 |
| `template_get` | 3/85 |

- **`composition_update` + `ehr_status_update` (wire 400, deterministic)** —
  one shared root cause: EHRbase rejects the RFC-9110-quoted `If-Match`
  entity-tag form with a 400. The released docs text itself mandates the
  quoted form (ITS-REST overview `Requests_and_responses.md`
  §"If-Match and accidental overwrites": `If-Match: "8849182c-…::openEHRSys.example.com::2"`),
  and in a live reproduction (recorded on issue #266 — the committed run
  record carries statuses, not bodies) EHRbase 400s even its own returned
  `ETag` echoed back verbatim, succeeding only on the non-standard unquoted
  form. EHRbase non-conformance; the record stands.
- **`tags_put` + `tags_read` (wire 404, deterministic)** — the item-tag paths
  (`/ehr/{ehr_id}/tags`, `/ehr/{ehr_id}/composition/{uid}/tags`, …) are members
  of the STABLE EHR API in released ITS-REST 1.1.0 (RM grounding:
  `RM/docs/common/master07-tags.adoc`); EHRbase serves no such routes
  ("No resource found at path"). A released-STABLE surface absent from EHRbase is
  non-conformance, not a citable N/A; the record stands.
- **`template_get` (partial)** — the endpoint is fully functional when
  reproduced (200 for JSON and XML); the small error share sits in a load
  window whose `ward_query` p99 was ~10.9 s. Load-window transient, no defect
  on either side; honest error observation.

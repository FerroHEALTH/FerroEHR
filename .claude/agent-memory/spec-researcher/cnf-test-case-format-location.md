---
name: cnf-test-case-format-location
description: Where the CNF platform test-case format, data-set matrices, content decision-tables, and robot fixtures live in the vendored spec
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

CNF Platform Conformance Test Schedule = `docs/specs/openehr/CNF/docs/platform_test_schedule/`.

- **Test-case format definition** (normative): `master03-overview.adoc` §"Test Case <SERVICE_COMPONENT>.<operation>-<test-specific id>" — the 4-row table `Description | Pre-conditions | Post-conditions | Flow` (an implementation adds a 5th `Test runners` row). Key normative sentence: "A 'test' is therefore the execution of a particular test case with a particular data set." Two test aspects: API conformance + Data Validation conformance.
- **Functional (API) suites**: master06=EHR/EHR_STATUS, master07=EHR_COMPOSITION (incl. versioning: preceding_version_uid, change_type CREATE/MODIFY, lifecycle_state openehr::523|deleted|), master04=Definitions/OPT (I_DEFINITION_ADL14 validate/upload/get/delete + versioning notes), master05=query-def, master08=contribution, master09=directory, master10=demographic, master11=querying, master12=admin, master13=messaging.
- **Data-set matrices** live inline in the functional chapters as asciidoc `[cols=...]` tables (e.g. master06 §Test Data Sets = the 16-row is_queryable/is_modifiable/subject/other_details/ehr_id table). The 16 rows are duplicated verbatim inside the robot `[Template]` data-driven table.
- **Content (Data Validation) decision-tables**: master15 (composition), master16 (entry), master17.1–17.7 (data types). master17.3 (quantity) = DV_ORDINAL/DV_SCALE/DV_COUNT/DV_QUANTITY/DV_PROPORTION/DV_INTERVAL<>. Columns = input attrs (magnitude/units/symbol/value…) + constraint columns (C_DV_QUANTITY.list, C_INTEGER.range/list…) + `expected` (accepted/rejected) + `constraints violated`. Each ROW is a data set; case × row = one test.
- **Robot fixtures**: `docs/specs/openehr/CNF/tests/platform/robot/` — suites under `<INTERFACE>/<operation>/*.robot`; fixtures under `_resources/test_data_sets/` (ehr/{valid,invalid}, compositions/{CANONICAL_JSON,CANONICAL_XML,FLAT,STRUCTURED,TDD}, valid_templates/*, invalid_templates/{empty_file,removed_mandatory_elements,multiple_elements,...}, contributions, query). Fixture naming carries the verdict: `__full.json` = valid, `__invalid_wrong_structure.json` / `__invalid_opt_doesnt_exist.json` = invalid; invalid EHR_STATUS files named by defect (007_ehr_status_is_modifiable_missing.json). Subject id placeholder `__AUTO-GENRATED-BY-TEST__` randomized per run.
- **The PROFILES book (a SEPARATE CNF document, not the schedule)**:
  `CNF/docs/profiles/master03-profiles.adoc` — **THREE tables, not one**
  (re-counted 2026-08-21): `== Functional` L14-70, `== Non-Functional` L75-83
  (Security & Privacy: Signing / Anonymous EHRs), `== Other Non-Functional`
  L88-93 (External Data Format = XML, JSON). L9 is the binding sentence: "In
  order to obtain `CORE` or `STANDARD` conformance, all mentioned capabilities
  must be met in testing". Functional row groups + declared rowspans:
  `.5+` Definitions L18, `.7+` EHR Persistence L29, `.3+` Demographic
  Persistence L41, `.3+` Querying L49, `.6+` Admin L53, `.2+` Messaging L60,
  `.7+` REST APIs L63. L18 ticks **ADL 1.4 Archetype provisioning CORE +
  STANDARD** (as is ADL 1.4 OPT) while ADL 2 is OPTIONS-only — the books
  require a capability ITS-REST publishes no endpoint for. L63-68 = the REST
  API rows (DEFINITION/EHR CORE; QUERY STANDARD; DEMOGRAPHIC/ADMIN/MESSAGE
  OPTIONS).
- **TWO malformed rowspans in that file, in DIFFERENT tables** (upstream-report
  #2526; arithmetic: cells+span-slots must be a multiple of the 5 columns):
  Functional `.7+|*REST APIs*` L63 declares 7 and supplies **6** rows
  (24 content cells + 7 = 31, not ÷5; `.6+` would give exactly 30), and
  Non-Functional `.3+|*Security & +\n Privacy*` L79 declares 3 and supplies
  **2** (8 + 3 = 11, not ÷5; `.2+` gives 10). Every other Functional group's
  span matches its row count exactly (5/7/3/3/6/2). Do NOT describe the
  Security & Privacy defect as being in "the same table" as REST APIs.
- **`SMART` appears NOWHERE in the whole `CNF/docs/` tree** (`grep -rni smart`
  = zero hits) — the profiles book never gained a SMART row despite the
  ITS-REST SMART sub-spec existing.
- **CNF cites SM operation names that do not exist**: `master13-func_tc_messaging.adoc`
  L51/L64/L77 name `I_EHR_EXTRACT.export_ehr()` / `export_ehr_extract()` —
  the SM declares `export_ehrs` / `export_ehr_extracts` on
  `I_EHR_EXTRACT_SERVICE` (interface name differs too); `I_TDD.*` vs
  `I_TDD_SERVICE.*` likewise. master04 does the same for `validate_opt`/`get_opts`.
- NOTE per project memory ecc-own-conformance-framework: this repo's ECC does NOT map to these Robot suites; treat as spec-format reference only.

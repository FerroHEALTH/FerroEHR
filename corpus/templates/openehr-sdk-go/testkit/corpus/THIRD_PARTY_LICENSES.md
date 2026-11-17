# Third-party licenses — vendored fixtures

Some fixtures under this directory are vendored from upstream projects. Per the upstream license terms, copyright and license notices are retained here. The repository-wide inventory is [`LICENSING.md`](../../LICENSING.md); a full copy of Apache License 2.0 is [`licenses/Apache-2.0.txt`](../../licenses/Apache-2.0.txt).

## ehrbase/openEHR_SDK

**Source:** https://github.com/ehrbase/openEHR_SDK  
**Commit:** `e07110a2d969d6971ff157b93288f64ee30764af` (2026-11-02, `develop` with the 2.36.0 release merged), the `ehrbase/openEHR_SDK` line of [`UPSTREAM_PINS.txt`](UPSTREAM_PINS.txt), for every entry in this section  
**Path within source:** `test-data/src/main/resources/` and `validation/src/test/resources/`

```
Copyright 2021–2026 vitasystems GmbH and Hannover Medical School.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

(full text: [`licenses/Apache-2.0.txt`](../../licenses/Apache-2.0.txt))

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

**RM-only (`rm/`):** `minimal_evaluation.json`, `compo_with_nested_party_related.json`, `ehr_status_other_details_simple.json`, `nested_folder.json`, `test_all_types.v1.xml`, `simple_empty_folder.xml` — from `test-data/src/main/resources/{composition,ehr,folder}/canonical_json/` and `{composition,folder}/canonical_xml/` in the commit above. Every file is byte-identical to its upstream copy at that commit; `minimal_evaluation.json` was refreshed to it (upstream commit `66845f98` rewrote its `start_time` zone from `+0000` to `+00:00`).

**Template triplets (`templates/` + `compositions/`):** `nested.en.v1` from `test-data/src/main/resources/operationaltemplate/` and `composition/canonical_json/`. The other four come from the SDK's validation tests, `validation/src/test/resources/operational_templates/` and `validation/src/test/resources/composition/`: `IDCR Problem List.v1` under the same file names, and three under different ones: `cluster-slot.ehrbase.org.v0` (`cluster-slot.ehrbase.or.v0.json`), `IDCR - Laboratory Test Report.v0` (`IDCR-LaboratoryTestReport.opt` + `IDCR-LabReportRAW1.xml`), and `IDCR -  Adverse Reaction List.v1` (`IDCR - Adverse Reaction List.v1.*`, one space after the dash upstream).

**WebTemplate parity reference (`webtemplate/`):** `constrain_test.opt` (from `operationaltemplate/constrain_test.opt`) + `constrain_test.webtemplate.json` (from `webtemplate/constrain_test.json`), byte-identical at the commit above (and at the earlier `e57511c6…` and `22b01e0c…` pins), except the OPT's `constraints` section, edited as **Modifications** below says. Vendored as the REQ-106 / PROBE-075 WebTemplate structural-parity oracle (ADR-0014); chosen because it pins no template-level node name anywhere, so its golden carries zero name predicates and it exports cleanly, and because it exercises the core datatype set (DV_TEXT / CODED_TEXT / QUANTITY / COUNT / ORDINAL / DATE_TIME / DURATION / PROPORTION). Not part of the SDK runtime.

**REQ-111 sibling-naming oracles (`webtemplate/`):** `Corona_Anamnese.opt` + `Corona_Anamnese.webtemplate.json` (upstream `corona_anamnese.opt` / `webtemplate/corona_anamnese.json`) and `GECCO_Diagnose.opt` + `GECCO_Diagnose.webtemplate.json` (upstream `GECCO_Diagnose.opt` / `webtemplate/GECCO_Diagnose.json`), byte-identical at the commit above. Vendored unmodified as the REQ-111 oracles ([PR 84](https://github.com/Cadasto/openehr-sdk-go/pull/84)) covering the gap's two failure modes: `Corona_Anamnese` failed loudly (four `SECTION.adhoc.v1` siblings reuse one archetype, and eight reused screening OBSERVATIONs under Symptome do the same — `webtemplate.Build` returned `ErrIDCollision`; its golden carries 350 name-predicate segments over 213 `aqlPath`s), `GECCO_Diagnose` diverged silently (it built, but 24 reference `aqlPath`s carry 30 name-predicate segments the builder did not emit). REQ-111 closed both, and the pair now serves as the PROBE-075 parity oracles at 230/230 and 34/34. Not part of the SDK runtime.

**ACTION parity reference (`webtemplate/`):** `test_all_types.en.v1.webtemplate.json` (from `webtemplate/test_all_types.json`), byte-identical at the commit above. Its OPT is not copied again: `crossformat/test_all_types/template.opt` below is byte-identical to `operationaltemplate/Test_all_types.opt` at the same commit, and the parity test reads it there. Vendored unmodified as the PROBE-075 oracle for an ACTION: the only reference golden in this tree whose template holds one, so it pins the ACTION's in-context `time` and merged `ism_transition` node. Not part of the SDK runtime.

**FLAT serialisation conformance corpus (`flat-conformance/`):** `templates/conformance_ehrbase.de.v0.opt` (from `operationaltemplate/`) + 34 `compositions/ehrbase_conformance_*.json` (from `composition/flat/simSDT/conformance/`), at the commit above; its per-file `sha256` and upstream paths are recorded in [`flat-conformance/MANIFEST.txt`](flat-conformance/MANIFEST.txt), regenerated by `make flat-sync` and verified by `make flat-check` (the offline gate `make ci` runs; `make flat-drift` adds a network upstream-drift report). Vendored unmodified as the PROBE-086 oracle: upstream-authored FLAT bodies this SDK did not produce, so decoding and re-encoding them tests the REQ-053 codec against the reference serialiser rather than against itself. Not part of the SDK runtime.

**Cross-format sets (`crossformat/`):** six sets from `test-data/src/main/resources/` at commit `e07110a2d969d6971ff157b93288f64ee30764af`. `corona/` holds `composition/canonical_json/compo_corona.json`, `composition/flat/simSDT/corona.json` and `composition/flat/structured/corona.json`. `ehrn_abdm/` holds `operationaltemplate/EHRN-ABDM-OPConsultRecord.v2.0.opt` and `ehrb_adbm_op_consult_record.json` from `composition/canonical_json/` and `composition/flat/simSDT/`. `test_all_types/` holds `operationaltemplate/Test_all_types.opt`, `composition/canonical_json/all_types_no_multimedia.json` and `composition/flat/simSDT/test_all_types.json`. `alternative_events/` holds `operationaltemplate/AlternativeEvents.opt`, `composition/canonical_json/alternative_events.json` and `composition/flat/simSDT/AlternativeEvents2.json`. `multi_occurrence/` holds `operationaltemplate/ehrbase_multi_occurrence.de.opt` and `multi_occurrence.json` from `composition/canonical_json/` and `composition/flat/simSDT/`. `multi_list/` holds `operationaltemplate/Multi_list.opt` and `multi_list.json` from `composition/flat/simSDT/` and `composition/flat/structured/`. The `corona` set has no OPT of its own: it points at `webtemplate/Corona_Anamnese.opt` above, which is byte-identical to `operationaltemplate/corona_anamnese.opt` at this commit. Vendored unmodified, under the fixed file names of a set (`template.opt`, `canonical.json`, `flat.json`, `structured.json`). Every file's upstream path and `sha256` is recorded in [`crossformat/MANIFEST.txt`](crossformat/MANIFEST.txt), which `scripts/ingest-crossformat.sh` regenerates. Input to the PROBE-105 cross-format parity probe. Not part of the SDK runtime.

**Modifications:** Filename stems match operational `template_id` values; the WebTemplate JSON stem is suffixed `.webtemplate` to disambiguate it from canonical-composition JSON; the `flat-conformance/` tree keeps upstream filenames verbatim (its manifest hashes them); no clinical content edits. One file is edited so that it validates against openEHR `Template.xsd`: in `webtemplate/constrain_test.opt` each child under `constraints` gains the `rm_type_name`, `occurrences` and `node_id` it lacked, copied from the definition node it constrains, and the `DV_PROPORTION` default value gains the `type` the RM requires (`2`, the only value the definition allows). An XML comment at its top says so; the parser ignores that section, so the WebTemplate golden is unchanged.

## ehrbase (integration-tests Robot)

**Source:** https://github.com/ehrbase/integration-tests (split out of the former `ehrbase/ehrbase` monorepo)  
**Path within source:** `tests/robot/_resources/test_data_sets/` (read from a local clone of the upstream, located by `ROBOT_ROOT`); the AQL suite corpus also reads `tests/robot/AQL/`  
**Pinned commit:** the `ehrbase/integration-tests` line of [`UPSTREAM_PINS.txt`](UPSTREAM_PINS.txt), shared by every entry in this section. [`ROBOT_SOURCE.txt`](ROBOT_SOURCE.txt) records it for the curated fixtures (a provenance pin, not a per-file `sha256` lock); the AQL suite corpus, `aql/suites/`, is vendored by its own ingest and records it in [`aql/suites/AQL_SOURCE.txt`](aql/suites/AQL_SOURCE.txt).

```
Copyright (c) 2019 Vitasystems GmbH and Hannover Medical School.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

(full text: licenses/Apache-2.0.txt)
```

**Vendored:** Minimal-entry and `Test_dv_*` template triplets under `templates/` + `compositions/`; `persistent_minimal.en.v1`; `family_history.v.1.2.3`, `my_spanish_template_v0`, `terminology_test.ehrbase.org.v1`, `terminology_test2.ehrbase.org.v1` (added at pin `b4625fc`); the `alternative_types.en.v1` OPT, byte-identical to `valid_templates/alternative_types/alternative_types.opt` (in EHRbase since 2019; copied by hand and renamed, so `scripts/ingest-robot-fixtures.sh` does not reproduce it; its compositions are CODE24's, below); flat `rm/ehr_status_*` and `rm/folder_*` JSON; `submissions/*.json` CONTRIBUTION create wire from `contributions/`; under `aql/suites/`, the Robot AQL suites from `tests/robot/AQL/` and every AQL combination CSV from `tests/robot/_resources/test_data_sets/aql/fields_and_results/**/combinations/`. One suite, `AQL/__init__.robot`, carries its own Apache-2.0 copyright notice, which the byte copy keeps.

**Cross-format sets (`crossformat/`):** four sets from `tests/robot/_resources/test_data_sets/`, at the same pin. `consult_record/` holds `valid_templates/all_types/EHRN-ABDM-OPConsultRecord.v2.0.opt` and `consult_record__.json` / `consult_record__.xml` from `compositions/CANONICAL_JSON/`, `CANONICAL_XML/`, `FLAT/` and `STRUCTURED/`. `family_history/` holds `valid_templates/all_types/family_history.opt` and `family_history.v2__.json` / `family_history.v2__.xml` from `compositions/CANONICAL_JSON/` and `CANONICAL_XML/`. `nested/` holds `compositions/CANONICAL_XML/nested.en.v1__full_without_links.xml` and `compositions/FLAT/nested.en.v1__full_without_links.xml.flat.json`. `persistent_minimal/` holds `compositions/CANONICAL_XML/persistent_minimal.en.v1__full_without_links.xml` and `compositions/FLAT/persistent_minimal.en.v1__full_without_links.xml.flat.json`. The `nested` and `persistent_minimal` sets have no OPT of their own. `nested` points at `templates/nested.en.v1.opt`, the openEHR_SDK triplet described above; its manifest `opt` line records source `robot` and `valid_templates/nested/nested.opt` because those bytes match. `persistent_minimal` points at `templates/persistent_minimal.en.v1.opt`, the Robot-vendored file named in this section, and its manifest `opt` line is also `robot` (`valid_templates/minimal_persistent/persistent_minimal.opt`). Vendored unmodified, under the fixed file names of a set (`template.opt`, `canonical.json`, `canonical.xml`, `flat.json`, `structured.json`). Every file's upstream path and `sha256` is recorded in [`crossformat/MANIFEST.txt`](crossformat/MANIFEST.txt), which `scripts/ingest-crossformat.sh` regenerates.

**Modifications:** Flat `rm/` and `submissions/` filenames; composition JSON stems match operational `template_id`; no clinical content edits. One file is edited so that it validates against openEHR `Template.xsd`: `templates/clinical_content_validation.opt` drops the `ITEM_TABLE.rotated` constraint, an attribute RM 1.2.0 does not define, and the ELEMENT `at0005` that sits directly under the `ITEM_SINGLE` with no `item` attribute, with its term definition ([README.md](README.md), constraint templates). Re-ingest via `scripts/ingest-robot-fixtures.sh`, which makes both edits and stamps the source commit into [`ROBOT_SOURCE.txt`](ROBOT_SOURCE.txt). The `aql/suites/` files are unmodified upstream bytes under their upstream names, each at its path relative to `tests/robot/`; re-ingest via `scripts/ingest-robot-aql.sh`, which stamps [`aql/suites/AQL_SOURCE.txt`](aql/suites/AQL_SOURCE.txt).

## ehrbase/ehrbase (server test resources)

**Source:** https://github.com/ehrbase/ehrbase  
**Pinned commit:** the `ehrbase/ehrbase` line of [`UPSTREAM_PINS.txt`](UPSTREAM_PINS.txt), shared by every entry in this section. [`ehrbase-service/MANIFEST.txt`](ehrbase-service/MANIFEST.txt) records it for the templates under `ehrbase-service/`, with each file's upstream path and `sha256`.  
**Path within source:** `service/src/test/resources/knowledge/`

```
Copyright (c) 2019 Vitasystems GmbH and Hannover Medical School.

Licensed under the Apache License, Version 2.0 (full text: licenses/Apache-2.0.txt)
```

**Vendored:** the `BMI` OPT (`templates/BMI.opt`, upstream `operational_templates/BMI.opt`, unchanged there since `296a1693d`, 2019-10-08); its compositions are CODE24's, below. Ten OPTs under `ehrbase-service/templates/`, each named after its template id. From `operational_templates/`: `careplan.v1`, `DiADeM Assessment.v0`, `IDCR - Medication List.v0`, `IDCR - Relevant contacts.v0`, `IDCR - Service Request.v0`, and `IDCR  - Laboratory Order.v0` (upstream `IDCR  - Laboratory Order.opt`). From `opt/`: `IDCR - Procedures List.v1`, `IDCR - End of Life Patient Preferences.v0`, and `IDCR  - Generic MDT Output Report.v0` (upstream `IDCR  - Generic MDT Output Report.opt`). From the folder itself: `IDCR - Immunisation summary.v0`. [`scripts/ingest-ehrbase-service.sh`](../../scripts/ingest-ehrbase-service.sh) copies the ten and regenerates their manifest. Not part of the SDK runtime.

**Modifications:** `BMI.opt` has its leading byte-order mark and CRLF line endings removed; its content is otherwise identical. The ten files under `ehrbase-service/` are unmodified upstream bytes, byte-order marks included; only their file names change, to their template ids.

## CaboLabs/openEHR-SDK

**Source:** https://github.com/CaboLabs/openEHR-SDK  
**Pinned commit:** `de0e2e9fd6f6b2da22ca178ff6dba02f4d70dd4c` (`master`)  
**Path within source:** `src/main/resources/`

```
Licensed under the Apache License, Version 2.0 (full text: licenses/Apache-2.0.txt)
```

The repository's `LICENSE` names no copyright holder; the project is published by CaboLabs.

**Vendored:** the `Vital signs monitoring` template and its composition, both unmodified upstream bytes (the OPT keeps its byte-order mark and CRLF line endings):

| File | Upstream path | Git blob |
|---|---|---|
| `templates/Vital signs monitoring.opt` | `opts/com.cabolabs.openehr_opt.namespaces.default/vital_signs_monitoring.opt` | `17c59506ab63e17e27c55ca2d507af2e6c932248` |
| `compositions/Vital signs monitoring.json` | `canonical_json/vital_signs_monitoring.json` | `0616434f3b27360cf700289ffce027a95d3423c7` |

To refresh, copy both files from a checkout of a newer commit and update the commit and blobs here; `git hash-object --no-filters` gives a file's blob.

## openEHR CKM

**Source:** openEHR Clinical Knowledge Manager, template `1013.26.380` "Vital signs" (project *Vital signs*, International Core Resources, draft of 2021-03-08), operational template export `https://ckm.openehr.org/ckm/rest/v1/templates/1013.26.380/opt`, fetched 2026-11-07 (`sha256` `3a0d31bd3b5dc632…`, git blob `ab7508b210e96d2df4ccec5b712e6763236241b7`).

```
Licensed under Creative Commons Attribution-ShareAlike (CKM content), as are the archetypes it holds
```

**Vendored:** `templates/Vital signs CKM.opt`. **Modifications:** renamed so its id, and the id the reference Web Template derives from it, differ from `vital_signs`: its `template_id` (in the header and on the root node) and `concept` read `Vital signs CKM`, its uid is a new one, and the name it pins on the COMPOSITION, `vital_signs2` upstream, reads `Vital signs CKM`. Nothing else in it is edited. The derived work stays under the same terms.

## RippleOSI/Ripple-openEHR

**Source:** https://github.com/RippleOSI/Ripple-openEHR  
**Pinned commit:** `9d501747b1a5c6c712539ea9b415c5d96b000105` (`master`)  
**Path within source:** `technical/operational/IDCR - Vital Signs Encounter.opt`

```
Licensed under the Apache License, Version 2.0 (full text: licenses/Apache-2.0.txt)
```

The repository's `LICENSE` names no copyright holder; the repository is published under the RippleOSI organisation.

**Vendored:** `templates/IDCR - Vital Signs Encounter.v1.opt`, renamed to its template id, unmodified upstream bytes (git blob `dc7193c6d1d6593f301a730eee6dc0019429c0cb`; byte-order mark and CRLF line endings kept). It is the original Ocean Template Designer export: it still constrains `is_integral` on a DV_PROPORTION, which the Reference Model defines as a function, where the copy in the EHRbase server's tests had those constraints removed by hand.

**Derived:** `templates/IDCR - Vital Signs Encounter - updated.v1.opt`, assembled for this SDK from that template and the current published CKM archetypes (encounter.v1, respiration.v2, inspired_oxygen.v1, pulse.v2, body_temperature.v2, acvpu.v1, blood_pressure.v2, pulse_oximetry.v1, news2.v1, clinical_synopsis.v1), whose content is under Creative Commons Attribution-ShareAlike; the derived template is under the same terms.

## CODE24 (Cadasto)

**Files:** CODE24-sourced templates under `templates/` paired with `compositions/` (see [README.md](README.md)), including the OPT-only `Referral Request.v1`; benchmark `vital_signs` and `clinical_notes.v0` (made by Medblocks together with CODE24). The `alternative_types.en.v1` and `BMI` compositions are CODE24's, but their OPTs are EHRbase's (above). `clinical_notes.v0.opt`, an Archetype Designer export that another tool later re-serialised, has that tool's changes undone where they break the schema: the root carries no `xsi:type` (it had `OPT`, a type no schema defines), the XML declaration is back, and `match_negated` follows `existence` again instead of preceding `rm_attribute_name`; nothing else in it is edited. `vital_signs.opt`, an Ocean Template Designer export, has two edits. Its four empty `xs:boolean` interval flags are spelt `true`. The existence of each OBSERVATION's `protocol`, each event's `state` and their `items` is 0..1, the Reference Model's, where the export wrote 1..1. `blood_pressure_strict_existence.opt` is that export's blood pressure OBSERVATION alone, with each existence as exported, under its own template id and uid.

**License:** MIT

Sample clinical and template-definition artefacts contributed by CODE24 for SDK parser, validation, and serialization testing. No patient data.

---
name: cnf-ambiguity-register-verification-anchors
description: Where the CNF ambiguity register's own semantics and blast radius live when re-adjudicating entries (disposition authority, which citations are machine-gated, option/statement carriers, per-entry gates) plus the verified false-silence traps
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# Re-adjudicating Veredictum's `artifacts/registers/ambiguities.yaml`

Navigation only — the vendored spec text stays the oracle for every claim.
The register is a MAPPING keyed `AMB-<n>:` (not a YAML list) — a list-shaped
extractor returns zero entries.

## Where the register's own vocabulary is defined (in-repo authority)
- Veredictum's `src/vocab.rs` `enum Disposition` (~L264-281) — the SIX
  variants with their doc comments. These are the definitions the artifacts are
  authored against and they are BROADER than prose paraphrases:
  `FixedHandling` = "Handling encoded directly in bindings/cases";
  `Editorial` = "Editorial defect in the schedule text itself";
  `StatementDeclared` = "No normative cases; statement-declared behaviour only".
  `FixedHandling` and `Editorial` OVERLAP for a CNF-guide defect corrected
  against a released component (filed both ways in practice).
- Behavioural branching is thin: `verdict.rs` (~L524 options selection, ~L619
  report_only non-gating), `validate.rs` (~L323), `model/register.rs` (~L67
  report_only|editorial REQUIRE `upstream_issue` — fixed_handling does NOT,
  ~L76 option_select needs >= 2 options). `fixed_handling` vs `editorial`
  changes NO gating.
- Schema: Veredictum's `schemas/ambiguity-register.schema.json`.

## The citation gap to remember
The `spec-ref` gate (`validate.rs::check_spec_refs`, `check_corpus_spec_refs`,
binding `unrealized.source`/`extension.source`, wire_surface Axis-3 `source`)
does **NOT** resolve the register's own `source` strings — only that OTHER
artifacts' `AMB-nn` references exist. So phantom `§Section` names, wrong section
attributions and quote drift inside register entries accumulate unchecked;
always re-read the cited file rather than trusting the entry.

## Blast radius / carriers
- Referencing artifacts: `grep -rln "AMB-<n>\b" "$VD/artifacts" --include="*.yaml"` (`$VD` = the pinned Veredictum checkout)
  (cases under `artifacts/schedule/**`, bindings under `artifacts/bindings/its-rest/**`,
  `artifacts/vocab/wire_surface.yaml`, `artifacts/vocab/outcomes.yaml`,
  `artifacts/corpus/MANIFEST.yaml`).
- `option_select` carriers = `docs/conformance/party/<party>/statement.json`
  `options` (+ `served_extensions` for extension families). A disposition with no
  field in that file has no declaration channel — check before trusting
  "statement-declared".
- Per-entry implementation hooks worth knowing: the AMB-42 realizability gate is
  Veredictum's `src/exec/content_synth.rs::unrealizable_row`; served extension
  route families are `artifacts/vocab/wire_surface.yaml` `served_extensions:`.
- Upstream reports: `gh issue view <n>` — confirmed reports are CLOSED by design
  (terminal state, owner ruling 2026-08-21) with labels `upstream-report` +
  `upstream-confirmed`; CLOSED is NOT staleness. Repointed `upstream_issue`
  numbers are deliberate too.

## VERIFIED false-silence traps (each felled a register claim first-hand)
- **A comparison/case-folding claim is NOT spec-silent.** BASE
  `base_types/master05-identification_package.adoc` §Composite Identifiers and
  Case (L164-177) binds "**All** composite identifiers" to be case-preserving AND
  **case-insensitive** ("two identifiers identical apart from case … identify the
  same thing"), and §Composite Identifiers (L75-85) scopes that to the WHOLE
  `OBJECT_ID` hierarchy incl. `GENERIC_ID`/`HIER_OBJECT_ID` — i.e. the id inside a
  `PARTY_REF`. Entries that call it "the OBJECT_VERSION_ID case rule" are wrong.
- **The extancy anchor for "version at time T" IS grounded** in RM
  `common/master06-change_control_package.adoc` §Committal and Audits **L90**
  ("`_time_committed_` … should reflect the time of committal to an EHR server,
  i.e. the time of availability to other users") + §Subsequent Local
  Modifications **L278** (state-at-time queries compare against commit times).
  What stays unassigned: interval closedness, branch participation, future-dated
  T, and trunk-vs-any-branch for "latest".
- **`Last-Modified` really is both-tier silent**: it occurs ONLY in ITS-REST
  `docs/overview/Requests_and_responses.md` §ETag and Last-Modified (L166-198)
  and NOWHERE in the OAS tree (no `headers/Last-Modified*.yaml`, no response
  declares it). `ETag` by contrast is declared per-response (`headers/ETag*.yaml`)
  — and `responses/200_EHR.yaml` declares only `Content-Type`, while
  `201_EHR.yaml` declares `ETag_EHR` + `Location_EHR`.
- **Per-API ITS-REST docs prose is structurally absent**: `docs/{ehr,definition,
  demographic,admin,system}/Description.md` are 1.3-1.7 KB stubs; only
  `docs/overview/**` and `docs/query/**` carry prose. A per-API docs-text silence
  needs no re-verification beyond that fact. `grep -ri subject docs/**` = ZERO.
- **`201_EHR.yaml` is the ONE create-201 without** "If the `Prefer` header is
  missing or set to `return=minimal`, the body is empty" (present verbatim in
  201_COMPOSITION / 201_PERSON / 201_directory / 201_CONTRIBUTION /
  201_Template_adl1_4_upload / 201_Template_adl2_upload). All 8 create ops
  declare `'201'` only; every update op declares 200 AND 204
  (`204_version_updated.yaml` assigns the minimal branch in the indicative).
- **SM `is_modifable` misspelling** lives at `SM/docs/UML/classes/i_ehr_status.adoc`
  **L80 = set_ehr_modifiable** — the only site in all of SM/docs; the
  "treated as active" contradiction is the SEPARATE clear_ehr_modifiable row.

## Released-text spelling/citation traps seen repeatedly
- ITS-REST spells it **"fulfill"** (Resources.md §XML/JSON/Simplified Formats);
  register entries quote "fulfil".
- `Resources.md` §Data representation is a `#` PARENT whose `##` children
  (§XML Format, §JSON Format, §Simplified Formats, §Alternative data formats,
  §Datetime format) carry the Accept/406 + Content-Type MUSTs.
- The ITS-REST HTTP authority is **RFC 9110** (not RFC 7231, which it supersedes).
- `REVISION_HISTORY`'s "most-recent-first" sentence sits in the **Description**
  row (whose text merely BEGINS with the word "Purpose") — there is no Purpose row.
- In-repo test paths in artifacts predate the one-binary layout: the real files are
  `app/ferroehr-rest/tests/it/*.rs`.
- The `_type` MUST sentence ("MUST be the uppercase class name from the RM
  specification") is in `Resources.md` **`## JSON Format`** (~L109-111). There is
  NO `§Resource representation` section in that file — entries citing it are
  citing a phantom heading, and the sentence governs the value's SPELLING, not
  which classes a schema position admits.
- The versioned/non-versioned resource CLASSIFICATION ("non-versioned
  resources: EHR, CONTRIBUTION, RESULT_SET") is in `Resources.md`'s PREAMBLE
  (L1-9), before the first heading; "§Resources" is the doc title, not a heading.
- **RFC 9110 §13.2.1** ("A server MUST ignore all received preconditions if its
  response to the same request without those conditions ... would have been a
  status code other than a 2xx or 412") is the rule that decides 404-vs-412
  whenever a REQUIRED `If-Match` meets a nonexistent resource. The full
  worked analysis is already written into
  Veredictum's `artifacts/schedule/directory/I_EHR_DIRECTORY.update_directory-empty_ehr.yaml`
  — read that case before re-deriving it.

## Per-family wire anchors (verified 2026-08-21)
- Directory: SM `UML/classes/i_ehr_directory.adoc` = 10 ops; SIX declare
  `ehr_id` against a `has_ehr (an_ehr_id)` precondition (has_directory,
  has_path, create_directory, get_directory, update_directory,
  delete_directory). Released ops = create/update/delete/get_at_time/
  get_by_version_id only (no versioned_directory, no probe routes).
- Contribution: `operations/contribution_create.yaml` declares 201/400/404/409
  and **no 422**; `demographic_contribution_create.yaml` declares 201/400/409.
  A 422 adjudication there rides `Requests_and_responses.md §HTTP status codes`
  "Additional status codes MAY be used ...", which entries often omit.
- Item tags: SEVEN typed families (composition, ehr_status + the five party
  routes) plus two collection GETs (`ehr_tags_get`, `demographic_tags_get`);
  the five party ops are byte-identical modulo the type name. The
  `(key, target_path)` identity is TIER-1 docs text
  (`Requests_and_responses.md §openehr-item-tag ...`, ~L114), not just OAS.

## Verified counts / premise traps (batch AMB-122..136, 2026-08-21)
- **`SM UML/classes/i_definition_query.adoc` declares EXACTLY 8 functions**
  (has_query, valid_query, store_query, store_query_set, list_queries,
  list_matching_queries, delete_query, queries_count) — `grep -c '^|\*[a-z_]*\* ('`.
  Any entry saying "nine I_DEFINITION_QUERY operations" is wrong. `store_query`'s
  `Pre_valid_query: is_valid_query(a_query_text)` is wrong in NAME **and ARITY**
  (declared `valid_query(a_query_text, a_type)` = 2 mandatory args).
  `QUERY_DESCRIPTOR` DOES have `version [0..1]` ("Query semver.org version number")
  while no I_DEFINITION_QUERY call takes a version.
- **There is NO `openehr-template-id` PARAMETER anywhere in the released OAS** —
  `parameters/header/` has 13 files, none of them; `composition_create.yaml` does
  not declare it either. The header exists ONLY as the docs-text MUST
  (`Requests_and_responses.md §openehr-template-id`). So "route X does not declare
  the openehr-template-id parameter" distinguishes nothing.
- **`ITS-REST docs/simplified_formats/master05-rm_mapping.adoc` has 43 top-level
  `==` sections** (COMPOSITION, ENTRY subtypes, CLUSTER/ELEMENT, DV_* …). The true
  claim is "the only VERSIONED-OBJECT ROOT mapped is `== COMPOSITION`; zero
  PERSON/ORGANISATION/ROLE/AGENT/GROUP/PARTY sections". AMB-152 words this right,
  AMB-128 words it wrong.
- **Guarded-read asymmetry in SM ch.6**: `i_party_relationship.adoc` has preconditions
  ONLY on update (L52-53) + delete (L70) — **none of its three reads is guarded**;
  `i_party.adoc` guards `get_party` (L32) and `get_party_at_version` (L94, misnamed
  `has_party_version`) but NOT `get_party_at_time`. Also: NO demographic UPDATE has a
  content-validity precondition anywhere, though both updates declare error
  `content_invalid`; `valid_content` appears only at `i_demographic_service.adoc:21,37`
  (the two CREATEs).
- **Request-body schemas DO constrain the class**: `schemas/demographic/Person.yaml`
  = `_type: enum [PERSON]` + `x-discriminator-value: PERSON`; `UVersionable.yaml`
  carries a `discriminator.mapping` of the 5 party types. Cite this in any
  400-vs-422 wrong-subtype adjudication. `uid` is OPTIONAL in the schema chain
  (`common/Locatable.yaml` requires only name + archetype_node_id) — the direct
  counterpart of RM PARTY's `Uid_mandatory` invariant.
- **No regex patterns on the query identifiers**: `schemas/query/QueryVersion.yaml`
  and `QueryName.yaml` are bare `type: string` (+ example), and
  `parameters/path/version.yaml` has no `pattern` — nothing in the OAS constrains a
  stored-query version's shape. `headers/Location_Query.yaml`'s example does show a
  full `…/org.openehr::compositions/1.0.1`.
- **Path layout trap**: the overview/query/definition/demographic `*.md` prose lives
  at `ITS-REST/specifications/docs/**`, while `simplified_formats`,
  `simplified_data_template` and `smart_app_launch` `*.adoc` live at `ITS-REST/docs/**`.
  Register entries cite both as "docs/…".
- **`demographic_contribution_get.yaml` declares `Accept_canonical` yet reuses the
  shared `responses/200_CONTRIBUTION.yaml`**, whose description describes selecting a
  Simplified Formats MIME type (and whose `Content-Type` header is
  `ContentType_LOCATABLE`) — an unregistered released contradiction (AMB-57 covers
  only the versions[i].data-vs-schema half; no entry names this operation).

## Released-text citation traps confirmed 2026-08-21 (batch AMB-137..151)

- **`409_EHR.yaml` vs `409_EHR_with_id.yaml` are DIFFERENT branches on DIFFERENT
  operations.** `ehr_create.yaml` declares `409_EHR.yaml` ("a conflict with an
  already existing EHR with the same **subject id, namespace pair**, whenever
  EHR_STATUS is supplied"); `ehr_create_with_id.yaml` declares
  `409_EHR_with_id.yaml` ("Unable to create a new EHR due to a conflict with an
  already existing EHR. Can happen when the supplied **`ehr_id` is already used
  by an existing EHR**"). Entries that attribute the subject-pair wording to
  `ehr_create_with_id` are mis-cited — and the correct file is a STRONGER ground
  for "an id may be re-created after a physical delete" (the conflict is
  conditioned on an EXISTING EHR).
- **Only the two admin ops declare `202` anywhere in the released OAS**
  (`grep -rln "'202'" operations/`). The overview status table is exactly SIXTEEN
  rows (200/201/204/400/401/403/404/405/406/408/409/412/415/422/500/501) under
  "The following subset is used in this specification", closed by "Additional
  status codes MAY be used as long as they do not conflict with the predefined
  codes" — the clause that legalizes both 202 and any 400 an operation does not
  itself declare (`admin_ehr_delete_all` declares 202/204/404/405 and NO 400).
- **CNF `docs/profiles/master03-profiles.adoc` L63 rowspan defect:**
  `.7+|*REST APIs*` declares SEVEN rows over only SIX (L63-68:
  DEFINITION/EHR/DEMOGRAPHIC/QUERY/ADMIN/MESSAGE). Every other block matches its
  count (.5+/5, .7+/7, .3+/3, .3+/3, .6+/6, .2+/2). Cite the row list, never "the
  seven-row REST-APIs table".
- **SM admin orphans are total:** `platform_service`, `export_format`,
  `compression_format`, `encoding_format` have **no `include::` anywhere** in
  SM/docs (grep `classes/<f>.adoc`); `export_spec` + `dump_load_fail_report` are
  included by `master15-admin_service.adoc` L19+L21 only. `has_ehr` is DECLARED
  in `i_ehr_service.adoc` L16 alone (used in preconditions by i_ehr_composition /
  i_ehr_contribution / i_ehr_directory / i_admin_service).
- **CNF `master12-func_tc_admin.adoc` covers NINE SM admin ops and omits
  `load_ehrs`** (sections at L43/57/70/83/96/109/122/135/148).

## Verified anchors / premise traps (batch AMB-152..170, 2026-08-21)

- **`|raw` IS defined for STRUCTURED.** ITS-REST `docs/simplified_formats/master04-basic_concepts.adoc`
  **L655**: "The `|raw` attribute is a special bypass mechanism that enables direct
  embedding of pre-serialized openEHR canonical JSON into **flat or structured
  format inputs**." Only the STRUCTURED PLACEMENT (which property carries it) is
  silent. §Field Identifiers L493-502 = the six components (item 6 = `|raw`).
- **Simplified Formats chapter map**: master04 headings — Field Identifiers 493,
  Node ID Generation Rules 512 (7 steps; step 7 = Uniqueness), Instance Indexing
  573 (NO bound/sparse rule), Raw canonical JSON 653, Format variants 697
  (Flat 699 / Structured 740: rule 2 L753 vs rule 5 L756), Conversion 814
  (Flat→Structured 816, Structured→Flat 831 step 6 L840), Open Value-Sets 925,
  Validation 939. master05 = **43 `==` sections** (COMPOSITION only VO root);
  master06 = **17 `==` ctx sections** (compound keys: `work_flow_id|id` L98,
  `participation_identifiers:1|id:1` L124, `health_care_facility|name` L141,
  `link:0|type` L288); the ghost `ctx/namespace` is master06 **L107**.
- **master05 grep handles**: `^| \`/territory\`` = 6 hits (L90 COMPOSITION legit +
  195/311/451/580/711 = the five ENTRY tables); `/encoding` table row = ZERO
  anywhere; `^| \`_other_reference_ranges\`` = exactly 8; `|id_scheme` Integer at
  1654/1713/1784 vs String 1415; `dta` L3124; `\meaning` L2973; "one one of" ×2
  (1022/1028); `/_expiry_time` Yes L323-326; OBJECT_REF `|scheme` L1478+ vs
  examples' `|id_scheme`; `/relationship` L1478 vs `/_relationship` L1802; the
  vendor-parity NOTE L1485. **En-route defect NOT in the #1600 docket**: §DV_URI
  L2202 + §DV_EHR_URI L2228 link `RM/Release-1.0.4/...` while the other 41
  "See RM specification" links use `RM/latest/...`.
- **`served_extensions` = 20 families** (each `never_gates: true`), not 14 —
  a `yaml.safe_load` over the checkout's `artifacts/vocab/wire_surface.yaml`, counting `served_extensions`.
- **`NON_SM_REST_OPERATIONS` = 29 ops across FIVE pseudo-interfaces**
  (`I_ITS_REST_SYSTEM` 1, **`I_ITS_REST_SMART.discovery` 1**, `ITEM_TAGS` 23,
  `REVISION_HISTORY` 3, `VERSIONED_PARTY` 1) — Veredictum's `src/validate.rs`.
- **SM does contain the string "tag"** (3 unrelated hits: `stored_query_execute_spec.adoc:25`
  + `adhoc_query_execute_spec.adoc:26` "tagged String values", `defined_term.adoc:24`
  "language-region tag", plus `etag=` in `docs/openehr_block_diagram.xml`). The TRUE
  claim is "no SM interface/class/attribute models ITEM_TAG". `revision_history` IS zero.
- **`docs/system/Description.md` L5 contains the word "endpoints"** ("describes
  service endpoints, resources and operations") — only the manifest FIELD list is absent.
  `endpoints` as a MEMBER appears solely in `schemas/others/Options.yaml` (array of
  string, no description, no `required`, five-entry example).
- **408 is declared in the OAS too**: all six `query_execute_*` operations `$ref`
  `responses/408_Query.yaml` ("maximum query execution time reached") — so the
  status-table row is NOT "the sole released mention"; the withheld TRIGGER is the silence.
- **Resources.md has TWO "Additional…MAY" sentences**: L68 (alternative data
  FORMATS) and `Requests_and_responses.md` L237 (status codes). Neither is about routes.
- **ITS-XML global-element inventory (definitive, parse `xs:schema/xs:element`)**:
  nsv1 = archetype(+AOM2), composition, extract, extract_request, items(LOCATABLE),
  template(OPERATIONAL_TEMPLATE) + template(TEMPLATE), version, versioned_object;
  nsv2 adds `result_set`(QUERY_RESPONSE) + `query_request` only. `abstract="true"`
  sits on the LOCATABLE/VERSION **complexTypes**, never on the elements.
  `versioned_object` element = nsv1 `ALL/Extract.xsd` **L8** typed X_VERSIONED_OBJECT
  (mandatory total_version_count + extract_version_count, versions restricted to
  ORIGINAL_VERSION); in nsv2 the TYPE lives in `RM/*/EhrExtract.xsd`, the ELEMENT in
  `RM/*/documents/Extract.xsd`. nsv1 has NO Ehr.xsd/Demographic.xsd (no EHR_STATUS,
  CONTRIBUTION, PARTY types); FOLDER nsv1 `ALL/Structure.xsd` L34 = folders+items,
  nsv2 `RM/latest/Common.xsd` L196 adds `details`; ITEM_TAG absent from both;
  `ORIGINAL_VERSION.data` = `xs:anyType` (nsv1 Version.xsd L21, nsv2 Common.xsd L148);
  CONTRIBUTION complexType = nsv2 `RM/latest/Common.xsd` **L183-189**.
- **CNF explicitly excludes performance**: `CNF/docs/guide/master03-overview.adoc`
  §Product Scope **L70** — "Non-functional conformance (performance, etc) is not
  addressed by this guide." (stronger than "the table has no such category").
- **`capability_matrix.yaml` = 7 `workload_exclusion` rows in FOUR kinds**
  (destructive mid-measurement ×4, "definition administration…", "one-shot by
  nature", "the bulk load IS every measured run's own seeding phase"); the
  `evidence_exception` field no longer occurs anywhere in `artifacts/` (only in
  register prose + `src/schema.rs`/`validate.rs`).
- **Option gating is the single key `option:` on a case** (not `options:`); the
  ferroehr statement declares one arm per family, so AMB-167's nine re-grounded
  415 WRITE rows landed as not-applicable in the Veredictum 0.1.6 report
  ("option …-unsupported: the ICS does not declare this register branch").
- The 13 released operations carrying the ITEM_TAG echo sentence = composition
  create/update, ehr_status_update, and create+update for the five party types
  (`grep -rl "will return ITEM_TAGs as they were set by the server" operations/`) —
  directory ops do NOT carry it.

## Verified anchors / premise traps (batch AMB-186..200, 2026-08-21)

- **RM `common/master06-change_control_package.adoc` section→line map** (345 lines):
  Overview 3, Basic Semantics 19, Typing 21, Versioned Objects 25, Version and its
  Subtypes 31, Virtual Version Tree 45, **Contributions 56** (the five change-kind
  bullets L60-65 + the defective attestation row **L66**), Committal and Audits 82,
  **Digital Signature 94** (process L96, two-depths L98, serialisation rule L104,
  `[.tbd]` L106-107), **Attestation 113** (L115 = "at any time after committal of the
  content being attested" — NOT "at any point in time"; L121 = "at a later point in
  tme", the spec's own typo), Versioning Semantics 127, **Version Lifecycle 129**
  (L133 designates the DIAGRAM "the formal state machine"), Incomplete Content 141
  (**L145** = the "rejected by the API" sentence; L147 = the NOTE on allowed
  invalidity), **Abandoned and Inactive States 155** (the 7-row table = **L165-186**),
  Logical Deletion 190, Version Identification 201, **Local Versioning 217** (the
  `'1.1.1'` example = **L226**), **Distributed Versioning 228** (branching REQUIRED =
  **L240**), Semantics in Distributed Systems 242, Copying/The Copy Operation 244-246
  ("never modified … faithful copy" = **L259**), **Subsequent Local Modifications 261**
  (branch-on-copied = **L263**), Version Merging 280 (`other_input_version_uids`
  updated = **L296**), Disjoint Merging 300 (**has a procedure**, L308-321), **Moving
  Version Containers 325** (trunk-after-move = **L329**; NO procedure, NO marker, NO
  operation — the asymmetry vs §Disjoint Merging is the point), Class Descriptions 335.
- **`RM/docs/UML/diagrams/RM-version_lifecycle.svg` RASTERIZES FULLY LEGIBLY**
  (`qlmanage -t -s 2400`; zero `<text>` elements, all outlined paths). Its transition
  inventory is a strict SUPERSET of the §Abandoned-and-Inactive 7-row table:
  `create_draft`→INCOMPLETE, `create_final`→COMPLETE, `complete`, `update` (INCOMPLETE
  self, COMPLETE self, COMPLETE→INCOMPLETE), `delete` from INCOMPLETE/COMPLETE/
  ABANDONED/INACTIVE, `abandon`, `retrieve` (×2), `deactivate`, `reactivate`, and
  **`revert` OUT of DELETED to BOTH COMPLETE and INCOMPLETE**. So "the seven-row table
  is enforced" is the wrong authority — L133 makes the diagram the state machine.
- **`responses/422_COMPOSITION.yaml` IS A PHANTOM FILE** — it exists in NEITHER the
  decomposed `ITS-REST/specifications/responses/` NOR the bundled
  `crates/openehr-its/vendor/rest-oas/`. The ONLY 422 response definition is
  `responses/422.yaml` ("content type and syntax is correct, could be converted to a
  resource, but there are semantic validation errors, such as the underlying template
  is not known or is not validating the supplied resource"), `$ref`d by exactly **12
  operations** (composition create/update + create/update for the five party types).
  The phrase "information about the errors in the provided COMPOSITION" occurs NOWHERE
  in the ITS-REST tree. `contribution_create.yaml` declares 201/400/404/409 only, so
  any 422 there needs `Requests_and_responses.md` **L237** ("Additional status codes
  MAY be used as long as they do not conflict with the predefined codes").
- **FALSE-SILENCE TRAP — the `ehr_id` shape IS tier-1 assigned**:
  `docs/overview/Glossary_and_conventions.md` **L14** — "`ehr_id` | The value for an
  EHR identifier, stored under EHR.ehr_id.value, **in a form of a HIER_OBJECT_ID,
  usually an `UUID` or a `GUID`**". `Resources.md` L24 does the same for
  `versioned_object_uid`. So the docs text sides with the wide HIER_OBJECT_ID reading
  against `parameters/path/ehr_id.yaml`'s `format: uuid`; a UUID-only narrowing is a
  deployment choice, NOT the oracle-order reading. BASE grammar: `master05-
  identification_package.adoc` L229 `uid = iso_oid | uuid | internet_id`, L245
  `hier_object_id = uid_based_id`, L247 `root = uid`.
- **OBJECT_ID breadth per artifact**: `schemas/base_types/UObjectId.yaml` = all SIX
  (+ discriminator mapping); `UUidBasedId.yaml` = TWO (HIER_OBJECT_ID,
  OBJECT_VERSION_ID); the narrowing hop is `UObjectRefOfUidBasedId.yaml` (allOf
  ObjectRef, overrides `id`), which `Folder.yaml` `items` alone `$ref`s. `ObjectRef.yaml`
  keeps `UObjectId` and requires all three of namespace/type/id. RM `FOLDER` has **NO
  Invariants section at all**. ITS-JSON `definitions.OBJECT_REF.properties.id` = the
  same six `_type` values.
- **`SM UML/classes/i_party_relationship.adoc` = 6 ops, and only FIVE take
  `a_versioned_party_rel_id`** — `get_party_relationship_at_version` (L81-82) takes
  **`a_party_rel_version_id`**. Exactly FOUR declare `versioned_object_does_not_exist`
  (get L31, get_at_time L44, update L60, delete L78). `VERSIONED_PARTY_RELATIONSHIP`
  occurs NOWHERE in RM (grep = 0) while `org.openehr.rm.demographic.versioned_party.adoc`
  exists. Released OAS has PARTY_RELATIONSHIP as a SCHEMA only (`schemas/demographic/
  {PartyRelationship,SeePartyRelationship,ListOfPartyRelationship}.yaml`, reached from
  `Party.yaml`) and **no `/demographic/party_relationship` route** in the 28 demographic paths.
- **Commit-wire vs read-wire merge/import shapes**: `schemas/ehr/OriginalVersion.yaml`
  declares `other_input_version_uids`; `UpdateVersion.yaml` does NOT (its 6 properties
  = preceding_version_uid, signature, lifecycle_state, attestations, data, commit_audit);
  `NewContribution.yaml` types `versions` items as `UpdateVersion` with no `oneOf`/
  discriminator; SM `UML/classes/update_version.adoc` likewise lacks it (and types
  `lifecycle_state` as `Terminology_code` 1..1).
- **The IMPORTED_VERSION shape conflict, per component**: RM `imported_version.adoc`
  = ONE attribute (`item` 1..1) + FOUR `(effected)` functions; ITS-XML
  `components/RM/Release-1.1.0/Common.xsd` **L136-167** (VERSION abstract =
  contribution/commit_audit/signature? ; `data` declared on ORIGINAL_VERSION L148 as
  `xs:anyType minOccurs=0`; IMPORTED_VERSION extends VERSION with `item` only);
  released OAS `schemas/ehr/Version.yaml` `required: [contribution, commit_audit,
  data]` + `ImportedVersion.yaml` `allOf Version` — same in bundled
  `ehr-codegen.openapi.yaml`. The three `UMImportedVersionOf{Composition,EhrStatus,Party}`
  files all exist.
- **`EXTRACT_MANIFEST.entities` item-cardinality is `{lower:1, upper_unbounded:true}`
  + `is_mandatory: true` in ALL FIVE vendored RM BMMs** (1.0.2/1.0.3/1.0.4/1.1.0/1.2.0),
  while the adoc class table shows only `1..1` (attribute existence). `EXTRACT_SPEC`
  has `manifest` 1..1 AND `criteria` 0..1 `List<DV_PARSABLE>` ("Queries specifying the
  contents of this Extract") — the criteria channel the prose invokes DOES exist. The
  "may not specify any entities" sentence is `ehr_extract/master04-common_package.adoc`
  **L132** under `==== Content Specification` (L125).
- **Zero released operation is named for a lifecycle transition or a container move**
  (`grep '^operationId' operations/*.yaml` against abandon|deactiv|reactiv|retriev|
  revert|lifecycle|move = 0 of **97** operation files).
- **DUPLICATE PAIR: AMB-189 ≡ AMB-197** — same two invariants (version.adoc L69 +
  version_tree_id.adoc L64), same master06 L226 `'1.1.1'` example, same conclusion,
  same `report_only`, same handling. Upstream **#1677 and #1749 are duplicate reports
  of one defect**. AMB-189 is the richer survivor (it alone cites §Distributed
  Versioning L240 — the spec MANDATING the shape its own invariant forbids).

## Two traps that manufacture FALSE outbound reports (verified 2026-08-22)
- **RFC 2119 is declared in EXACTLY ONE vendored component.**
  `ITS-REST/specifications/docs/overview/Preface.md` **L41-43** binds the keyword
  set "when, and only when, they appear in all capitals". Case-insensitive grep
  for `rfc 2119|rfc2119` over the whole vendored tree hits ONLY that file + the
  3 `computable/OAS/overview-*.openapi.yaml` copies. **BASE / RM / AM / QUERY /
  SM / CNF declare no conformance vocabulary at all** — so a lowercase "should"
  in BASE or RM is NOT an RFC-2119 SHOULD, and writing it as "SHOULD" in an
  outbound report imports normative force the released text never claims (an easy
  upstream deflection). Quote the sentence with its own casing.
- **A `<<_anchor,...>>` xref may resolve OUTSIDE the chapter, into an
  `include::`d external artifact.** Before calling a cited rule/name "dangling",
  locate the anchor's OWNING section (`grep -n "^=\+ " <chapter>` + grep the
  whole component for the anchor id) and check whether that section is an
  include of an unvendored file — the vendored docs tree can be silent while the
  published document resolves fine. Worked example (and the exact false report it
  produced): [[adl2-cadl-primitive-types-location]] §NOT a dangling reference.

## ITS-REST response-file census anchors (verified 2026-08-22)
Use these instead of re-listing: **12** `responses/201_*.yaml` — `201_EHR.yaml` is
the ONLY one omitting the `Prefer`-minimal empty-body sentence (the other **11**
carry it verbatim) and also the only 201 with NO `Content-Type` header. **15**
`responses/200_*updated*.yaml` = 7 `*_ItemTagList_updated` (each BYTE-IDENTICAL to
its `_retrieved` sibling, so it says "successfully retrieved" on a PUT and never
mentions `Prefer`, though all 7 tag PUT ops DO declare the `Prefer` parameter) +
8 real update responses (all 8 carry the `Prefer` sentence). `responses/404.yaml`
keys 404 on "the request parameters", and the 5 party-create paths
(`/demographic/{person,agent,group,organisation,role}`) declare NO path parameter
while their ops declare `'404': ../responses/404.yaml` (L32-33 of each). Only
TWO operationIds in the release end `.yaml` (`definition_query_{,version_}store`,
propagated into the 3 `definition-*.openapi.yaml` bundles) out of 95 distinct
operationIds across 97 operation files. FOUR untyped `type: array`/`items: {type:
object}` placeholders exist, not one: `schemas/demographic/See{Capability,Contact,
PartyIdentity,PartyRelationship}.yaml` — declared as components in BOTH
`demographic.openapi.yaml` (L217-224) and `definition.openapi.yaml` (L115-122),
`$ref`d by no operation or schema, and shadowed by a properly typed
`schemas/demographic/PartyRelationship.yaml`.

---
name: sm-subject-proxy-ch10-location
description: SM Platform ch.10 (Subject Proxy) map — the 9 sections, the 18 §10.9 includes, the 4 text-free UML SVGs + 2 foreignObject-text conceptual SVGs, the 4 orphan class files, and the confirmed defect classes (zero invariants, zero exceptions, PLATFORM_SERVICE gap, PROC not vendored)
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.10 "Subject Proxy Service (SPS)" — navigation

Largest platform chapter. Shares the master02/master03 conventions catalogued
in [[sm-ehr-service-chapter5-location]]; the only outbound in-SM dependency is
`RESULT_SET` (ch.8, see [[sm-query-service-chapter8-location]]).

## File map
`SM/docs/openehr_platform/master10-subject_proxy_service.adoc` = **230 lines**,
9 sections: Overview L3-13 · Subject Variable Naming L15-25 · Service Interface
L27-41 (5-bullet op list + package SVG) · Data Structures L43-57 (5-bullet
entity list + SVG) · Samples L59-67 · Bindings L69-79 · Persistence L81-83
(**2 sentences, the whole section**) · Usage L85-203 (2 kotlin snippets + 2 YAML
sub-sections `=== Specifying a Data-set` L148, `=== Specifying a Binding` L173)
· Class Descriptions L205-230 = **18 `include::`**.

The 18: `i_subject_proxy_service`, `subject_proxy`, `subject_variable`,
`subject_data_set`, `data_set_result`, `sample`, `data_frame_sample`,
`openehr_sample`, `hl7v2_sample`, `hl7_fhir_sample`, `variable_sample`,
`variable_value`, `variable_value_{single,list,time_series}`, `i_data_binding`,
`env_binding`, `data_frame`. 15 service calls / 11 preconditions.

## Orphan class files — VERIFIED not included by §10.9 (or any chapter)
`sp_variable_def.adoc`, `sp_variable_category.adoc` (an all-empty enum:
state/problem_dx/vital_signs/medication/past_procedure), `s_dv_boolean.adoc`,
`t.adoc`. The first three ARE linked from `UML/class_index.adoc` → their
`platform.html#_sp_variable_def_class` anchors are DANGLING in the published
body (same defect class as ch.8's `RESULT_QUERY_DESCRIPTOR`). `t.adoc` is not
even in class_index.

## Diagrams — 4 text-free UML + 2 text-bearing conceptual
- `UML/diagrams/SM-platform.interface.subject_proxy{,-structure,-sample,-binding}.svg`
  = 0 `<text>`, 136/184/77/110 `<path>`. `rsvg-convert -w 2600` renders all
  four fully legible. They are the ONLY source of: `I_SUBJECT_PROXY_SERVICE`
  **inherits I_STATUS**; qualified-association Hash keys (`SUBJECT_PROXY.variables`
  key `name`, `.data_sets` key `id`, `SUBJECT_DATA_SET.variables` key `name`
  with element bound **1..\***, `VARIABLE_VALUE_TIME_SERIES.value` key `time`);
  generic constraints `T > Any`; the bindings `VARIABLE_SAMPLE = SAMPLE<VARIABLE_VALUE>`
  and `OPENEHR_SAMPLE = SAMPLE<RESULT_SET>`; private visibility on
  `SUBJECT_DATA_SET.using_app_ids`/`.last_result`, `SUBJECT_VARIABLE.last_frame`
  and **`DATA_FRAME.execute()`**; and the entire shape of the PROC types
  `SYSTEM_CALL`/`QUERY_CALL`/`API_CALL`/`PARAMETER_DEF` + `SYSTEM_CALL.definition [0..1]`
  (untyped) + `PARAMETER_DEF.type : EL_TYPE_DEF`.
- `openehr_platform/diagrams/spo_conceptual.svg` (§10.1) + `spo_context.svg`
  (§10.8) HAVE text but the `<text>` fallbacks are TRUNCATED with "…" — the full
  labels live in the draw.io `<foreignObject>` `<div>`s; extract those, do not
  rasterize. `spo_context` names a `OracleMPI_sample` class that exists nowhere
  in the model.
- `SM-platform.definition.svg` classifies `SUBJECT_PROXY_SERVICE` as a
  **"Retrieval Service"** exposing only `I_SUBJECT_PROXY_SERVICE`
  (`I_DATA_BINDING` is not a component interface); `SM-platform-packages.svg`
  confirms the `interface::subject_proxy` package.

## Total silence outside SM
`subject_proxy|SUBJECT_VARIABLE|ENV_BINDING|DATA_FRAME` appear in NO other
vendored component: no CNF chapter (platform_test_schedule has no subject-proxy
master file) and no `CNF/tests/platform/robot/I_SUBJECT_PROXY_SERVICE/`, no
ITS-REST surface, no RM anchor. No other SM chapter xrefs any ch.10 class
(closed island). §10.8.1 L150 is the ONLY mention of a REST API for SPS.

## Cross-component grounding that is NOT vendored
`DATA_FRAME.primary_method`/`fallback_method` type `SYSTEM_CALL` links to
`/releases/PROC/{proc_release}/task_planning.html` — **PROC is not a vendored
component**, and `{openehr_proc_overview}` (§10.1 L13) /
`{openehr_decision_language}` (§10.8 L150) / `{proc_release}` are attributes
defined in the un-vendored AA_GLOBAL boilerplate. `EL_TYPE_DEF` is in no
vendored file either (not in LANG).

## Chapter-wide defect classes (all first-hand)
- **ZERO invariants** on all 18 classes (only `update_audit` + `terminology_relation`
  carry Invariant rows in the whole SM class set); **ZERO exceptions** on all 15
  calls; exactly ONE postcondition in the chapter (`SUBJECT_VARIABLE.is_global`),
  and it CONTRADICTS its own Meaning cell.
- `PLATFORM_SERVICE` (master03/common) omits **Subject_proxy** (and Terminology)
  though master02 L39's service table lists both → SPS unnameable in the 4
  `I_ADMIN_SERVICE` calls that take a `PLATFORM_SERVICE`.
- No `SUBJECT_PROXY_CALL_STATUS_TYPE` descendant exists although master03
  §Representing Call Status invites one → the only failure code available is
  generic `precondition_violation`.
- 4 unresolved `TODO:` lines in normative text (register_binding L113,
  subject_category L31, ask_user L41, get_frame subject_id L33).
- `SUBJECT_APP_DATA_SET` (§10.8 L127) is a class that exists nowhere.
- Duplicated figure captions: ".Typical Subject Proxy situation" (L10 + L145)
  and ".Subject Proxy structures" (L56 + L66).
- The §10.8 YAML omits mandatory attributes (`ENV_BINDING.env_id`,
  `SUBJECT_DATA_SET.subject_id`, `SUBJECT_VARIABLE.{is_manual,frame_id,frame_path}`),
  uses `frame_id:`/`name:` for `DATA_FRAME.id`/`SUBJECT_DATA_SET.id`, puts
  `query_text` on `!!API_CALL`, and writes `PT2m` — invalid per BASE
  `Time_definitions.valid_iso8601_duration` (`P[nnY][nnM][nnW][nnD][T[nnH][nnM][nnS]]`,
  `BASE/docs/UML/classes/org.openehr.base.foundation_types.time_definitions.adoc` L193-198).
- `:spec_status: TRIAL`; amendment record 0.9.7 "Add Subject Proxy Service",
  T Beale, 01 Apr 2021 — the chapter has had no entry since. SM pinned @
  `23ffc4711c`.

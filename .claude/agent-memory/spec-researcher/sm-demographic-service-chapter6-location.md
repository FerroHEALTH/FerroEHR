---
name: sm-demographic-service-chapter6-location
description: Where SM Platform ch.6 (Demographic service) reference text lives — master06 is include-only, the §6.1 SVG is the ONLY source of the inheritance chain + the UPDATE_VERSION<T> bindings, the 17-op inventory, and the confirmed defect set incl. the exact 2-site valid_content scope
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.6 "Demographic Service" — navigation

Companion to [[sm-ehr-service-chapter5-location]] (same structural pattern) and
[[demographic-api-location]] (the ITS-REST wire side).

`SM/docs/openehr_platform/master06-demographic_service.adoc` is **22 lines,
INCLUDE-ONLY** — the shortest service chapter: §Overview = ONE sentence + the
package SVG (L5-L9); §Class Definitions = **5** `include::` pulls (L13-L21),
ZERO own prose. Every requirement lives in `SM/docs/UML/classes/`:
`i_demographic_service` (4 ops), `i_party` (7), `i_party_relationship` (6),
`uv_party` (0 features), `uv_party_relationship` (0). **17 operations total.**

## THE §6.1 DIAGRAM IS READABLE — and carries content the tables lack
`SM/docs/UML/diagrams/SM-platform.interface.demographic.svg`: **0 `<text>`,
171 `<path>`**, `viewBox 0 0 823 653`. `rsvg-convert -w 2600` (or 5200 +
`magick -crop` for detail) renders fully legible. ONLY source of:
- the chain **`I_STATUS` <- `I_VALIDITY_CHECKER` <- {I_DEMOGRAPHIC_SERVICE,
  I_PARTY, I_PARTY_RELATIONSHIP}** (one shared-trunk generalization; NO class
  table has an `*Inherit*` row) — so `last_call_failed`/`last_call_status` +
  `definitions_valid`/`content_valid` are inherited members of all three;
- **the generic bindings**: `UV_PARTY` = `UPDATE_VERSION<PARTY>`,
  `UV_PARTY_RELATIONSHIP` = `UPDATE_VERSION<PARTY_RELATIONSHIP>`, parameter
  `T > Any`. The two class tables say only `Inherit: UPDATE_VERSION` — NO binding;
- return multiplicity split: `update_party`/`update_party_relationship` return
  **`UUID [1]`** while `create_party`/`create_party_relationship` return bare
  `UUID` (tables give no multiplicity for either);
- `a_time : Iso8601_date_time` with **NO `[1]`** (both `*_at_time` ops) —
  contradicts the tables' `[1]`;
- confirms `i_party(a_versioned_party_id)` / `i_party_relationship(...)` are
  **untyped in the source model**, not a rendering loss;
- NO demographic status-type specialisation is drawn (contrast ch.5's
  `EHR_CALL_STATUS_TYPE`) — `CALL_STATUS -> CALL_STATUS_TYPE (+code 1)` only.

## Confirmed defects (verified first-hand, line-exact)
- **`valid_content` is a 2-SITE ch.6 defect, NOT 4** — `i_demographic_service.adoc:21`
  + `:37` only (label `__Pre_content_valid__` vs expression `valid_content(...)`,
  disagreeing on the SAME line; declared name is `content_valid`,
  `i_validity_checker.adoc:22`). SM-wide `valid_content` = 6 sites (the other 4 are
  ch.5: `i_ehr_composition:104,125`, `i_ehr_directory:47,97`). **`i_party.adoc` and
  `i_party_relationship.adoc` contain NO `valid_content` at all** — their defect is
  the opposite: they declare error `content_invalid` with NO content-validity
  precondition. (Any brief citing i_party.adoc:71 / i_party_relationship.adoc:62 as
  `valid_content` sites is wrong — those lines are `definition_unknown` ERROR bullets.)
- `i_party.adoc:94` precondition calls **`has_party_version`**; declared function is
  `has_party_version_id` (`:22`). Only bare-`has_party_version` site in all of SM.
- `definition_unknown` + `content_invalid` (4 sites each in ch.6) exist in **NO
  enumeration** — not `call_status_type.adoc`, not `ehr_call_status_type.adoc`, not
  `definition_call_status_type.adoc`, and there is **no `DEMOGRAPHIC_CALL_STATUS_TYPE`
  file**, although master03 L17 defines the descendant-enumeration mechanism.
- `party_id_does_not_exist` IS declared (`call_status_type.adoc:44`, "Party with
  provided id not found") but is used by **zero ch.6 ops** — only
  `i_admin_archive.adoc:37` + `i_admin_service.adoc:71`.
- All version ids typed `UUID`, but master02 §Global Naming Conventions L165-166
  defines `_version_uid_` as `uuid::system::N` — unrepresentable in a UUID. Ch.5's
  `i_ehr_composition.adoc:18,69` uses `OBJECT_VERSION_ID` for the same role =
  in-spec precedent proving the ch.6 typing wrong.
- `delete_party` / `delete_party_relationship` postcondition `not has_party(...)`
  contradicts RM `common/master06-change_control_package.adoc` §Logical Deletion
  L192 ("information can only ever be logically deleted") + L58 indelibility, and
  contradicts ch.5's `delete_composition` prose (which spells out the `523|deleted|`
  procedure). Ch.6 gives NO deletion procedure prose.
- **ZERO `.Parameters` blocks** in all 5 ch.6 files (ch.5's i_ehr_composition has one).
- `create_party` has NO postcondition though `has_party` exists; ch.5's
  `create_composition:105` has `Post_has_composition: has_composition(..., Result)`.

## Silences (no ch.6 op at all)
No party search/list/query (must already know the UUID), no `get_versioned_party`
(ch.5 HAS `get_versioned_composition`), no revision-history op, no
demographic-contribution interface, no `has_party_relationship_version_id`
(asymmetric with I_PARTY), no concrete-subtype selector — one `create_party` for all
5 RM PARTY descendants (RM `PARTY` is **abstract**), where ITS-REST has 5 route
quintets. `a_time` is mandatory here but `[0..1]` + "if no time supplied, get the
latest" in ch.5.

## Cross-cutting rules ch.6 inherits (NOT in master06)
`master03-common_package.adoc` §Version Update Semantics **L21 names `PARTY`
explicitly** ("a `COMPOSITION`, `PARTY` or similar … implicitly require … a new
`CONTRIBUTION` … `ORIGINAL_VERSION` … new `VERSIONED_OBJECTS`") — the SM-side proof a
party write emits CONTRIBUTION+AUDIT; L23 `UPDATE_AUDIT` (server generates
`time_committed`+`system_id`; ATTESTATION supplied in full); L25
`preceding_version_uid` mandatory except first version, `lifecycle_state` always;
L29 the `UV_XX` derivation convention (its example text says `VU_XX` in
`update_version.adoc:29` — transposition defect); L17 the descendant-enumeration
mechanism. `master02-overview.adoc` §Interface Calls L60 (formal equivalence +
"transactionally protected"), §Functional Style L109 (authn/authz already done),
L143 ("any single call constitutes a self-standing transaction"), §Global Naming
Conventions L162-171, §Anatomy L68-77 (the pre/post/exception template).
§List Handling does NOT apply — ch.6 has no container-returning call.
Payload classes: `update_version.adoc`, `update_audit.adoc` (both in master03).

## CNF = total vacuum, and provably mis-attributed
`CNF/docs/platform_test_schedule/master10-func_tc_demographic.adoc` (203 lines):
Test Environment TBD, Test Data Sets TBD, 12 op sections x 2 cases, every body
"TBD", case names "aaaa"/"bbbb". Covers 12 of the 17 ops (omits `i_party`,
`i_party_relationship`, `has_party`, `has_party_version_id`,
`has_party_relationship`) and attributes **all 12 to `I_DEMOGRAPHIC_SERVICE`**
though 10 belong to I_PARTY/I_PARTY_RELATIONSHIP. Mechanism is evidenced: it
DEFINES `:i_party_link:` + `:i_party_relationship_link:` at **L5-L6 and never
references them** (grep returns only the definitions). CNF master03-overview /
master02-glossary say NOTHING about demographic — no conformance-level statement.
**NO demographic/party Robot suite exists** (`CNF/tests/platform/robot/` has 9
I_*/SECURITY dirs, none demographic; "party" greps hit only contribution fixtures).

## Status / pin
`manifest_vars.adoc` -> `:spec_status: TRIAL`; amendment record "SM Release 1.0.0
(unreleased)", last entry 13 Dec 2021; **0.9.1 (18 Oct 2017) = "Added demographic
interface calls"** — never revised since, which explains why the ch.5 fixes
(SPECPR-305, the `get_composition_xx` argument-type correction of 28 Feb 2019)
never propagated to ch.6. Vendored @ `23ffc4711c`.

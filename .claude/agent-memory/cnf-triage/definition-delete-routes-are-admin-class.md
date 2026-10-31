---
name: definition-delete-routes-are-admin-class
description: The two extension definition-artefact DELETEs became Admin class (#2071) — the 5 red rows are catalogue bin, and the readonly/uncovered-branch fallout
metadata:
  type: project
---

Commit `0e2863b54` (issue #2071, 2026-08-07) moved
`DELETE /definition/archetype/adl1.4/{archetype_id}` and
`DELETE /definition/artefact/adl2/{artefact_id}` from the Clinical operation
class to Admin — `EXTENSION_ADMIN_ROUTES` in
`app/ferroehr-rest/src/extensions/access/authz/mod.rs`. The uploads stayed
Clinical.

**Why:** the neighbouring `DELETE /admin/template/{template_id}` had the same
blast radius and got Admin only from its `/admin/` path prefix.

**How to apply.** The classification STANDS — nothing released governs it (see
[[authz-class-is-own-design]]); the Service Model is reference text and
justifies nothing, and the
CNF `master04` §Implementation recommendations privilege sentence is
inadmissible (stalled component, explicitly non-normative, scoped to physically
deleting OPTs where a logical-delete tier exists — this definition store has
none). The operative reason is own-design blast-radius judgement.

Consequences a future triage will meet:
- The five semantics cases (`delete_archetype-existing`/`-non_existing`,
  `delete_artefact-existing`/`-non_existing`/`-malformed_artefact_id`) drive the
  default `sut` principal (roles `[USER]`) and now observe 403 — CATALOGUE bin,
  fixed with `on: admin` on the delete step.
- Nothing cases the new branch "authenticated non-admin, non-readonly is refused
  on these deletes".
- The `delete_artefact` DELETE was the ONLY write in the `adl2-archetype`
  extension family, so `I_DEFINITION_ADL2.delete_artefact-readonly_forbidden`
  can no longer isolate the read-only restriction anywhere on that surface.
- Guard prose in both `-readonly_forbidden` cases still asserts "these routes
  carry the coarse `Clinical` operation class … require no ADMIN role" — false.
- `I_DEFINITION_ADL14.delete_opt` is bound `unrealized` (AMB-17) so its four
  cases are N/A, and `DELETE /admin/template/{template_id}` has NO case at all
  (`vocab/wire_surface.yaml` family `admin-extension-routes`, `never_gates`) —
  so no executed case anywhere pins a definition-artefact delete as admin.

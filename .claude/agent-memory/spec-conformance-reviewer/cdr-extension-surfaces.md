---
name: cdr-extension-surfaces
description: Recurring verification checklist for "our own extension" ferroehr-rest REST surfaces (non-spec endpoints)
metadata:
  type: project
---

For `ferroehr-rest` surfaces flagged "OUR OWN EXTENSION" (admin deletes, contribution list, etc.), the house bar is: follow the vendored spec where it speaks, else mirror the sibling surface's established design. Confirmed checks that pay off:

- **Flag discipline**: module/fn doc must carry the exact "no openEHR spec governs this — our own design/extension" flag. Verified present + correct on the admin template/query deletes and the contribution-list surface (2026-07-18, branch claude/admin-ui-features-2).
- **Error mapping round-trip**: extension service methods return `StatusError`; the `?` from `ServiceError` must map to the intended HTTP. `ServiceError::Conflict`→`StatusError(CompositionAlreadyExists)`→409; `NotFound`→`VersionedObjectDoesNotExist`→404. NotFound collapses the concrete resource kind generically, so an extension's 404 body carries a generic status code — acceptable for extensions.
- **ABAC registration** (`extensions/access/pep.rs`): `mode_of` + `kind_of`. EHR-scoped list reads use `Mode::Pre` (ehr_id path param, subject gate) like `ehr_get_by_id`/`contribution_create`; single-resource GETs use `Mode::Post`. A list surface pre-checked coarser than the by-uid post-check is acceptable when it returns only non-clinical metadata.
- **Call chains and SQL round-trip counts are re-derived from the code, never from a document** (owner ruling 2026-07-26; a standing map under-counts round trips — contribution-list is 3 trips: ehr_exists probe + count-with-audit-JOIN + list). Derive trip counts from the actual `count_*`/`list_*` bodies.
- **utoipa::path presence**: every extension handler needs `#[utoipa::path]` (we serve only our own generated OpenAPI). All three surfaces had them.
- **Case-insensitive id resolve** must match the schema's CI unique index: `lower(x)=lower($1)` matches `ux_template_store_template_id_ci` / the stored-query `lower(...)` PK columns; grounded in BASE master05 §Composite Identifiers and Case.

Template `version` field (ITS-REST `TemplateMetadata.version`): optional + `deprecated: true`, NOT nullable in the OAS — so emit-only-when-present is *correct* and emitting explicit `null` would be a schema violation. Value is "taken from `template_id`" (filter_version param), so id-`.vN`-axis derivation is spec-faithful.

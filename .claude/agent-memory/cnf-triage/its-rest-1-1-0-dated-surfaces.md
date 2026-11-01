---
name: its-rest-1-1-0-dated-surfaces
description: The ITS-REST Amendment_record dating table — which surfaces arrived in Release-1.1.0 vs 1.0.3 and earlier; the ground for every case/header version floor
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

`docs/specs/openehr/ITS-REST/specifications/docs/overview/Amendment_record.md`
is the authoritative date line for every `applies: { its_rest: … }` floor. It is
HTML-in-markdown; strip tags to read it.

**Release-1.1.0 additions** (a party declaring 1.0.3 is out of scope for all of
these): SPECITS-95 UPDATE_AUDIT typing (5.9) · SPECPR-472 `system_id` on
`openehr-audit-details` (5.8) · SPECITS-92 RM sync + **SPECITS-84 Simplified
Formats MIME types on CONTRIBUTION** (5.7) · SPECITS-86 `template_id`/`version`
filters on listTemplates (5.6) · SPECITS-58 `/example` sub-resource,
SPECITS-34 SM-derived resources, SPECITS-87 ADL2 deprecations, SPECITS-46 `aql`
name fix (5.5) · **SPECITS-61 Simplified Formats consolidation + the Accept/
Content-Type headers that support them** (5.4) · SPECITS-74 `Location`
deprecation, **SPECITS-82 `W/` ETag**, SPECITS-50 minimal responses (5.3) ·
**SPECITS-77 ITEM_TAGs**, **SPECITS-80 admin EHR delete**, SPECITS-75 header
renames (`openEHR-AUDIT_DETAILS` → `openehr-audit-details`, etc.) (5.2) ·
**SPECITS-70 Demographic API endpoint**, SPECITS-73 yaml restructure (5.1/5.0).

**Release-1.0.3** contains exactly ONE item: SPECITS-66 "Migrate REST API specs
to OpenAPI Specification" (4.1). **Release-1.0.2** carries SPECITS-41 **"Add
double quotes to ETag and If-Match headers"** (3.3) — so the quoted entity-tag
form is floor-proof at 1.0.2. **Release-1.0.1** carries SPECITS-38 "Fix response
status code for semantic validation errors" (2.5) — 422-for-semantic is
floor-proof at 1.0.1.

**How to apply:** before attributing a comparison-party red row to upstream,
date the surface here. A 404/406 on a 1.1.0-dated surface against a 1.0.3 party
is a selection question, not an upstream finding.

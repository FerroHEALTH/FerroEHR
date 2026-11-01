---
name: amb54-contribution-412-etag-overpromise
description: CATALOGUE defect — AMB-54(b) promised a latest-version ETag on the contribution-route 412; no released sentence grounds it and the route has no unique version referent
metadata:
  type: project
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

`I_EHR_CONTRIBUTION.commit_contribution-stale_preceding_version` step 3 went
red on `header ETag: … got none` (status 412 matched). Attributed **catalogue**
2026-07-27, confirmed first-hand:

- `ITS-REST .../docs/overview/Requests_and_responses.md` §If-Match and
  accidental overwrites — the ETag sentence is conditioned on the header:
  "**If a service receives this header**, and the condition evaluates to
  `false` … MUST respond with … `412` … and **SHOULD** return also latest
  `version_uid` in the `ETag`". `POST /ehr/{ehr_id}/contribution` has no
  `If-Match` parameter and the case sends none → antecedent false, and it is a
  SHOULD even when true.
- Same file §HTTP status codes, 412 row: "conditions given in the request
  **header fields**" — the contribution precondition lives in the member body.
- `.../docs/overview/Resources.md` — CONTRIBUTION is listed under
  "**non-versioned** resources"; the route's own ETag identity is the
  contribution uid (the binding captures `contribution_etag_uid` from it).
- No unique referent: `SM .../UML/classes/i_ehr_contribution.adoc`
  `commit_contribution(… versions: List<UPDATE_VERSION>[1] …)` +
  `RM .../UML/classes/org.openehr.rm.common.contribution.adoc` "a list of
  versions, which may include paths pointing to **any number** of versionable
  items" — "the latest version_uid" is per-member, so one ETag cannot serve it
  in general. The app is right to omit it.

Contrast that proves the app is not at fault: every route where the antecedent
DOES hold passes (`update_composition-stale_if_match`,
`update_directory-/delete_directory-stale_if_match`,
`set_ehr_queryable-stale_if_match`, `update_party-weak_etag_stale`) — the
`error_with_meta` decoration in `app/ferroehr-rest` covers exactly those.

**How to apply:** a register `handling:` text is a catalogue artifact and a
suspect — an adjudicated status code does NOT license attaching further
required headers to it "by analogy with a sibling route". Check the antecedent
scope of a SHOULD sentence before a binding turns it into a gating matcher.
Open consistency item (not a defect yet): the direct-route bindings gate the
same SHOULD via the `latest-version-uid` matcher; there the antecedent holds and
the referent is unique, so it is defensible, but its strength is SHOULD.

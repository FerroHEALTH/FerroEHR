---
name: commit-member-uid-is-service-generated
description: A CONTRIBUTION version member never carries its own uid — UpdateVersion.yaml has no uid; AMB-89 refuses it, so fixtures must not spell one
metadata:
  type: project
---

The commit member is an `UPDATE_VERSION`, and the version identity is
service-generated (the Service Model says so too, but it is reference text,
never an authority: owner ruling 2026-10-27):

- Released OAS `ITS-REST/specifications/schemas/ehr/UpdateVersion.yaml` — six
  properties (`preceding_version_uid`, `signature`, `lifecycle_state`,
  `attestations`, `data`, `commit_audit`); `NewContribution.yaml` types
  `versions` items as `UpdateVersion`, no `oneOf`, no discriminator.
- The docs text is SILENT on the remedy (ignore vs refuse) → register AMB-89,
  `disposition: fixed_handling`, upstream #1535: the SUT refuses `uid` / `item`
  / a foreign `_type` with 400 (`app/ferroehr/src/versioning/contribution.rs`
  `reject_foreign_version_identity`). The catalogue pins that in
  `commit_contribution-imported_version_member` steps 2–3.

So a corpus fixture used as a pre-built member (driver passes through verbatim
when `data._type == "ORIGINAL_VERSION"`,
`driver.rs::contribution_member`) must NOT spell `uid`. Reproduced 2026-08-03:
`signed_version.v1.json` / `attested_version.v1.json` → 400; the same members
with `uid` deleted → 201, and the client `signature` is still served back
byte-identical, so the uid is inert for the cases' subject.

Latent contradiction to watch (filed as an en-route finding): the app accepts
`_type: "ORIGINAL_VERSION"` on a member while refusing the `uid` that RM makes
1..1 mandatory on that class
(`RM/docs/UML/classes/org.openehr.rm.common.original_version.adoc`), so no
honest ORIGINAL_VERSION instance is committable.

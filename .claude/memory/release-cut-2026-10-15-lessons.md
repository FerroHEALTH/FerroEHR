---
name: release-cut-2026-10-15-lessons
description: "v4.3.9 cut lessons: the release lane reads every check and security/vex/ from the TAG's tree, so a gate failure after tagging burns the version (v4.3.7 arm64 linkage, v4.3.8 new Debian CVEs); scan before tagging"
metadata:
  node_type: memory
  type: project
  originSessionId: 5f27e25c-822b-425f-8d0b-ab988feea1c5
  modified: 2026-10-15T11:45:25.307Z
---

The 2026-10-15 cut took three tags. `v4.3.7` stopped on the arm64 legs: the new VEX linkage check (#3694) required the dynamic loader in DT_NEEDED, which glibc lists on amd64 only (arm64 has it as PT_INTERP only); fixed in #3790. `v4.3.8` passed every build and stopped at the `vulnerability record`: three Debian CVEs without a fix (cyrus-sasl, krb5 ×2) appeared in `ferroehr-postgres` after the tag; judged in `postgres-os.openvex.json` in #3791. Both stopped before anything published (draft release, untagged images), but the `release-tags` ruleset forbids moving a tag, so each failure meant a new patch version. The owner had the milestones renamed up by one each time (rename the milestones highest-first: it moves every issue and keeps due dates), the changelog section renamed, all pins, chart version and the draft GHSAs' `patched_versions` moved, and the unpublished draft release deleted (a draft is not found by `gh release delete <tag>` once GraphQL is rate-limited; delete it by id over REST).

**Why:** `release.yml` checks out `github.sha` (the tag), so a fix that lands on main never reaches a running release.

**How to apply:** before tagging, run the release's gates against the merge commit: the vex-linkage self-test, and `scripts/checks/vex-coverage.sh` over the newest scan reports (download the `vulnerability-record` artifact of the last release run, or scan main's images) with the purls `release.yml` passes. A new OS finding with no Debian fix gets a VEX statement in the release PR, not after the tag. Release advisories are drafted as GHSAs before the cut (one per Security entry, a VEX path instead only for a dependency nothing shipped reaches) and published with `PATCH …/security-advisories/<ghsa> state=published` once the GitHub release is out. Related: [[release-cut-2026-09-16-lessons]], [[agent-pushes-release-tags]].

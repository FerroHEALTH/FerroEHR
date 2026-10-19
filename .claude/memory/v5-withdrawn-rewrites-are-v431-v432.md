---
name: v5-withdrawn-rewrites-are-v431-v432
description: "The live release line (owner 2026-10-19): v4.3.11 the patch cut today, v5.0.0 the demographics release, v5.1.0 the larger follow-ups, v6.0.0 the platform and application-layer refactors; supersedes the 2026-09-16 withdrawal of v5"
metadata:
  type: project
---

On 2026-09-16 the owner withdrew the v5 milestones and put the greenfield rewrites into the 4.3 patch line (v4.3.1, the storage rewrite #3337; v4.3.2, the platform seams #3377 and the lean application layer #3086). That ruling no longer describes the tracker: the milestones v5.0.0, v5.1.0 and v6.0.0 exist again and hold work.

The release line as of 2026-10-19:

- **v4.3.11**: the patch cut on 2026-10-19 with the quick fixes (viewer formatting and favicon, the FerroSYS documentation, the FerroUI 0.1.6 rollout, the flaky dump-load test).
- **v5.0.0**: the demographics release, cut the same day. The owner decided on 2026-10-19 to put the demographics work here: the `openehr-sdt` party builder from a party Web Template and the caller-chosen `rm_version` (#3810, #3809), the two `openehr-sdt` example fixes (#3779, #3780), and AQL rooted at a demographic class, the Cadasto experimental extension subset FerroBRIDGE issues (#3818).
- **v5.1.0**: the larger follow-ups that do not fit today's cut.
- **v6.0.0**: the platform and application-layer refactors (the FerroSYS access-log service #3661, the platform seams #3377, the storage re-evaluation #3337 and their children).

**Why:** the owner re-grouped the work by release theme, so a note saying v5 is withdrawn would send new issues to the wrong place.

**How to apply:** an issue found while working goes to the current milestone (see [[new-issues-go-to-current-milestone]]); demographics work goes to v5.0.0 until it is cut and then to v5.1.0; platform or application-layer refactors go to v6.0.0. When a decision is open, decide and record it on the issue rather than ask. Related: [[design-record-updated-in-the-same-turn]], [[rewrite-breaks-everything-shipped-means-released]].

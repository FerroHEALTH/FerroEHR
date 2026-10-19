---
name: new-issues-go-to-current-milestone
description: "Owner direction 2026-10-19: one milestone at a time; an issue filed while working goes in the CURRENT milestone, whatever its version; supersedes the 2026-09-14 next-patch, never-v5 ruling"
metadata:
  type: feedback
---

The tracker works one milestone at a time. An issue filed while working a milestone, an en-route finding, a follow-up or an upstream report included, goes in the CURRENT milestone and is fixed before the cut (fix-first). That holds whatever the current milestone's version is, a patch, a minor or a major such as v5.0.0.

The exceptions: a `blocked-upstream` issue carries no milestone, and work the owner has placed in a later themed milestone (the release line in [[v5-withdrawn-rewrites-are-v431-v432]]) stays there. An issue blocked from the current cut by something outside this repository moves to the next milestone with a comment saying why.

**Why:** on 2026-09-14 the owner said "add all these new ones to milestone 4.3.1 please, not to v5", when v5.x held the withdrawn application-layer rewrites. The v5.0.0, v5.1.0 and v6.0.0 milestones exist again (owner, 2026-10-19), so "never v5.x" would now send work away from the release being cut. The owner's direction on 2026-10-19 is one milestone at a time, with en-route issues in the current one.

**How to apply:** `scripts/gh/fields.sh new … --milestone v<current>` for every issue filed this cycle. Before ending a session, re-check the current milestone's open issues; it is cut at zero. Related: [[pr-closes-one-keyword-per-issue]], [[component-fixes-ride-current-patch]].

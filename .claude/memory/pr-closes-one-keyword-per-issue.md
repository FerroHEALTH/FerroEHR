---
name: pr-closes-one-keyword-per-issue
description: "A PR body must repeat the closing keyword for EVERY issue — \"Closes #1, #2, #3\" closes only #1; and every issue found en route goes in the CURRENT milestone, never the next"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7a390181-e5e7-4508-9316-33a300a4aedb
  modified: 2026-08-11T20:01:59.900Z
---

**One closing keyword per issue.** `Closes #2261, #2264, #2265` closes only
**#2261** — GitHub parses the keyword against a single reference, not against a
comma list. Write `Closes #2261. Closes #2264. Closes #2265.` (or `Closes #2261,
closes #2264, closes #2265`).

**Why:** PR #2272 listed five issues behind one `Closes` and left four open
after merging. The work was done and merged; the tracker said otherwise, which
is the failure the auto-close exists to prevent.

**Every issue found en route goes in the CURRENT milestone.** Owner directive
(2026-08-11): "everything that comes up will be added to this milestone". Do not
file into the next milestone because something looks like follow-up work — the
owner decides what slips, and a milestone is the release's promise, so parking
work in the next one silently removes it from this cut.

How to apply:
- Repeat the keyword per issue, then VERIFY after merge (`gh issue view <n>
  --json state`) — the parse is silent when it fails.
- `scripts/gh/fields.sh new … --milestone v<current>` on every new issue, including
  upstream-reports and follow-ups, unless the owner says otherwise. The one
  standing exception is `blocked-upstream`, which carries no milestone at all
  (it cannot promise a delivery).

Related: [[en-route-findings-always-filed]], [[autonomous-phase-flow]].

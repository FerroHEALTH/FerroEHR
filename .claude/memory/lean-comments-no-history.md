---
name: lean-comments-no-history
description: comments are rare, short and describe only the code as it is; never history, old figures or the former design; a changed comment is rewritten whole (owner hard rule 2026-11-23, every family repo)
metadata:
  type: feedback
---

A comment exists only when the code cannot say it: a spec citation, a
non-obvious why, a constraint. The code speaks for itself, and most items
need one summary line or nothing. A comment never describes the old way: no
"where it sent 41", "once X moved", "fell by its drop", no measurement
history, no issue-by-issue narrative. When code changes, the comment is
rewritten whole to describe the new code alone, never extended with another
"then ... now ..." sentence. History lives in the PR and git.

Owner, 2026-11-23, on a 13-line ceiling doc comment that narrated every past
measurement: "anywhere and everywhere this is not allowed ... we need lean
and small comments ... only a comment if it really needs it because the code
should speak for itself ... never describe the old things only the new, so
always fully rewrite the comment". Applies to every FerroHEALTH repository
and to every worker.

**Why:** comments go stale faster than code, and then the two disagree;
narrated history is an AI tell and adds lines nobody needs.

**How to apply:** every worker brief carries the rule; review every diff's
comments for history and length; a ceiling or budget constant gets at most
one line naming its issue, never its measurement trail. A sweep deletes
existing narration. Related: [[no-compat-shims]],
[[family-memories-every-repo]].

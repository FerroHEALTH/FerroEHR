---
name: worker-wait-loops
description: Never wait with a pgrep -f whose pattern the loop's own command line contains; run long commands in the background, one wait per output; push a finished branch rather than holding it for local reruns; stop a process by PID, never pkill (2026-10-16)
metadata:
  type: feedback
---

Workers left shells polling for hours with `until ! pgrep -f "<command>"`: the pattern matches the loop's own command line, so it never ends. One loop from a finished worker ran 4.5 hours, and the owner found "13 shells running ... holding up things".

**How to apply:**
- Run a long command in the background and wait for its completion notification. Never poll with `pgrep -f` on a string the loop itself contains; keep one wait per output.
- A polling loop (CI checks, a merge queue) sleeps at least 60 seconds between reads.
- A finished branch waits for nobody (owner, 2026-10-16: "for 2 sub aganats we are still wiating for the last 2:40 hours!!! this is not acceptable"): push once your own tests and the local gates pass, and let CI run the rest.
- Stop a stuck process by its PID. Never `pkill` or `killall`, which stop every worker's run of that command.

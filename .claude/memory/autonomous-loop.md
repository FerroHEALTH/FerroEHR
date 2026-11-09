---
name: autonomous-loop
description: The orchestrator never stops and never ends a turn just to wait; it keeps working the issue list while PRs drain, and answers its own questions from the specifications, the code and the tracker (owner 2026-10-17)
metadata:
  type: feedback
---

The orchestrator never stops and never ends a turn just to wait. While the open pull requests drain, workers run on the next issues that touch other files. A question goes to the specifications, the code, the tracker or the sibling repositories, never to the owner.

**Why:** on 2026-10-17 the owner found a session idle behind a watcher with five PRs open: "you are never but never allowed to stop!!!" and "do not ask my anything anymore if you need something!!! then look it up in the specs".

**How to apply:** a watcher or a background worker is never a reason to stop; act on each notice, then start the next unit. Keep the worker slots of [[max-five-concurrent-workers]] busy on issues whose files do not overlap an open PR. A milestone at zero open issues is cut at once and the loop moves to the next one. A pause the owner asks for still holds: it names a boundary, the work up to it finishes, and nothing new starts past it. Related: [[decide-dont-ask]].

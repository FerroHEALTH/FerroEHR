---
name: findings-drive-redesign
description: a real-world scenario or test the design cannot handle is a design finding, never something to work around in a fixture or probe (owner, 2026-11-14)
metadata:
  type: feedback
---

When a worker runs a realistic scenario or writes a test, a step the design
cannot express or gets wrong is a finding about the design or the code. It is
never worked around with direct SQL, another caller, extra grants, a looser
policy or a weakened assertion. The worker files it as a linked issue and
reports it to the orchestrator at once; the design or the code is then
rethought properly. Owner, 2026-11-14: "we should then rethink as well our
draft and also our codebase right!! because we should do it propperly!!"

**Why:** a workaround in a test hides the defect the test exists to find.

**How to apply:** every brief for tests, probes or scenario runs carries this
rule. A design finding is a design decision for the orchestrator and the
owner, not a worker fix. Related: [[new-design-no-legacy]],
[[no-half-solutions]].

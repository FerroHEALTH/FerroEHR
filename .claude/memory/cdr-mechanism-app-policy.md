---
name: cdr-mechanism-app-policy
description: the CDR provides the generic mechanism; national rules and controller policy come from the deployment or the application on top as content, never from code (owner, 2026-11-15)
metadata:
  type: feedback
---

FerroEHR supplies the generic mechanism: the evaluator, the refusal codes,
the record of the decision. Which national law applies, which retention
period holds, and which role or organisation type may reach data are the
controller's decisions, supplied by the deployment or the application on top
as content the mechanism reads. The one exception is EU law that binds the
product everywhere (EHDS, [[ehds-above-openehr]]).

**Why:** policy questions belong to the application layer, not to the owner
or the code.

**How to apply:** for a policy question, design the content shape the
controller fills and ship no national catalogue; never hard-code an answer
and never ask the owner to pick one. Related: [[findings-drive-redesign]],
[[new-design-no-legacy]].

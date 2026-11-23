---
name: new-design-no-legacy
description: in a breaking release, build the new design in full and keep nothing of the old one; never relay a worker's "kept for compatibility" choice as a question (owner, 2026-11-14)
metadata:
  type: feedback
---

When a release is a breaking one, the new design is built completely and
every piece of the old one goes in the same programme: no field kept so old
records read back, no mark kept for one consumer, no old gate kept beside the
new one, no default carried over from a removed key.

**Why:** the owner asked on 2026-11-14 why old parts were kept when the point
of the breaking change was to do it the proper new way.

**How to apply:** when a worker reports something kept for compatibility,
rule on it yourself: file and fix its removal. Put the no-legacy rule in
every worker brief. Ask the owner only about real design trade-offs, never
"keep the old thing or not". Related: [[no-half-solutions]],
[[no-compat-shims]].

---
name: multi-generation-refactor-no-legacy
description: "During the #1936 multi-generation program, legacy/old-idea residue is never carried — file it as a sub-issue of #1936 and queue it"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: dc90e7e8-afc4-4863-a4a2-d90e4bb606e0
  modified: 2026-08-05T11:23:06.543Z
---

Owner directive (2026-08-05, during the #1936 multi-generation spec-version
program): when work under this program surfaces an old-idea/legacy construct
(e.g. the crate-level `SPEC_VERSION` const that contradicted the selected
`Generation`), do NOT keep or work around it — remove it properly, and when
the removal is out of the current child's scope, file it (`scripts/gh/fields.sh new`) and
`scripts/gh/rel.sh parent <new> 1936` so it queues inside the program.

**Why:** the refactor's value is doing the multi-generation design fully
correctly; carrying legacy semantics silently poisons the new model.

**How to apply:** treat "found legacy residue" exactly like the repo's
en-route-findings rule ([[en-route-findings-always-filed]]) but with the
parent edge mandatory: sub-issue of #1936, milestoned into the program's
queue. Examples already handled in-session: crate-root `SPEC_VERSION`
removed (enum + generation-module consts are the only authorities);
`#1944` (generate the hand-written `*_impl.rs` surface) parented under
#1936 by the owner.

**Profile coupling (owner HARD RULE 2026-08-05):** `spec_profile = stable`
selects RM 1.1.0 + BASE 1.2.0 + LANG 1.0.0; `development` selects RM 1.2.0 +
BASE 1.3.0 + LANG 1.1.0. LANG is IN the coupled set — never RM+BASE only.
Generation modules are named by COMPONENT VERSION (`v1_0`, `v1_1`, `v1_2`);
a component version's several published spec files are UNITS inside one
generation (LANG v1_1 = bmm + bmm3), never separate generations.

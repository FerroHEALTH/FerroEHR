---
name: published-crates-cite-specs-only
description: "Owner hard rulings 2026-08-04 — AMB register ids live ONLY in the conformance instrument (Veredictum); openehr-* AND app/* sources never reference them (nor instrument paths / the CNF schedule as authority)"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 05904df7-8f63-41af-8762-dd50ffe8db19
  modified: 2026-08-04T17:59:49.922Z
---

Owner rulings 2026-08-04 (angry, explicit, widened the same day): the
conformance instrument's machinery (then the ambiguity register's `AMB-nnn`
ids; since Veredictum 0.3 its catalogue paths, option names and `report:`
numbers) is referenced ONLY inside the instrument's own repository
(Veredictum). The `openehr-*` crates
(published on crates.io) AND the `app/*` crates cite ONLY the vendored
openEHR spec text and official external docs — never an AMB id, never
instrument/catalogue paths, never the CNF schedule as the stated
authority.

**Why:** a published crate's docs reach docs.rs readers who have no repo;
internal register tokens are dead references there, and they leak the
conformance instrument into the spec layer.

**How to apply:** a spec-silent adjudication in a crate/app comment states
the decision + the released-text ground in place ("no released text assigns
X — adjudicated Y"); the register keeps pointing at the CODE, never the
other way. AMB ids stay legal only in Veredictum and in this repository's
`docs/`. Swept 2026-08-04 (10 crate sites + 128 app sites).
Related: [[en-route-findings-always-filed]].

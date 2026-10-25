---
name: cnf-strategy-doc-kept-permanently
description: "Owner rulings 2026-07-22, 2026-07-23 and 2026-08-26: the CNF 2.0 design record lives in the Veredictum repository as its root ARCHITECTURE.md; never re-create a copy in FerroEHR; Veredictum is being rewritten, so the record is input, not authority (owner, 2026-10-25)"
metadata:
  type: feedback
---

The CNF 2.0 design record, a living reference document with a
community-facing head, is exempt from the delete-on-implementation rule
(owner rulings 2026-07-22 and 2026-07-23). It moved with the instrument on
2026-08-26 (#2811): it is `ARCHITECTURE.md` at the root of the Veredictum
repository, and FerroEHR carries no copy.

Veredictum is being rewritten (owner, 2026-10-25, [[family-architecture]]),
so neither its verdicts nor this record are authority. The vendored openEHR
specification text stays the only oracle.

**Why:** the record holds the §8.14 population-anchored performance-class
model (POC 2, S 15, L 150, R 1,500 peak arrivals/s floors, p99 at most 1 s,
workload mixes with their OECD, Eurostat and NHS derivation) that FerroEHR's
earlier measured performance claims were built against. Two copies of a
design record is how one of them goes stale.

**How to apply:** read it only from the official remote at the pinned tag
(`gh api repos/Vernum-Projecten/Veredictum/contents/ARCHITECTURE.md?ref=v<pin>`,
pin in `scripts/lib/veredictum.sh`), never from a local checkout or cache
([[veredictum-other-session]]). Treat its figures as input to check, not as
a standard a claim is judged against. Never re-create a FerroEHR copy. Code
citations point at `docs/specs/openehr/` only. See [[owner-work-style]].

---
name: spec-chapter-audit-programs
description: "Owner's preferred compliance method — one milestone per spec component, one issue per chapter (decomposed per §X.Y section), whole-codebase sweep, fix-first cadence"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 871531fb-1884-468e-9033-ae616ae2eb2b
  modified: 2026-08-01T06:30:12.656Z
---

Owner directive 2026-07-29: systematic code compliance is driven as **one
milestone = one spec component, one issue per spec CHAPTER**, each chapter
issue sweeping the WHOLE codebase via `/spec-audit`. The same shape covers
the per-endpoint ITS-REST surface audit, and `v4.0.0` applies it to the
admin console.

**Decomposition (owner directive 2026-07-30):** each chapter issue decomposes
into **one sub-issue per spec §X.Y section** (a chapter with 0–1 `== `
sections stays the audit unit itself) — auditing a whole chapter at once
risks skipping a section like RM common §6.3 Versioning Semantics. Section
counts derive from `grep '^== ' <master file>` in the vendored spec tree.

**Audit bodies are behaviour-explicit:** a requirement is the WHOLE normative
content — structural (classes/invariants) AND behavioural (state machines,
procedures, identifier rules, prose semantics, normative even when no
invariant encodes them) AND wire-visible — and the audit question is whether
the RUNTIME BEHAVIOUR matches the text, never merely "the types exist".

**Fix-first cadence (owner ruling 2026-07-26) — never accumulate a findings
backlog:** per unit, audit → file fix issues (typed label, `spec:*`, the
program's milestone) → implement, test, PR and MERGE every one of them
BEFORE the next unit's audit begins (P1 first; the orchestrator takes
versioning/critical-path fixes, workers the mechanical ones, within the worker cap) → each
fix PR carries its CNF pinning case + changelog entry → zero-drift CNF run →
close the unit's issue → next unit. Later units then audit the FIXED shared
layer instead of re-documenting known defects.

**Mechanics that work:** post the walked checklist as an issue comment (that
comment IS the record), tick body checkboxes via `gh issue edit`, close a
finding-free unit by hand with the record attached; read-only
`spec-conformance-reviewer` agents in parallel handle bulk chapter audits
well (evidence-verified checklists pasted from their reports); the program
closes on a zero-drift pipeline run.

**The load-bearing lesson:** adding SMALL spec-cited WIRE cases is what finds
application escape classes — in-crate suites are example-based and blind to
whole input shapes. The sharpest catch was the untagged-node escape class
(#1431): validation dispatched on the wire `_type` tag while canonical JSON
legally omits it on concretely-declared slots, so untagged nodes skipped ALL
RM invariants (JSON committed what XML refused); fixed by effective-type
resolution over the BMM-generated RM model plus a corpus-wide
tag-independence property.

**Scope adjudication:** CDS/GDL2 is NOT a CDR component — it is a separate
application layer consuming the CDR — so it carries no program in this
ladder (#716, closed with the spec citation).

**How to apply:** when a new compliance push starts, propose this shape:
parent program issue + per-chapter children (normative chapters only, front
matter excluded by adjudication) + per-section grandchildren, findings as fix
issues under the cadence above, zero-drift pipeline at program close. Which
component programs are open vs released is read from the milestones
(`gh api … /milestones`), never copied forward. Milestone routing for a fix
whose program has closed: [[component-fixes-ride-current-patch]].

**SM-program refinements (v3.19.0, 2026-08-21):** DEVELOPMENT-state model
documents (SDF, Simplified IM-B) get adjudication-verification chapters —
the audit question is "does the never-implement adjudication hold and is it
flagged", walkable in-session with no researcher fan-out. Upstream defects
consolidate into per-defect-class reports (14 for the whole SM), never
per-cell issues. SM UML diagrams are text-free path art carrying normative
structure absent from the class tables (inheritance, generic bindings,
multiplicities) — researchers must rasterize them (`rsvg-convert`), and the
spec-tree README now records this. Extraction researchers run max 2 ahead of
the in-session walk; the walk + records + closes stay with the orchestrator.

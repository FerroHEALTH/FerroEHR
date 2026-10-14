---
name: aql-row-grain-bindings
description: AQL SELECT-path row-grain bindings (left_bind / bind_select_paths), strict CONTAINS, attribute_child path steps — the aggregate cross-contamination and WHERE-depends-on-SELECT pitfalls to re-check on any later AQL change
metadata:
  type: feedback
---

Reviewed 2026-10-14 (uncommitted fix for Cadasto reports b1-b4) against QUERY master03-syntax.adoc §Aggregate functions, §DISTINCT, §Containment, §SELECT and BASE architecture_overview/master11-paths.adoc §Basic Syntax.

- Binding aggregate ARGUMENTS into the shared FROM (select.rs bind_select_paths collects `SelectValue::Aggregate { arg: Some(t) }`) makes COUNT(*), COUNT(path), SUM, AVG depend on sibling aggregates (row multiplication / cross product of independent repeated steps). MIN/MAX and COUNT(DISTINCT) are immune. Spec ground: §Aggregate functions input = expression "applied to data items filtered by FROM and WHERE"; §COUNT(*) = rows of the result set; §DISTINCT "returns all data items selected by the FROM and WHERE clauses as rows".
- WHERE reads through SELECT bindings only when the prefix is shared (step equality includes predicates); otherwise any-match per leaf, so `WHERE sys>=140 AND dia>=90` still pairs across events when SELECT does not bind the event.
- Codec path shape (storage/codec.rs prune_children): child path = `<parent path><attr>.` or `<parent path><attr><i>.`, parent_num = parent num; attribute_child's `^attr[0-9]*\.$` relies on it. Analyzer hop = all step types in openehr_rm v1_2 is_structure_root (incl. EVENT_CONTEXT, FEEDER_AUDIT, ELEMENT).
- 523-with-data is now refused on direct commit (lifecycle.rs reject_deleted_with_data) and on import (message/import.rs), so "no root node <=> lifecycle 523" holds; supersedes the §6.3 memory's "523+data committable".
- How to apply: on any AQL change, test aggregate combos and WHERE-without-SELECT conjunctions over a two-event observation; check CHANGELOG for row-count-visible changes.

Round 2 (2026-10-14, same uncommitted diff): aggregates now walk items per row (select.rs aggregate_over_items) — COUNT(*) contamination closed; combine NULL/0 semantics verified correct. Remaining patterns to re-check:
- Aggregates ignore WHERE at item grain (WHERE is existential per row when SELECT binds nothing) while a non-aggregate SELECT of the same path is filtered per bound node: COUNT(sys)/AVG(sys) WHERE sys>=140 counts the <140 events too. §Aggregate "data items filtered by FROM and WHERE".
- Conjunction step-sharing (predicate.rs node_tests) only covers a flat AND chain of bare positive single-path tests; `A AND (B OR C)`, `A AND NOT B`, path-op-path, fn(path) still cross-pair/first-match (distributivity broken).
- Test smell: COUNT(DISTINCT) fixtures with all-distinct values never exercise DISTINCT.

Round 3 (2026-10-14, condition.rs WherePlan: each WHERE step bound by one EXISTS at the LCA of its readers):
- LCA placement is not compositional: the same sub-expression changes meaning with context (NOT C alone = no node; `A AND NOT C` sharing a step = some node). Breaks §AND ("true when both operands true": `A AND NOT C` can keep a row `NOT C` rejects) and the §NOT De Morgan equivalence the spec itself states. Re-check any "bind at smallest subtree" design for NOT-boundary sensitivity.
- Aggregate queries still apply the row-level WHERE; when a step the aggregate walk fixes has its scope under a NOT, row-level is universal while item-level is per node, so COUNT/AVG lose items a projection keeps (systolics 120,160, WHERE NOT(sys<140): projection 1 row, COUNT 0).
- fn(path) operands never enter the step plan: first-match scalar, no sharing — book's any-match/shared claims overreach for them.

Round 4 (2026-10-14, condition.rs: ONE clause-wide binding set, `EXISTS(<bindings> WHERE <whole clause>)`):
- Verified sound: the whole formula (promoted/uid/EHR/VERSION scalars included) sits inside the one EXISTS, so per-assignment De Morgan/AND-monotone/distribution hold; adding a path never changes the projection of assignments (every parent gets >=1 child, NULL when none), so compositionality holds; aggregates' item filter (walk steps fixed) implies the row filter, and walk step_conds == WHERE binding conds.
- Live defect class: AQL `EXISTS` is per-assignment, so on a multi-step path `NOT EXISTS p` keeps a row where SOME branch lacks p even though p's data exists elsewhere — contradicts QUERY master03 §EXISTS ("data associated with the specified path exists") + §NOT. Single-step paths (the spec's own `c/content[...]` example) are fine.
- list_test (list fragment) is 2-valued (EXISTS over elements): `NOT (links = 'x')` is TRUE on an absent list while every other absent-path test is NULL.
- Flat-shape left_bind uses whole-row `unnest(ARRAY(SELECT n ...))`: whole-row Vars detoast `data` for every bound node (perf, not semantics).

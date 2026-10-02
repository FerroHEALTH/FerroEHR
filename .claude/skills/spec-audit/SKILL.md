---
name: spec-audit
description: >
  Audits an implemented subsystem, endpoint group, or diff against the
  vendored openEHR spec text and the CNF Platform Conformance Test Schedule
  (docs/specs/openehr/), producing a findings list of divergences with spec
  citations. Use when the user asks "are we following the spec", "audit X
  against the spec/CNF", before closing a spec-facing phase, or after a
  conformance failure.
argument-hint: "<subsystem | API group | crate path | 'current diff'>"
---

# /spec-audit

Systematic conformance audit: our code vs the normative text. The output is a
**findings list**, not fixes — fixing is a separate step the user directs.

## Procedure

1. **Scope.** Resolve the argument to (a) the code under audit (crate/module
   paths, or `git diff` for "current diff") and (b) the governing spec
   surfaces via the map in `docs/specs/openehr/README.md` (e.g. COMPOSITION
   endpoints → `RM/docs/ehr` + `RM/docs/common` + `ITS-REST` +
   `CNF/docs/platform_test_schedule/master07-func_tc_ehr_composition.adoc`).
2. **Build the requirements checklist from the spec side first** (never from
   the code): for each relevant spec section / CNF test case, extract the
   testable requirements — preconditions, status codes, headers, payload
   shapes, invariants, error behaviour. CNF test-case ids become checklist
   ids. For large scopes, fan this out per-chapter to subagents
   (`spec-researcher`), one chapter each, and merge.
3. **Walk the checklist against the code** (read handlers/service/tests;
   run targeted tests where cheap). Classify each item:
   - ✅ conformant (evidence: file:line or passing test)
   - ❌ divergent (what the spec says vs what we do)
   - ⚠️ unimplemented / not yet in scope (name the phase that owns it)
   - 📝 spec-silent → needs a `// NOTE:` decision
4. **Report**: findings ranked by severity (wire-visible divergence > missing
   behaviour > internal), each with the spec citation
   (`docs/specs/openehr/<file>` + heading, or CNF test-case id) and the code
   location. State coverage honestly at TWO levels: the spec chapters *not*
   audited, AND — within audited chapters — the specific behaviours/edge cases
   (status-code branches, conditional headers, negotiation variants, error
   families) that no CNF case yet exercises. An untested spec behaviour is a
   COVERAGE GAP finding in its own right (`.claude/rules/testing.md` §CNF
   coverage), not merely an omission from the report.
5. **Record**: for real divergences AND coverage gaps, offer to file them as
   tracker issues (`scripts/gh/fields.sh new <type> <priority> <effort> …`,
   `spec:*`/`spec:CNF`-labelled per `.claude/rules/issue-workflow.md`; link them into the relevant program with
   `scripts/gh/rel.sh`) — never silently fix-and-forget; for spec-silent
   findings, suggest the `// NOTE:` text.

## Rules

- The spec text + CNF cases are the oracle; EHRbase is prior art only — a
  finding is never "EHRbase does X" but "the spec says X (citation)".
- Never adjust a test or expectation to match the implementation (testing.md).
- Ambiguity is a finding, not a judgement call — surface it.

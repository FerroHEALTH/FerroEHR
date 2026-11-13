# Example Cedar policies

Illustrative embedded-Cedar policies for the FerroEHR ABAC layer
(no openEHR spec governs authorization — our own design; the shipped rules
live in `.claude/rules/auth.md`). Write a file of this directory (or your
own policies) as the `cedar-policies` admin register,
`PUT {ext}/admin/registers/cedar-policies` with `{"policies": "<the file>"}`,
to try the embedded engine; the write is refused unless the set validates.

`consent.cedar` shows the shape of a consent rule against the shipped schema —
a decision over subject and resource attributes in NIST SP 800-162's sense:

- **Principal** `User { organization?, patient?, roles, scopes }` — the caller.
  The uid is the authenticated subject (`User::"<sub>"`), and `roles`/`scopes`
  carry what the caller actually holds, so a policy can be written about a
  specific caller, a role, or a scope — not only about attributes.
- **Resource** (`Ehr`, `EhrStatus`, `Composition`, `Contribution`, `Query`,
  `Directory`) — carries the per-combination candidate `patient?` / `template?`.
- **Action** `"<kind>.<mode>"` — e.g. `composition.create`, `query.execute`.
- **Context** `{ operation_id: String }` — the generated operation id, so a rule
  can key on one operation rather than a whole family.

## Semantics to know

- Cedar is **deny-by-default**; a request is permitted only if some `permit`
  matches and no `forbid` matches (`forbid` overrides `permit`).
- Multiple `permit` policies are **OR-combined**, so conditions that must ALL
  hold belong in a **single** `permit` whose `when` carries them. Adding a second
  `permit` widens access; `forbid` is what narrows it, and it overrides every
  `permit`.
- Multi-valued attributes (contribution/query template sets, query patient sets)
  are fanned out by the engine as a cartesian product with **all-must-permit**;
  a request that touches nothing asks nothing and so is not denied, which the
  corpus test in `tests/it/authz_cedar_engine.rs` pins.
- A policy that **cannot be evaluated is not a decision.** Unsafe optional-
  attribute reads are refused at load by schema validation; anything that still
  errors at evaluation (arithmetic overflow, say) surfaces as a fail-closed
  `500`, never as a skipped policy — otherwise an erroring `forbid` would
  silently stop forbidding.

Replace the allow-list / consent bodies with your deployment's rule (or a set
backed by your own Cedar entity store).

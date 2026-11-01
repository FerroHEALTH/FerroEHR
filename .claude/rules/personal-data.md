---
paths: ["app/ferroehr/src/service/demographic/**", "app/ferroehr/src/service/linkage/**", "app/ferroehr/migrations/demographic/**", "app/ferroehr*/src/**/identifier_scan*", "app/ferroehr*/src/**/outbox*"]
---

# Personal data and the pseudonymisation boundary

The server keeps clinical content and the identities it belongs to in separate
schemas, reachable by separate database roles. The paths this rule is scoped to
are the boundary itself: the demographic schema, the services over it and the
crossings between the domains, the identifier scanner, and the outbox payload builders that carry data
out of the process. A change here is a change to what can be re-identified, and
GDPR Art. 25 makes that a design-time duty
(https://eur-lex.europa.eu/eli/reg/2016/679/oj). No openEHR spec governs any of
this: our own design.

## The four rules

- **Synthetic data only:** every test, fixture, seed, example and screenshot
  uses invented values. Never write a real name, national identifier, address,
  phone number, email or date of birth into the repository, a commit message, an
  issue or a pull request body. A synthetic value that still has the shape of a
  real identifier carries `privacy-allow: <reason>` on the same line, which is
  what exempts it from the guard.
- **No identifiers in telemetry:** `tracing` fields, span names, metric labels
  and `Debug` output carry record identifiers (an EHR id, a version uid, a
  template id) and shapes. They never carry a subject's own data, so a `Debug`
  impl on a type holding personal data prints field names rather than field
  values. This is the PHI caveat in `reliability.md`, stated for the boundary it
  matters most on.
- **No grant across the domains:** a database role reaches the clinical schemas
  or the demographic ones, never both. A migration that grants across the two
  rejoins the identities to the records the split exists to separate, so a new
  `GRANT` names the role it serves and stays inside one domain. Widening an
  existing role across the boundary is the same defect written differently.
- **A read of personal data emits an access event:** a new code path that reads
  demographic data, or resolves a party or subject to its EHRs, records who read what, through the access-event
  model. A read with no event is a boundary crossing nobody can reconstruct
  afterwards.

## Enforcement

`scripts/checks/privacy-boundary.sh`, run per-PR by the `privacy-boundary-guard`
CI job. It fails a diff touching these paths whose pull request body has no
ticked "Privacy boundary" checklist, and it reads the added lines of every diff
for a nine-digit value passing the BSN eleven-test or a Dutch postcode followed
by a house number. Its own detectors run under `--self-test` before it judges
anything, so the data checks are exercised rather than trusted.

The path list in the guard is a set of patterns matched against the diff, not an
assertion that the paths exist. Several of them are still being built, and the
guard fires the moment the first file appears at one of those names.

The first two rules are wider than any guard: no check can tell a real name from
an invented one, and none can read intent out of a `tracing` field. They are
review-enforced at the boundary, and this file is what a reviewer holds a change
to. Contributors read the same rules in `CONTRIBUTING.md` § Personal data.

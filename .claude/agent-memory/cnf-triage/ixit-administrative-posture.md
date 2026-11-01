---
name: ixit-administrative-posture
description: The role-boundary premise is a DECLARED ixit datum (AMB-228) — six `-forbidden` rows guard out unless our own party ixit spells `administrative`
metadata:
  type: project
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

From veredictum 0.1.4, a case whose premise is a role boundary states
`requires: instances: { sut: { administrative: false } }` and SELECTION checks
it against `instances.<name>.administrative` in the party ixit
(`schemas/ixit.schema.json`; "Absent => undeclared, never a default").
Undeclared or opposite ⇒ `not_applicable` — our own design, since nothing on
the wire discloses a principal's roles (SM
`docs/openehr_platform/master02-overview.adoc` §Functional Style says access
control is delegated to the implementation; reference text only) — register
AMB-228.

**Six** cases carry it: `I_ADMIN_ARCHIVE.archive_ehrs-forbidden`,
`I_ADMIN_ARCHIVE.archive_parties-forbidden`,
`I_ADMIN_DUMP_LOAD.export_ehrs-forbidden`,
`I_ADMIN_SERVICE.contribution_count-forbidden`,
`I_DEFINITION_ADL14.delete_archetype-clinical_forbidden`,
`I_DEFINITION_ADL2.delete_artefact-clinical_forbidden` (AMB-228's prose says
"five" — an editorial off-by-one).

**How to apply:** this is a fourth artifact class, outside the three bins — the
party ixit lives in THIS repo at
`docs/conformance/party/ferroehr/ixit.json`. Our deployment does run the
split (`sut` = USER, `admin` = ADMIN), so `"administrative": false` on `sut`
and `true` on `admin` is a true declaration, not an expectation adjustment,
and it restores the six rows. A guarded row here is a coverage loss to fix
locally, never a Veredictum defect.

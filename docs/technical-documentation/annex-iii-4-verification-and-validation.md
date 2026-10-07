# Annex III 4: verification and validation results

EHDS Annex III point 4 (`docs/law/eu/ehds/text.html`): "The results and
critical analyses of all verifications and validation tests undertaken to
demonstrate conformity of the EHR system with the requirements laid down in
Chapter III, in particular the applicable essential requirements." EHDS
Art. 37(2) adds "a reference to the results obtained from a European digital
testing environment referred to in Article 40". CRA Annex VII point 6
(`docs/law/eu/cra/text.html`) asks for "reports of the tests carried out to
verify the conformity of the product with digital elements and of the
vulnerability handling processes with the applicable essential cybersecurity
requirements as set out in Parts I and II of Annex I".

## What exists

- **The logging component's controls (Annex II 1.1 and 3).** The
  [hazard log](../../website/book/src/compliance/hazard-log.md) names, for each
  hazard, the tests that show its control holds, by file and test name; the
  Annex II 3.2 elements are asserted field by field in
  `app/ferroehr/tests/it/audit_ehds_mapping.rs`, including the gaps. The tests
  run on every pull request and their results are the CI runs of the release
  commit.
- **Annex II, row by row.** The
  [readiness page](../../website/book/src/compliance/ehds-readiness.md) gives
  the status and the evidence for each requirement, read against the text on
  the date it names. It is the critical analysis; it is the manufacturer's,
  and no authority has examined it.
- **The openEHR specifications.** The conformance record under
  `docs/conformance/ferroehr/` (the statement, the report and the per-case
  results of an independent instrument's runs), published on
  [`conformance.md`](../../website/book/src/conformance.md). It shows
  conformity to the openEHR specifications, which is not conformity to
  Annex II.
- **The CRA Annex I requirements.** The evidence and the status of each point
  of Part I(2) and Part II in the
  [risk assessment](../../website/book/src/compliance/cra-risk-assessment.md),
  and the security tests listed in
  [`cra-annex-vii-2-design-and-vulnerability-handling.md`](cra-annex-vii-2-design-and-vulnerability-handling.md).

## What is missing

- **The European digital testing environment (EHDS Art. 40).** Art. 40(3) has
  the manufacturer use it before placing an EHR system on the market and put
  the results here. The Commission has not published it, and its common
  specifications under Art. 40(4) have not been adopted. Running it against
  the logging component, and linking its results here, is #3620.
- **A test report per release.** The test results exist as CI runs and
  committed records, not as one report bound to each release. CRA Annex VII 6
  applies from 11 December 2027.

State: partial.

# CRA Annex VII 3: the cybersecurity risk assessment

CRA Annex VII point 3 (`docs/law/eu/cra/text.html`): "an assessment of the
cybersecurity risks against which the product with digital elements is
designed, developed, produced, delivered and maintained pursuant to Article
13, including how the essential cybersecurity requirements set out in Part I
of Annex I are applicable".

- **The assessment.**
  [`compliance/cra-risk-assessment.md`](../../website/book/src/compliance/cra-risk-assessment.md):
  the risk register on the intended purpose and the reasonably foreseeable use,
  each point of Annex I Part I(2) with whether it applies, how it is
  implemented, the evidence and the open issue, the justification for each
  requirement or limb that does not apply (CRA Art. 13(4)), the outbound data
  flows, and Part II.
- **The EHDS-side assessment.** CRA Art. 13(4), as EHDS Art. 104(1) replaces
  it, lets the cybersecurity risk assessment of a product within Art. 32(5a)
  "be part of the risk assessment required by those Union legal acts". The
  [hazard log](../../website/book/src/compliance/hazard-log.md) assesses the
  patient-safety hazards of the logging component under EHDS Annex II 1.1.
- **The basis.** The
  [intended-purpose statement](../../website/book/src/compliance/intended-purpose.md)
  and the [threat model](../../website/book/src/threat-model.md).

Revisions: the assessment's own revision table; it is revised with every
release that touches an asset or a control in its register, when a
vulnerability or incident shows a risk it does not name, and when the
intended purpose changes (CRA Art. 13(3) and (7)).

State: available. Each open point of Annex I it records names its tracker
issue.

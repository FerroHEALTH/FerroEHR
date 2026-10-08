# Shared responsibility

Which obligation under EU and national health-data law the software can
carry, and which stays with the organisation that deploys it, is drawn once
for the EHR system FerroEHR and FerroBRIDGE form together. The tables, one
per law (the GDPR, the EHDS, the CRA, and the national sections for the
Netherlands, Germany and Switzerland), and the account of when Cadasto B.V. is
also your processor, are published in the FerroHEALTH book:
[The EHDS EHR system: shared responsibility](https://ferrohealth.eu/docs/ehds/shared-responsibility.html).

This page keeps what is particular to a FerroEHR deployment.

## What a FerroEHR deployment sends out by default

The one thing a FerroEHR deployment sends out by default is the
[usage report](../usage-report.md): a random instance id, the version, the
licence grant type and coarse performance figures, with no patient data.
Cadasto B.V. receives it as controller for its own purposes, and its
collector also stores the IP address each report comes from. The usage report
page says how to turn it off.

## Where FerroEHR documents its side

The middle column of each table in the FerroHEALTH book, what the software
provides, points at FerroEHR's own pages:

- **[Compliance overview](index.md):** the legal sources, and what FerroEHR
  ships, act by act and country by country, with the tracker issue for each
  control.
- **[Control matrix](control-matrix.md):** the live status of every declared
  control, generated from the tracker.
- **[Security](../security.md)** and the **[threat model](../threat-model.md):**
  how each control is configured, and the risk that survives it.
- **[Audit trail](../audit.md):** what the access log records and how to read
  it back.
- **[Retention, restriction and objection](retention.md):** the retention
  register, the restriction of processing and the research objection.
- **[Data protection impact assessment](../security/dpia.md)**,
  **[records of processing](../security/records-of-processing.md)** and the
  **[go-live checklist](../security/go-live-checklist.md):** the companion
  guidance for the controller's own documents.
- **[Intended purpose](intended-purpose.md#the-security-environment-ferroehr-assumes):**
  the security environment FerroEHR assumes the deployment provides.

None of these pages tells you whether your deployment satisfies an
obligation. That depends on your legal basis, your organisation, your
infrastructure and your operating practice, none of which a supplier can see.

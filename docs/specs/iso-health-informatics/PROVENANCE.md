# ISO 27789 and ISO 22600: not vendored

**Nothing but this record is in this directory, and nothing else may be.**
ISO sells these standards under copyright. They are not fetched, not
vendored, not quoted and not paraphrased clause by clause here.

| Standard | Title | Edition | Publisher's page |
|---|---|---|---|
| ISO 27789:2021 | Health informatics — Audit trails for electronic health records | 2, October 2021, replacing ISO 27789:2013 | <https://www.iso.org/standard/75313.html> |
| ISO 22600-1:2014 | Health informatics — Privilege management and access control — Part 1: Overview and policy management | 1, 2014, confirmed 2020 | <https://www.iso.org/standard/62653.html> |
| ISO 22600-2:2014 | Health informatics — Privilege management and access control — Part 2: Formal models | 1, 2014, replacing ISO/TS 22600-2:2006 | <https://www.iso.org/standard/62654.html> |
| ISO 22600-3:2014 | Health informatics — Privilege management and access control — Part 3: Implementations | 1, 2014, replacing ISO/TS 22600-3:2009 | the ISO catalogue, under ISO/TC 215 |

Recorded 2026-11-07 by `scripts/vendor/iso-health-informatics.sh`. The
iso.org pages answer an automated client with `403`; the editions above were
read from the national standards bodies' catalogue entries (EVS, Estonia, for
EN ISO 27789:2021, valid from 2021-11-01; BSI Knowledge for ISO 22600-1 and
-3) and the ISO committee pages for ISO 22600-1 and -2.

## What the access log design takes from them

Only what the publishers' own public summaries state, cited by standard and
edition:

- ISO 27789:2021, scope (the public abstract): a common framework of audit
  trigger events and audit data for EHRs, for systems that create a secure
  audit record whenever a user reads, creates, updates or archives personal
  health information; each record identifies at minimum the user, the subject
  of care, the function performed and its date and time; the scope is limited
  to actions on the EHR under the access policy of its domain, and excludes
  system management and security audit logs. Annex A gives audit scenarios and
  Annex B an overview of audit log services. A revision (ISO/PRF 27789) is in
  preparation.
- ISO 22600-1:2014, scope (the public abstract): privilege management and
  access control across policy domains, with a template for documenting the
  policy agreement between domains so that parties can compare them.
- ISO 22600-2:2014, scope (the public abstract): formal models built on
  ISO/IEC 10746, among them the policy, role, authorization and delegation
  models.

## Why the design still works without the text

Every requirement the design rests on is also stated by a source this
repository vendors: EHDS Annex II 3.2 for the content of a record, DICOM
PS3.15 A.5 and RFC 3881 (both cache only) and IHE BALP and FHIR AuditEvent
for the structure. ISO 27789 is named as the standard those messages trace
to, and ISO 22600 as the model of the policy and the delegation a record
names. What either standard requires beyond its public summary is checkable
only against the standard, by someone who holds it.

Do not add files to this directory. Re-run
`scripts/vendor/iso-health-informatics.sh` to rewrite this record.

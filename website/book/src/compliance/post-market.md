# Complaints, incidents and vulnerabilities

Cadasto B.V. is the manufacturer of every FerroHEALTH product, FerroEHR
included, and runs one post-market procedure for all of them: complaints,
serious incidents under the EHDS, actively exploited vulnerabilities and
severe incidents under the CRA, corrective action, withdrawal and recall,
security advisories, vulnerabilities in integrated components, cooperation
with the authorities and the cessation of operations. That procedure, with
its quotations from the regulations, is published in the FerroHEALTH book:
[Post-market](https://ferrohealth.eu/docs/post-market.html). The
manufacturer's name, address and single point of contact are on
[The manufacturer](https://ferrohealth.eu/docs/manufacturer.html).

This page keeps what is particular to FerroEHR.

## Reporting about FerroEHR

- **A vulnerability**, including one you have seen exploited: report it
  privately through
  [GitHub private vulnerability reporting][pvr] on the FerroEHR repository, or
  by email to [info@cadasto.com](mailto:info@cadasto.com) with "FerroEHR
  vulnerability" in the subject, as
  [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#reporting-a-vulnerability)
  says. If you have seen it exploited, say so.
- **Anything that harmed a person, or could have:** write to
  [info@cadasto.com](mailto:info@cadasto.com) with "FerroEHR incident" in the
  subject.
- **Any other complaint:** write to [info@cadasto.com](mailto:info@cadasto.com)
  with "FerroEHR complaint" in the subject, or open a
  [GitHub issue](https://github.com/FerroHEALTH/FerroEHR/issues/new/choose)
  when the report can be public.

Say which version you run (`ferroehr --version`, or the image tag) and how it
is deployed. Never send patient data: describe the case, or build a synthetic
one. A deployment attaches the JSON document `ferroehr report` writes: it
names the version, commit, features, specification pins, migration level,
deployment posture and redacted configuration, and carries no credential and
no patient data ([the command](../installation/config-cli.md#ferroehr-report)).

## FerroEHR's registers

The registers of EHDS Art. 30(1)(o) and the upstream register of CRA
Art. 13(6) are kept per product. FerroEHR's are public, in its repository:

- [`docs/registers/complaints.tsv`][complaints]: every complaint about
  FerroEHR, whatever the channel, with the versions it concerns, how it was
  classified, the record of the work on it and its outcome.
- [`docs/registers/non-conforming-versions.tsv`][non-conforming]: every
  finding that released FerroEHR versions do not conform, with the
  requirement missed, whether it was a serious incident or notified under the
  CRA, the corrective action, and when the authorities and the users were
  told.
- [`docs/registers/upstream-reports.tsv`][upstream]: every vulnerability
  reported to the maintainer of a component FerroEHR integrates, with the
  channel, the upstream reference, whether a fix was shared and the outcome.

No row names the person who complained or carries patient data, and the
upstream register names components, never people. A vulnerability enters the
registers when its advisory is published.

## Possible serious incidents in FerroEHR

The manufacturer treats these FerroEHR events as possible serious incidents
under EHDS Art. 2(2)(r): a committed version lost or attributed to the wrong
EHR; a read or an AQL result that returns another patient's data; data under a
[restriction of processing](retention.md#restriction-of-processing) served on
a path that should refuse it; an access the [audit trail](../audit.md) did not
record; a signature verification that accepts a tampered version.

## A withdrawn FerroEHR release

A published FerroEHR release is immutable, so a non-conforming version is
never changed or deleted. The fix ships in a new patch release. A withdrawn
version's floating image tags (`<major>.<minor>` and `latest` of the server,
viewer and PostgreSQL images) move to the correcting release, and the version
is listed as unsupported in
[`SECURITY.md` § Supported versions][supported]. Its own version tag, its
image digests and its chart version stay published, so a deployment pinned to
any of them keeps running the withdrawn version until the deployment moves.

Security advisories for FerroEHR are GitHub security advisories on the
FerroEHR repository, and the `### Security` entry of its changelog carries
each advisory's `GHSA-` identifier.

The maintainers' step-by-step checklist for FerroEHR, with the release tooling
it runs, is
[`docs/post-market.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/post-market.md).
The [technical documentation readiness](technical-documentation.md) page shows
what of the documentation an authority may ask for exists today.

[pvr]: https://github.com/FerroHEALTH/FerroEHR/security/advisories/new
[complaints]: https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/registers/complaints.tsv
[non-conforming]: https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/registers/non-conforming-versions.tsv
[supported]: https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions
[upstream]: https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/registers/upstream-reports.tsv

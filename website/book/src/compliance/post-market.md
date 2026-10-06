# Complaints, incidents and vulnerabilities

This page says how to complain about FerroEHR, how to report a vulnerability
or an incident, what its manufacturer records, and what it does when a
released version turns out not to conform. It describes the manufacturer's
procedures under two regulations: Regulation (EU) 2025/327 on the European
Health Data Space (the EHDS), Articles 30, 43, 44 and 45, and Regulation (EU)
2024/2847, the Cyber Resilience Act (the CRA), Articles 13 and 14. The
maintainers' step-by-step checklist for the same procedures is
[`docs/post-market.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/post-market.md).

This page is not legal advice, and it says nothing about whether FerroEHR or
a deployment meets either regulation. It says what the manufacturer does.

<!-- toc -->

> [!WARNING]
> Every quotation on this page is from the Official Journal texts vendored in
> the repository under
> [`docs/law/eu/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/docs/law/eu):
> `ehds/text.html` for the
> [EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) and `cra/text.html`
> for the [CRA](https://eur-lex.europa.eu/eli/reg/2024/2847/oj). Check the
> publisher before you rely on a statement here.

## When these duties apply

The CRA's reporting duty in Article 14 applies from 11 September 2026 (CRA
Art. 71(2)), and Article 69(3) extends it to "all products with digital
elements that fall within the scope of this Regulation that have been placed
on the market before 11 December 2027", so it covers every FerroEHR release.
The rest of the CRA, including the corrective measures of Article 13(21),
applies from 11 December 2027. The EHDS applies from 26 March 2027 (EHDS
Art. 105), and Articles 30, 43, 44 and 45 are not among the provisions it
gives a later date. The procedures below run now for all of them.

## The manufacturer

Cadasto B.V. is the manufacturer of each tagged FerroEHR release (the
binaries, the container images, the Helm chart and the source archive). EHDS
Art. 30(1)(g) asks for the manufacturer's name, postal address and digital
contact details, with "a single point at which the manufacturer can be
contacted":

| | |
|---|---|
| Name | Cadasto B.V. |
| Postal address | Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands |
| Single point of contact | [info@cadasto.com](mailto:info@cadasto.com) |
| Website | <https://www.cadasto.com/contact/> |

The technical side, security reports and releases, is handled by the
maintainer, Ruben Talstra.

## Making a complaint or a report

EHDS Art. 30(1)(n) asks the manufacturer to "establish channels of complaint
and keep distributors informed thereof". Pick the channel by what the report
contains:

- **A vulnerability, including one you have seen exploited:** report it
  privately through
  [GitHub private vulnerability reporting][pvr],
  as [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md)
  says. If you have seen it exploited, say so: that starts the CRA clock
  described [below](#actively-exploited-vulnerabilities-and-severe-incidents-cra-art-14).
- **Anything that harmed a person, or could have:** write to
  [info@cadasto.com](mailto:info@cadasto.com) with "FerroEHR incident" in the
  subject, so it is handled as a possible serious incident from the first
  hour.
- **Any other complaint:** write to [info@cadasto.com](mailto:info@cadasto.com)
  with "FerroEHR complaint" in the subject, or open a
  [GitHub issue](https://github.com/FerroHEALTH/FerroEHR/issues/new/choose)
  when the report can be public.

Say which version you run (`ferroehr --version`, or the image tag) and how it
is deployed, and describe what happened. Never send patient data: describe
the case, or build a synthetic one.

## The registers

EHDS Art. 30(1)(o) asks the manufacturer to "keep a register of complaints
and a register of non-conforming EHR systems and keep distributors informed
thereof". Both are kept in the repository, where anyone can read them:

- [`docs/registers/complaints.tsv`][complaints]:
  every complaint, whatever the channel, with the versions it concerns, how
  it was classified, the record of the work on it and its outcome.
- [`docs/registers/non-conforming-versions.tsv`][non-conforming]:
  every finding that released versions do not conform, with the requirement
  missed, whether it was a serious incident or notified under the CRA, the
  corrective action, and when the authorities and the users were told.

No row names the person who complained or carries patient data. A
vulnerability enters the registers when its advisory is published, so a row
never discloses one before its fix.

## Corrective action, withdrawal and recall

When the manufacturer considers, or has reason to believe, that released
versions "are not or are no longer in conformity with the essential
requirements laid down in Annex II", it takes "without undue delay any
necessary corrective action", or recalls or withdraws them (EHDS
Art. 30(1)(i)). CRA Art. 13(21) asks for the same "immediately" where a
product or the manufacturer's processes do not conform with the CRA's Annex
I. The steps:

1. The finding enters the register of non-conforming versions.
2. The fix ships in a new patch release. A published FerroEHR release is
   immutable, so a non-conforming version is never changed or deleted.
3. A withdrawn version's floating image tags (`<major>.<minor>` and `latest`
   of the server, viewer and PostgreSQL images) move to the correcting
   release, and the version is listed as unsupported in
   [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions).
   Its own version tag, its image digests and its chart version stay
   published, so a deployment pinned to any of them keeps running the
   withdrawn version until the deployment moves.
4. The national authorities of each Member State where the version was made
   available or put into service are told of the non-conformity, of the
   corrective action "including the timetable for implementation", and of the
   date the version was "brought into conformity or been recalled or
   withdrawn" (EHDS Art. 30(1)(i)).
5. Distributors, the authorised representative, importers and users are told
   of "the non-conformity and of any corrective action, recall or withdrawal"
   (EHDS Art. 30(1)(j)): through a security advisory or an announcement, the
   release notes of the correcting version, and directly where the
   manufacturer knows them.

The same routes carry any "mandatory preventive maintenance of the EHR
systems and its frequency" (EHDS Art. 30(1)(k)).

## Serious incidents (EHDS Art. 44)

EHDS Art. 2(2)(r) defines a serious incident as "any malfunction or
deterioration in the characteristics or performance of an EHR system made
available on the market that directly or indirectly leads, might have led or
might lead to any of the following: (i) the death of a natural person or
serious harm to a natural person’s health; (ii) serious prejudice to a
natural person’s rights; (iii) serious disruption of the management and
operation of critical infrastructure in the health sector".

For a clinical data repository, the manufacturer treats these as possible
serious incidents: a committed version lost or attributed to the wrong EHR; a
read or an AQL result that returns another patient's data; data under a
[restriction of processing](retention.md#restriction-of-processing) served on
a path that should refuse it; an access the [audit trail](../audit.md) did not
record; a signature verification that accepts a tampered version.

EHDS Art. 44(7) sets the report:

- **To whom:** "the market surveillance authorities of the Member States
  where such serious incident occurred and of the Member States where such
  EHR systems are placed on the market or put into service". Each Member
  State designates its authority, and "The Commission and the Member States
  shall make that information publicly available" (Art. 43(2)).
- **What:** the incident, including "a description of the corrective action
  taken or envisaged by the manufacturer".
- **When:** "immediately after the manufacturer has established a causal link
  between the EHR system and the serious incident or the reasonable
  likelihood of such a link and, in any event, not later than three days
  after the manufacturer becomes aware of the serious incident involving the
  EHR system". The three days run from awareness, so the report goes in on
  time while the cause is still being established and is completed later.

Where an authority finds that an EHR system "has caused harm to the health or
safety of natural persons", the manufacturer "shall immediately provide
information and documentation" to the affected person or user (Art. 44(3)).
Where a serious incident concerns personal data protection, the market
surveillance authority informs the data protection supervisory authorities
(Art. 44(6)); a deployment's own duties under the GDPR remain its own.

A deployment attaches the JSON document `ferroehr report` writes to such a
report: it names the version, commit, features, specification pins, migration
level, deployment posture and redacted configuration, and carries no
credential and no patient data ([the command](../installation/config-cli.md#ferroehr-report)).

## Actively exploited vulnerabilities and severe incidents (CRA Art. 14)

CRA Art. 14(1) has the manufacturer "notify any actively exploited
vulnerability contained in the product with digital elements that it becomes
aware of", and Art. 14(3) "any severe incident having an impact on the
security of the product with digital elements". An actively exploited
vulnerability is "a vulnerability for which there is reliable evidence that a
malicious actor has exploited it in a system without permission of the
system owner" (CRA Art. 3(42)).

The notification goes "via the single reporting platform" that ENISA runs,
to "the CSIRT designated as coordinator of the Member State where the
manufacturers have their main establishment in the Union", and is
"simultaneously accessible to ENISA" (Art. 14(7)). For Cadasto B.V. that is
the Netherlands. It comes in three steps, each counted from when the
manufacturer becomes aware:

| Step | Deadline | Article |
|---|---|---|
| Early warning | "within 24 hours" | Art. 14(2)(a), 14(4)(a) |
| Vulnerability or incident notification | "within 72 hours" | Art. 14(2)(b), 14(4)(b) |
| Final report | a vulnerability: "no later than 14 days after a corrective or mitigating measure is available"; an incident: "within one month after the submission of the incident notification" | Art. 14(2)(c), 14(4)(c) |

The CSIRT may delay passing a notification on to other CSIRTs on "justified
cybersecurity-related grounds" (Art. 16(2)), on the terms of Commission
Delegated Regulation (EU) 2026/881; one of them is that the manufacturer has
said a fix is expected within 72 hours (its Art. 3(a)). Users are told of the
vulnerability or incident and of the measures they can take (Art. 14(8))
through a GitHub security advisory on the repository and the release notes
of the fixing release. [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md)
carries the same account for a reporter.

An event can be an EHDS serious incident and a CRA notification at once. Each
report is then made, each on its own clock.

## NIS2

EHDS Art. 44(7) makes the serious-incident report "without prejudice to
incident notification requirements under Directive (EU) 2022/2555", the NIS2
Directive. NIS2 Art. 23 has each Member State require essential and
important entities to notify significant incidents to their CSIRT or
competent authority, and healthcare providers are a sector in its Annex I. A
hospital running FerroEHR may owe that notification under its own national
law. The manufacturer's EHDS report and CRA notification do not stand in for
it, and the hospital's notification does not stand in for them. The
manufacturer gives the hospital the facts it needs: the advisory, the
affected versions and the mitigations.

## The support period

The support period of each release is five years from the month it is
published, and the security update for a release in its support period ships
in the newest release; there are no backports. A withdrawn release is
unsupported. [`SECURITY.md` § Supported versions][supported]
has the full statement, the CRA articles it rests on, and the list of
withdrawn releases.

## Cooperation with the authorities

On request, the manufacturer gives a market surveillance authority "all the
information and documentation necessary to demonstrate the conformity" of
FerroEHR, in an official language of the Member State concerned (EHDS
Art. 30(1)(l)), "in paper or electronic form" and "in a language which can be
easily understood by that market surveillance authority" (EHDS Art. 30(5);
CRA Art. 13(22)), and cooperates on any action to bring FerroEHR into
conformity or to eliminate its risks (EHDS Art. 30(1)(m), Art. 44(1)). A
request goes to the single point of contact above. An authority may restrict,
recall or withdraw an EHR system whose manufacturer does not cooperate or
whose information "is incomplete or incorrect" (EHDS Art. 43(5)). The
[technical documentation readiness](technical-documentation.md) page shows
what of that documentation exists today.

[pvr]: https://github.com/FerroHEALTH/FerroEHR/security/advisories/new
[complaints]: https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/registers/complaints.tsv
[non-conforming]: https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/registers/non-conforming-versions.tsv
[supported]: https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions

# Cyber Resilience Act

The Cyber Resilience Act (CRA, Regulation (EU) 2024/2847) sets cybersecurity
requirements for "a software or hardware product and its remote data
processing solutions" (CRA Art. 3(1)) made available on the EU market, and
duties for the manufacturer that places it there. This page states FerroEHR's
position under it: who the manufacturer is, which duties apply from when, how
conformity will be assessed, and which questions are open.

<!-- toc -->

> [!WARNING]
> The quotations are from the Official Journal text vendored in the repository
> at
> [`docs/law/eu/cra/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/cra/text.html)
> ([EUR-Lex](https://eur-lex.europa.eu/eli/reg/2024/2847/oj)). The amendments
> EHDS Article 104 makes to the CRA are read from
> [`docs/law/eu/ehds/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/ehds/text.html).
> No conformity assessment has been carried out, no EU declaration of
> conformity exists and FerroEHR carries no CE marking. This page is the
> manufacturer's reading, not legal advice.

## The manufacturer and the product

Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, is the
manufacturer of each tagged FerroEHR release in the sense of CRA Art. 3(13):
the person who "develops or manufactures products with digital elements \[…\]
and markets them under its name or trademark, whether for payment,
monetisation or free of charge". Each tagged release is one product: the
`ferroehr` binaries, the `ferroehr`, `ferroehr-viewer` and `ferroehr-postgres`
container images, the Helm chart and the source archive. The nine `openehr-*`
crates published on crates.io under Apache-2.0 are products of their own and
are not covered on this page.

Its single point of contact (CRA Art. 13(17)) is
[info@cadasto.com](mailto:info@cadasto.com), beside GitHub private
vulnerability reporting ([`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#reporting-a-vulnerability)).

## Not free and open-source software

CRA Art. 3(48) defines free and open-source software as software "made
available under a free and open-source licence which provides for all rights
to make it freely accessible, usable, modifiable and redistributable".
FerroEHR is published under the Business Source License 1.1, whose Additional
Use Grant allows production use for non-commercial purposes only; any other
production use, including the delivery of health care, needs a commercial
licence from Cadasto B.V. ([licensing](../licensing.md)). The licence does not
provide for all of those rights, so FerroEHR is not free and open-source
software under the CRA. The provisions written for such software do not apply
to it: the open-source software steward regime of CRA Art. 24, and the option
of CRA Art. 32(5) to use the internal control procedure for an important
product whose technical documentation is public.

Each version becomes available under the Apache License 2.0 four years after
its publication (the Change Date in `LICENSE`). The position above is about
each release as Cadasto B.V. places it on the market.

## What applies, and from when

| Duty | Applies from | Text |
|---|---|---|
| Report actively exploited vulnerabilities and severe incidents (Art. 14) | 11 September 2026, for every release, including those published before 11 December 2027 | Art. 71(2); Art. 69(3) |
| The essential requirements of Annex I, the manufacturer's obligations of Art. 13, technical documentation, conformity assessment, the declaration and the CE marking | 11 December 2027 | Art. 71(2) |
| A product placed on the market before 11 December 2027 | the requirements reach it only "if, from that date, those products are subject to a substantial modification" | Art. 69(2) |

FerroEHR ships a new release often, and each tagged release is placed on the
market when it is published, so the releases published from 11 December 2027
are the ones the full Regulation measures.

## Reporting (Art. 14)

An actively exploited vulnerability or a severe incident having an impact on
FerroEHR's security is notified by Cadasto B.V. to the CSIRT designated as
coordinator and to ENISA through the single reporting platform: an early
warning within 24 hours of becoming aware, a notification within 72 hours,
and a final report (CRA Art. 14(2) and (4)). The procedure, and how users are
told, is in [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#what-cadasto-bv-reports-to-the-authorities-cra-art-14)
and on the [Complaints, incidents and vulnerabilities](post-market.md) page.

## The support period

The support period of each release is five years from the month it is
published. CRA Art. 13(8) sets "at least five years" and has the period
reflect "the length of time during which the product is expected to be in
use"; a clinical data repository is expected to be in use for longer. The end
date, month and year, is printed in each release's notes (Art. 13(19)). The
security update for a release in its support period ships in the newest
release, as Art. 13(10) allows where users of earlier versions can take the
newest one free of charge. The reasoning is recorded in the technical
documentation
([`cra-annex-vii-4-support-period.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/technical-documentation/cra-annex-vii-4-support-period.md)),
and the release-by-release table is in
[`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions).

## How conformity will be assessed

Cadasto B.V. declares FerroEHR, with FerroBRIDGE, as an EHR system under the
EHDS (Regulation (EU) 2025/327). EHDS Article 104 amends the CRA for that
case:

- **The procedure.** CRA Art. 32(5a), inserted by EHDS Art. 104(3):
  "Manufacturers of products with digital elements that are classified as EHR
  systems under Regulation (EU) 2025/327 \[…\] shall demonstrate conformity
  with the essential requirements set out in Annex I to this Regulation using
  the relevant conformity assessment procedure provided for in Chapter III of
  Regulation (EU) 2025/327."
- **One set of technical documentation.** CRA Art. 31(3), as EHDS Art. 104(2)
  replaces it: "a single set of technical documentation shall be drawn up
  containing the information referred to in Annex VII and the information
  required by those Union legal acts". It is kept in the repository
  ([technical documentation](technical-documentation.md)).
- **One risk assessment.** CRA Art. 13(4), as EHDS Art. 104(1) replaces it:
  for such a product, "the cybersecurity risk assessment may be part of the
  risk assessment required by those Union legal acts". The
  [CRA risk assessment](cra-risk-assessment.md) and the
  [hazard log](hazard-log.md) are its two parts.
- **One declaration.** CRA Art. 28(3): "Where a product with digital elements
  is subject to more than one Union legal act requiring an EU declaration of
  conformity, a single EU declaration of conformity shall be drawn up". No
  declaration has been drawn up yet.

The CRA requirements are walked point by point in the
[CRA risk assessment](cra-risk-assessment.md); the open points are
issues on the tracker,
among them secure decommissioning
([#3642](https://github.com/FerroHEALTH/FerroEHR/issues/3642)), the Annex II
user information per release
([#3650](https://github.com/FerroHEALTH/FerroEHR/issues/3650)) and a release
gate over security-relevant paths
([#3649](https://github.com/FerroHEALTH/FerroEHR/issues/3649)).

## Default product, or important product of class I

CRA Art. 7(1): products "which have the core functionality of a product
category set out in Annex III shall be considered to be important products
with digital elements", and "the integration of a product with digital
elements which has the core functionality of a product category set out in
Annex III shall not in itself render the product in which it is integrated
subject to the conformity assessment procedures referred to in Article
32(2) and (3)". Implementing Regulation (EU) 2025/2392 gives each category's
technical description.

- **Cadasto B.V.'s reading: a default product.** FerroEHR's core functionality
  is storing, versioning and querying health records
  ([intended purpose](intended-purpose.md)). None of the categories of Annex
  III names that function.
- **The competing reading: class I, point 1.** Annex III class I point 1 is
  "identity management systems and privileged access management software and
  hardware, including authentication and access control readers", and the
  technical description in Implementing Regulation (EU) 2025/2392 Annex I
  covers products "that provide mechanisms for authentication or
  authorisation", including "access management systems that control access of
  natural persons, legal persons, devices or systems to digital resources".
  FerroEHR authenticates and authorises every request to the records it holds.
  Read that way, the authentication and authorisation are a core function, not
  only an integrated one. Class I point 7, security information and event
  management, describes products that "collect data from multiple sources";
  FerroEHR's access log records FerroEHR's own accesses.
- **What the answer changes.** For a class I product whose manufacturer has not
  applied harmonised standards, common specifications or a certification
  scheme in full, CRA Art. 32(2) requires a third-party procedure (module B
  with C, or module H). Whether Art. 32(5a) displaces that for an EHR system
  is part of the same question.

This is a question for counsel, and nothing in this repository answers it.

## A deployment Cadasto B.V. hosts as a service

Cadasto B.V. may host FerroEHR as a service for other organisations. The CRA
reaches "remote data processing solutions", which recital 11 describes as
"data processing at a distance for which the software is designed and
developed by or on behalf of the manufacturer \[…\], the absence of which
would prevent the product with digital elements from performing one of its
functions". Recital 12 adds that "Directive (EU) 2022/2555 applies to cloud
computing services and cloud service models, such as Software as a Service
(SaaS)", and that cloud solutions are remote data processing solutions "only
if they meet the definition laid down in this Regulation". The text does not
say how a hosted deployment of a self-hostable product is classified. Whether
such a deployment is inside the CRA's product scope, and whether Cadasto B.V.
is then an essential or important entity under NIS2, are questions for
counsel.

In a hosted deployment Cadasto B.V. also operates the instance and is the
customer's processor under a GDPR Art. 28 contract
([shared responsibility](shared-responsibility.md)). Cadasto B.V. holds ISO
9001, ISO/IEC 27001 and NEN 7510 certification for its management system;
whether the certified scope covers a hosted FerroEHR service is an open owner
fact, and a certificate never covers the product itself.

## Who the CRA binds in a self-hosted deployment

The manufacturer's duties are Cadasto B.V.'s. An organisation that runs an
unmodified release for itself is a user of the product and not its
manufacturer. CRA Art. 22(1) makes a person "that carries out a substantial
modification of a product with digital elements and makes that product
available on the market" a manufacturer; whether an organisation that modifies
the source and only runs the result for itself is reached is a question for
counsel, as is the corresponding EHDS question.

## Questions for counsel

1. Whether FerroEHR is an important product of class I under Annex III and
   Implementing Regulation (EU) 2025/2392, and whether CRA Art. 32(5a) changes
   the procedure for it.
2. Whether CRA Art. 13(10) holds for commercial licensees under BUSL-1.1, and
   whether security updates are free of charge for every commercial licence
   (Annex I Part II(8)) ([#3647](https://github.com/FerroHEALTH/FerroEHR/issues/3647)).
3. Whether a deployment that modifies the source and puts the result into
   service becomes a manufacturer itself (CRA Art. 21 and 22, EHDS Art. 34).
4. Whether a deployment Cadasto B.V. hosts as a service is remote data
   processing outside the CRA's product scope, and whether Cadasto B.V. is then
   an essential or important entity under NIS2.

## Related

- [CRA risk assessment](cra-risk-assessment.md): Annex I, point by point.
- [Technical documentation](technical-documentation.md): the one set for the
  EHDS and the CRA.
- [Complaints, incidents and vulnerabilities](post-market.md): the reporting
  channels and procedures.
- [EHDS readiness](ehds-readiness.md): the EHR system the CRA route runs
  through.

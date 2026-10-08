# Cyber Resilience Act

The manufacturer's side of the Cyber Resilience Act (CRA, Regulation (EU)
2024/2847) is the same for every FerroHEALTH product: who the manufacturer
is, which duties apply from when, the reporting of actively exploited
vulnerabilities and severe incidents, the support-period policy, how
conformity is assessed through the EHDS, a deployment Cadasto B.V. hosts as a
service, who the CRA binds in a self-hosted deployment, and the questions for
counsel. That account is published in the FerroHEALTH book:
[The CRA](https://ferrohealth.eu/docs/cra.html).

Each FerroHEALTH product is a product with digital elements of its own, with
its own risk assessment and its own Annex II information. This page keeps
FerroEHR's.

> [!WARNING]
> No conformity assessment has been carried out, no EU declaration of
> conformity exists and FerroEHR carries no CE marking. The CRA text this page
> cites is vendored at
> [`docs/law/eu/cra/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/cra/text.html)
> ([EUR-Lex](https://eur-lex.europa.eu/eli/reg/2024/2847/oj)).

## The manufacturer and the product

Cadasto B.V. is the manufacturer of each tagged FerroEHR release
([The manufacturer](https://ferrohealth.eu/docs/manufacturer.html)). Each
tagged release is one product: the `ferroehr` binaries, the `ferroehr`,
`ferroehr-viewer` and `ferroehr-postgres` container images, the Helm chart and
the source archive. The nine `openehr-*` crates published on crates.io are
products of their own; the information for their integrators is on
[Rust crates](../crates.md#information-for-integrators-cra-annex-ii-point-8f).

Vulnerabilities in FerroEHR are reported as
[`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#reporting-a-vulnerability)
says.

## FerroEHR's CRA documents

- **[CRA risk assessment](cra-risk-assessment.md):** the cybersecurity risks
  of FerroEHR, and how each point of CRA Annex I applies to it, with its
  evidence and its open work.
- **[CRA information and instructions to the user](cra-user-information.md):**
  the Annex II information for FerroEHR, point by point. The notes of each
  GitHub release link the copy of that page frozen for the release, and the
  release lane refuses to publish a release whose notes do not.
- **[Technical documentation](technical-documentation.md):** the single set
  for the EHDS and the CRA (CRA Art. 31(3) as EHDS Art. 104(2) replaces it),
  kept in the repository.
- **[Hazard log](hazard-log.md):** the patient-safety hazards of the access
  log, the EHDS part of the risk assessment.

## FerroEHR's product facts under the CRA

- **Not free and open-source software.** FerroEHR is published under the
  Business Source License 1.1, whose Additional Use Grant allows production
  use for non-commercial purposes only, so it is not free and open-source
  software in the sense of CRA Art. 3(48) ([licensing](../licensing.md)).
- **The support period** of each release is five years from the month it is
  published. The end date, month and year, is printed in each release's notes,
  and the release-by-release statement is
  [`SECURITY.md` § Supported versions](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions).
  The reasoning is in the technical documentation
  ([`cra-annex-vii-4-support-period.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/technical-documentation/cra-annex-vii-4-support-period.md)).
- **The conformity route.** Cadasto B.V. declares FerroEHR, with FerroBRIDGE,
  as an EHR system under the EHDS, so the CRA requirements are assessed through
  the EHDS procedure (CRA Art. 32(5a), inserted by EHDS Art. 104(3)). The
  status per EHDS requirement is on [EHDS readiness](ehds-readiness.md).

## Default product, or important product of class I

Cadasto B.V. reads FerroEHR as a default product: its core functionality is
storing, versioning and querying health records
([intended purpose](intended-purpose.md)), and none of the categories of CRA
Annex III names that function. The competing reading is Annex III class I
point 1, "identity management systems and privileged access management
software and hardware, including authentication and access control readers":
the technical description in Implementing Regulation (EU) 2025/2392 Annex I
covers products "that provide mechanisms for authentication or
authorisation", and FerroEHR authenticates and authorises every request to the
records it holds. Class I point 7, security information and event management,
describes products that "collect data from multiple sources"; FerroEHR's
access log records FerroEHR's own accesses.

For a class I product whose manufacturer has not applied harmonised
standards, common specifications or a certification scheme in full, CRA
Art. 32(2) requires a third-party procedure. This is a question for counsel,
and nothing in this repository answers it.

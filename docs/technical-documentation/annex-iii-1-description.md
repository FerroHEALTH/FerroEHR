# Annex III 1: a detailed description of the EHR system

EHDS Annex III point 1 (`docs/law/eu/ehds/text.html`) asks for "a detailed
description of the EHR system", in ten points. CRA Annex VII point 1
(`docs/law/eu/cra/text.html`) asks for the same general description of the
product with digital elements; its points (a), (b) and (d) are answered here
beside the EHDS points they repeat. Book pages are cited from
`website/book/src/`, which is frozen per release at
`https://ferroehr.eu/docs/vX.Y.Z/`.

## 1(a) Intended purpose, date and version (CRA Annex VII 1(a))

"its intended purpose, and the date and version of the EHR system"

- **Intended purpose:** the manufacturer's statement,
  [`compliance/intended-purpose.md`](../../website/book/src/compliance/intended-purpose.md),
  statement version 1 of 2026-10-06. A change to it is a new statement version
  named in the release notes, and a substantial modification under CRA
  Art. 3(30).
- **Version:** the tag `vX.Y.Z` of the release, the workspace `version` in
  `Cargo.toml`, and what the running server reports in `ferroehr --version`
  and on `GET /management/info`.
- **Date:** the date on the release's heading in
  [`CHANGELOG.md`](../../CHANGELOG.md) (`## [X.Y.Z] - YYYY-MM-DD`), which the
  release lane requires and from which the support period is computed.
- **The other product of the system:** FerroBRIDGE, with its own version and
  date on its own releases.

State: available.

## 1(b) The categories of personal electronic health data

"the categories of personal electronic health data that the EHR system has
been designed to process"

All six priority categories of EHDS Art. 14(1), and any category a Member
State adds in national law, as far as the operational templates the deploying
organisation loads describe them; the access log, which names health
professionals and patients; the identities and the link to them, in their own
domains. Written in
[`compliance/intended-purpose.md`](../../website/book/src/compliance/intended-purpose.md#the-data-ferroehr-is-designed-to-process)
and repeated on the
[information sheet](../../website/book/src/compliance/information-sheet.md).

State: available.

## 1(c) Interaction with hardware or software outside the system

"how the EHR system interacts or can be used to interact with hardware or
software that is not part of the EHR system itself"

- The clinical applications a provider connects, over the openEHR REST API
  (ITS-REST Release-1.1.0) and AQL 1.1:
  [`using-the-api/index.md`](../../website/book/src/using-the-api/index.md).
- PostgreSQL 18, the identity provider (OpenID Connect), the TLS termination,
  and the platform: the security environment in
  [`compliance/intended-purpose.md`](../../website/book/src/compliance/intended-purpose.md#the-security-environment-ferroehr-assumes).
- Every connection FerroEHR opens on its own initiative, with what it carries:
  [`compliance/cra-risk-assessment.md`](../../website/book/src/compliance/cra-risk-assessment.md#outbound-data-flows-annex-i-part-i2g).
- The optional integrations (terminology servers, change events, the FHIR audit
  feed, S3 multimedia): [`beyond-core/index.md`](../../website/book/src/beyond-core/index.md).
- FerroBRIDGE, the other product of the EHR system, reads FerroEHR over the
  same REST API; the FerroEHR Viewer is a management interface over it
  ([`viewer/index.md`](../../website/book/src/viewer/index.md)).

State: available.

## 1(d) Versions of relevant software, and update requirements (CRA Annex VII 1(b))

"the versions of relevant software or firmware and any requirement related to
version update"; CRA Annex VII 1(b): "versions of software affecting
compliance with essential cybersecurity requirements"

- Runtime dependencies: PostgreSQL 18, 18.6 or newer, with the `btree_gist`
  extension, which the server installs ([`docs/VERSIONS.md`](../VERSIONS.md));
  Kubernetes `>=1.36.0-0` for the Helm chart (`kubeVersion` in
  `deploy/helm/ferroehr/Chart.yaml`).
- The openEHR specification versions implemented: on the
  [information sheet](../../website/book/src/compliance/information-sheet.md#e-standards-formats-and-specifications).
- Every third-party component of a release, with its version: the software
  bills of materials attached to the release (see
  [`cra-annex-vii-2-design-and-vulnerability-handling.md`](cra-annex-vii-2-design-and-vulnerability-handling.md)).
- Update requirements: upgrades in place with append-only migrations, the
  image pinned by digest, and the support period of each release
  ([`operations.md`](../../website/book/src/operations.md#upgrades),
  [`SECURITY.md`](../../SECURITY.md#supported-versions)).

State: available.

## 1(e) The forms in which it is placed on the market or put into service

"the description of all forms in which the EHR system is placed on the market
or put into service"

Cadasto B.V. places each tagged release on the market in four forms, all from
`https://github.com/FerroHEALTH/FerroEHR`:

- the `ferroehr` binaries for Linux on x86_64 and aarch64, as GitHub release
  assets;
- the container images `ghcr.io/ferrohealth/ferroehr`,
  `ghcr.io/ferrohealth/ferroehr-viewer` and
  `ghcr.io/ferrohealth/ferroehr-postgres`;
- the Helm chart at `oci://ghcr.io/ferrohealth/charts`;
- the source archive of the tag.

A release is put into service in one of two ways. A deploying organisation
installs it and runs it for itself, or has a processor run it
([`installation/index.md`](../../website/book/src/installation/index.md)).
Or Cadasto B.V. hosts it as a service for a customer, which EHDS Art. 26(2)
treats as put into service ("EHR systems offered as a service [...] shall be
considered as having been put into service"); Cadasto B.V. is then the
operator of that deployment and the customer's processor under a GDPR Art. 28
contract.

The `openehr-*` crates on crates.io are products of their own, under
Apache-2.0, and are not a form of the EHR system.

State: available.

## 1(f) The hardware it is intended to run on

"the description of hardware on which the EHR system is intended to run"

Linux on x86_64 (amd64) or aarch64 (arm64), as a container on Kubernetes or
Docker Compose or as the binary; the Helm chart's default resources (requests
250m CPU and 256 MiB memory, limits 2 CPU and 1 GiB per replica, two
replicas); a PostgreSQL 18 cluster the deploying organisation provisions.
Written in the
[instructions for use](../../website/book/src/compliance/instructions-for-use.md#what-it-runs-on).
The measured performance runs record the environment each ran on
([`performance.md`](../../website/book/src/performance.md)).

State: partial. No minimum hardware specification is published; the chart's
defaults are a starting point the deploying organisation sizes for its load.

## 1(g) The system architecture (CRA Annex VII 2(a))

"a description of the system architecture explaining how software components
build on or feed into each other and integrate into the overall processing,
including, where appropriate, labelled pictorial representations"

[`concepts/architecture.md`](../../website/book/src/concepts/architecture.md)
and [`concepts/storage.md`](../../website/book/src/concepts/storage.md), with
their diagrams; the design record [`docs/architecture.md`](../architecture.md),
whose section "Regulatory position (EHDS and CRA)" names the modules of the
logging component.

State: available.

## 1(h) Technical specifications, variants and configurations

"the technical specifications, such as features, dimensions and performance
attributes, of the EHR system and any variants or configurations and
accessories [...] including a detailed description of the data structures,
storage and input/output of data"

- Configuration: [`installation/configuration.md`](../../website/book/src/installation/configuration.md)
  and its sub-pages, every key with its default.
- Data structures and storage: [`concepts/storage.md`](../../website/book/src/concepts/storage.md).
- Input and output: the REST API and AQL
  ([`using-the-api/index.md`](../../website/book/src/using-the-api/index.md),
  [`querying-aql.md`](../../website/book/src/querying-aql.md)); the access log's
  records ([`audit.md`](../../website/book/src/audit.md)).
- Performance attributes: [`performance.md`](../../website/book/src/performance.md).

State: available.

## 1(i) Every change through the lifecycle

"a description of any change made to the system throughout its lifecycle"

[`CHANGELOG.md`](../../CHANGELOG.md), one section per release, which the
release notes reproduce; the revision tables of this tree (in
[`README.md`](README.md)), of the
[risk assessment](../../website/book/src/compliance/cra-risk-assessment.md#revisions)
and of the [hazard log](../../website/book/src/compliance/hazard-log.md).

State: available.

## 1(j) Instructions for use, and installation instructions (CRA Annex VII 1(d))

"the instructions for use for the user and, where applicable, installation
instructions"; CRA Annex VII 1(d): "user information and instructions as set
out in Annex II"

[`compliance/instructions-for-use.md`](../../website/book/src/compliance/instructions-for-use.md),
attached to each release, and the installation pages it points to. The CRA
Annex II information and instructions to the user are
[`compliance/cra-user-information.md`](../../website/book/src/compliance/cra-user-information.md),
point by point; the notes of each release link the copy of that page frozen
for the release, and the release lane refuses notes without the link.

State: available.

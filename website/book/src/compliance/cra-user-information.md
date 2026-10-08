# CRA information and instructions to the user

The Cyber Resilience Act (CRA, Regulation (EU) 2024/2847) has each product
"accompanied by the information and instructions to the user set out in Annex
II, in paper or electronic form" (CRA Art. 13(18)). This page carries that
information for FerroEHR, point by point in the order of Annex II. Each
section quotes what the point asks and gives FerroEHR's information for it,
with the source the information is kept in.

Annex II opens: "At minimum, the product with digital elements shall be
accompanied by:". The product is each tagged FerroEHR release, as Cadasto B.V.
places it on the market: the `ferroehr` binaries, the `ferroehr`,
`ferroehr-viewer` and `ferroehr-postgres` container images, the Helm chart and
the source archive ([the manufacturer and the product](cra.md#the-manufacturer-and-the-product)).

<!-- toc -->

> [!WARNING]
> The quotations are from the Official Journal text vendored in the repository
> at
> [`docs/law/eu/cra/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/cra/text.html)
> ([EUR-Lex](https://eur-lex.europa.eu/eli/reg/2024/2847/oj)). No conformity
> assessment has been carried out, no EU declaration of conformity exists and
> FerroEHR carries no CE marking. This page gives the information Annex II
> lists; it does not say that FerroEHR, or a deployment of it, meets the
> Regulation.

## Which copy to read

This book is frozen with every release, so the copy of this page that belongs
to release `vX.Y.Z` is
`https://ferroehr.eu/docs/vX.Y.Z/compliance/cra-user-information.html`. The
notes of each GitHub release link that copy, and the release lane refuses to
publish a release whose notes do not. The copy at `/docs/dev/` describes the
code on `main`, which is not a release.

CRA Art. 13(18) has information provided online kept "accessible,
user-friendly and available online for at least 10 years after the product
with digital elements has been placed on the market or for the support
period, whichever is longer". A frozen book version is never rebuilt or
deleted, and each release tag, which carries the page's source, is protected
against deletion ([`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#repository-security-settings--the-posture-of-record)).

The information is written in English. Art. 13(18) asks for "a language which
can be easily understood by users and market surveillance authorities". No
translation is published; a market surveillance authority that needs one asks
the single point of contact below.

## 1. The manufacturer

"the name, registered trade name or registered trademark of the manufacturer,
and the postal address, the email address or other digital contact as well
as, where available, the website at which the manufacturer can be contacted;"

| | |
|---|---|
| Manufacturer | Cadasto B.V. |
| Postal address | Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands |
| Email | [info@cadasto.com](mailto:info@cadasto.com) |
| Website | <https://www.cadasto.com/contact/> |

The source of these details is `app/ferroehr/src/manufacturer.rs`, the one
place they are written in the code. The running server reads them from there
for its startup banner, `ferroehr --version` and `GET /management/info`, the
viewer shows them on every screen, and the container images carry them in the
`eu.ferroehr.image.manufacturer` label
([the manufacturer and the deployment report](../operations.md#the-manufacturer-and-the-deployment-report)).
The [information sheet](information-sheet.md#a-the-manufacturer) repeats them.
Cadasto B.V. is the manufacturer of every FerroHEALTH product, and the
FerroHEALTH book describes it once for all of them:
[The manufacturer](https://ferrohealth.eu/docs/manufacturer.html).

## 2. The single point of contact for vulnerabilities

"the single point of contact where information about vulnerabilities of the
product with digital elements can be reported and received, and where the
manufacturer's policy on coordinated vulnerability disclosure can be found;"

- **Report a vulnerability** to [info@cadasto.com](mailto:info@cadasto.com),
  with "FerroEHR vulnerability" in the subject, or through
  [GitHub private vulnerability reporting](https://github.com/FerroHEALTH/FerroEHR/security/advisories/new).
  Both reach the same people and the same procedure. Do not open a public
  issue.
- **Receive information about vulnerabilities** from the GitHub security
  advisories of the repository, published with the release that fixes each
  one and readable by machine through
  `GET https://api.github.com/repos/FerroHEALTH/FerroEHR/security-advisories`.
- **The coordinated vulnerability disclosure policy** is the manufacturer's,
  shared by every FerroHEALTH product: the response times, the agreed
  disclosure date, the safe harbour, the credit and what each advisory
  carries are on
  [Security](https://ferrohealth.eu/docs/security.html) in the FerroHEALTH
  book. What is particular to FerroEHR, the reporting routes and the scope, is
  [`SECURITY.md` § Reporting a vulnerability](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#reporting-a-vulnerability),
  also published as
  [`security.txt`](https://ferroehr.eu/.well-known/security.txt) under RFC 9116.

## 3. Name, type and unique identification

"name and type and any additional information enabling the unique
identification of the product with digital elements;"

- **Name:** FerroEHR.
- **Type:** software. A clinical data repository server that stores, versions
  and queries openEHR health records over the openEHR REST API and AQL.
- **Version:** the release tag `vX.Y.Z`. The running server prints it in
  `ferroehr --version` and returns it as `server_version` on
  `GET /ferroehr/rest/status`; `ferroehr report` adds the commit and the build
  date ([`ferroehr report`](../installation/config-cli.md#ferroehr-report)).
- **The artefacts of a version:** the release assets carry a checksum and a
  signed build provenance each, and the container images are identified by
  digest, with the version and the commit in their
  `org.opencontainers.image.version` and `org.opencontainers.image.revision`
  labels ([verifying releases](../verifying-releases.md)).
- **Release date:** the date on the release's heading in the
  [changelog](https://github.com/FerroHEALTH/FerroEHR/blob/main/CHANGELOG.md),
  `## [X.Y.Z] - YYYY-MM-DD`
  ([information sheet, (b)](information-sheet.md#b-name-version-and-release-date)).

## 4. Intended purpose, security environment, essential functionalities and security properties

"the intended purpose of the product with digital elements, including the
security environment provided by the manufacturer, as well as the product's
essential functionalities and information about the security properties;"

- **Intended purpose:** FerroEHR stores, versions and queries the structured
  health records of one healthcare provider, serves them to that provider's
  clinical applications through the openEHR REST API and AQL, and records
  every access to them. FerroEHR's part of the statement, with the data it is
  designed to process, is the [intended purpose](intended-purpose.md); the
  statement for the EHR system FerroEHR forms with FerroBRIDGE, with its
  users, is in the FerroHEALTH book
  ([intended purpose](https://ferrohealth.eu/docs/ehds/intended-purpose.html)).
- **Security environment:** what FerroEHR relies on and does not provide
  itself: TLS termination, an OpenID Connect identity provider, protection of
  the database, a hardened platform and off-box copies of the access log
  ([the security environment FerroEHR assumes](intended-purpose.md#the-security-environment-ferroehr-assumes)).
- **Essential functionalities:** listed in the intended-purpose statement
  ([what FerroEHR is for](intended-purpose.md#what-ferroehr-is-for)).
- **Security properties:** each point of CRA Annex I Part I(2), with how
  FerroEHR implements it and the evidence, is in the
  [CRA risk assessment](cra-risk-assessment.md#annex-i-part-i2-point-by-point).
  The controls themselves are described on [Security](../security.md), and the
  risk that remains at each trust boundary in the
  [threat model](../threat-model.md).

## 5. Circumstances that may lead to significant cybersecurity risks

"any known or foreseeable circumstance, related to the use of the product with
digital elements in accordance with its intended purpose or under conditions
of reasonably foreseeable misuse, which may lead to significant cybersecurity
risks;"

- **Use in accordance with the intended purpose:** the risk register of the
  [CRA risk assessment](cra-risk-assessment.md#how-the-risks-were-analysed)
  rates each risk with the shipped controls, and the
  [limitations you should know](instructions-for-use.md#limitations-you-should-know)
  name the ones that matter before you rely on FerroEHR, among them the shipped
  defaults that favour a first boot and the reach of anyone with a database
  connection.
- **Reasonably foreseeable misuse:** the
  [foreseeable misuse table](intended-purpose.md#reasonably-foreseeable-use-and-misuse)
  lists each case (real patient data under the `sandbox` profile,
  authentication off, a plaintext listener, several organisations in one
  instance, a release run after its support period, and others), what
  FerroEHR does against it, and what remains.
- **What is not defended against:** the
  [threat model](../threat-model.md#what-is-explicitly-not-defended-against)
  lists it.

## 6. The EU declaration of conformity

"where applicable, the internet address at which the EU declaration of
conformity can be accessed;"

No EU declaration of conformity has been drawn up, so there is no address to
give. The declaration and the CE marking apply from 11 December 2027 (CRA
Art. 71(2)). For FerroEHR, which Cadasto B.V. declares with FerroBRIDGE as an
EHR system under the EHDS, the CRA asks for one declaration covering both
acts (CRA Art. 28(3)). When it exists, it is published as an asset of each GitHub
release it covers and as a page of the book frozen for that release, and this
section gives its address. The record of the declaration is
[`annex-iii-6-declaration-of-conformity.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/technical-documentation/annex-iii-6-declaration-of-conformity.md)
in the technical documentation.

## 7. Technical security support and the end of the support period

"the type of technical security support offered by the manufacturer and the
end-date of the support period during which users can expect vulnerabilities
to be handled and to receive security updates;"

- **The support period** of a release is five years from the month it is
  published. **Its end date**, month and year, is printed in the "Support
  period" section of that release's notes, and the running server states it on
  its boot banner, in `ferroehr --version`, under `support` in
  `GET /management/info` and as `support.status` in
  `GET /ferroehr/rest/status`
  ([how long the version you pinned is supported](../operations.md#how-long-the-version-you-pinned-is-supported)).
- **The type of support:** Cadasto B.V. handles vulnerabilities in a release
  during its support period, and the security update ships in the newest
  release, as a security-only patch where that is technically feasible. There
  are no maintenance branches, no long-term-support line and no backports
  (CRA Art. 13(10)). Every fixed vulnerability gets a GitHub security advisory
  with its severity, the affected and fixed versions, and what to do.
- **The source:**
  [`SECURITY.md` § Supported versions](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions),
  which also gives the periods of the Helm chart and the `openehr-*` crates,
  and the reasoning in the technical documentation
  ([`cra-annex-vii-4-support-period.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/technical-documentation/cra-annex-vii-4-support-period.md)).

## 8. Detailed instructions

"detailed instructions or an internet address referring to such detailed
instructions and information on:"

The detailed instructions are the [instructions for use](instructions-for-use.md)
of the same release, which this book carries and which ride each GitHub
release as `ferroehr-vX.Y.Z-instructions-for-use.md`. Each point below names
the part that answers it.

### 8(a) Secure use from commissioning on

"the necessary measures during initial commissioning and throughout the
lifetime of the product with digital elements to ensure its secure use;"

- **Commissioning:** check what you pulled
  ([verifying releases](../verifying-releases.md)), install
  ([installing](instructions-for-use.md#installing)), set the deployment
  profile to `production` and configure the access log, authentication and the
  pseudonymisation boundary
  ([configuring a deployment in the European Union](instructions-for-use.md#configuring-a-deployment-in-the-european-union)),
  then walk the [go-live checklist](../security/go-live-checklist.md) before
  the instance holds real patient data.
- **Throughout its lifetime:** the maintenance tasks and how often each falls
  due, from applying security updates to testing backups and acting on an
  access-log chain finding
  ([maintenance, and how often](instructions-for-use.md#maintenance-and-how-often)).

### 8(b) How changes to the product can affect the security of data

"how changes to the product with digital elements can affect the security of
data;"

- **Configuration.** The shipped defaults favour a first boot: the `sandbox`
  profile, TLS off, the open per-EHR access default and the access log failing
  open. The `production` profile refuses to start while one of the
  separations it checks is open and not accepted by name, and a gap you accept
  by name is stated on every boot and on `GET /ferroehr/rest/status`
  ([`deployment_profile`](../installation/configuration.md#deployment_profile)).
  A configuration change that opens such a gap shows there.
- **Integrations.** Every outbound integration is off by default. Turning one
  on sends data to another system, and the
  [outbound data flows](cra-risk-assessment.md#outbound-data-flows-annex-i-part-i2g)
  table says what each one sends and where.
- **Upgrades.** A release can carry database migrations. They are append-only,
  and a rolling upgrade stays compatible with the previous schema for the
  window in which both versions run; take a backup first
  ([upgrades](../operations.md#upgrades)).
- **Changes to the source.** A binary or image built from modified source is
  not a Cadasto B.V. release: its build provenance and signatures do not
  verify, and the risk assessment of the release does not describe it.
  Whether an organisation that modifies the source and puts the result into
  service becomes a manufacturer itself is a question for counsel
  ([the CRA, the manufacturer side](https://ferrohealth.eu/docs/cra.html)).

### 8(c) Installing security-relevant updates

"how security-relevant updates can be installed;"

Subscribe to the release feed
(`https://github.com/FerroHEALTH/FerroEHR/releases.atom`) and poll the
security advisories; the server does not look for newer releases itself.
Verify the new release ([verifying releases](../verifying-releases.md)), take
a backup, then deploy it by digest and let it apply its migrations
([upgrades](../operations.md#upgrades)). On Kubernetes that is a
`helm upgrade` to the chart version that ships the new release
([Kubernetes & Helm, upgrades](../installation/kubernetes.md#upgrades)).

### 8(d) Secure decommissioning, and removing user data

"the secure decommissioning of the product with digital elements, including
information on how user data can be securely removed;"

[Decommissioning](../operations-decommissioning.md) takes an instance out of
service: what to export and keep first, `ferroehr db erase` with its dry run
and confirmation, which removes the multimedia blobs and every schema with all
data and settings, and what the erase cannot reach and you remove yourself
(backups, WAL archives, replicas, the chart's Secrets and claims, the database
roles, configuration files, the data held by systems the instance sent to, and
the physical media). The command and the page are the deliverables of
[#3642](https://github.com/FerroHEALTH/FerroEHR/issues/3642). To start again
from an empty instance of the same release, follow
[returning to the original state](../operations-reset.md).

### 8(e) Turning off automatic installation of security updates

"how the default setting enabling the automatic installation of security
updates, as required by Part I, point (2)(c), of Annex I, can be turned off;"

FerroEHR installs no update by itself, so there is no such default setting to
turn off. Annex I Part I(2)(c) asks for automatic security updates "where
applicable", and the risk assessment records why the limb does not apply to
FerroEHR: it is a clinical server run under its operator's change control, an
upgrade can carry a database migration the operator schedules and backs up
for, an update installed on the manufacturer's schedule would restart the
service at a moment no clinician or operator chose, and a channel into the
deployment for the manufacturer would be an inbound path into a system holding
special-category data
([the justification](cra-risk-assessment.md#automatic-security-updates-annex-i-part-i2c-and-part-ii7)).
The operator installs each update as described in 8(c).

### 8(f) Information for an integrator

"where the product with digital elements is intended for integration into
other products with digital elements, the information necessary for the
integrator to comply with the essential cybersecurity requirements set out in
Annex I and the documentation requirements set out in Annex VII."

The FerroEHR server, its images and its chart are not intended for
integration into another product with digital elements. Other software
connects to a running instance over the openEHR REST API, as FerroBRIDGE does.

The nine `openehr-*` crates published on crates.io are libraries intended for
integration into other products, and each is a product of its own. The
information for their integrators is on
[Rust crates § Information for integrators](../crates.md#information-for-integrators-cra-annex-ii-point-8f).

## 9. The software bill of materials

"If the manufacturer decides to make available the software bill of materials
to the user, information on where the software bill of materials can be
accessed."

Cadasto B.V. makes the software bills of materials available. Each release
publishes a CycloneDX SBOM per server binary, an SPDX SBOM of the source tree,
an SBOM on each container image, a CycloneDX SBOM per `openehr-*` crate and an
SBOM attested on the Helm chart. Where each one is and how to verify it is in
[the SBOMs, one per published artefact](../verifying-releases.md#the-sboms-one-per-published-artefact).

## Related

- [Cyber Resilience Act](cra.md): the manufacturer, what applies from when,
  and the questions for counsel.
- [Instructions for use](instructions-for-use.md) and the
  [information sheet](information-sheet.md): the documents that accompany each
  release under the EHDS.
- [Complaints, incidents and vulnerabilities](post-market.md): the reporting
  channels and procedures.

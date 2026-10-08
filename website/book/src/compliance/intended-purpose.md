# Intended purpose

Cadasto B.V. states the intended purpose of the EHR system that FerroEHR and
FerroBRIDGE form together, under the EHDS (Regulation (EU) 2025/327) and the
Cyber Resilience Act (CRA, Regulation (EU) 2024/2847), once for both
products: the statement and its version, who uses the system, how it is
operated (self-hosted, or hosted by Cadasto B.V.), and what the intended
purpose excludes, the medical-device purposes of the MDR among them. That
statement is published in the FerroHEALTH book:
[The EHDS EHR system: intended purpose](https://ferrohealth.eu/docs/ehds/intended-purpose.html).

This page keeps FerroEHR's part of it: its essential functions, the data it
is designed to process, the environment it runs in and relies on, and the use
and misuse that can be foreseen for it (CRA Annex II point 4, EHDS Annex III
1(b)). The [CRA risk assessment](cra-risk-assessment.md), the
[hazard log](hazard-log.md) and the [claims review](claims-review.md) are
written against it.

<!-- toc -->

> [!WARNING]
> The quotations are from the Official Journal texts vendored in the repository
> under [`docs/law/eu/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/docs/law/eu).
> This page says what the manufacturer intends. It does not say that FerroEHR,
> or a deployment of it, meets either regulation: no conformity assessment has
> been carried out and no EU declaration of conformity exists.

## What FerroEHR is for

FerroEHR stores, versions and queries the structured health records of one
healthcare provider, and serves them to the software that provider's health
professionals and patients use. It holds each record as openEHR compositions
with their full version history, and serves them through the openEHR REST API
(ITS-REST Release-1.1.0) and the Archetype Query Language (AQL 1.1). It has no
clinical user interface of its own: health professionals reach it through the
clinical applications the provider connects to it.

FerroEHR records every access to the records it holds in its access log, the
European logging software component of EHDS Art. 2(2)(o).

Its essential functions, in the sense of CRA Annex II point 4:

- storing EHRs, compositions, EHR status and folders with an append-only
  version history and contribution-atomic commits
  ([storage](../concepts/storage.md));
- answering AQL queries over the stored records ([querying](../querying-aql.md));
- validating every write against the operational template it claims and the
  Reference Model invariants ([templates and validation](../templates-validation.md));
- keeping the identity of the record subject apart from the clinical record,
  in a separate demographic domain and a separate linkage domain
  ([the pseudonymisation boundary](../security.md#the-pseudonymisation-boundary));
- authenticating every caller and deciding every request
  ([authentication](../security.md#authentication),
  [authorization](../security.md#authorization));
- recording every access, refusals included, and serving the log back through
  IHE ITI-81 ([audit trail](../audit.md));
- restriction of processing, the research objection and the retention register
  ([retention, restriction and objection](retention.md));
- physical deletion, whole-repository dump and load, and EHR Extract export
  ([admin APIs](../operations-admin-apis.md)).

The security properties that go with these functions are on the
[Security](../security.md) page; the risk that survives each control is in the
[threat model](../threat-model.md).

## The data FerroEHR is designed to process

EHDS Annex III 1(b) and Art. 38(2)(d) ask for "the categories of personal
electronic health data that the EHR system has been designed to process".

**Clinical content**, in the clinical domain: whatever the operational
templates the deploying organisation loads describe. This includes the
priority categories of EHDS Art. 14(1): "(a) patient summaries; (b) electronic
prescriptions; (c) electronic dispensations; (d) medical imaging studies and
related imaging reports; (e) medical test results, including laboratory and
other diagnostic results and related reports; and (f) discharge reports", and
any category a Member State adds in national law. Which of them an instance
holds depends on the templates it loads, and the deployment declares that in
the [`[audit.categories]`](../installation/config-audit.md#auditcategories)
map, which classifies every access record. FerroEHR ships no such map. Images
are held as `DV_MULTIMEDIA` attachments, optionally offloaded to an
S3-compatible store ([S3 multimedia](../beyond-core/s3-multimedia.md)), or as
references to an imaging archive; FerroEHR implements no DICOM image transfer.

**Identities**, in the demographic domain: parties (persons, organisations,
roles) and their identifiers, with national identifiers optionally sealed
under AES-256-GCM ([the sealed identifier](../threat-model.md#the-sealed-identifier-and-its-lookup-digest)).

**The link between the two**, in the linkage domain: which party an EHR
belongs to.

**The access log**: who accessed which record, when, from which origin and in
which priority category. It names health professionals and patients.

**Operating data**: the configuration, metrics, traces and logs, which carry no
identified data ([telemetry](../operations.md#observability)), and the daily
[usage report](../usage-report.md), which carries instance facts and coarse
performance aggregates and no patient data.

## The operational environment

Each organisation gets its own FerroEHR instance, whether it runs FerroEHR
itself or Cadasto B.V. hosts it; the two operating models are described in the
[FerroHEALTH book](https://ferrohealth.eu/docs/ehds/intended-purpose.html).
The public sandbox at `sandbox.ferroehr.eu` is a demonstration holding demo
data, wiped nightly.

- **One instance per organisation.** FerroEHR is single-tenant: several
  organisations are served by several instances, each with its own database
  and its own database roles ([one instance, one organisation](../security.md#one-instance-one-organisation)).
- **Runtime.** The distroless, non-root container image on Kubernetes through
  the Helm chart, on Docker Compose, or the binary built from source, on Linux
  (amd64 or arm64) ([installation](../installation/index.md)).
- **Database.** A PostgreSQL 18 cluster the deploying organisation provisions;
  the Helm chart deploys none.
- **Profile.** `deployment_profile = "production"`, which refuses to start
  while a separation is open and not accepted by name
  ([go-live checklist](../security/go-live-checklist.md#2-the-deployment-profile-is-production-and-the-server-boots)).
  The shipped default is `sandbox`, which must not hold real personal data and
  says so on the banner, in the log and on `GET /rest/status`.
- **Expected time in use.** Longer than five years, which is why the support
  period of each release is five years
  ([`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions)).

### The security environment FerroEHR assumes

These are the parts of the environment the manufacturer designs FerroEHR to
rely on and does not provide itself (CRA Annex II point 4). The
[threat model](../threat-model.md#actors) names each as a trusted or
semi-trusted actor.

- **TLS termination.** Every connection to the API is encrypted, either by the
  server's own listener (`[server.tls]`, TLS 1.3 by default, mutual TLS
  available) or by an ingress or proxy in front of it. The shipped default is
  `[server.tls] enabled = false`; the hop behind a proxy is the deploying
  organisation's, and the server cannot see whether it is encrypted
  ([B1 network and protocol](../threat-model.md#b1--network-and-protocol),
  [server configuration](../installation/config-server.md)).
- **An identity provider.** Users and their credentials live in the deploying
  organisation's OpenID Connect provider, which issues the tokens FerroEHR
  verifies. FerroEHR issues no credentials and manages no identity lifecycle,
  and a compromised identity provider defeats every control below it
  ([enterprise identity providers](../identity-providers.md),
  [authentication](../security.md#authentication)). Basic authentication
  against a configured list is for service accounts and development.
- **Database protection.** The database is reached only by FerroEHR's own
  credentials, one per domain and each least-privilege, over a network path
  restricted to the server, with TLS to the database, encryption at rest at
  the volume layer, and backups taken per domain. Anyone with a database
  connection is past every API control
  ([B6 the database](../threat-model.md#b6--the-database),
  [database roles](../operations.md#database-roles-and-least-privilege),
  [TLS and database security](../operations.md#tls-and-database-security),
  [backup and recovery](../operations.md#backup-and-point-in-time-recovery)).
- **The platform.** A hardened cluster or host, operators the organisation
  trusts, and network policy around the instance
  ([cluster hardening](../installation/kubernetes-hardening.md):
  [the cluster](../installation/hardening-cluster.md),
  [images](../installation/hardening-supply-chain.md),
  [the workload](../installation/hardening-workload.md),
  [network and policy](../installation/hardening-network-policy.md),
  [secrets, detection and response](../installation/hardening-detection-response.md)).
- **Off-box copies.** A forwarding sink for the access log that the server's
  identity cannot rewrite, and a monitored backup
  ([B7 the audit trail](../threat-model.md#b7--the-audit-trail)).

## Reasonably foreseeable use and misuse

The CRA distinguishes "reasonably foreseeable use", use that "is not
necessarily the intended purpose" but "is likely to result from reasonably
foreseeable human behaviour or technical operations or interactions" (Art.
3(24)), from "reasonably foreseeable misuse", use "not in accordance with its
intended purpose, but which may result from reasonably foreseeable human
behaviour or interaction with other systems" (Art. 3(25)).

Foreseeable uses the manufacturer accepts: a teaching or research instance
holding synthetic or pseudonymised data; population queries over a
pseudonymised cohort, which FerroEHR serves with small-cell suppression
([cohort queries](../querying-aql.md#cohort-queries-across-the-pseudonymisation-boundary)); the store behind a patient-facing
service of the provider; a test system for an application vendor.

Foreseeable misuse, and what FerroEHR does about it:

| Misuse | What FerroEHR does | What remains |
|---|---|---|
| Real patient data in an instance running the shipped `sandbox` profile | states the profile on the banner, in the log, on `GET /rest/status` and in the viewer | nothing refuses it; the Helm chart and the Compose files declare `sandbox` explicitly, and the chart's values say what a deployment holding patient data sets |
| The quickstart Compose file in production, with its development user and RBAC off | the README and the Compose file call these development defaults | nothing refuses it; the [go-live checklist](../security/go-live-checklist.md) is the control |
| `auth.enabled = false` on a reachable network | `production` refuses it at boot unless `auth_off` is accepted by name (#3638) | an accepted gap is stated on every boot; the `sandbox` profile does not refuse it |
| A plaintext listener on a routable address with no TLS ingress in front | `production` refuses it at boot unless `plaintext_listener` is accepted by name; a loopback bind is exempt (#3638); a separate management listener (`management.port`), which binds every interface in plain HTTP, opens the same gap (#3668) | a deployment behind a TLS-terminating ingress accepts `plaintext_listener`, and nothing checks that the ingress exists or that the management port is unreachable off the host |
| Several organisations in one instance | no tenant model exists to misconfigure; one instance per organisation is documented | nothing in the software can detect it ([B5](../threat-model.md#b5--instance-separation)) |
| Real patient data on the public sandbox | the sandbox is labelled as demo data and wiped nightly | nothing refuses it |
| The access log switched off, or failing open, in production | `production` refuses `audit_off` and `audit_fails_open` unless each is accepted by name | an accepted gap is stated on every boot, and the organisation owns it |
| The access log shipped over UDP syslog in production | `production` refuses a syslog feed with `transport = "udp"` unless `audit_syslog_udp` is accepted by name (#3669) | an accepted gap is stated on every boot; nothing checks that the UDP path stays on a trusted segment |
| Change events published to a plaintext `amqp://` broker in production | `production` refuses an enabled change-event outbox on an `amqp://` URL without `tls = true` unless `plaintext_broker` is accepted by name (#3676) | an accepted gap is stated on every boot; nothing checks that the broker path stays on a trusted segment |
| Reads and writes straight against the database | the domain roles limit what each credential reaches | such access bypasses authorisation and the access log ([B6](../threat-model.md#b6--the-database)) |
| Structural validation taken for clinical correctness | validation checks structure, invariants and terminology bindings only | clinical governance is the organisation's ([threat model](../threat-model.md#what-is-explicitly-not-defended-against)) |
| openEHR conformance results, or these pages, taken for regulatory conformity | the [compliance overview](index.md) says FerroEHR holds no certification or declaration | none |
| A release run after its support period ends | the end date is in every release's notes and in `SECURITY.md`; the running server states it on the boot banner, in `ferroehr --version`, on `GET /management/info` and `GET /rest/status`, and logs a warning at boot and daily once it has passed (#3639) | nothing refuses to run; the server does not look for newer releases |

## What the intended purpose excludes

The exclusions that hold for the whole EHR system, the medical-device purposes
of the MDR first, are in the
[FerroHEALTH statement](https://ferrohealth.eu/docs/ehds/intended-purpose.html).
For FerroEHR they mean:

- **No diagnosis or advice.** FerroEHR stores and returns records as written;
  it computes no diagnosis, score, alert or treatment advice.
- **Hosting for several organisations.** One instance serves one organisation.
- **Identity management.** FerroEHR verifies identities that another system
  issues.
- **The interoperability component.** Exchange in the European electronic
  health record exchange format is FerroBRIDGE's part of the EHR system.

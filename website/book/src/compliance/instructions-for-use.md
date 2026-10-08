# Instructions for use

The instructions for use that accompany each FerroEHR release, free of charge,
as EHDS Art. 30(1)(d) asks: "clear and complete instructions for use"
(Regulation (EU) 2025/327). They say what FerroEHR needs to run, how to set
up a deployment in the European Union, what maintenance it needs and how
often (Art. 30(1)(k)), and the limitations of its interoperability and
security features that you should know before you rely on it (Art. 28(b)).
They also point to the information and instructions to the user of the Cyber
Resilience Act (CRA, Regulation (EU) 2024/2847, Annex II), which
[CRA information and instructions to the user](cra-user-information.md)
carries point by point and which the table at the end maps. The
[information sheet](information-sheet.md) accompanies them.

Most of what these instructions point to is documented in detail elsewhere in
this book; this page is the route through it, and the order to take it in.

<!-- toc -->

> [!NOTE]
> These instructions are published in the book of each release, at
> `https://ferroehr.eu/docs/vX.Y.Z/compliance/instructions-for-use.html`, and
> attached to each GitHub release as
> `ferroehr-vX.Y.Z-instructions-for-use.md`, a plain-text Markdown file a
> screen reader reads as text. Use the copy of the release you run. The
> article texts are those vendored at
> [`docs/law/eu/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/docs/law/eu).
> Following these instructions does not make a deployment meet any regulation;
> the duties of the controller and the processor stay theirs.

## Who these instructions are for

FerroEHR is installed and operated in one of two ways:

- **Self-hosted.** A healthcare provider runs the instance for its own records,
  or has a processor run it on its behalf. The provider's operators, or the
  processor's, follow these instructions. Cadasto B.V. is the manufacturer and
  does not operate the instance.
- **Hosted by Cadasto B.V.** Cadasto B.V. runs the instance as a service for a
  customer, under a GDPR Art. 28 processing contract. Cadasto B.V.'s operators
  follow these instructions, and the customer reads them to know what the
  service is set up to do and what stays with the customer as controller.

Either way the instance serves one organisation
([one instance, one organisation](../security.md#one-instance-one-organisation)),
and the clinical applications connected to it are developed against the
[REST API](../using-the-api/index.md) and [AQL](../querying-aql.md).

## What it runs on

- **Platform:** Linux on x86_64 (amd64) or aarch64 (arm64). The published
  container images are distroless and run as a non-root user.
- **Forms:** the container images on Kubernetes through the Helm chart (the
  production path; the chart needs Kubernetes 1.36 or newer), on Docker
  Compose, or the `ferroehr` binary
  ([installation](../installation/index.md)).
- **Resources:** the Helm chart's defaults are two replicas, each requesting
  `cpu: 250m` and `memory: 256Mi` with limits of `cpu: "2"` and
  `memory: 1Gi` (`resources` in the chart's `values.yaml`). They are a
  starting point; size them for your load. No minimum hardware specification
  is published. The [performance](../performance.md) page records the hardware
  each measured run used.
- **Database:** a PostgreSQL 18 cluster (18.6 or newer) that you provide. The
  Helm chart deploys none. The server installs the `btree_gist` extension.
- **Around it:** TLS termination, an OpenID Connect identity provider, database
  protection and a hardened platform, which FerroEHR relies on and does not
  provide ([the security environment FerroEHR assumes](intended-purpose.md#the-security-environment-ferroehr-assumes)).

## Installing

1. Choose the path and install: [Kubernetes & Helm](../installation/kubernetes.md)
   for production, [Docker Compose](../installation/compose.md) for evaluation.
   The quickstart Compose file runs a development user with access control off;
   never put real patient data into it.
2. Check what you pulled before you run it
   ([verifying releases](../verifying-releases.md)).
3. Pin the image by digest and the chart by version
   ([pin two versions](../installation/kubernetes.md#pin-two-versions-not-one)).
4. Harden the cluster the chart runs in
   ([cluster hardening](../installation/kubernetes-hardening.md)).

## Configuring a deployment in the European Union

These are the settings a deployment that holds real patient data in an EU
Member State sets. Each links its reference.

### The deployment profile

```toml
deployment_profile = "production"
deployment_accepts = []
```

The shipped default is `sandbox`, which must not hold real personal data.
`production` refuses to start while a separation is open and not accepted by
name in `deployment_accepts`: shared database credentials or clusters, an
undeclared pseudonym namespace, the access log off, failing open or shipped
over UDP syslog, the open per-EHR access default, migrations on a runtime
credential, authentication off, a plaintext listener on a routable address
(the separate management port included), change events published to a plain
`amqp://` broker, an audit FHIR feed on an `http://` URL, a multimedia store
with `allow_http = true`. Each refusal names the gap
and what to change
([`deployment_profile`](../installation/configuration.md#deployment_profile)).
A gap you accept by name is stated on every boot and on
`GET /management/status`. Behind a TLS-terminating ingress you accept
`plaintext_listener` by name. The Helm chart and the Compose files declare
`sandbox`, the binary's default. On Kubernetes, install with the chart's
`values-production.yaml`. It sets `production`, makes the separations a values
file can make, and accepts `plaintext_listener` behind a TLS ingress; separate
database clusters come from the connection strings you give it
([the production overlay](../installation/kubernetes.md#the-production-overlay)).
Replace its example names with your own.

### The access log

```toml
[audit]
enabled = true
fail_mode = "closed"
purpose_codes = ["TREAT", "ETREAT"]   # the codes you agree with your callers
emergency_purpose_codes = ["ETREAT"]  # marks an EHDS Art. 11(5) emergency access
legal_basis = "gdpr-art-9-2-h"

[audit.store]
enabled = true
retention_years = 5                   # at or above the floor that applies to you

[audit.syslog]
enabled = true
transport = "tls"                     # or [audit.fhir_feed] for a FHIR ARR
```

- **Fail closed.** `fail_mode = "closed"` refuses an operation whose access
  record cannot be taken, with a `503`. The shipped `open` serves it and
  counts the loss.
- **Forward a copy off the box.** The local store shares the database it
  audits. Forward the log to a sink the server's own credentials cannot
  rewrite, over TLS syslog or the FHIR feed. The syslog sink ships with UDP
  transport, which loses records silently; set `tls`. `production` refuses
  UDP unless `audit_syslog_udp` is accepted by name
  ([getting the log out](../audit.md#getting-the-log-out)).
- **Retention.** Set `[audit.store] retention_years` (or `retention_days`).
  In every EU Member State the server refuses a horizon shorter than three
  years (EHDS Art. 9(2)), and five years in the Netherlands, for the
  jurisdictions your `[privacy.identifier_scan]` rules name; `0` keeps every
  record forever
  ([retention, and who chooses it](../audit.md#retention-and-who-chooses-it),
  [`[audit.store]`](../installation/config-audit.md#auditstore-the-local-audit-record-repository)).
- **The priority categories.** Declare which of your templates carry which
  EHDS Art. 14(1) category in `[audit.categories]`, so every access record
  names the categories it served (Annex II 3.2(c)). FerroEHR ships no map, and
  without one every access is recorded `unclassified`
  ([`[audit.categories]`](../installation/config-audit.md#auditcategories)).
  The same map keys retention periods on a category
  ([retention by category](retention.md#keying-a-period-on-a-priority-category)).

### Authentication and access

- Authenticate through your OpenID Connect provider
  ([enterprise identity providers](../identity-providers.md)). Basic
  authentication is for service accounts and development.
- Declare the assurance level patient data requires in
  [`[auth.oidc.assurance]`](../installation/config-auth.md#authoidcassurance-the-assurance-level-patient-data-requires),
  mapping each `acr` value your identity provider emits to an eIDAS level, and
  declare how a token names the natural person in
  [`[auth.oidc.professional]`](../installation/config-auth.md#authoidcprofessional-the-natural-person-behind-a-token).
  Both are off by default: until you configure them your identity provider
  alone decides the assurance level, and a token issued to a client
  application is recorded as that client, not as a person.
- Set `[authz.rbac] ehr_access_default = "restricted"` and write the per-EHR
  access settings, or accept `open_ehr_access_default` by name with the reason
  ([per-EHR access control](../security.md#per-ehr-access-control-ehr_access)).

### The pseudonymisation boundary and identifiers

Give the demographic and linkage domains their own database credentials,
declare your pseudonym namespaces, keep the identifier scanner strict, and
turn on national-identifier sealing if you store national identifiers
([go-live checklist](../security/go-live-checklist.md),
[privacy and data minimisation](../installation/config-privacy.md)).

### The usage report

Every deployment sends a daily usage report to Cadasto B.V. by default, with no
patient data ([usage report](../usage-report.md)). Turn it off with
`[usage_report] enabled = false`.

### Before it holds real patient data

Walk the [go-live checklist](../security/go-live-checklist.md), and write the
[DPIA](../security/dpia.md) and the
[records of processing](../security/records-of-processing.md); they are the
controller's.

## Maintenance, and how often

EHDS Art. 30(1)(k) has the manufacturer inform users "of any mandatory
preventive maintenance of the EHR systems and its frequency". Cadasto B.V.
treats the first four rows as mandatory for a deployment holding patient data.

| Maintenance | How often | How |
|---|---|---|
| Apply security updates | as soon as a release carries one; each release is supported for five years from the month it is published, and the fix for a release in that period ships in the newest release (CRA Art. 13(8) and (10)) | subscribe to the release feed and poll the security advisories ([how long a version is supported](../operations.md#how-long-the-version-you-pinned-is-supported)), then [upgrade](../operations.md#upgrades); the server's `support.status` reads `ended` once its period has passed |
| Take and test backups, one per domain | daily backups (the chart's backup jobs run nightly when enabled); a restore test at least once a year and after each upgrade that migrates the schema | [backup and point-in-time recovery](../operations.md#backup-and-point-in-time-recovery) |
| Act on an access-log chain finding | the server verifies the chain daily by default (`[audit.store] verify_interval_seconds = 86400`); act on every finding | the `audit_chain` readiness indicator and the `atna_audit_chain_findings` metric ([tamper evidence](../audit.md#tamper-evidence)) |
| Watch the access log's loss counters | continuously, with an alert | the `atna_audit_*` metrics ([observability](../operations.md#observability)) |
| Review the access log | on the schedule your organisation's policy sets | ITI-81 retrieval or your forwarding sink ([audit trail](../audit.md)) |
| Sweep for stale decomposition | after a release whose changelog says so | [storage integrity](../operations-admin-apis.md#storage-integrity) |
| Rotate the national-identifier key | on your key policy, and at once if the key is exposed | [rotating the key](../operations.md#rotating-the-national-identifier-key) |
| Read the retention due list | on your retention schedule | [reading what is due](retention.md#reading-what-is-due) |

The retention reaper of the access log runs hourly by itself and needs no
action.

## Limitations you should know

EHDS Art. 28(b) prohibits "failing to inform the professional user of likely
limitations related to interoperability or security features of the EHR
system in relation to its intended purpose". These are the ones Cadasto B.V.
knows of for this release.

**Interoperability**

- FerroEHR does not provide or receive the European electronic health record
  exchange format. FerroBRIDGE carries it, and its content waits on
  implementing acts under EHDS Art. 15(1) that have not been adopted.
- FerroEHR maps no FHIR resources. FHIR R4 mapping is FerroBRIDGE's; FerroEHR
  uses FHIR for the access log's `AuditEvent` (with the ITI-81 retrieval) and
  for external terminology servers only. A configuration that still carries
  `[fhir]` or `[fhir.outbound]` is refused at boot as unknown keys.
- Of the access-log renderings, the DICOM message carries neither the
  accessing organisation, the purpose, the origins nor the categories, because
  DICOM PS3.15 defines no element for them; the FHIR `AuditEvent` carries all
  four.
- Records in the demographic domain carry no priority category, because no
  priority category lives there
  ([audit trail](../audit.md#the-ehds-logging-elements-mapped)). Every other
  access record carries one, `none` or `unclassified` included.

**Security**

- The shipped defaults favour a first boot: the `sandbox` profile, TLS off,
  the open per-EHR access default, the access log failing open. The
  `production` profile refuses them unless accepted by name.
- No authentication assurance level is required, and no natural person is
  asked of a token, until `[auth.oidc.assurance]` and
  `[auth.oidc.professional]` are configured.
- An emergency access in the vital interest of the patient (EHDS Art. 11(5))
  is marked only when the caller declares one of
  `[audit] emergency_purpose_codes`
  ([the emergency mark](../audit.md#emergency-access-ehds-art-115)). The
  mark lifts no restriction, and FerroEHR holds no EHDS Art. 8 restriction
  today (planned, [#3682](https://github.com/FerroHEALTH/FerroEHR/issues/3682)).
- Clinical content is not encrypted by the application at rest; encryption at
  rest is the database's and the disk's
  ([the justification](cra-risk-assessment.md#application-level-encryption-at-rest-annex-i-part-i2e)).
- Physical deletion and `ferroehr db erase` remove rows, schemas and blobs;
  they do not reach filesystem blocks, WAL archives, replicas or backups
  ([what the erase does not reach](../operations-decommissioning.md#what-the-erase-does-not-reach)).
- Anyone with a database connection is past every API control, the access log
  included ([the database](../threat-model.md#b6--the-database)).
- Structural validation checks structure, invariants and terminology
  bindings, not clinical correctness.

The full list of foreseeable misuse, with what FerroEHR does about each, is in
the [intended purpose](intended-purpose.md#reasonably-foreseeable-use-and-misuse);
the risk that survives each control is in the [threat model](../threat-model.md).

## Taking a deployment out of service

Export what you must keep ([EHR Extract](../beyond-core/messaging.md),
[dump](../operations-admin-apis.md#dump-and-load)), then remove all data and
settings with `ferroehr db erase` and remove what it cannot reach (backups, WAL
archives, replicas, the chart's Secrets and claims, the database roles and
the configuration files) under your own procedure. The steps and the retention
duties that come first are on [Decommissioning](../operations-decommissioning.md).

## The CRA user information (CRA Annex II)

The full information for each point, with what the point asks, is on
[CRA information and instructions to the user](cra-user-information.md). The
notes of each GitHub release link the copy of that page frozen for the
release.

| CRA Annex II | Where |
|---|---|
| 1. The manufacturer's name, postal address, email and website | [information sheet, (a)](information-sheet.md#a-the-manufacturer) |
| 2. The single point of contact for vulnerabilities, and the coordinated vulnerability disclosure policy | [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#reporting-a-vulnerability) |
| 3. Name, type and unique identification | [information sheet, (b)](information-sheet.md#b-name-version-and-release-date) |
| 4. Intended purpose, the security environment, the essential functionalities and the security properties | [intended purpose](intended-purpose.md), [security](../security.md), and the statement for the EHR system at <https://ferrohealth.eu/docs/ehds/intended-purpose.html> |
| 5. Circumstances that may lead to significant cybersecurity risks | [limitations](#limitations-you-should-know), [foreseeable misuse](intended-purpose.md#reasonably-foreseeable-use-and-misuse) |
| 6. The internet address of the EU declaration of conformity | none exists ([technical documentation](technical-documentation.md#declaration-of-conformity)) |
| 7. The technical security support, and the end date of the support period | [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions), and the release notes |
| 8(a). Secure commissioning and use over the lifetime | [installing](#installing), [configuring](#configuring-a-deployment-in-the-european-union), [maintenance](#maintenance-and-how-often) |
| 8(b). How changes to the product can affect the security of data | [configuration](../installation/configuration.md); a change to the source is a change to the product |
| 8(c). How security-relevant updates are installed | [upgrades](../operations.md#upgrades) |
| 8(d). Secure decommissioning, and removing user data | [taking a deployment out of service](#taking-a-deployment-out-of-service) |
| 8(e). Turning off automatic security updates | does not apply: FerroEHR installs no update by itself ([the justification](cra-risk-assessment.md#automatic-security-updates-annex-i-part-i2c-and-part-ii7)) |
| 8(f). Information for an integrator | the server is not intended for integration into another product with digital elements; FerroBRIDGE meets it over the REST API. The information for integrators of the `openehr-*` crates is on [Rust crates](../crates.md#information-for-integrators-cra-annex-ii-point-8f) |
| 9. Where the software bill of materials is | [the SBOMs](../verifying-releases.md#the-sboms-one-per-published-artefact) |

## Contact

Questions, complaints and serious incidents go to Cadasto B.V. at
[info@cadasto.com](mailto:info@cadasto.com)
([complaints, incidents and vulnerabilities](post-market.md)). The
manufacturer's post-market procedure, which handles them for every
FerroHEALTH product, is at <https://ferrohealth.eu/docs/post-market.html>.

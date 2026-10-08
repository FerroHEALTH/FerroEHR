# CRA risk assessment

This page is the cybersecurity risk assessment Cadasto B.V., the manufacturer
of each tagged FerroEHR release, keeps under the Cyber Resilience Act
(Regulation (EU) 2024/2847). CRA Art. 13(2) has the manufacturer "undertake an
assessment of the cybersecurity risks associated with a product with digital
elements and take the outcome of that assessment into account during the
planning, design, development, production, delivery and maintenance phases".
Art. 13(3) says what the assessment holds: "an analysis of cybersecurity risks
based on the intended purpose and reasonably foreseeable use, as well as the
conditions of use", and an indication of "whether and, if so in what manner,
the security requirements set out in Part I, point (2), of Annex I are
applicable to the relevant product with digital elements and how those
requirements are implemented", and of how Part I point (1) and the
vulnerability handling of Part II are applied. Where a requirement does not
apply, Art. 13(4) asks for "a clear justification to that effect" in the
technical documentation. The assessment is point 3 of the technical
documentation of CRA Annex VII.

The page is versioned with the release: this book is frozen with every release
at `/docs/vX.Y.Z/`, so each release's documentation carries the assessment
that applied to it. The statuses below are those of revision 2 of the
assessment, 2026-10-07 (the table at the end lists the revisions).

<!-- toc -->

> [!WARNING]
> Every quotation is from the Official Journal text vendored in the repository
> at
> [`docs/law/eu/cra/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/cra/text.html)
> ([EUR-Lex](https://eur-lex.europa.eu/eli/reg/2024/2847/oj)); none of its
> three corrigenda touches Article 13 or the Annexes cited here. EHDS
> Art. 104 amends CRA Art. 13(4) and inserts Art. 32(5a); those changes are
> read from `docs/law/eu/ehds/text.html`. No conformity assessment has been
> carried out and no EU declaration of conformity exists. A status on this
> page describes the evidence the repository holds, never conformity.

## Scope and dates

- **The products assessed.** Each tagged release: the `ferroehr` binaries, the
  `ferroehr`, `ferroehr-viewer` and `ferroehr-postgres` images, the Helm chart
  and the source archive. The nine `openehr-*` crates published on crates.io
  under Apache-2.0 are products of their own; this page covers them only where
  a row says so.
- **Who the requirements bind.** Annex I Part I sets properties of the product,
  which the manufacturer ensures (CRA Art. 6(a), Art. 13(1)). Part II sets
  the manufacturer's processes; its chapeau reads "Manufacturers of products
  with digital elements shall:". A deploying organisation that runs an
  unmodified release is the addressee of neither.
- **When.** Annex I, Art. 13 and Art. 32 apply from 11 December 2027; Art. 14
  from 11 September 2026 (CRA Art. 71(2)). The assessment runs now so that
  the gaps are closed before that date.
- **The EHDS route.** Cadasto B.V. declares FerroEHR with FerroBRIDGE as an EHR
  system. For such a product, CRA Art. 32(5a), inserted by EHDS Art. 104(3),
  routes the Annex I conformity through the EHDS Chapter III procedure, and
  CRA Art. 13(4) as EHDS Art. 104(1) replaces it lets this assessment "be part
  of the risk assessment required by those Union legal acts". The
  [hazard log](hazard-log.md) is that EHDS-side assessment for the logging
  component, and the [technical documentation](technical-documentation.md)
  carries both.

## The basis: intended purpose, use and environment

The [intended-purpose statement](intended-purpose.md) is the basis Art. 13(3)
asks for. In one paragraph: FerroEHR stores, versions and queries the
structured health records of one healthcare provider, serves them to that
provider's clinical applications through the openEHR REST API and AQL, and
records every access. It runs one instance per organisation, self-hosted by
the organisation or hosted by Cadasto B.V. as a service, on a PostgreSQL 18
cluster provisioned for that instance, behind TLS, with users in the
organisation's identity provider. Its foreseeable misuse (the `sandbox`
profile holding real data, authentication off, a plaintext listener, several
organisations in one instance) is listed on that page with what the product
does against each.

**The assets** are those of the [threat model](../threat-model.md#assets):
the clinical payload, the demographic parties and their identifiers, the
EHR-to-subject link, the access log, the version history, the signing keys,
the credentials, instance separation and availability.

**The expected time in use** is longer than five years, so the support period
of each release is five years from the month it is published (CRA Art. 13(8);
[`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions)
carries the reasoning, which is Annex VII point 4).

## How the risks were analysed

CRA Art. 3(37) defines a cybersecurity risk as "the potential for loss or
disruption caused by an incident", "expressed as a combination of the magnitude
of such loss or disruption and the likelihood of occurrence of the incident".
The analysis starts from the [threat model](../threat-model.md), which walks
eight trust boundaries (B1 to B8) and states the control and the residual risk
at each. The register below rates each risk on the intended purpose and the
shipped controls: the magnitude by the asset it reaches, the likelihood by
what an actor needs to cause it. The ratings are the manufacturer's judgement,
on three levels (high, medium, low).

| # | Risk | Magnitude | Likelihood with the shipped controls | Controls | Annex I Part I(2) |
|---|---|---|---|---|---|
| C1 | An unauthenticated caller reads or writes clinical data | high | low under the intended purpose; high when `auth.enabled = false` is misused under `sandbox` or accepted by name under `production` | authentication on by default with a boot refusal when no mechanism is configured; `production` refuses `auth.enabled = false` unless `auth_off` is accepted by name; per-request authorisation | (b), (d) |
| C2 | An authenticated caller reaches EHRs outside their remit | high | medium: the shipped `EHR_ACCESS` default is `open`, which `production` refuses unless accepted by name | RBAC, ABAC, per-EHR `EHR_ACCESS`; every access recorded | (d), (l) |
| C3 | Data in transit is read or altered on the network | high | low behind a TLS ingress or with the server's TLS; high on a plaintext listener | rustls TLS 1.3 and mutual TLS, off by default; `production` refuses a plaintext main listener on a routable address, a separate plain-HTTP management listener, and a UDP syslog audit feed, unless `plaintext_listener` or `audit_syslog_udp` is accepted by name; a boot warning on a plaintext routable bind | (e) |
| C4 | Someone with a database connection reads, rewrites or deletes data, bypassing the API | high | depends on the deploying organisation's database protection | least-privilege roles per domain; append-only versions; version signatures verified on read; the hash-chained access log | (e), (f) |
| C5 | An access happens and the log does not show it, or the log is altered | high | low with `fail_mode = "closed"` and an off-box sink; medium with the shipped `fail_mode = "open"` | refusals always recorded; `production` refuses audit off and audit failing open unless accepted; the hash chain | (f), (l) |
| C6 | A national identifier or identifying content leaks into the clinical record, telemetry or a report | high | low | the identifier scanner in `strict` mode; log sanitising; reports with no identifiers | (e), (g) |
| C7 | A malformed or hostile payload crashes or corrupts the server | medium | low | memory-safe Rust with `unsafe` forbidden; overflow checks in release; fuzzed parsers; caught panics | (j), (k) |
| C8 | The service is made unavailable | high (clinical availability) | medium: an authorised caller holding resources is not defended against | admission limit with `503` shedding; rate limits; body, header and statement limits; replicas and a disruption budget | (h) |
| C9 | A vulnerable component ships in a release | high | medium | `cargo deny` advisories gate; image scan before tagging; weekly re-scan; Dependabot; VEX for argued findings | (a) |
| C10 | A tampered artefact reaches a deployment | high | low | SLSA Build L3 provenance; SBOM attestations; signed tags; immutable releases; a signed chart | (f) |
| C11 | Data stays recoverable after the organisation removes it | medium | medium: the erase removes every schema and blob, and backups, WAL, replicas and the media stay the organisation's to remove | physical delete of EHRs; `ferroehr db erase` for all data and settings; the decommissioning page | (m) |
| C12 | An outbound integration sends more than its purpose needs, or to the wrong place | medium | low: every integration is off by default | the flows table below | (g), (i) |

## Annex I Part I(1): the level of cybersecurity

"Products with digital elements shall be designed, developed and produced in
such a way that they ensure an appropriate level of cybersecurity based on the
risks." The manufacturer applies it through the register above: each risk has
a control, and each control is traced to a point of Part I(2) below. The text
names no measures, so the level is set against the risks of a clinical data
repository holding special-category data, where disclosure cannot be undone
and unavailability is a patient-safety matter.

## Annex I Part I(2), point by point

The chapeau reads: "On the basis of the cybersecurity risk assessment referred
to in Article 13(2) and where applicable, products with digital elements
shall:". Each entry gives the requirement, whether it applies, how FerroEHR
implements it, the evidence, the status (met, partial or gap, describing the
evidence only) and the open work.

### (a) No known exploitable vulnerabilities

"be made available on the market without known exploitable vulnerabilities"

- **Applies:** yes.
- **How:** `cargo deny` fails the build on any RustSec advisory and on yanked
  crates, with each accepted advisory dated and reasoned in `deny.toml`. Each
  image is scanned by Trivy before it is tagged, so only a passing digest gets
  a tag, and the published images are re-scanned weekly. Every release also
  scans each image and server binary with no severity floor and unfixed
  findings included, joins every finding with its OpenVEX statement, and is
  refused, before any image is tagged, while a finding has none; the joined
  record and the raw reports are release assets. The Rust advisory statements
  are checked against the resolved dependency graph.
- **Evidence:** `deny.toml`; `trivy.yaml`; `.github/workflows/scan-and-tag.yml`;
  `.github/workflows/image-scan.yml`; `security/vex/`;
  `scripts/checks/vex-reachability.sh`; `scripts/checks/vex-coverage.sh`;
  `.github/workflows/release.yml` (`vulnerability-record`);
  [the exploitability record](../verifying-releases.md#the-exploitability-record).
- **Status:** partial. Every finding of the full scan carries a judgement or
  the release does not ship. The image binaries are built without
  `cargo auditable`, so the image scans do not see the Rust dependency graph;
  the server binary's graph is covered by the scan of the release tarballs,
  and the viewer binary's by no scan. The CRA does not define "known".
- **Open:** the image binaries built with `cargo auditable`, so each image
  scan sees its Rust dependencies (#3675).

### (b) Secure by default, and reset to the original state

"be made available on the market with a secure by default configuration,
unless otherwise agreed between manufacturer and business user in relation to
a tailor-made product with digital elements, including the possibility to
reset the product to its original state"

- **Applies:** yes. The tailor-made carve-out does not: a public release is
  not a tailor-made product agreed with a business user.
- **How:** authentication on, with a boot refusal when no mechanism is
  configured; RBAC on; the admin and management surfaces off; every
  integration off; Swagger UI `private`; the rate limiter on; version signing
  on; the identifier scanner `strict`. Weak settings are refused at boot: an
  `alg: none` token, a non-HTTPS issuer, a short HMAC secret, an Argon2id hash
  below the floor. The chart runs non-root with a read-only root filesystem,
  all capabilities dropped, `RuntimeDefault` seccomp and an ingress
  NetworkPolicy, and its validation fails a render that loses any of them.
- **Evidence:** `app/ferroehr/assets/ferroehr.default.toml`;
  `app/ferroehr/src/config/auth.rs`; `app/ferroehr/src/config/deployment.rs`;
  `deploy/helm/ferroehr/values.yaml` (`config.deployment_profile`);
  `docker-compose.yml`; `deploy/helm/validate.sh`;
  `scripts/deploy-probes/reset.sh`;
  [the workload](../installation/hardening-workload.md);
  [returning to the original state](../operations-reset.md).
- **Why `sandbox` is the default:** the binary, the chart and the Compose
  files all ship and declare `deployment_profile = "sandbox"`. A `production`
  default would refuse to start until the deployment has separate database
  credentials per domain, a declared subject namespace, a closed audit fail
  mode and a schema-preparation credential of its own, none of which a first
  boot, an evaluation or the quickstart has. An installation that cannot start
  invites accepting every gap by name only to make it start, which records a
  choice nobody made deliberately. A `sandbox` makes no such choice for the
  operator and stays loud instead: it names every open separation on the boot
  banner, in the log and on `/management/status`, and
  `GET /ferroehr/rest/status` reports the profile, so a sandbox cannot pass for
  a production instance. It also means no existing deployment changes
  behaviour by upgrading. The instructions for a deployment holding patient
  data are to set `production`, which refuses every open separation that is
  not accepted by name.
- **Which shipped defaults a production holder keeps:** these are secure as
  shipped and `production` raises nothing about them: authentication on,
  RBAC on, the admin API and every management endpoint off, Swagger UI
  `private`, the rate limiter on, version signing on in `digest` mode with
  read-time verification `strict`, the identifier scanner `strict` over every
  shipped rule, and the audit trail on with the local store and
  `retention_days = 0`, which the server refuses at boot where a jurisdiction
  in force sets a ceiling. These must change, and `production` refuses them:
  `authz.rbac.ehr_access_default = "open"`, `audit.fail_mode = "open"`, an
  empty `privacy.subject_namespaces`, one database credential for every
  domain, and `db.migrate = "apply"` on the runtime credential. Two may be
  accepted by name when the deployment closes them outside the server:
  `plaintext_listener`, when a TLS-terminating ingress fronts the port, and
  `shared_cluster`, when one PostgreSQL cluster holds the domains in separate
  schemas under separate roles. The usage report is on by default; it carries
  no health data ([outbound data flows](#outbound-data-flows-annex-i-part-i2g)).
- **Reset:** the book documents returning an instance to its original state,
  configuration and database, for Docker Compose, Kubernetes and a single
  binary. The deployment probe harness runs the Compose procedure and reads
  the outcome back: the database volume is deleted, an EHR written before the
  reset is gone, and the server reports the shipped configuration.
- **Status:** partial. The shipped profile is `sandbox`, so a deployment
  holding patient data is secure only after it sets `production` and closes
  the five refused defaults above. The Kubernetes and single-binary reset
  procedures are documented and not exercised by a probe.
- **Open:** a probe of the Kubernetes reset procedure (#3677).

### (c) Security updates, automatic updates, notification and postponement

"ensure that vulnerabilities can be addressed through security updates,
including, where applicable, through automatic security updates that are
installed within an appropriate timeframe enabled as a default setting, with
a clear and easy-to-use opt-out mechanism, through the notification of
available updates to users, and the option to temporarily postpone them"

- **Applies:** the security-update limb, yes. The automatic-update limb, no:
  see [the justification](#automatic-security-updates-annex-i-part-i2c-and-part-ii7).
  The notification limb, yes: the sentence does not place it under "where
  applicable", and this assessment reads it as applying. Postponement is
  inherent, because the deploying organisation decides when to upgrade.
- **How:** a fix ships in the next tagged release, as a security-only patch
  unless a recorded reason makes that infeasible; there are no backports
  (CRA Art. 13(10)). Users learn of it through the GitHub release, its notes,
  the security advisory and the `### Security` changelog section; the release
  feed (`https://github.com/FerroHEALTH/FerroEHR/releases.atom`) and the
  advisory listing
  (`GET https://api.github.com/repos/FerroHEALTH/FerroEHR/security-advisories`)
  are the subscription route `SECURITY.md` names. The running server states
  its support period (five years from the month of its release date) on the
  boot banner, in `ferroehr --version`, on `GET /management/info` and on
  `GET {rest root}/status` as `support.status`, and once the period has ended
  it logs a warning at boot and once a day (Art. 13(19)).
- **No in-product update check.** The server does not contact anyone to learn
  of a newer release. The point names "the notification of available updates
  to users" and is silent on whether the product itself must deliver it.
  FerroEHR has no end-user interface of its own: its users are the deploying
  organisation's operators and the applications that call its API, and an
  update reaches a deployment only through the operators' change control. The
  notice goes to them through the feed and the advisory listing. A check from
  the server would add an outbound connection from every instance holding
  special-category data, would fail wherever the organisation blocks egress
  (as the chart's egress policy lets it), and would reach the operators only
  as a log line.
- **Evidence:** [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions);
  `app/ferroehr/src/support.rs`; `app/ferroehr/src/banner.rs`
  (`banner_states_the_support_period`);
  [how long a version is supported](../operations.md#how-long-the-version-you-pinned-is-supported);
  [upgrades](../operations.md#upgrades).
- **Status:** met for the security-update and notification limbs, with the
  notice delivered outside the product as stated above.
- **Open:** none.

### (d) Protection from unauthorised access, and reporting it

"ensure protection from unauthorised access by appropriate control mechanisms,
including but not limited to authentication, identity or access management
systems, and report on possible unauthorised access"

- **Applies:** yes. The list is open ("including but not limited to"); the duty
  is appropriate mechanisms.
- **How:** Basic (Argon2id) and OIDC bearer authentication; RBAC, Cedar or
  remote ABAC, and per-EHR `EHR_ACCESS`; an ABAC block that is enabled but
  cannot be built aborts boot. Every refused access (`401`, `403`) is recorded
  in the access log and counted in the `auth_failures` metric.
- **Evidence:** `app/ferroehr-rest/src/extensions/access/`;
  `app/ferroehr-rest/src/system_log/middleware.rs`;
  `app/ferroehr/src/telemetry/metrics.rs`;
  [authentication](../security.md#authentication),
  [authorization](../security.md#authorization);
  `app/ferroehr/src/config/deployment.rs`
  (`production_refuses_auth_off_unless_accepted_by_name`);
  [deployment profile](../installation/configuration.md#deployment_profile).
- **Status:** met. Reporting is met. `production` refuses
  `auth.enabled = false` at boot unless `auth_off` is accepted by name, and an
  accepted gap is stated on every boot. The shipped `sandbox` profile accepts
  it, which (b) records.
- **Open:** none beyond (b).

### (e) Confidentiality of stored, transmitted and processed data

"protect the confidentiality of stored, transmitted or otherwise processed
data, personal or other, such as by encrypting relevant data at rest or in
transit by state of the art mechanisms, and by using other technical means"

- **Applies:** yes. The means are a menu ("such as"); application-level
  encryption at rest of the clinical payload is not used, with
  [the justification](#application-level-encryption-at-rest-annex-i-part-i2e).
- **How:** in transit, rustls with TLS 1.3 by default and mutual TLS, or a
  TLS-terminating ingress; integrations offer TLS (`amqps`, syslog over TLS,
  HTTPS-only S3 by default). At rest, protected national identifiers sealed
  under AES-256-GCM with a keyed lookup digest. Other technical means: three
  pseudonymisation domains on separate credentials, checked by `production`;
  secrets that never render; log sanitising.
- **Evidence:** `[server.tls]` in `ferroehr.default.toml`;
  [B1](../threat-model.md#b1--network-and-protocol),
  [B6](../threat-model.md#b6--the-database),
  [the sealed identifier](../threat-model.md#the-sealed-identifier-and-its-lookup-digest),
  [TLS and database security](../operations.md#tls-and-database-security).
- **Status:** partial. The shipped transport default is plaintext, the AMQP
  and syslog integrations default to plaintext when switched on, and
  identifier sealing is off by default. `production` refuses a plaintext main
  listener on a routable address unless `plaintext_listener` is accepted by
  name, and a loopback bind is exempt; a deployment behind a TLS-terminating
  ingress accepts the gap by name, because the ingress carries the encryption
  (`app/ferroehr/src/config/deployment.rs`,
  `production_refuses_a_routable_plaintext_listener_and_exempts_loopback`).
  The separate management listener (`management.port`) binds every interface
  in plain HTTP, so setting it opens the same gap
  (`production_refuses_a_plain_management_listener_on_its_own_port`), and a
  syslog audit feed over UDP opens `audit_syslog_udp`
  (`production_refuses_a_udp_audit_syslog_unless_accepted_by_name`).
- **Open:** `production` does not refuse the AMQP integration on a plaintext
  `amqp://` URL (#3676).

### (f) Integrity, and reporting corruptions

"protect the integrity of stored, transmitted or otherwise processed data,
personal or other, commands, programs and configuration against any
manipulation or modification not authorised by the user, and report on
corruptions"

- **Applies:** yes, to data, programs and configuration. The configuration
  limb is split with the deploying organisation, with
  [the justification](#configuration-integrity-annex-i-part-i2f).
- **How:** data: immutable versions, each signed (digest or OpenPGP) and
  verified on read in `strict` mode by default; the access log is
  hash-chained in the database, and `UPDATE`, `DELETE` and `TRUNCATE` on it are
  refused. Programs: SLSA Build L3 provenance and SBOM attestations, cosign on
  the chart, signed tags, immutable releases, a read-only root filesystem.
  Configuration: unknown keys and unknown `FERROEHR__` variables are refused
  at boot.
- **Evidence:** `app/ferroehr/src/versioning/integrity.rs`;
  `app/ferroehr/migrations/audit/0003_tamper_chain.sql`;
  `app/ferroehr/src/system_log/chain_check.rs`;
  `app/ferroehr/tests/it/audit_chain.rs`
  (`the_chain_check_reports_a_tampered_record_on_the_health_surface`);
  `app/ferroehr/src/config/strict.rs`;
  [tamper evidence](../audit.md#tamper-evidence);
  [verifying releases](../verifying-releases.md).
- **Status:** met. A signature failure on read is refused and logged. The
  server verifies the access-log chain one minute after boot and then every
  `[audit.store] verify_interval_seconds` (default `86400`; `0` turns it off).
  A finding is logged at `ERROR`, counted in `atna_audit_chain_findings` with
  `kind` `damage` or `unverifiable`, and shown by the `audit_chain` indicator
  on `GET /health/readiness` (`DOWN` when damaged, `DEGRADED` when
  unverifiable), which never flips readiness. Each replica runs its own check.
  A deployment that sets the interval to `0` reports a corruption only when
  an operator runs `audit.verify_audit_chain()`.
- **Open:** none.

### (g) Data minimisation

"process only data, personal or other, that are adequate, relevant and limited
to what is necessary in relation to the intended purpose of the product with
digital elements (data minimisation)"

- **Applies:** yes, measured against the [intended purpose](intended-purpose.md).
- **How:** the clinical record carries pseudonymous subject references; the
  identifier scanner refuses national identifiers in clinical content; the
  identity, the link and the access log sit in their own domains; telemetry
  carries no identified data; the access-log category classification never
  reaches spans, logs or metrics; `ferroehr report` identifies no person.
- **Evidence:** `app/ferroehr/src/privacy/`;
  `app/ferroehr/src/telemetry/log_sanitize.rs`;
  [privacy and data minimisation](../installation/config-privacy.md);
  [the pseudonymisation boundary](../security.md#the-pseudonymisation-boundary).
- **Status:** met for the processing the intended purpose needs; each outbound
  flow is justified in [the flows table](#outbound-data-flows-annex-i-part-i2g).

### (h) Availability, also after an incident

"protect the availability of essential and basic functions, also after an
incident, including through resilience and mitigation measures against
denial-of-service attacks"

- **Applies:** yes. The measures named are an open list.
- **How:** an admission limit sheds load with `503` and `Retry-After`; per
  principal and per address rate limits, on by default; body limits, header
  read timeouts, HTTP/2 stream caps, statement and query timeouts and a
  result-row cap; panics caught; two replicas, a disruption budget and
  topology spread in the chart; backup and point-in-time recovery documented.
- **Evidence:** `app/ferroehr-rest/src/overload.rs`;
  `app/ferroehr-rest/src/rate_limit.rs`; `app/ferroehr-rest/src/limits.rs`;
  [request limits](../security.md#request-limits-and-rate-limiting);
  [backup and recovery](../operations.md#backup-and-point-in-time-recovery).
- **Status:** met. The residual risk is stated in the threat model: an
  authorised caller holding resources is not defended against, and the limiter
  is per process.

### (i) Minimal impact on other devices and networks

"minimise the negative impact by the products themselves or connected devices
on the availability of services provided by other devices or networks"

- **Applies:** yes.
- **How:** every outbound integration is off by default and bounded when on
  (retry limits, batch sizes, poll intervals, connect and request timeouts).
  The usage report sends at most one start report per ten minutes per instance
  and one daily report, each at most 64 KiB. A default-deny egress policy is
  available in the chart.
- **Evidence:** `ferroehr.default.toml`; `deploy/helm/ferroehr/values.yaml`
  (`networkPolicy.egress`); [usage report](../usage-report.md#when-a-report-is-sent).
- **Status:** met.

### (j) Limited attack surface

"be designed, developed and produced to limit attack surfaces, including
external interfaces"

- **Applies:** yes.
- **How:** optional surfaces are off by default (admin, management, SMART,
  multimedia, events, FHIR, the terminology API); Swagger UI is `private`;
  optional integrations are compiled behind features; the runtime images are
  distroless `nonroot` with no shell; the chart installs an ingress
  NetworkPolicy.
- **Evidence:** `docker/Dockerfile`; `docker/viewer/Dockerfile`;
  [operational surfaces](../security.md#operational-surfaces-what-is-reachable-and-by-whom).
- **Status:** met.

### (k) Exploitation mitigation

"be designed, developed and produced to reduce the impact of an incident using
appropriate exploitation mitigation mechanisms and techniques"

- **Applies:** yes.
- **How:** Rust with `unsafe_code = "forbid"` across the workspace and
  `overflow-checks = true` in release builds; a non-root UID, no shell, a
  read-only root filesystem, all capabilities dropped, `RuntimeDefault`
  seccomp and user namespaces; database credentials split so one leaked DSN
  reaches one domain.
- **Evidence:** `Cargo.toml` (`[workspace.lints.rust]`, `[profile.release]`);
  [the workload](../installation/hardening-workload.md);
  [secrets, detection and response](../installation/hardening-detection-response.md).
- **Status:** met.

### (l) Recording and monitoring, with an opt-out

"provide security related information by recording and monitoring relevant
internal activity, including the access to or modification of data, services
or functions, with an opt-out mechanism for the user"

- **Applies:** yes.
- **How:** the access log records every API access, every record a read or a
  query served, and every refusal, in DICOM PS3.15 and FHIR `AuditEvent`
  form, with syslog and FHIR-feed sinks and ITI-81 retrieval; metrics and
  traces cover the rest. The opt-out is `[audit] enabled` and
  `[audit.store] enabled`; `production` refuses audit off unless accepted by
  name.
- **Evidence:** `app/ferroehr/src/system_log/`;
  `app/ferroehr-rest/src/system_log/`; [audit trail](../audit.md);
  [observability](../operations.md#observability).
- **Status:** met. A deploying organisation may owe the log under the EHDS or
  national law, so whether it may use the opt-out is outside the CRA.

### (m) Secure removal of all data and settings, and secure transfer

"provide the possibility for users to securely and easily remove on a
permanent basis all data and settings and, where such data can be transferred
to other products or systems, ensure that this is done in a secure manner"

- **Applies:** yes.
- **How:** `ferroehr db erase --confirm <instance>` removes all data and
  settings in one operation: every multimedia blob, then the five schemas with
  every domain, the access log, the stored settings and the instance id. Run
  without `--confirm` it erases nothing and prints what it would remove and the
  value that confirms it. `DELETE {base}/admin/ehr/{ehr_id}` and
  `DELETE {base}/admin/ehr/all` remove single EHRs with every version, the
  linkage row and orphaned multimedia; template and stored-query deletes
  exist. Transfer runs over the authenticated API and `{base}/admin/dump`,
  over TLS when the deployment enables it.
- **Evidence:** `app/ferroehr/src/decommission.rs`;
  `app/ferroehr/tests/it/decommission.rs`;
  [decommissioning](../operations-decommissioning.md);
  [physical deletion](../operations-admin-apis.md#physical-deletion);
  [dump and load](../operations-admin-apis.md#dump-and-load).
- **Status:** met, shipped in #3642. The decommissioning page lists what the
  erase cannot reach and the deploying organisation removes: backups, WAL
  archives and replicas, the chart's Secrets and claims, the database roles,
  configuration files, and the data held by systems the instance sent to.
  Sanitising the physical media is the organisation's. A single party has no
  physical delete over REST, because the openEHR Admin API defines none; the
  page says so and names the alternatives.

## Requirements and limbs that do not apply

CRA Art. 13(4): "Where certain essential cybersecurity requirements are not
applicable to the product with digital elements, the manufacturer shall include
a clear justification to that effect in that technical documentation."

### Automatic security updates (Annex I Part I(2)(c) and Part II(7))

Part I(2)(c) asks for automatic security updates "where applicable", and Part
II(7) for distribution "where applicable for security updates, in an automatic
manner". Neither applies to FerroEHR:

- FerroEHR is a clinical server run under its operator's change control: the
  deploying organisation for a self-hosted instance, Cadasto B.V. for an
  instance it hosts as a service. An upgrade can carry a database migration
  that the operator schedules, backs up for and verifies, so in both cases
  the operator applies an update, never the product by itself.
- A clinical data repository is a system whose unavailability is a
  patient-safety matter (threat model, asset "Availability"). An update the
  manufacturer installs on its own schedule would restart the service and
  migrate the schema at a moment no clinician or operator chose.
- The operator controls the host, the cluster, the image registry it pulls
  from and the network. FerroEHR has no channel of its own into a deployment,
  and adding one would be an inbound path into a system holding
  special-category data, against (j). Where Cadasto B.V. hosts an instance,
  it applies updates as that instance's operator under its own change
  control, not through the product.
- Updates are distributed securely by other means: signed, immutable releases
  with provenance, digest-pinned images, the moving `<major>.<minor>` tags an
  organisation can follow with its own tooling, and the advisory that tells it
  to.

So the automatic-installation limb and the opt-out that goes with it do not
apply, and CRA Annex II point 8(e) (how to turn automatic installation off)
has nothing to describe. The notification limb still applies (#3639).

### Application-level encryption at rest (Annex I Part I(2)(e))

Part I(2)(e) names encryption at rest as one of the means ("such as"). FerroEHR
does not encrypt the clinical payload before it reaches PostgreSQL:

- The payload is stored as canonical openEHR JSON that AQL queries inside;
  encrypting those fields in the application would make them unqueryable,
  which defeats the intended purpose.
- Encryption at rest is provided where the data is stored: at the volume or
  disk layer, which the deploying organisation provisions
  ([TLS and database security](../operations.md#tls-and-database-security)).
- The fields whose disclosure re-identifies a person directly, the protected
  national identifiers, are sealed in the application (AES-256-GCM), because
  they are looked up by digest and never queried by content.
- Confidentiality at rest is further carried by "other technical means": the
  separated domains and credentials, and least-privilege roles.

### Configuration integrity (Annex I Part I(2)(f))

Part I(2)(f) lists "configuration" among what must be protected against
modification "not authorised by the user". FerroEHR protects the configuration
it reads: unknown keys and unknown `FERROEHR__` variables stop the boot, secrets
are read from mounted files, and the effective configuration is printed
redacted on demand. It does not sign or checksum its configuration file. The
file lives on the deploying organisation's host or in its Kubernetes secrets
and config maps, under that organisation's access control, and whoever can
change it there is, by the organisation's own decision, a user authorised to
configure the product. Protecting it against other modification is the
platform's job ([secrets, detection and response](../installation/hardening-detection-response.md)).

## Outbound data flows (Annex I Part I(2)(g))

Every connection FerroEHR opens on its own initiative, with what it carries
and why. The database connections are the product's own storage and are not
listed.

| Flow | Default | Carries | Purpose, against the intended purpose |
|---|---|---|---|
| Usage report to `report.ferropulse.eu` | on | instance id, version, commit, specification profile, licence grant type, deployment method, uptime, database health, PostgreSQL major version, CPU and memory ranges, and daily performance aggregates ([fields](../usage-report.md#what-a-report-contains)) | Not part of the clinical purpose: it serves the manufacturer's view of which versions, licence types and deployment sizes run. It carries no health data and no data from a request; the collector refuses an unknown member; the body is at most 64 KiB; `ferroehr usage-report --print` shows it before it is sent; one switch turns it off. Cadasto B.V. decided on 2026-10-06 to keep it on by default and accepts the residual risk, after an adjudication that read ePrivacy Art. 5(3) as requiring prior consent ([privacy notice](../usage-report.md#privacy-notice)) |
| OIDC discovery and keys from the configured issuer | when OIDC is configured | the issuer's well-known URL; no request data | verifying the tokens the provider's identity provider issues |
| Remote policy decision point (`[authz.abac.remote]`) | off | the attributes of the request being decided | the authorisation decision the deployment delegates to its own policy service |
| Terminology server (`[terminology.external]`) | off | codes and value-set references to validate, expand or subsume | validating terminology bindings on write and expanding `TERMINOLOGY()` in AQL |
| Change events over AMQP (`[events]`) | off | commit notifications, free of PHI by default | telling the provider's other systems that a record changed |
| FHIR outbound over AMQP (`[fhir.outbound]`) | off | mapped FHIR resources, which carry PHI | feeding the provider's FHIR consumers; enabling it is an explicit decision |
| Multimedia to S3 (`[multimedia]`) | off | `DV_MULTIMEDIA` bytes above a size threshold, keyed by content hash | storing large attachments of the record outside the database |
| Access log to syslog (`[audit.syslog]`) and to a FHIR feed (`[audit.fhir_feed]`) | off | access records | an off-box copy of the access log, which the threat model asks for |
| Traces and metrics over OTLP (`telemetry.otlp_endpoint`) | off | spans and metrics, with identified data sanitised out | operating the instance |

Each integration except the usage report carries only what its purpose in the
provider's deployment needs, and only to a destination the deploying
organisation configures.

## Annex I Part II: vulnerability handling

Part II binds Cadasto B.V. These are processes, carried out partly through the
repository's tooling; none is a property of the software a deployment runs.

| Point | How the manufacturer applies it | Status | Tracker |
|---|---|---|---|
| (1) Identify components; SBOM | an SBOM per published artefact: CycloneDX of the cargo graph per binary, SPDX per image, SPDX of the repository, CycloneDX per `openehr-*` crate attested against its `.crate` archive, and CycloneDX per chart version naming its images by digest, explained in [the SBOMs](../verifying-releases.md#the-sboms-one-per-published-artefact); vulnerabilities in VEX, `deny.toml`, the changelog and each release's exploitability record | met | shipped, #3643 |
| (2) Remediate without delay; security updates apart from functionality | acknowledgement in 5 and assessment in 10 working days; a fix ships as a security-only patch unless the release notes record why that is infeasible (`SECURITY.md`) | met as policy from this change on | shipped, #3644 |
| (3) Regular security tests and reviews | daily fuzzing of seven parsers, CodeQL, OpenSSF Scorecard, clippy with the reliability lints, `cargo deny`, image scans, conformance runs, the deploy probe | met | none |
| (4) Disclose fixed vulnerabilities | a GitHub security advisory for every fixed vulnerability, with CVSS, affected and fixed versions and remediation, and the criteria for a delay (`SECURITY.md`, `docs/post-market.md`); no advisory has been published yet | partial: the commitment is written, the record starts with the next fix | shipped, #3645 |
| (5) Coordinated vulnerability disclosure | `SECURITY.md` and `security.txt` | met | none |
| (6) Share vulnerability information; a contact address | GitHub private reporting and `info@cadasto.com`; component vulnerabilities reported upstream and recorded in a register | met as policy from this change on | shipped, #3646, #3648 |
| (7) Distribute updates securely | provenance, attestations, signed tags, immutable releases, digest-pinned and scanned images, crates.io Trusted Publishing; the automatic limb does not apply ([above](#automatic-security-updates-annex-i-part-i2c-and-part-ii7)) | met | none |
| (8) Disseminate updates without delay, free of charge, with advisories | public, immutable releases and moving tags; advisories under (4); whether "free of charge" holds for every commercial licence is a point for counsel | partial | shipped, #3645; counsel's answer open, #3647 |

## Security-relevant paths

These are the paths of the repository that hold an asset or a control of the
register above. A release that changes any of them since the previous release
is refused unless this page gains a row in its Revisions table:
`scripts/checks/cra-risk-assessment.sh --since <previous tag>` runs in the
release lane, and the same script refuses a declared path that no longer
exists, so the list cannot rot. CRA Art. 13(14) asks for procedures that keep
each product of a series in conformity as its design changes.

<!-- security-relevant-paths:start -->
- `app/ferroehr-rest/src/extensions/access` authentication and the RBAC/ABAC enforcement point
- `app/ferroehr-rest/src/smart` SMART scope enforcement
- `app/ferroehr/src/config/auth.rs` the authentication configuration
- `app/ferroehr/src/config/authz.rs` the authorization configuration
- `app/ferroehr/src/config/deployment.rs` the deployment profiles and their refusals
- `app/ferroehr/src/system_log` the access log and the audit chain
- `app/ferroehr-rest/src/system_log` the audit middleware
- `app/ferroehr/migrations/audit` the audit store
- `app/ferroehr/src/versioning/signature` version signing
- `app/ferroehr/src/versioning/integrity.rs` the integrity check of stored versions
- `app/ferroehr/src/privacy` the pseudonymisation boundary
- `app/ferroehr/src/decommission.rs` the erase of all data and settings
- `docker/Dockerfile` the server image
- `docker/viewer/Dockerfile` the viewer image
- `docker/postgres/Dockerfile` the database image
- `deploy/helm/ferroehr/templates` the chart's security contexts and policies
- `deploy/helm/ferroehr/values.yaml` the chart's shipped defaults
<!-- security-relevant-paths:end -->

## Revisions

CRA Art. 13(3) has the assessment "updated as appropriate during a support
period", and Art. 13(7) has the manufacturer update it "where applicable" as
vulnerabilities and third-party information arrive. It is revised:

- with every release whose changes touch an asset or a control in the register
  (the [security-relevant paths](#security-relevant-paths), gated at release,
  #3649);
- when a vulnerability or an incident shows a risk the register does not name;
- when the [intended purpose](intended-purpose.md) changes.

| Version | Date | Change |
|---|---|---|
| 1 | 2026-10-06 | First assessment, from the CRA Annex I audit of #3611 |
| 2 | 2026-10-07 | (a): the per-release exploitability record (#3636); (b): the declared profile in the chart and the Compose files, the reasons for `sandbox` and the shipped defaults a production holder keeps, and the reset procedure (#3637); Part II(1): SBOMs for the crates and the chart (#3643); the declared security-relevant paths and their release gate (#3649); (m): the erase command and the decommissioning page (#3642) |

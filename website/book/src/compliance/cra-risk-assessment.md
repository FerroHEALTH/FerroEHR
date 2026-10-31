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
  component, and the [technical documentation](https://ferrohealth.eu/docs/ehds/technical-documentation.html)
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

**The expected time in use** is longer than five years, so Cadasto B.V.
commits to keep publishing security fixes for at least five years after each
release, shipped only in the newest release, which is the one release
supported (CRA Art. 13(8) and (10);
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
| C2 | An authenticated caller reaches EHRs outside their remit | high | medium: the shipped `EHR_ACCESS` default is `open`, which `production` refuses unless accepted by name | RBAC, ABAC, per-EHR `EHR_ACCESS`; `production` refuses the role gate off (`rbac_off`) and an `open` `EHR_ACCESS` default (`open_ehr_access_default`) unless each is accepted by name; every access recorded | (d), (l) |
| C3 | Data in transit is read or altered on the network | high | low behind a TLS ingress or with the server's TLS; high on a plaintext listener | rustls TLS 1.3 and mutual TLS, off by default; `production` refuses a plaintext main listener on a routable address, a separate plain-HTTP management listener, and an AMQP integration on a plaintext `amqp://` broker, unless `plaintext_listener` or `plaintext_broker` is accepted by name; the syslog audit feed sends over TLS only (`ihe-atna`); a boot warning on a plaintext routable bind | (e) |
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
  of the three images is scanned by Trivy by its pushed digest, on both
  platforms, before it is tagged, so only a passing digest gets a tag, and the
  published images are re-scanned weekly. Every shipped binary, in the release
  tarballs and in the images, is built with `cargo auditable`, so it carries
  its Rust dependency list and an image scan sees the crate graph. Every
  release also scans each image and server binary with no severity floor and
  unfixed findings included, joins every finding with its OpenVEX statement,
  and is refused, before any image is tagged, while a finding has none; the
  joined record and the raw reports are release assets. The Rust advisory
  statements are checked against the resolved dependency graph, and an
  operating-system statement that argues from what a binary loads carries
  that linkage as data, which the release lane reads off both Rust binaries
  on both architectures and refuses a contradiction of (#3694).
- **Evidence:** `deny.toml`; `trivy.yaml`; `.github/workflows/scan-and-tag.yml`;
  `.github/workflows/image-scan.yml`; `security/vex/`;
  `scripts/checks/vex-reachability.sh`; `scripts/checks/vex-coverage.sh`;
  `scripts/checks/vex-linkage.sh`;
  `.github/workflows/release.yml` (`vulnerability-record`);
  [the exploitability record](../verifying-releases.md#the-exploitability-record).
- **Status:** partial. Every finding of the full scan carries a judgement or the
  release does not ship, and the scan reads the Rust dependencies of the
  server and viewer binaries from the binaries themselves (#3674, #3675). The
  CRA does not define "known". Eight OS findings are judged `affected` and
  have no upstream fix: CVE-2026-8674 (a long resolver search domain) in all
  three images; three libxml2 and two glibc JISX0213 findings in the
  `ferroehr-postgres` image that a database login reaches through
  PostgreSQL's XML functions, which FerroEHR itself never calls (#3673); and
  the krb5 CVE-2026-107778 in `ferroehr-postgres`, reached only through a
  GSSAPI configuration FerroEHR does not ship. Each carries an action
  statement in `security/vex/`. Re-checked for 4.3.9 on 2026-10-15: the seven
  earlier statements are still `affected`, CVE-2026-107778 joined them, and
  #3708 is open, so the status stays partial.
- **Open:** the eight `affected` findings (#3708), until Debian ships fixes that a
  rebuild picks up; each such rebuild is a security-only patch with its
  advisory.

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
  `alg: none` token, a symmetric algorithm, a non-HTTPS issuer, an Argon2id
  hash below the floor. The chart runs non-root with a read-only root filesystem,
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
  domain, and `db.migrate = "apply"` on the runtime credential. Three may be
  accepted by name when the deployment closes them outside the server:
  `plaintext_listener`, when a TLS-terminating ingress fronts the port,
  `plaintext_broker`, when the path to the AMQP broker stays on a trusted
  segment, and `shared_cluster`, when one PostgreSQL cluster holds the domains
  in separate schemas under separate roles. The usage report is on by
  default; it carries no health data ([outbound data flows](#outbound-data-flows-annex-i-part-i2g)).
- **Reset:** the book documents returning an instance to its original state,
  configuration and database, for Docker Compose, Kubernetes and a single
  binary. The deployment probe harnesses run the Compose and the Helm
  procedures and read the outcome back: the database volume or the database is
  gone, an EHR written before the reset is not found, the reinstalled server
  migrates the empty database (`P-K8S-RESET-*`), and the Compose server reports
  the shipped configuration (`P-RESET-*`).
- **Status:** partial. The shipped profile is `sandbox`, so a deployment
  holding patient data is secure only after it sets `production` and closes
  the five refused defaults above. The single-binary reset is documented
  and not exercised by a probe; its database step is the one the Helm probe
  runs.
- **Open:** none for the reset.

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
  the end month of the commitment for its release (five years from the month
  of its release date) on the boot banner, in `ferroehr --version`, on
  `GET /management/info` and on `GET /management/status` as
  `support.status`, and once that month has passed it logs a warning at boot and once a day (Art. 13(19)).
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
- **How:** Basic (Argon2id) and bearer authentication through the
  `ferrohealth-smart` crate: asymmetric signature algorithms only, the
  RFC 9068 profile by default, and each issuer's metadata and keys read at
  boot. Every route carries a SMART on openEHR scope requirement, and a
  `patient/` grant reaches only the EHR of the token's launch context. Then
  RBAC, Cedar or remote ABAC, and per-EHR `EHR_ACCESS`; an ABAC block that is enabled but
  cannot be built aborts boot. A restricted `EHR_ACCESS` default holds with
  the role gate off, and is refused at boot with authentication off. With a
  patient claim configured, the ABAC patient scope applies to the EHR row and
  to every versioned object on every query plan and on the direct EHR,
  composition and status routes. An EHR that records no subject is refused
  to a patient caller, and so is one whose subject id another namespace also
  holds, unless `[authz.abac] patient_namespace` names the namespace the
  claim is issued in. Every refused access (`401`, `403`) is recorded
  in the access log and counted in the `auth_failures` metric.
- **Evidence:** `app/ferroehr-rest/src/extensions/access/`;
  `app/ferroehr-rest/src/system_log/middleware.rs`;
  `app/ferroehr/src/telemetry/metrics.rs`;
  [authentication](../security.md#authentication),
  [authorization](../security.md#authorization);
  `app/ferroehr/src/config/deployment.rs`
  (`production_refuses_auth_off_unless_accepted_by_name`,
  `production_refuses_rbac_off_and_a_restricted_default_stays_closed`);
  [deployment profile](../installation/configuration.md#deployment_profile).
- **Status:** met. Reporting is met. `production` refuses
  `auth.enabled = false` at boot unless `auth_off` is accepted by name, and
  `[authz.rbac] enabled = false` unless `rbac_off` is; an accepted gap is
  stated on every boot. The shipped `sandbox` profile accepts both, which (b)
  records.
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
  HTTPS-only S3 by default); a renewed server certificate is served without a
  restart, on `SIGHUP` and every `[server.tls] reload_interval_s`. At rest, protected
  national identifiers sealed under AES-256-GCM with a keyed lookup digest,
  per party version, inside the party commit; a write response serves the
  sealed form, an import or admin load carrying a protected value in plaintext
  is refused, and a dump carries the sealed rows as ciphertext. Other
  technical means: three
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
  (`production_refuses_a_plain_management_listener_on_its_own_port`). The
  syslog audit feed sends over RFC 5425 TLS only, through `ihe-atna`
  (`app/ferroehr/tests/it/system_log_tls_roundtrip.rs`,
  `tls_octet_counted_frame_round_trips`). An
  enabled change-event outbox publishing to a plain `amqp://` broker opens
  `plaintext_broker` (#3676,
  `production_refuses_a_plaintext_amqp_broker_unless_accepted_by_name`).
  An enabled audit FHIR feed on an `http://` URL opens `plaintext_audit_feed`,
  and an enabled multimedia store with `allow_http` opens
  `plaintext_object_store` (#3689,
  `production_refuses_a_plaintext_audit_feed_and_object_store_unless_accepted`).
- **Open:** none.

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
  hash-chained in the database over every column of a record, the render
  policy its FHIR document is rendered under is checked against its digest,
  and `UPDATE`, `DELETE` and `TRUNCATE` on it are refused. The
  runtime roles read the migration bookkeeping of their schema and cannot
  write it, and the server refuses to start while one can. Programs: SLSA Build L3 provenance and SBOM attestations, cosign on
  the chart, signed tags, immutable releases, a read-only root filesystem.
  Configuration: unknown keys and unknown `FERROEHR__` variables are refused
  at boot.
- **Evidence:** `app/ferroehr/src/versioning/integrity.rs`;
  `app/ferroehr/migrations/audit/0003_tamper_chain.sql`;
  `app/ferroehr/migrations/audit/0011_chain_digest_v2.sql`;
  `app/ferroehr/src/system_log/chain_check.rs`;
  `app/ferroehr/tests/it/audit_chain.rs`
  (`the_chain_check_reports_a_tampered_record_on_the_health_surface`,
  `the_current_digest_covers_every_column`,
  `every_hashed_column_is_tamper_evident`);
  `app/ferroehr/migrations/clinical/0014_bookkeeping_read_only.sql` and its
  party and linkage twins;
  `app/ferroehr/src/config/strict.rs`;
  [tamper evidence](../audit.md#tamper-evidence);
  [verifying releases](../verifying-releases.md).
- **Status:** met. A signature failure on read is refused and logged. The
  server verifies the access-log chain one minute after boot and then every
  `[audit.store] verify_interval_seconds` (default `86400`; `0` turns it off).
  A finding is logged at `ERROR`, counted in `atna_audit_chain_findings` with
  `kind` `damage` or `unverifiable`, and shown by the `audit_chain` indicator
  on `GET /health/readiness` (`DEGRADED` when damaged or
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
  multimedia, events, the terminology API); Swagger UI is `private`;
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
  exist. Transfer runs over the authenticated API and `/ferroehr/rest/extensions/v1/admin/dump`,
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
| Authorization-server metadata and keys from each `[auth]` issuer | when an issuer is configured | the issuer's well-known URLs; no request data | verifying the tokens the provider's identity provider issues |
| Token introspection (`[auth] mode = "introspection"`) | off | each bearer token presented to FerroEHR, with the client credential of `[auth.introspection.client]` or the token of `[auth.introspection.bearer]` | asking the issuer whether an opaque access token is active (RFC 7662) |
| Patient identity source over PIXm (`[ehr_index] provider = "ihe"`) | on (the default provider) | the subject identifier and its assigning authority as ITI-83 specifies, with the client credential of `[ehr_index.oauth]` when set | resolving a subject to the pseudonym its EHR carries; the source is never asked to resolve a pseudonym back |
| Client-credentials token requests (`[audit.fhir_feed.oauth]`, `[authz.abac.remote.oauth]`, `[terminology.server.oauth]`) | off | the product's own client credential and the requested scope; no request data | authenticating the product to the deployment's audit feed, policy service and terminology server |
| Remote policy decision point (`[authz.abac.remote]`) | off | the attributes of the request being decided | the authorisation decision the deployment delegates to its own policy service |
| Terminology server (`[terminology.server]`) | off | codes and value-set references to validate, expand or subsume | validating terminology bindings on write and expanding `TERMINOLOGY()` in AQL |
| Change events over AMQP (`[events]`) | off | commit notifications, free of PHI by default | telling the provider's other systems that a record changed |
| Multimedia to S3 (`[multimedia]`) | off | `DV_MULTIMEDIA` bytes above a size threshold, keyed by content hash | storing large attachments of the record outside the database |
| Access log to syslog (`[audit.syslog]`) and to a FHIR feed (`[audit.fhir_feed]`) | off | access records | an off-box copy of the access log, which the threat model asks for |
| Traces and metrics over OTLP (`telemetry.otlp.endpoint`) | off | spans and metrics, with identified data sanitised out | operating the instance |

Each integration except the usage report carries only what its purpose in the
provider's deployment needs, and only to a destination the deploying
organisation configures.

## Annex I Part II: vulnerability handling

Part II binds Cadasto B.V. These are processes, carried out partly through the
repository's tooling; none is a property of the software a deployment runs.
The manufacturer's policy is the same for every FerroHEALTH product and is
published in the FerroHEALTH book
([security](https://ferrohealth.eu/docs/security.html),
[post-market](https://ferrohealth.eu/docs/post-market.html)); the steps bound
to FerroEHR's repository are `docs/post-market.md`.

| Point | How the manufacturer applies it | Status | Tracker |
|---|---|---|---|
| (1) Identify components; SBOM | an SBOM per published artefact: CycloneDX of the cargo graph per binary, SPDX per image, SPDX of the repository, CycloneDX per `openehr-*` crate attested against its `.crate` archive, and CycloneDX per chart version naming its images by digest, explained in [the SBOMs](../verifying-releases.md#the-sboms-one-per-published-artefact); vulnerabilities in VEX, `deny.toml`, the changelog and each release's exploitability record | met | shipped, #3643 |
| (2) Remediate without delay; security updates apart from functionality | acknowledgement within about 5 and assessment within about 10 business days; a fixed release within 7, 30 or 90 days of the confirmed assessment for a Critical, High or Medium vulnerability and with the next release for a Low one; a fix ships as a security-only patch unless the release notes record why that is infeasible (the FerroHEALTH security policy, linked from `SECURITY.md`) | met as policy from this change on | shipped, #3644 |
| (3) Regular security tests and reviews | daily fuzzing of seven parsers, CodeQL, OpenSSF Scorecard, clippy with the reliability lints, `cargo deny`, image scans, conformance runs, the deploy probe | met | none |
| (4) Disclose fixed vulnerabilities | a GitHub security advisory for every fixed vulnerability, with CVSS, affected and fixed versions and remediation, and the criteria for a delay (the FerroHEALTH security policy, `SECURITY.md`, `docs/post-market.md`); no advisory has been published yet | partial: the commitment is written, the record starts with the next fix | shipped, #3645 |
| (5) Coordinated vulnerability disclosure | the FerroHEALTH security policy, with FerroEHR's reporting routes and scope in `SECURITY.md` and `security.txt` | met | none |
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
| 2 | 2026-10-07 | (a): the per-release exploitability record (#3636), the viewer image scanned before it is tagged (#3674) and every release binary and image binary built with `cargo auditable` (#3675); (b): the declared profile in the chart and the Compose files, the reasons for `sandbox` and the shipped defaults a production holder keeps, and the reset procedure (#3637); Part II(1): SBOMs for the crates and the chart (#3643); the declared security-relevant paths and their release gate (#3649); (m): the erase command and the decommissioning page (#3642); (e): a plaintext `amqp://` broker refused under `production` (#3676); (e): a plain `http://` audit FHIR feed and a multimedia store with `allow_http` refused under `production` (#3689); (b): the Helm reset run by the Kubernetes probe harness (#3677) |
| 3 | 2026-10-08 | Part II: the manufacturer's vulnerability-handling and post-market policy is cited from the FerroHEALTH book, where it moved for every FerroHEALTH product; the measures are unchanged (#3701) |
| 4 | 2026-10-08 | Reviewed for release 4.3.6 against the security-relevant paths changed since 4.3.5: the outbound FHIR emitter and its AMQP flow are gone with the in-tree connector (#3080), so the outbound-flow table names change events alone on AMQP; (a): the `ferroehr-postgres` image drops GnuPG and `gosu`, and every operating-system finding of the three images carries an OpenVEX judgement in `security/vex/` (#3673) |
| 5 | 2026-10-15 | Reviewed for release 4.3.9 against the security-relevant paths changed since 4.3.6. (d): the ABAC patient scope covers queries that read only the EHR row and every query plan, the direct routes refuse a subject-less or ambiguous EHR to a patient caller, and `[authz.abac] patient_namespace` names the claim's namespace (#3771); `ehr_access_default = "restricted"` holds with the role gate off, is refused at boot with authentication off, and the role gate off is the `rbac_off` gap `production` refuses (#3742); the viewer's Basic sign-in follows only a same-origin `next`. (e): protected national identifiers sealed per party version inside the commit, write responses serving the sealed form, plaintext protected values refused on import and admin load (#3772); the server certificate re-read every `reload_interval_secs` and EC keys in SEC1 or PKCS#8 (#3765, #3761); the OIDC `503` and a JWK Set parse error no longer quote the provider's URLs or key material (#3666); the CDR and the viewer verify an identity provider against the platform trust store (#3767). (f): digest version 2 of the audit chain over every column (#3686); the runtime roles read-only on the migration bookkeeping, checked at boot in every database a pool reaches; version-signing errors carry their cause once (#3684). (a): rustls 0.23.46 for RUSTSEC-2026-0346 to -0349, and the release lane checks the linkage the OS VEX statements rest on (#3694); the seven `affected` findings of #3708 are unchanged. The chart: in-chart clients follow the server's TLS (#3714), FerroBRIDGE as an opt-in, cluster-internal workload (#3713), `client_auth: required` refused at render (#3760), an Ingress in front of a TLS listener sending TLS to it (#3766), and the migration Job under its own hook ServiceAccount. No risk added to the register |
| 6 | 2026-10-19 | Reviewed for release 5.0.0 against the security-relevant paths changed since 4.3.11: in `app/ferroehr/src/privacy/detect.rs` the Finnish personal identity code rule cites Valtioneuvoston asetus väestötietojärjestelmästä 128/2010, 2 §, in place of the agency's web page (#3840); the detection and its control-character table are unchanged. No risk added to the register |
| 7 | 2026-10-20 | Reviewed for release 5.1.0 against the security-relevant paths changed since 5.0.0. (d): authentication and the SMART on openEHR scope check move onto the shared `ferrohealth-smart` crate (#3872): only asymmetric algorithms verify a token, so the HMAC key is gone, every route carries a scope requirement, and a `patient/` grant is confined to the launch context's EHR, with the route parameter read from the full request path. The audit FHIR feed and the remote policy service authenticate with an OAuth 2.0 client-credentials token, and a policy service that cannot be reached with one fails the request closed (#3829), so the outbound-flow table names the token requests and the issuer metadata read at boot. (e): the chart mounts a database CA for `sslmode=verify-full` (`database.caConfigMap`), which the Kubernetes probe observed against four TLS clusters (#3788); a rotated mTLS client CA is re-read without a restart (#3769). No risk added to the register |
| 8 | 2026-10-20 | Reviewed for release 5.2.0 against the security-relevant paths changed since 5.1.0. (d): authentication moves to `ferrohealth-smart` 0.3.1 (#3875): the crate now caches verified Basic credentials under a keyed hash (`[auth.basic] cache_ttl_seconds`, default off), refuses a bearer value outside the RFC 6750 §2.1 `b64token` syntax as `invalid_token`, and refuses a blank `sub`, each of which FerroEHR held itself before; `[auth] scopeless_subjects` is honoured under the SMART on openEHR requirement. The EHDS Art. 8 restriction of access hides restricted data from every professional route, query and export, with an emergency override by a configured role and a declared emergency purpose (#3682). (e): a party's protected national identifiers are read back only through an admin route that also needs `authz.rbac.identifier_reader_role`, and every read is an access record (#3776); an EHR extract never carries a sealed identifier between deployments (#3777). No risk added to the register |
| 9 | 2026-10-22 | Reviewed for release 5.3.0 against the security-relevant paths changed since 5.2.0. (l): the access log is written through FerroAUDIT's `ehds-logging` and `ihe-atna` 0.2.4 (#3887, #3902): the record carries the token's issuer, client and `jti`, the ITS-REST operation id and the outcome again, names each category by its HL7 Europe `<system>\|<code>` token and says what the classification was read from (audit migration 0012); the syslog feed sends over RFC 5425 TLS only, so the UDP transport and its `audit_syslog_udp` deployment gap are gone. (d): authentication moves to `ferrohealth-smart` 0.4.0, and a Basic request the verified-credential cache admits writes no login record (#3895); the SMART launch route reads `ehr_id` with the one form decoder every query parameter now uses (#3891). (a): `rustls-webpki` 0.103.16 and the webpki roots 1.0.10 for RUSTSEC-2026-0381 to -0383 (GHSA-pmpg-vfwr-99qv). Both images build on Rust 1.99.1 with the builder digest re-resolved; the other paths changed only in tests. No risk added to the register |
| 10 | 2026-10-22 | Reviewed for release 5.3.2 against the security-relevant paths changed since 5.3.0; the v5.3.0 and v5.3.1 tags exist but their releases never published, so 5.3.2 is the first release since 5.2.0 and revision 9 covers the rest of its changes. (a): the distroless runtime base of both images is re-resolved to `ca-certificates` 20250419+deb13u1 for DSA-6572-1 (#3926, GHSA-pmpg-vfwr-99qv), the only package that differs from the previous base, which removes the compromised eMudhra (emSign) intermediates from the system trust store. No risk added to the register |
| 11 | 2026-10-24 | Reviewed for release 5.4.0 against the security-relevant paths changed since 5.3.3. (f): in `[signing] mode = "pgp"` an ECDSA or Ed448 signing key is refused at boot, so the constant-time defect of RUSTSEC-2026-0387 in `crypto-bigint` 0.5.5 is reached only over public keys and signatures, in verification (#3965, `security/vex/rust-advisories.openvex.json`); the signer takes the newest subkey that may sign, never a revoked or expired one, the primary key only when its self-signature grants signing, and no subkey while the primary key is revoked or expired (RFC 9580 §5.2.3.10, §5.2.3.13; #3967, #3969, #3971). (l): the audit drain closes its syslog connection with a TLS `close_notify` when it exits, so the records it sent last are not lost (#3959, `ihe-atna` 0.2.5). (a): the viewer image build no longer downloads the Tailwind standalone binary, one fewer third-party build input (#3932). No risk added to the register |
| 12 | 2026-10-24 | Reviewed for release 5.4.1 against the security-relevant paths changed since 5.4.0. (d): under SMART an authenticated request for a method the route does not serve is answered `405` with `Allow` before the scope check, and a request without a credential is still `401`, so the scope check runs only on served methods (#3987). (e): the CDR asks one terminology server under `[terminology.server]` with one client credential, mounted by the chart as `secrets.terminologyClientSecret`; its token endpoint URL is masked in every rendering of the grant; terminology inputs travel in a `POST` body, so a code no longer appears in a URL; the per-server mutual-TLS certificates and pinned trust bundles are removed, and the server is verified against the platform trust store (#3085). The chart's NetworkPolicy wording follows the renamed key without a rule change. No risk added to the register |
| 13 | 2026-10-25 | Reviewed for release 5.4.2 against the security-relevant paths changed since 5.4.1. (f): in `[signing] mode = "pgp"` a valid Certification Revocation of a Direct Key signature, created at or after it, cancels that signature, so neither its signing flag nor its key expiration lets the primary key sign (RFC 9580 §5.2.1.13, §5.2.3.10; #3972). (l): an access record whose patient lookup failed says `unavailable` instead of `not-found`, so a search by the patient's identifier can tell a failed lookup from an EHR without a subject (Regulation (EU) 2025/327 Art 9(1); #3900); `ihe-atna` 0.2.7 answers the syslog connection's liveness probe without a timer tick, which removes a millisecond wait per record from the audit drain (#3954). (d): `ferrohealth-smart` 0.5.0 refuses authorization-server metadata that repeats a member name (RFC 8259 §4; #4002). (a): `utoipa-swagger-ui` 10.0.2 takes the yanked zip 8.6.0 and its three extraction advisories out of the build (#3799; GHSA-vv5m-jxxh-4627, GHSA-4v9h-p7m9-rh26, GHSA-4684-h2q9-2vcp). No risk added to the register |
| 14 | 2026-10-27 | Reviewed for release 5.5.0 against the security-relevant paths changed since 5.4.2. (a): the zstd bindings move to zstd 0.14.3, zstd-safe 8.2.0 and zstd-sys 2.1.2, past four RUSTSEC advisories that response compression and decompression reach (#4095, GHSA-cwhv-87pq-3c3c); `ferrohealth-smart` 0.7.1 masks the userinfo of a grant-table URL no parser accepts (#4038, GHSA-vphj-3f8c-5hpx); the PostgreSQL image base is re-pinned to the 2026-10-21 respin of 18.6 (#4084). (d): opaque access tokens are checked by token introspection under `[auth] mode = "introspection"` (#4033); `[auth] ca_file` trusts a private issuer CA on every platform, with its chart value (#3909); the issuer metadata file is read through the shared parse (#4165); the caller's roles come from `ferrohealth-smart` and the policy decision from `ferrohealth-authz`, with `[authz.rbac] user_role` retired (#4028, #4038); Basic credentials are cached for 300 seconds by default (#4088); a cohort query and an AQL query rooted at a demographic class apply the population gate (#4247). (g): the EHR index asks an IHE patient identity source over PIXm by default and a server without one refuses to start unless `[ehr_index] provider = "local"` is set, so subject identifiers leave the instance only to the source the deployment names (#3380, #4242); both new flows are in the outbound data flows table, and the chart's `usageReport.enabled=false` now reaches FerroTERM, FerroBRIDGE and FerroPIX (#4270). (j): the server and the viewer run on the family HTTP stack of `ferrohealth-server` (#4137, #4139), the server reads only the configuration file it is named (#4087), and Template Data Document import and its routes are removed (#4199). (l): the audit trail takes a forwarded client address only from a `[server] trusted_proxies` peer (#4025); syslog frames are delivered from `ihe-atna`'s spool with retries (#4029); the ITI-81 retrieval is read and answered by `ihe-atna` and requires a `date` (#4030, #4040); the access record carries the data permit (#3379, #4120) and every access to one EHR names the EHR and the user (#4192); the chain check verifies from the last intact link (#4100, #4102). (m): each access record is deleted at its own retention end (#3899). No risk added to the register |
| 15 | 2026-10-28 | Reviewed for release candidate 5.6.0-rc1 against the security-relevant paths changed since 5.5.0. (h): `ferrohealth-server` 0.3.7 closes a plain-HTTP connection that sends no request header within `[server] header_read_timeout_ms`, counted from accept, and pauses accepting for up to a second after `EMFILE` or `ENFILE`, so silent connections from a client with no credentials no longer exhaust the file descriptors (#4283, GHSA-2mc2-h3jq-c5wg). (d): the patient-data gate and the SMART route table judge a route by the namespace it is served under, ITS-REST under `/openehr/v1/`, FHIR and IHE under `/fhir/r4/` and the extensions under `/extensions/v1/`, so every moved route keeps its gate and its scope (#4330). (f), (l): the audit store renders each record's `AuditEvent` from its columns under the render policy the record names, the chain digest covers every column, the search tokens included, in one version, each render policy and search token is checked against its digest, and audit migration 0017 refuses a store holding records of an earlier release (#4300, #4302, #4313); a deployment that declares no category map classifies through the map FerroEHR ships, and the production profile refuses one with no map at all as the `no_category_map` gap (#4298). (a): the server and viewer images build on the refreshed `gcr.io/distroless/cc-debian13:nonroot` digest. No risk added to the register |
| 16 | 2026-10-29 | Reviewed for release candidate 5.6.0-rc2 against the security-relevant paths changed since 5.5.0, after revision 15 reviewed 5.6.0-rc1, which was tagged and not published. (d): the ABAC request carries `category` and `origin` resource attributes on composition, contribution and query operations, which the Cedar schema declares and a remote PDP binding may name (#4299). (l): a body without `FEEDER_AUDIT` committed directly by a client the `[origins]` register names records that client's origin instead of the server's `system_id` (#4299). (m): a retention period may be keyed on a registered origin, and the due list lists such a period over the objects of that origin (#4299). (b): the Helm chart has no default deployment profile and refuses to render until `config.deployment_profile` names `sandbox` or `production`, so an installation no longer runs the sandbox posture by omission (#4294). (d): a `[server.tls]` table that sets `client_ca_file` under `client_auth = "none"` is refused at load on `ferrohealth-server` 0.3.9, since such a listener asks no client certificate (#4349, GHSA-52mf-qg39-4q4j). (l): the FHIR rendering of an unclassified access record carries the BALP `entity:data` slice (#4353). No risk added to the register |
| 17 | 2026-10-29 | Reviewed for release candidate 5.6.0-rc3 against the security-relevant paths changed since 5.6.0-rc2. (d): the authentication layer, the ABAC enforcement point and the EHR access gate answer their `401` and `403` as RFC 9457 problem details, and under the FHIR base as an `OperationOutcome`, with the same status and the same decision as before; the refusal names the request id as its `instance` (#4342). (l): FerroEHR takes `ehds-logging` and `ihe-atna` 0.3.2, and the FHIR `AuditEvent` names the object an access touched by its openEHR object id as `entity.what.identifier` (#4366); the audit store writes a drained batch, its search tokens and its token sets in one transaction, and the GIN index on `search_tokens` collects new entries in a 512 kB fast-update list (GIN `fastupdate`), set by audit migration 0019 with 0017 kept at its 5.6.0-rc2 text, so the chain and its digests are unchanged and an audit store that 5.6.0-rc2 migrated upgrades in place (#4375, #4424); the fail-closed `503` of the audit middleware is problem details (#4342). (h): `ferrohealth-server` 0.4.0 closes a kept-alive connection idle for `[server] idle_timeout_ms` (120000 by default), and `[server] header_read_timeout_ms` counts from the first byte of each later request head, so a silent connection is still closed after accept by the head bound and an idle one by the idle bound (#4417). (b): the chart runs FerroPIX 0.0.12 by digest with its chart 0.3.4, whose pods run without service links and in a user namespace by default (#4392). No risk added to the register |
| 18 | 2026-10-31 | Reviewed for release candidate 5.6.0-rc4 against the security-relevant paths changed since 5.6.0-rc3. (d): with `[authz.rbac] enabled`, a bearer token whose SMART on openEHR resource scopes are all `system/` scopes passes the role gate on clinical operations, and its scopes then decide what it reaches through the SMART check that runs after the role gate; a `user/` or `patient/` token, a token with no SMART on openEHR scope and a Basic user still need a role, and an admin operation still needs the admin role (#4467). (j), (a): the viewer image ships one server binary with FerroUI's stylesheet, theme and scripts compiled into it, with no `/site` tree, no WebAssembly and no `cargo-leptos` build tool, and its Content-Security-Policy is `script-src 'self'; style-src 'self'` with no nonce and no `'wasm-unsafe-eval'` (#4433). (d): clinical migration 0026 and party migration 0018 grant `MAINTAIN` on the partitioned `node`, `version` and `vo_attestation`, and on `clinical.ehr`, to `ferroehr_clinical` and `ferroehr_party`, which adds `ANALYZE`, `VACUUM`, `CLUSTER`, `REINDEX` and `LOCK TABLE` on tables those roles already write, and the threat model's grant table names it (#4453). No risk added to the register |
| 19 | 2026-10-31 | Reviewed for release candidate 5.6.0-rc5 against the security-relevant paths changed since 5.6.0-rc4, which was tagged and not published: none of the declared paths changed. (a): the 5.6.0-rc4 vulnerability record stopped the release on CVE-2026-84243, the glibc gettext `LANGUAGE` directory traversal, which has no Debian fix for trixie; it is judged `not_affected` in the three images, since the `ferroehr` and `ferroehr-viewer` programs never leave the C locale and in `ferroehr-postgres` only the operator's container environment can set `LANGUAGE` (#4500, `security/vex/ferroehr-os.openvex.json`, `security/vex/postgres-os.openvex.json`). The viewer takes its Content-Security-Policy from `ferrohealth-server` 0.4.2 with the policy unchanged (#4439). No risk added to the register |

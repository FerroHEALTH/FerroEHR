# Go-live checklist

Run this before a FerroEHR deployment holds real patient data. Every item is
something you set and then read back from the running system, because a
separation that was configured and never verified is a separation nobody has
seen work.

None of it makes a deployment compliant with anything. It is the set of
technical properties the software can hold, checked rather than assumed. The
organisational half stays yours.

<!-- toc -->

## 1. Role separation, verified from outside

Create the five domain roles, give each login role membership of exactly one,
and point each pool at its own DSN:

```toml
[db]
url             = "postgres://app_ehr:***@pg:5432/ferroehr"

[storage.party]
url = "postgres://app_party:***@pg:5432/ferroehr"

[storage.linkage]
url = "postgres://app_linkage:***@pg:5432/ferroehr"
migrate_url     = "postgres://ferroehr_migrator:***@pg:5432/ferroehr"
```

Then check it:

```shell
ferroehr db verify
```

It exits 0 only when the database carries exactly this build's migrations and
no runtime role can read a domain it does not own, and it issues no DDL doing
it. A breach names the role, the object kind and the schema-qualified object.
The server runs the same check at every boot and refuses to serve on a breach,
so this is the early warning rather than the only one.

The statements that create the roles, and what each one may reach afterwards,
are [Operations → Database roles](../operations.md#database-roles-and-least-privilege)
and [the threat model](../threat-model.md#what-each-database-credential-can-reach).

> [!WARNING]
> A login role that is a member of two domain roles crosses the boundary that
> `ferroehr db verify` reports as intact: the check measures the group roles.
> Give each login role one membership. The clinical login role's membership in
> `ferroehr_clinical` also covers the local Audit Record Repository, which that
> role writes; it holds no grant in the other two domains.

- [ ] Five domain roles exist, each `NOINHERIT` and a member of no other.
- [ ] Each login role is a member of exactly one domain role.
- [ ] `ferroehr db verify` exits 0.

## 2. The deployment profile is `production`, and the server boots

```toml
deployment_profile = "production"
```

Under `production` the server refuses to start while a separation is open and
not accepted by name. The refusal lists every open gap with its finding and its
remedy, so one boot tells you the whole list:

```text
deployment_profile = "production" refuses to start: this deployment has not
made the separations production asserts. Make them, or accept each one by name
in deployment_accepts (which is then stated on every boot and on /management/status):
  - shared_credential: ...
```

The gaps are `shared_credential`, `shared_cluster`, `open_subject_namespace`,
`audit_off`, `audit_fails_open`, `audit_syslog_udp`, `open_ehr_access_default`,
`migrate_on_runtime_credential`, `auth_off`, `plaintext_listener`,
`plaintext_broker`, `plaintext_audit_feed` and `plaintext_object_store`. `shared_cluster` is read from
`pg_control_system().system_identifier` on each pool, the audit pool included,
rather than from the DSN text, so two names for one cluster do not pass it.

Two of them are the shipped compatibility defaults, kept as they are because
the conformance instrument and the development stacks need them: the
`EHR_ACCESS` default is `open`, and the audit trail fails open. A production
deployment either changes both, or names each in `deployment_accepts` and
carries the decision.

`plaintext_listener` opens whenever the main listener speaks plain HTTP on an
address reachable off the host. Behind a TLS-terminating ingress, accept it by
name: the entry records that the ingress carries the encryption. It also opens
when a separate management listener (`management.port`) is set, because that
listener binds every interface and never speaks TLS: unset the port so the
surface shares the main listener, or accept the gap by name and keep that port
off any network clinical clients or the internet reach.

`audit_syslog_udp` opens when the syslog feed is on with `transport = "udp"`,
which ships the access log unencrypted and without delivery confirmation. Set
`transport = "tls"`, or accept the gap by name where the UDP path stays on a
trusted segment and the local store or the FHIR feed holds the durable copy.

`plaintext_broker` opens when the change-event outbox (`[events]`) is on with an
`amqp://` URL and `tls = false`, which publishes change events to the broker
unencrypted. Use
an `amqps://` URL or set `tls = true`, or accept the gap by name where the path
to the broker stays on a trusted segment.

`plaintext_audit_feed` opens when the audit FHIR feed (`[audit.fhir_feed]`) is on
with an `http://` URL, and `plaintext_object_store` when the multimedia store is
on with `allow_http = true`. Use `https://` for both, or accept the gap by name
where the repository or the store is reached on a trusted segment.

A gap you have decided to run with goes in `deployment_accepts` by name. It is
then stated on every boot and on `GET /management/status`, so it is run
rather than hidden.

- [ ] `deployment_profile = "production"` and the server starts.
- [ ] `[authz.rbac] ehr_access_default` and `[audit] fail_mode` are the values
      this deployment intends, not the ones it inherited.
- [ ] Every entry in `deployment_accepts` is a decision someone recorded, not a
      leftover.
- [ ] `GET /ferroehr/rest/status` shows the profile and an accepted-gap list
      you recognise.

## 3. Identifier scanning is on and strict

```toml
[privacy.identifier_scan]
mode  = "strict"
rules = ["ch-ahvn13", "ch-epd-pid", "de-kvnr", "fi-hetu", "gb-nhs-number", "nl-bsn", "no-fodselsnummer", "se-personnummer"]
```

Both are the defaults, so the check is that nothing turned them off. `strict`
refuses a clinical write carrying a value one of the active rules claims, and
names the RM path and the rule without echoing the value. Every rule the build
ships is active unless you narrow the list.

Add `patterns` for the identifier kinds no build can ship a rule for: local
medical-record numbers, payer references, a postcode paired with a house
number. Each is a Rust regular expression compiled at boot, and one that does
not compile is a boot error naming it.

If you are migrating existing content, run `mode = "warn"` first, read what it
reports, then switch to `strict`. Going live in `warn` means the scanner
observes and nothing acts.

- [ ] `mode = "strict"`.
- [ ] The `rules` list covers every jurisdiction whose identifiers your content
      can carry.
- [ ] Local identifier kinds have a `patterns` entry.

## 4. The pseudonym namespaces are declared

```toml
[privacy]
subject_namespaces = ["your-pseudonym-service"]
```

Empty is the default, and empty means the subject rule is out of force:
whatever a client sends becomes the clinical subject reference. Declaring a
namespace turns the rule on. From then on `EHR_STATUS.subject.external_ref`
must name a listed namespace and carry a UUID, the server stamps `ehr.posture`,
and a database trigger refuses a non-UUID subject whichever session writes it.

The name is a deployment fact: it identifies the service that mints your
pseudonyms, and no server can invent it for you. Where FerroEHR mints them
itself, `link_as_subject` uses the first declared namespace.

- [ ] `subject_namespaces` names at least one namespace.
- [ ] The `open_subject_namespace` gap no longer appears on
      `GET /ferroehr/rest/status`.

## 5. Backups, one per domain

Backups leave the database with none of the grants attached, so one dump per
cluster undoes the split. Configure one per domain, each with its own
credential and its own volume. On Kubernetes that is the `backup` values:

```yaml
backup:
  enabled: true
  clinical:
    existingSecret: ferroehr-backup-clinical
    persistentVolumeClaim: ferroehr-backup-clinical
  demographic:
    existingSecret: ferroehr-backup-party
    persistentVolumeClaim: ferroehr-backup-party
  linkage:
    existingSecret: ferroehr-backup-linkage
    persistentVolumeClaim: ferroehr-backup-linkage
```

Each credential is a read-only role on its own domain, and it cannot be the
pool's: each domain's runtime role is revoked from the other domains, so a dump
taken through one would be silently partial. The chart refuses to
render when two domains name the same claim, which is the mistake that quietly
puts the join back together.

Three things the chart does not do: it provisions no storage, it prunes no old
dumps, and no network policy of ours selects the dump pods.

- [ ] Three dumps, three credentials, three volumes.
- [ ] The demographic and linkage volumes have the narrower audience.
- [ ] A restore has been rehearsed, and the rehearsal is recorded.
- [ ] Retention and pruning of the dumps is somebody's job, and they know it.

## 6. The audit trail is on, durable, and kept long enough

```toml
[audit]
enabled = true
purpose_header = "x-purpose-of-use"

[audit.store]
enabled = true
retention_days = 0    # or a value at or above your jurisdiction's floor
```

Auditing is on by default with the local store as the sink. What a production
deployment adds is a copy off the box, because a local repository shares the
blast radius of the database it audits:

```toml
[audit.syslog]
enabled = true
transport = "tls"

# or, for a RESTful ARR
[audit.fhir_feed]
enabled = true
```

Decide `fail_mode` deliberately. `open` logs a dropped record and serves the
request; `closed` refuses an auditable operation with `503` when the record
cannot be taken. `closed` is the posture that makes "no unaudited access" true.

Retention has a jurisdictional floor. Where one of your active identifier rules
names a jurisdiction with a registered floor, a shorter horizon is a boot error
naming the floor. Every EU Member State carries three years from each date of
access (EHDS Art. 9(2)); the Netherlands five years, from the
[Besluit vaststelling bewaartermijn logging](https://wetten.overheid.nl/BWBR0042391);
Switzerland one year (DSV Art. 4 Abs. 5). The floors are in calendar years, so
state the horizon as `[audit.store] retention_years`, which is compared
exactly; a `retention_days` is held to the most days those years can span
(1096, 1830 and 366). The [audit trail page](../audit.md#retention-and-who-chooses-it)
lists the floors with their sources. A deployment in a jurisdiction with no
floor registered sets its own horizon and records why.

Verify the chain works before you need it:

```sql
SELECT * FROM audit.verify_audit_chain();
```

- [ ] `[audit] enabled = true` with a durable sink.
- [ ] A copy is forwarded off the box, to a sink the server's own identity
      cannot rewrite.
- [ ] `fail_mode` is a decision, not a default.
- [ ] `retention_years` (or `retention_days`) is `0` or at or above the floor
      that applies to you.
- [ ] `audit.verify_audit_chain()` returns no rows on a healthy trail.

## 7. Schema preparation runs on its own credential

Preparing the schema spans every schema at once: the DDL of all five migration
sets under `migrate = "apply"`, all five `_sqlx_migrations` bookkeeping tables
under `"verify"`. No least-privilege runtime role can do either, `ferroehr_clinical`
included, so `[db] migrate_url` names the credential that can. Unset, it falls
back to `[db] url`, which is the single-credential posture.

- [ ] `[db] migrate_url` (or `migrate_url_file`) is set.
- [ ] `migrate = "verify"` if the schema is applied out of band, with
      `ferroehr db migrate` run under the migrator DSN first.

## 8. The paperwork exists

- [ ] A data protection impact assessment has been carried out and signed off.
      The technical half is [the DPIA page](dpia.md); the purposes, the lawful
      basis and the organisational measures are yours.
- [ ] The Art. 30 record of processing is written. The template is
      [Records of processing](records-of-processing.md).
- [ ] The processor agreements are in place for the database operator and every
      configured peer.
- [ ] Somebody has read
      [what the software cannot do for a data subject](../compliance/shared-responsibility.md).

## What this checklist does not cover

- **Your identity provider.** Every authorization stage reads claims it
  asserted. Its token lifetimes, binding and revocation are outside the
  software.
- **The platform.** Host hardening, network policy, image admission and secret
  management are
  [Cluster hardening](../installation/kubernetes-hardening.md).
- **Encryption at rest.** FerroEHR stores queryable JSONB and does not encrypt
  clinical payload before it reaches PostgreSQL. That is the database
  operator's layer.
- **Clinical governance.** Validation checks structure, invariants and
  terminology bindings. It does not know whether a recorded fact is true.

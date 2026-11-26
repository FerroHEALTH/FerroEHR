# Tamper evidence, seals and retrieval

How the audit trail shows it has not been changed, how a log is sealed and moved, and how
an audit record repository retrieves records (ITI-81) and authenticates nodes (ITI-19).

<!-- toc -->

## Tamper evidence

An audit trail that anything with the application's database password can
quietly rewrite is a log, not an accountability record. The local store is
therefore tamper-**evident**. The system records (`audit.audit_event`, the
system chain) and the access log's entries are each sealed in windows, each
seal following the one before it and covering every record of its window by
its position and content digest
([Seals, checkpoints and the transfer of a log](#seals-checkpoints-and-the-transfer-of-a-log)).

A request writes each system record once, in `audit.audit_event`, and never
changes it. Its position (`seq`) is drawn after its transaction took its
id, and its content digest is computed by the server when it writes the
record: the SHA-256 of the RFC 8785 form of the array
`["ferroehr.audit_event.v7", {column: value}]` over every column but `seq`
and the digest, with SQL `NULL` as `null`, booleans as booleans, text and
UUIDs as strings, integers (`integer[]` element by element) as decimal
strings, instants as `YYYY-MM-DDTHH:MM:SS.ffffffZ` in UTC, dates as
`YYYY-MM-DD`, `bytea` as lower-case hexadecimal and `jsonb` as its value
(`system_log::content_digest`, with the SQL that builds the object for an
auditor). A column added by a later release joins the digest under a new
form in the same release. A record a write still holds is in no search, no
feed and no window. Which records a sink still owes is told by its cursor
(`audit.delivery_cursor`): the records past it up to the delivery frontier,
and those it parked ([How a record is delivered](audit.md#how-a-record-is-delivered)).

The store keeps a record's facts in columns and renders its FHIR `AuditEvent`
from them when the record is read (ITI-81) or delivered (the FHIR feed). What
the rendering takes from the configuration (`source_id`, `enterprise_site_id`,
`server_host`, `value_if_missing`, the access policy's log retention and the
audit category register) is kept once per distinct configuration in
`audit.audit_render_policy`, keyed by the SHA-256 of its text, and every record
names the policy it was written under. A record therefore renders the same
document after the configuration changes, and the verification below reports a
policy whose text no longer matches its digest, as it reports a changed record.
Release 5.6.0 starts on an empty audit store: its migration refuses a store
that holds records written by an earlier release, and names the remedy.

Four controls sit on top of it, and they are separate on purpose:

- **The table refuses the ordinary rewrite paths outright.** An `UPDATE`, a
  `DELETE` and a `TRUNCATE` of a stored record are refused by the database,
  and so are a change to a seal or to a reaped position.
  Retention pruning goes through the one sanctioned deletion path, which keeps
  each removed record's position and content digest
  (`audit.audit_event_reaped`) so its window still verifies, and removes those
  only once every record of a sealed window is reaped, recording the window as
  folded; so reaping does not look like tampering, and tampering does not look
  like reaping.
- **The privileges are narrow.** The runtime role may record an event, stamp it
  forwarded, and read the trail back. It holds nothing that can rewrite a
  record, remove one, or alter a reaped position. That posture is only
  fully in force with `database.migrations = "verify"`; see
  [Operations](database-operations.md#database-roles-and-least-privilege), because a
  self-migrating server owns the schema and can therefore turn the enforcement
  off.
- **Verification is something you can run yourself.** For the system
  records it is `ferroehr audit-chain verify`, for the access log
  `ferroehr access-log verify` (or `POST /admin/access-log/verify`); each
  recomputes every sealed window, every record's content digest, and follows
  every seal from the one before it to the seal state. No finding means the
  trail is intact. A finding names one record or window (its position, the
  record's id and time when it is still there) and what is wrong with it:
  content modified after it was written, a render policy, search token or
  search set changed or missing, a record changed, removed or inserted inside
  a sealed window, or seals removed from the end.
- **The server runs the check itself, on a schedule.** With the local store on,
  the check runs one minute after boot and then every
  `[audit.store] verify_interval` (default `3600`, one hour; `0` turns
  it off). Each run verifies the windows sealed since the newest window an
  earlier run found intact, and the seal state, so its cost follows the
  records sealed since rather than the size of the trail. It walks each
  log's sealed windows from its own cursor, one snapshot per step, each
  sized to take about half a
  second, and each intact step moves the cursor, so a backlog of any size is
  verified in one run and no statement nears the
  `[database] statement_timeout`. After each step the run pauses at least as
  long as the step took, and longer the more the step's cost per record rose
  above the lowest the server has measured for that chain: a step that cost
  twice as much pauses four times as long, up to 16 times. A catch-up
  therefore takes about half a core of a quiet database and yields to the
  requests under load, where the lag can grow until the load falls. How far
  a cursor is behind is
  the `audit_chain_verification_lag` and `audit_chain_unverified_age_seconds`
  metrics; once the oldest unverified record has waited longer than
  `[audit.store] max_verify_lag` (default four hours), the `audit_chain`
  indicator reads `DEGRADED` and names the chain.
  A full pass
  runs every `[audit.store] full_verify_interval` (default `604800`,
  one week; `0` leaves it to the commands above) and after every run that found
  damage, so damage to an older record stays reported. A run that finds damage logs at `ERROR`, one summary line with the
  finding count and one line per finding (the first 20), and adds the count to
  the `atna_audit_chain_findings` counter with `kind = "damage"`. A run that
  cannot verify at all counts as `kind = "unverifiable"`, because a trail that
  cannot be checked is never a pass. The latest result is the `audit_chain`
  indicator on `GET /health/readiness`: `UP` while intact and `DEGRADED`
  when damaged or unverifiable; its detail on `GET /management/health` names
  the first finding. It never flips
  readiness, because the trail keeps recording while the damage is
  investigated. Alert on the counter or on that component, and use the
  commands above to see every finding. Each replica runs its own check.

> [!IMPORTANT]
> This is **detection, not prevention**, and the boundary is worth stating
> plainly. The unsigned seals are unkeyed, so a party with unrestricted
> write access to the `audit` schema (the database owner, or a superuser) can
> delete a record and recompute every seal after it; a signed seal kept
> outside the database shows that. A record newer than the last seal, at
> most a minute plus the frontier's lag, is protected by the refused changes
> and the grants alone. What closes that case is keeping the trail
> somewhere that party does not control. Enable
> `[audit.syslog]` or `[audit.fhir_feed]` so records leave the box as they are
> written, and give the server an app-role-only DSN so it is not that party.

## Seals, checkpoints and the transfer of a log

The records of the openEHR access log (`audit.access_log_record`) hold only
the accesses to EHR data, apart from the `audit_event` records above, the
system chain (an authentication, a template, party or application
operation). Neither carries a chain (#6242). A request writes each entry or
system record once, before its response, and every reader, the delivery and
the sealer read that row. What follows holds for both logs, each with its
own id sequence, holds, seals and tag, unless it names one.

- **The frontier.** A round on every replica's audit runtime, about once a
  second, moves the log's frontier: the highest entry id at or below which
  every entry is committed or never will be. Every insert takes its
  transaction's id before any row draws its entry id, so an entry id the
  round has seen belongs to a transaction that a later snapshot shows ended.
  A long writing transaction anywhere in the database delays the frontier
  and loses nothing: an entry is in the log from its own commit. The
  entries of a write whose commit is not yet confirmed are held, in no read,
  and the frontier the sealer and the delivery use stops below them.
- **A window a minute.** About once a minute the sealer seals every entry
  id past the last window, up to that bound, as one window. A seal is the
  SHA-256 of this text, which you can rebuild from the stored rows:

  ```text
  27:ferroehr.access_log.seal.v1   (the system records: 27:ferroehr.system_log.seal.v1)
  <length>:<window number>
  <length>:<first entry id>
  <length>:<last entry id>
  <length>:<number of entries>
  64:<the previous seal's digest>
  then, for each entry of the window in id order:
  <length>:<entry id>
  64:<the entry's content digest>
  ```

  Each field is its length in bytes in decimal, a colon, the field and a
  line feed; integers are decimal, digests lower-case hexadecimal. The first
  window follows the SHA-256 of the version tag. A tombstone keeps the content
  digest of the entry it replaced, and a reaped system record leaves its
  position and content digest, so its window verifies after the content is
  gone. `GET /admin/access-log/seals` answers the access log's seals over a
  span of entry ids.
- **The signed checkpoint.** With an OpenPGP key in the version-signing
  keyring, each seal is signed as the `ACCESS_LOG_CHECKPOINT` of the log up
  to the window's last entry id, whose `digest` is the seal's digest, in the
  form RM common `master04-generic_package.adoc` §Attestation gives
  `ATTESTATION._proof_`. A deployment without a key seals without signing,
  and `ferroehr access-log checkpoint` refuses. `ferroehr access-log
  checkpoint` and `POST /admin/access-log/checkpoint` seal every committed
  entry and answer the newest checkpoint; `GET /admin/access-log/checkpoint`
  answers the newest stored one, or the one at `sequence`.
- **What is left unsealed.** A record of either log newer than the last
  seal, at most a minute and the frontier's lag, is protected by the refused
  `UPDATE` and `DELETE` and by the runtime role's grants alone. The CRA risk
  assessment (C4, C5) and hazard H7 state that window.
- **Publish the checkpoints outside the database.** Someone who can write
  the database can remove the newest windows together with their seals, and
  nothing left in the database shows it. Set `[audit.access_log]
  publish_directory` to storage that person cannot rewrite: every 30 seconds
  the server writes the newest checkpoint there as its own file,
  `access-log-checkpoint-<last entry id>.json`, created once and never
  rewritten. Without it the `access_log_checkpoints` health indicator reads
  `DEGRADED`. To verify the log against a published checkpoint, run
  `ferroehr access-log verify --checkpoint <file>` with the newest file.
- **Verification.** `ferroehr access-log verify` and `POST
  /admin/access-log/verify` recompute every window from its rows and every
  column the log derives from an entry's content, follow the seals from one
  to the next to the sealer's state, and check the checkpoint's signature and
  that the seal at its position is the one it signs. A changed, removed or
  inserted record inside a sealed window is reported, as is a removed seal.
  They also verify every imported bundle.
- **The fold.** Once every record of a sealed window is a tombstone past its
  end, the retention run removes the window's entries and records the window
  as folded with the grounds its tombstones named. Its seal stays, so the
  seals still follow one another.
- **Moving the log with the EHR.** `ferroehr access-log export <ehr_id>` and
  `POST /admin/access-log/export?ehr_id=…` export one EHR's records as a
  bundle: each record with its content digest, signed whole by this system's
  keyring over its RFC 8785 form without its proof. It carries no record or
  digest of another person. `ferroehr access-log import <file>` verifies the
  signature against the certificates in `[audit.access_log]
  trusted_key_files` and every entry's digest, keeps the records with their
  `system_id`, `retain_until` and content digest, and records the import as
  an entry of this log that names the bundle's digest and a digest over the
  imported records' uids and content digests, so this log's seal covers
  them. An imported entry ends at the `retain_until` it states; its
  tombstone is kept as long again and then folds, leaving its uid, bundle
  and content digest so the bundle still verifies. A repeated import stores
  nothing twice.
- **The system records are sealed too.** The same sealer and key seal their
  windows (`audit.audit_event_seal`), each signed as a seal naming the
  window's last position and its digest. `ferroehr audit-chain seal` seals
  every committed record on demand and prints the newest signed seal, and
  `ferroehr audit-chain verify [--seal <file>]` recomputes every window,
  checks the seal's signature and that the window ending at its position has
  the digest it fixes, and exits 1 when the records do not verify. Retention
  removes a system record at its own end and keeps its position and content
  digest; once every record of a sealed window is reaped, the window's
  reaped positions are removed and the window recorded as folded.
- **The erasure is recorded once.** `POST /admin/access-log/erase` writes the
  tombstone and the entry that records the erasure in one transaction; the
  erasure is not written a second time as an `audit_event` record.

## Retrieving audit records (ITI-81)

The RESTful-ATNA **ITI-81 Retrieve ATNA Audit Event** transaction is a FHIR
search on `AuditEvent`, so it is served at FerroEHR's FHIR R4 base,
`/ferroehr/rest/fhir/r4`, beside the openEHR API rather than under it, where a
FHIR client or an IHE Audit Consumer expects `[base]/AuditEvent`. It is the one
FHIR route FerroEHR serves:

```text
GET /ferroehr/rest/fhir/r4/AuditEvent
```

It returns a FHIR `searchset` Bundle of the matching records' `AuditEvent`
documents, rendered from the store, newest first, with a `self` link and a
`next` link while more pages follow. The page carries the full match `total`
only when the query asks for it with `_total=accurate` or `_total=estimate`
(FHIR R4 Search §3.1.1.5.2): counting every match reads every record the
filter selects, and a page alone reads one record past what it serves. The query is read by `ihe-atna`, the family's audit
crate. At least one `date` parameter is required (RESTful ATNA
§3.81.4.1.2.1): a query without one is a `400`. Every search parameter
§3.81.4.1.2.2 says the repository shall support is applied. A parameter
with several comma-separated values matches any of them, and every
parameter of the query must match. A token takes the three forms of FHIR R4
Search §3.1.1.4.10: `code`, `|code` (recorded without a system) and
`system|code` (or `system|` for any code of that system). FerroEHR records
identifiers without a system, except the object of an access record, whose
system is `urn:openehr:object-id`: `entity.identifier=urn:openehr:object-id|<uid>`
and `entity.identifier=<uid>` find it, and a query that names another system
for an identifier matches nothing. Every parameter answers through an index. A
parameter on an element of the document (`address` and the `.identifier`,
`entity-type`, `entity-role`, `type` and `subtype` parameters) matches the
search tokens the store takes from the record's rendered document
(`audit.audit_search_token`, `audit.audit_search_set` and the record's
`search_tokens`): a system record's when the store writes it, an access-log
entry's when the frontier round renders it from the stored entry into
`audit.access_log_index`, about once a second, which go with the entry's
content when a tombstone replaces it. A search on such a parameter first
moves the index past every entry written before it, so it finds an entry
written a moment ago; when an open database transaction keeps it from doing
so within 2 seconds, the search answers `503` with `Retry-After`. So the
search finds what the served document says and renders only the page it
serves. An `entity.identifier` that carries
the record's request id or the object it names (the version uid of a create)
is read from the record's own column instead, so a request adds no row to
`audit.audit_search_token`. One GIN index per record table holds the search
tokens and a key for each of the request id and the touched resource id
(`audit.search_keys`), which answers `entity` and such an `entity.identifier`
as well; the search then compares the text. Retention deletes the tokens
that no remaining system audit record, access-log entry or search set names,
and never one a kept record names.

| Parameter | Matches |
|---|---|
| `date` | **required**, repeatable: a FHIR date value with a prefix (`eq` when none, `gt`, `lt`, `ge`, `le`, `sa`, `eb`); a day such as `2026-07-12` covers the whole day, read in UTC |
| `patient` | the recorded patient (EHR subject) id |
| `patient.identifier` | `entity.what.identifier` of the entity whose role is Patient |
| `agent` | the authenticated principal |
| `agent.identifier` | `agent.who.identifier` |
| `entity` | the touched resource id |
| `entity.identifier` | `entity.what.identifier` of any entity |
| `entity-type` | `entity.type` |
| `entity-role` | `entity.role` |
| `source.identifier` | `source.observer.identifier`, the audit source id |
| `type` | `type` |
| `subtype` | `subtype` (the REST interaction and the openEHR operation) |
| `address` | a part of `agent.network.address`, compared without regard to case |
| `outcome` | the outcome indicator: `0`, `4`, `8` or `12` (FerroEHR records `0`, `4` and `8`) |
| `action` | the action code: `C`, `R`, `U`, `D` or `E` |
| `_count` / `_offset` | paging; page size defaults to 50, capped at 1000; `_count=0` returns the total alone |
| `_total` | `accurate` or `estimate` adds the full match `total`; `none`, or no `_total`, serves the page without it; any other value is a `400` |

A malformed value is a `400` carrying a FHIR `OperationOutcome` with
`invalid`; another FHIR R4 `AuditEvent` parameter, which §3.81.4.1.2.2 does
not ask for, is a `400` with `not-supported`; a parameter the crate does not
know is ignored
(§3.81.4.1.3). Under RBAC the
unscoped retrieval is **admin-only**; a caller holding a
`read_subject_audit` grant, an app acting for the people it
serves, reads it **scoped to one subject**: one
`patient` or `patient.identifier` with one value is required, and the caller
is refused with a `403` that names the parameter when it omits
it. Reading a subject's log is itself recorded as an access naming that
subject and the record count served. An
[justified access](ehds-logging.md#justified-and-emergency-access) in the subject's log carries the clinician's reason
as the text of its purpose of use, and an emergency its mark as the
`ehds-emergency-access` entity. The surface answers `404` when the local
store is disabled.

The subject-scoped ITI-81 read stays beside the [access-log
resources](access-log.md#reading-the-access-log-over-its-rest). It serves the ATNA records
as FHIR `AuditEvent`s to a portal that holds a `read_subject_audit` grant and
names the subject itself; `GET /ehr/{ehr_id}/access_log` serves the openEHR
access-log entries of the same accesses to the person, or their
representative, under the person's own credential.

```bash
# Who accessed patient-42's data this month?
curl -u admin:pw \
  "https://cdr.example.org/ferroehr/rest/fhir/r4/AuditEvent?patient=patient-42&date=ge2026-07-01T00:00:00Z"
```

This is also the openEHR "record demerging" instrument: when data lands in
the wrong EHR, the patient-filtered audit search shows exactly who read it.

> [!NOTE]
> Audit records are themselves sensitive (they name patients, subjects and
> actions) so this endpoint is a PHI-disclosure surface and is authorized like
> any other.

## Node authentication (ITI-19, mutual TLS)

ATNA's second half is node authentication. `[server.tls]` terminates TLS
natively and can demand a verified client certificate:

```toml
[server.tls]
certificate_file = "/etc/ferroehr/server.pem"
key_file = "/etc/ferroehr/server.key"
min_version = "1.3"               # 1.3 (default) | 1.2
client_auth = "required"          # none | optional | required
client_ca_file = "/etc/ferroehr/client-ca.pem"
```

The protocol floor is **TLS 1.3 only** by default, as RFC 9325 §3.1.1 has it
supported. Setting `min_version = "1.2"` enables
1.2 *alongside* 1.3, never instead of it, for a client that genuinely cannot
do 1.3, such as an older integration engine or a pinned Java runtime. TLS 1.1
and 1.0 are refused when the configuration is read (RFC 8996).

With `client_auth = "required"`, only clients presenting a certificate
chaining to your explicit trust anchor complete the handshake: the IHE
mutually-authenticated-node posture. Deployments terminating TLS at an
ingress leave `[server.tls]` unset and enforce mTLS there instead.

> [!WARNING]
> The separate-port management listener (`management.listen`) stays **plain HTTP**
> even with `[server.tls]` set, and it binds all interfaces. Treat it as an
> internal surface and keep it off any publicly routed port. Under
> `deployment.profile = "production"` setting it opens the `plaintext_listener`
> gap, so the server refuses to start unless the gap is accepted by name.

Complete the posture with time synchronisation (IHE Consistent Time): run
NTP/chrony on every node so audit timestamps align across systems.

## Against independent ATNA implementations

The transactions above are tested by FerroEHR's own suites, which read the
profiles the way FerroEHR's authors read them. EHDS Annex II 1.4 asks that a
component meant to work with other products does so reliably, so
[`scripts/interop/atna.sh`](https://github.com/FerroHEALTH/FerroEHR/blob/main/scripts/interop/atna.sh)
runs the quickstart against software FerroEHR's authors did not write, each
from a published image:

| Transaction | Independent implementation | What the run requires |
|---|---|---|
| ITI-20 FHIR feed | HAPI FHIR, an open-source FHIR R4 server, as the Audit Record Repository, with IHE BALP 1.1.4 loaded | It accepts every record, and its validator finds no error against base R4 and the BALP profile each record names |
| ITI-20 syslog | syslog-ng over RFC 5425 TLS | Every message parses as RFC 5424, and its payload is a well-formed DICOM PS3.15 A.5.1 `AuditMessage` (libxml2) |
| ITI-81 | HAPI FHIR's AuditEvent search over the records the feed delivered | The same searches answer the same records on both |

```shell
cargo build --release -p loadtest
bash scripts/interop/atna.sh ghcr.io/ferrohealth/ferroehr:<tag> \
  ghcr.io/ferrohealth/ferroehr-postgres:<tag> atna-out
```

It prints every disagreement and exits non-zero on any. A disagreement is
either FerroEHR's or a shared crate's defect, filed and fixed, or the other
side's, recorded with the profile text that shows it.

The run of 2026-10-29 found no disagreement: HAPI FHIR accepted and validated
all 71 records of the window, syslog-ng parsed all 72 messages, and the ten
ITI-81 searches answered the same records on both repositories
([the record](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/conformance/interop/atna/2026-10-29.md)).

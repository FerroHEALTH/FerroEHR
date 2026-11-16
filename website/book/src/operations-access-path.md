# Access path metrics and runbooks

Every request that reaches an EHR passes the access path: the policy copies
an instance holds in memory, the access evaluator that decides each EHR, and
the access log that records the decision. This page lists the metrics you can
alert on for each part, the health indicator that goes with them, and what to
do when one of them reports a failure. No openEHR specification governs
operational metrics; the metrics, their labels and the runbooks are
FerroEHR's own design.

<!-- toc -->

## Reading these metrics

The metrics come from the same meter provider as the rest of the
[observability catalogue](operations.md#observability): the Prometheus
reader behind `/management/prometheus`, and the OTLP reader when you
configure `[telemetry.otlp]`. The names below are the names the Prometheus
exporter renders: a counter ends in `_total`, a histogram in seconds in
`_seconds_bucket`, `_seconds_sum` and `_seconds_count`, and a gauge in seconds
in `_seconds`.

Every label takes a value from a closed set that FerroEHR defines: a copy, a
listener, an outcome, a `refused-*` code, a chain. No label carries a patient
identifier, an EHR id, a party or clinical content, and no label grows with
the data, so the series count is fixed whatever the size of the repository.
An instance reports its own copies and its own listeners; with
[several instances](installation/several-instances.md), alert per instance.

## The policy copies and their listeners

An instance holds five policy copies, each kept fresh by a `LISTEN` on its
own notification channel. The `listener` label names the copy as the
`policy_listeners` health indicator does: `ehr_access` (each EHR's
`EHR_ACCESS` settings and the settings classes), `grants` (the grants and the
caller facts), `party_binding` (the lookups of the binding side table and the
bound `PERSON`s' names), `policy_registers` (the operational-policy registers)
and `access_policy`.

| Metric | Type | Labels | What it tells you |
|---|---|---|---|
| `policy_listener_up` | gauge | `listener` | `1` while the copy's `LISTEN` connection is up, `0` while it is lost. |
| `policy_listener_losses_total` | counter | `listener` | Connections lost after the listener had connected. |
| `policy_notifications_total` | counter | `listener` | Notifications the listener received. |
| `policy_notification_age_seconds` | gauge | `listener` | Time since the listener's newest notification; absent before the first. |
| `policy_copy_reads_total` | counter | `listener`, `reason` | Reads of the database a copy made on a request: `notified` after a notification marked it stale, `unlistened` because its listener was down. |
| `grants_refresh_duration_seconds` | histogram | `kind`, `outcome` | Each refresh of the grants copy: `kind` is `unchanged` (one revision read), `grants` (every grant and fact read again) or `facts` (the caller facts changed since the revision held); `outcome` is `ok` or `failed`. |

Alert on `policy_listener_up == 0` held for a few minutes, and on a rate of
`policy_copy_reads_total{reason="unlistened"}` above zero: both mean the
instance answers every request with a database read. A rise of
`grants_refresh_duration_seconds` is the grant and fact refresh lag.

## The binding copy

The `party_binding` copy holds what resolves a login to its `PERSON`: the
lookup of each login's key, for at most 65536 logins, and the name of each
bound `PERSON`, so a request whose caller it holds reads neither. A change of
the binding side table drops every lookup, and a new version of a `PERSON`
drops its name, on every instance once the notification arrives; the instance
whose own commit made the change drops its copy before it answers, so a
deprovisioned login is refused on its next request. In `access_copy_lookups`,
`access_copy_refills` and `access_copy_drops` the `copy` label is `binding`
for the lookups and `party_name` for the names.

## The per-EHR access copy and the class copy

The `ehr_access` copy holds at most 4096 EHRs' settings, and beside them the
settings classes a query is judged over
([the bound](security.md#per-ehr-access-control-ehr_access)). The `copy` label is `ehr` or `classes`.

The care relationships of the EHRs' subjects sit beside the grants, in two
caches with fixed defaults: the care relationships of at most 4096 subjects,
and the patients of care teams holding at most 262144 patients together. A
team with more patients than that is read on each use and never held. A miss
reads one indexed statement on the party pool, and the `grants` listener's
notification drops what it changed. A care relationship that cannot be read
answers `503`, so no decision is made without it.

| Metric | Type | Labels | What it tells you |
|---|---|---|---|
| `access_copy_entries` | gauge | `copy` | EHRs the per-EHR copy holds, and classes the class copy holds. |
| `access_copy_lookups_total` | counter | `copy`, `result` | Lookups while the listener is up: `hit` (served from the copy) or `miss` (this request read the database). A request that waited on another request's read counts as a hit. |
| `access_copy_refills_total` | counter | `copy`, `outcome` | Reads of the settings projection that fill the copy: `ok`, or `failed` (the request answered `503`). |
| `access_copy_drops_total` | counter | `scope` | Drops: `ehr` (one EHR's settings changed), `classes` (a new class), `every` (a listener connected or was lost, or a refill notification). |

What the copies and the care caches hold at scale is on the
[Performance](performance.md#the-access-path-at-scale) page.

The hit ratio is `rate(access_copy_lookups_total{result="hit"}[5m]) /
rate(access_copy_lookups_total[5m])`. Alert on any rate of
`access_copy_refills_total{outcome="failed"}`. While the listener is down
these lookups are not counted: each request reads the database and counts in
`policy_copy_reads_total{listener="ehr_access",reason="unlistened"}`.

## The access evaluator

| Metric | Type | Labels | What it tells you |
|---|---|---|---|
| `access_decisions_total` | counter | `outcome`, `code` | One decision per EHR a request reached: `outcome` is `admitted` or `refused`, and `code` is the `refused-*` code of the refusal (`none` for an admitted access). |
| `access_emergency_overrides_total` | counter | `lifted` | What an emergency lifted: `opt_out` (EHDS Art. 10(2)), `restriction` (EHDS Art. 11(5)), `consent` (a consent deny), `rule` (a deny rule of the access policy) or `policy` (steps 6 to 8, by the break-glass role). |

The `refused-*` codes are the evaluator's own (`refused-opt-out`,
`refused-restricted`, `refused-consent-denied`, `refused-policy-denied`,
`refused-sensitivity`, `refused-no-care-relationship`,
`refused-purpose-missing`, `refused-consent-required`,
`refused-not-subject`, `refused-not-represented`,
`refused-consent-reference-missing`); the access
record of each refusal names the step and the directive that decided it. A
refusal is the evaluator working as configured, so alert on a change in the
rate of a code against its usual level. Alert on every increase of
`access_emergency_overrides_total`: each one is an emergency access the
controller reviews.

## The access log

| Metric | Type | Labels | What it tells you |
|---|---|---|---|
| `access_log_entries_total` | counter | `outcome` | Audit records a request wrote into the intake before its response: `ok` or `failed`. A `failed` write refused its request with `503`. |
| `access_log_append_duration_seconds` | histogram | `outcome` | Time of one request's intake write, one transaction: the time a request waits for its records. |
| `access_log_chain_hold_duration_seconds` | histogram | `outcome` | How long one chainer round held the head of the access log. Its rate of sum is the lock occupancy: the fraction of time the head is held. |
| `audit_chained_total` | counter | `chain` | Records the chainer linked, `access_log` or `system`. Beside `access_log_entries_total` it shows whether the chainer keeps up. |
| `access_log_sign_duration_seconds` | histogram | `outcome` | Time from the start of a chainer round's checkpoint signature to its end, measured when the round stores the checkpoint; it runs beside the insert of the records while the head is held. Present only with an OpenPGP key. |
| `audit_seal_duration_seconds` | histogram | `chain`, `outcome` | Each scheduled seal run of the system chain (`chain` is `system`); `outcome` is `sealed`, `empty` or `failed`. The access log needs no schedule: every append seals itself. |
| `audit_seal_last_success_seconds` | gauge | `chain` | Unix time of the newest seal run of the system chain that did not fail. |
| `access_log_tombstones_total` | counter | `chain` | Entries the retention run replaced by their tombstones, `local` or `imported` from another system. |
| `access_log_folded_total` | counter | `chain` | Tombstones the retention run folded into gaps of the chain once they were past their own end, `local` or `imported`. |
| `access_log_gaps_total` | counter | `chain` | Gaps the retention run wrote, one per folded run of tombstones, `local` or `imported`. |
| `access_log_retention_duration_seconds` | histogram | `outcome` | Each hourly retention run over the access log. |

The seals run every five minutes when `[record.signing]` holds an OpenPGP
key, so a checkpoint age above fifteen minutes means at least two runs in a
row failed.

## Verification of the audit chains

| Metric | Type | Labels | What it tells you |
|---|---|---|---|
| `audit_chain_verification_duration_seconds` | histogram | `scope`, `result` | Each scheduled verification: `scope` is `full` or `since_checkpoint`; `result` is `intact`, `damaged` or `unverifiable`. |
| `audit_chain_last_verification_seconds` | gauge | (none) | Unix time of the newest finished verification. |
| `audit_chain_intact` | gauge | (none) | `1` when the newest verification found both chains intact, `0` when it found damage or could not run. |
| `atna_audit_chain_findings_total` | counter | `kind` | Findings of the verifications, `damage` or `unverifiable`. |
| `audit_chain_verification_lag` | gauge | `chain` | Positions of the chain (`system` or `access_log`) past its verification cursor: the head minus the cursor, measured after each step of a run. |
| `audit_chain_unverified_age_seconds` | gauge | `chain` | Seconds since the oldest record past the chain's cursor was recorded, `0` at the head. |

Alert on `audit_chain_intact == 0`, on `time() -
audit_chain_last_verification_seconds` above twice `[audit.store]
verify_interval`, and on `audit_chain_unverified_age_seconds` above
`[audit.store] max_verify_lag`.

A run walks each chain in steps of at most 20 000 records, one statement
each, and every intact step moves the cursor. A long backlog therefore shows
as a lag that falls during the run, never as `unverifiable`.

## The instances of a deployment

| Metric | Type | Labels | What it tells you |
|---|---|---|---|
| `instance_mismatches` | gauge | `differs` | Live instances of the deployment that differ from this one at its last comparison: `differs` is `signing_key` or `configuration`. Absent before the first comparison and after one that could not read the instance table. |

Each instance compares itself with every instance that touched its row in the
last 90 seconds, at start and every 30 seconds
([Running several instances](installation/several-instances.md)). Alert on
`instance_mismatches > 0`.

## Health indicators

The [health document](operations.md#choosing-a-health-endpoint) reports the
same failures as indicators. Each one reads `DEGRADED` and keeps the instance
in rotation, because every decision stays correct and the trail keeps
recording while you investigate.

| Indicator | Reads `DEGRADED` when |
|---|---|
| `policy_listeners` | a listener is lost, naming each one |
| `instances` | another live instance signs with another key or runs another configuration, naming each one |
| `audit_chain` | the newest verification found damage, or could not run |
| `audit_sender` | auditing is off, or the oldest intake record has waited more than 10 seconds for the chainer |
| `audit_delivery` | a forwarder cannot reach its Audit Record Repository |

## Runbooks

### A lost policy listener

- **Metric:** `policy_listener_up{listener="…"} == 0`, with
  `policy_copy_reads_total{reason="unlistened"}` rising.
- **Health:** `policy_listeners` reads `DEGRADED` and names the listener.
- **What happens:** the copy reads the database on every request until its
  listener connects again. Decisions stay correct; only latency and database
  load rise. The listener tries to connect again every second.
- **What to do:**
  1. Check the instance's log for `the policy listener failed` or `the policy
     listener could not connect`, with the channel name.
  2. Check that the instance reaches the PostgreSQL primary. A hot standby
     accepts no `LISTEN`.
  3. Check for a pooler in transaction mode between the instance and the
     database. It drops the `LISTEN`; the listeners need a direct or
     session-mode connection
     ([Running several instances](installation/several-instances.md)).
  4. Check `pg_stat_activity` for sessions ended by an operator or by
     `idle_session_timeout`; `policy_listener_losses_total` counts each loss.
  5. When the listener is back, `policy_listener_up` reads `1` and the
     indicator reads `UP`. The copy is dropped once on reconnect, so expect a
     short burst of `access_copy_lookups_total{result="miss"}`.

### A copy that does not refill

- **Metric:** `access_copy_refills_total{outcome="failed"}` rising, or
  `grants_refresh_duration_seconds_count{outcome="failed"}` rising.
- **Health:** `db` may read `DOWN`; `policy_listeners` may read `DEGRADED`.
- **What happens:** a request whose EHR is not in the copy answers `503
  Service Unavailable` with `Retry-After`, because a privacy control that
  cannot be read refuses rather than admits. A failed grants refresh keeps
  the grants held and marks them stale, so the next request reads again.
- **What to do:**
  1. Check `db` on the health document and `db_pool_connections`: a pool at
     its limit or an unreachable database fails every refill.
  2. Check the log for `EHR_ACCESS settings do not parse` or `a settings class
     does not read`. These name a stored row that does not read; the EHR or
     class stays withheld until its settings are committed again.
  3. A copy that refills and is dropped at once shows as a high rate of
     `access_copy_drops_total{scope="every"}`: a listener connecting and
     losing its connection in a loop. Treat it as a lost listener.

### Requests answer `503` because their records cannot be written

- **Metric:** `access_log_entries_total{outcome="failed"}` and
  `atna_audit_rejected_total` rising.
- **Health:** the readiness checks of the audit database.
- **What happens:** every record is written before its response, so a
  request whose records cannot be written is refused with `503` and
  `Retry-After: 1`. Nothing is dropped and nothing waits in memory.
- **What to do:**
  1. Read the log line `the audit records could not be written` for the
     cause: a full disk, a lost audit database, a statement timeout.
  2. Check the audit pool: `db_pool_acquire_duration_seconds` and the audit
     database's own load. `access_log_append_duration_seconds` is the time a
     request waits for its records.

### The chainer falls behind

- **Metric:** `audit_chained_total` growing slower than
  `access_log_entries_total{outcome="ok"}`, and the rate of
  `access_log_chain_hold_duration_seconds_sum` near 1.
- **Health:** `audit_sender` reads `DEGRADED` with the age of the oldest
  intake record.
- **What happens:** the records are stored and nothing is lost; the chains,
  the checkpoints and the feeds that deliver from the store are behind.
- **What to do:**
  1. Check `access_log_sign_duration_seconds`: each round signs one
     checkpoint while the head is held, and an RSA key is far slower than
     Ed25519 ([PGP signing](signing/pgp.md)).
  2. Read the log for `an audit chainer round failed`: a round that fails
     repeats every half second, and the error names the record it refused.
  3. One chain carries about 15 000 entries per second on the reference
     host; a sustained rate above it needs partitioned chains, the recorded
     next step (#5745).

### A chain verification that fails

- **Metric:** `audit_chain_intact == 0`, with
  `audit_chain_verification_duration_seconds_count{result="damaged"}` or
  `{result="unverifiable"}` rising.
- **Health:** `audit_chain` reads `DEGRADED` and names the finding count and
  the first damaged position.
- **What happens:** the trail keeps recording. A run that found damage is
  followed by a full pass, so the indicator stays degraded until a full pass
  finds both chains intact.
- **What to do:**
  1. `unverifiable` means the verification could not run: read the reason in
     the indicator's detail and the log, which is usually the audit
     database.
  2. For `damaged`, run `SELECT * FROM audit.verify_audit_chain();` and
     `SELECT * FROM audit.verify_access_log_chain();` to list each finding.
     Then run `ferroehr access-log verify` to check the newest checkpoint,
     and `ferroehr access-log verify --checkpoint <file>` with the newest
     checkpoint published outside the database.
  3. Compare the findings with a checkpoint kept outside the database
     ([Checkpoints and the transfer of a log](audit.md#checkpoints-and-the-transfer-of-a-log)).
     A record changed or deleted outside retention is a security incident:
     follow [complaints, incidents and
     vulnerabilities](compliance/post-market.md).
  4. Do not repair the chain by hand. The damaged records are the evidence.

### A signing key mismatch between instances

- **Metric:** `instance_mismatches{differs="signing_key"}` above `0` from the
  moment the second key starts. A checkpoint signed with the other key fails
  its proof under `ferroehr access-log verify` while `audit_chain_intact`
  reads `1`: the digests verify and the signature does not.
- **Health:** `instances` reads `DEGRADED` on every instance and names each
  one that signs with another key, from start
  ([Running several instances](installation/several-instances.md#every-instance-shares-the-same-signing-key-and-configuration)).
- **What happens:** every instance appends to one chain and seals each of
  its appends with its own `[record.signing]` key. An instance that verifies
  with another key reports each checkpoint the other signed.
- **What to do:**
  1. Give every instance the same `[record.signing]` key and the same
     configuration file
     ([Running several instances](installation/several-instances.md)).
  2. Keep the checkpoints already signed verifiable: list the public
     certificate of the key that signed them in `retired_key_files`.
  3. Run `ferroehr access-log verify` again; it exits 0 when the checkpoint
     verifies. `instances` reads `UP` again once the instance with the other
     key has stopped, and within 90 seconds of its last heartbeat when it
     stopped without removing its row.

### Rotating the signing key

- **Metric:** `audit_seal_duration_seconds_count{outcome="failed"}` and
  `access_log_sign_duration_seconds_count{outcome="failed"}` after the
  change show a key the server cannot sign with; an append whose checkpoint
  cannot be signed is not stored, and `access_log_entries_total` stops.
- **What to do:**
  1. Add a new signing subkey to the certificate, or move the old
     certificate's public half to `retired_key_files` and configure the new
     key ([Retired keys and rotation](signing/pgp.md#retired-keys-and-rotation)).
     A checkpoint records no key identifier, so the old key has to stay
     verifiable.
  2. Roll the change to every instance together. Instances that sign with
     different keys in the meantime write checkpoints the others cannot
     verify until the old public certificate is retired everywhere.
  3. Run `ferroehr access-log verify` after the next append. Watch
     `audit_seal_last_success_seconds` move for the system chain at the next
     run.

### A system chain seal that keeps failing

- **Metric:** `time() - audit_seal_last_success_seconds{chain="system"}`
  above fifteen minutes, with
  `audit_seal_duration_seconds_count{outcome="failed"}` rising.
- **What to do:** read the log for `the system chain could not be sealed`. A
  sign error means the keyring lost its OpenPGP key or its passphrase; a
  store error means the audit database. The next run tries again five
  minutes later, and `ferroehr audit-chain seal` seals one on demand.

### A checkpoint publication that fails

- **Health:** `access_log_checkpoints` reads `DEGRADED` and names the
  failure; the log says `the access-log checkpoint could not be published`.
- **What happens:** the log keeps recording and every append is still
  sealed in the database. Entries newer than the newest published checkpoint
  could be removed with their checkpoints without a trace outside the
  database until publication resumes.
- **What to do:**
  1. A write error names the file: check that the directory in
     `[audit.access_log] publish_directory` is mounted, writable by the
     server and not full.
  2. `already holds another checkpoint at the same sequence` means a file
     there holds a checkpoint this log did not seal at that position. Keep
     both: run `ferroehr access-log verify --checkpoint <file>` against the
     file and treat a finding as a security incident
     ([complaints, incidents and vulnerabilities](compliance/post-market.md)).
  3. The next run, 30 seconds later, publishes the newest checkpoint, which
     covers every entry below it.

### A retention run that falls behind

- **Metric:** `access_log_retention_duration_seconds` approaching the hourly
  interval, `access_log_retention_duration_seconds_count{outcome="failed"}`
  rising, `access_log_tombstones_total` flat while entries pass their
  `retain_until`, or `access_log_folded_total` flat while tombstones pass
  their own end.
- **What happens:** an entry past its retention stays readable until a run
  replaces it by its tombstone. A failed run keeps the batches it replaced
  and the next run continues.
- **What to do:**
  1. Read the log for `access-log retention failed` and its error.
  2. Check the audit database's load and locks: retention replaces entries
     in batches of 500 and folds tombstones in batches of 5000, each in its
     own transaction.
  3. Size the audit store for the access rate
     ([Storage sizing](operations.md#storage-sizing)).

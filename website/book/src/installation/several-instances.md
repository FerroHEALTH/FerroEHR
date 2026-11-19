# Running several instances

You can run two or more FerroEHR instances behind one load balancer, all
connected to one PostgreSQL server, with no sticky sessions. PostgreSQL is the
only source of truth. What an instance holds in memory is a copy that a
notification keeps fresh: each EHR's `EHR_ACCESS` settings and the settings
classes, the policy grants, the operational-policy registers and the access
policy. When an instance loses a copy, only latency changes. No openEHR
specification governs the process model; this page describes FerroEHR's own
design.

<!-- toc -->

## Try it with Compose

`docker-compose.replicas.yml` is an overlay for the quickstart that adds a
second instance, `ferroehr-2`, on the same database, configuration and key:

```bash
docker compose -f docker-compose.yml -f docker-compose.replicas.yml \
  --profile replicas up -d --wait
```

The first instance answers on `localhost:8080` and the second on
`localhost:8081`. The deployment probe runs this stack on every change: it
writes through one instance and reads through the other, sends writes to both
at once, verifies the access log and the system chain both instances appended
to, and starts the second instance with another key to see it reported.

## Every instance connects to the primary

Point every instance at the PostgreSQL primary. Each copy is kept fresh by a
`LISTEN` on a notification channel, and every commit that changes the policy
it copies notifies that channel when the commit lands. A hot standby accepts
no `LISTEN`, so an instance on a standby would never hear of a change. Every
write also needs the primary.

## A change made on another instance

A notification reaches the other instances after the commit that sent it,
usually within milliseconds. A request on another instance in that window is
judged on the settings that instance held, and it never runs under them:

- every read that leaves the store selects the revision of the EHR's
  `EHR_ACCESS` settings row and the EHR's subject beside its own columns, in
  the same statement, and a verdict judged on another revision or another
  subject admits nothing;
- every commit reads them on its own transaction and judges the EHR again
  from the rows that transaction reads;
- a query decides every EHR in its one SQL statement from the settings row it
  reads there.

So a restriction, an opt-out, a withdrawn consent or a change of subject
committed on one instance decides the next read or write that reaches its data
on every instance. The subject matters because the decision reads its care
relationships. A read
caught in the window answers as an EHR the caller may not see (`404`), even
when the change made the record more open, and the instance drops its copy so
the next request reads the settings again. The window lasts until the
notification arrives; while an instance's listener is down it reads the
settings on every request, so it has no window. No request adds a statement or
a transaction for this.

The other copies are confirmed in the request too, so no decision waits on a
notification. A request reads two revisions before it decides, each in one
indexed statement on its own connection:

- in the party store, the revisions of the grants, of the caller facts and of
  the binding side table (`binding_grant_revision`). The binding revision
  moves in the transaction of every change of a login's key, a membership, a
  representation or a key space's state. A held lookup is taken only when it
  was read at the revision the request read, and the grants and the caller
  facts are read again when the store's revisions are past the copy's;
- in the clinical store, the newest version of each policy register and of
  the access policy. A copy behind one of them is read before the request is
  judged.

So a deprovisioned login, a removed membership, a revoked grant, an ended care
relationship and a policy change committed on one instance decide the next
request on every instance, whatever the delay of the notification. The
notification still drops the copies early; the two reads are what a decision
rests on. A read request sends its own read, its access record and these two
statements.

## The listeners share one connection per database

Each instance keeps six policy copies fresh, each on a notification channel
of its own: four in the clinical database (the `EHR_ACCESS` settings, the
operational-policy registers, the access policy and the registered template
addresses, so a COMPOSITION commit learns whether its template is registered
without a statement) and two in the party database (the policy grants and the
party bindings). With `[pix]` set, the ITI-93 applier listens on a seventh,
in the party database. Every channel of one database is held on one
`LISTEN` connection, apart from the request pools: one connection in all when
the domains share a DSN, two when the party domain has one of its own. A
notification carries at most an id, and only a policy or definition change,
a new or moved EHR, a changed party binding and a stored ITI-93 message raise
one, so one connection carries them all. The session names itself
`ferroehr policy listener` in `pg_stat_activity`.

PgBouncer in transaction mode hands a connection back after each transaction
and drops a `LISTEN` with it. The request pools may run through such a pooler;
the listeners may not. Point them at the primary directly, or at a
session-mode pooler, with `[database] listen_url` (or `listen_url_file`).
Unset, they connect on `[database] url`. A domain in a database of its own
takes `[database.clinical] listen_url` or `[database.party] listen_url`; the
audit domain holds no listener, so its key is refused.

```toml
[database]
url = "postgres://ferroehr:…@pgbouncer:6432/ferroehr"         # transaction mode
listen_url = "postgres://ferroehr:…@postgres-primary:5432/ferroehr"
```

While a listener connection is down, every copy it carries reads the database
on every request. The
instance keeps serving and every decision stays correct. The health document
shows the lost listener under `policy_listeners`, which reads `DEGRADED` and
names each listener until it connects again. A degraded indicator does not
withdraw readiness, so the instance stays in rotation. The metric
`policy_listener_up` reads `0` for the lost listener; see [a lost policy
listener](../operations-access-path.md#a-lost-policy-listener).

The cost of a change on the database grows with the instances: each one
reads the change once, on its own notification. The reload load measured for
one, two and three instances is on the
[Performance](../performance.md#the-access-path-at-scale) page.

## Connection budget

Each instance opens at most `[database] connection_budget` connections, 40 by
default: one listener connection per database its listeners reach, four for
the audit runtime, and three pools, one per domain (clinical, party and
audit), that share the rest evenly. Unset, a pool holds four connections per core the instance may run
on, at least 10, within that share, and the server opens one connection per
core in each pool before it answers ready ([Sizing the
pool](config-server.md#sizing-the-pool)). At the default budget, an instance
on three cores or more holds up to 3 × 11 + 4 + 1 = 38 connections under load:

| Instances | Connections at the default budget | Fits PostgreSQL's default `max_connections = 100` |
|---|---|---|
| 1 | 38 | yes, leaving 59 for your tools |
| 2 | 76 | yes, leaving 21 for your tools |
| 3 | 114 | no |

Keep `instances × connection_budget` below PostgreSQL's `max_connections`
minus `superuser_reserved_connections` (3 by default) and the connections
your operators and tools need. A rolling update runs the new instance beside
the old ones, so count it too: the Helm chart sets a budget of 30, and its
two replicas plus the one a rollout adds open at most 90. A smaller budget
keeps a smaller burst: when `db_pool_acquire_duration_seconds` rises at your
load, raise PostgreSQL's `max_connections` and the budget with it. A
`max_connections` set per pool must fit the budget with the listener
connections, or the server refuses to start and names the budget. Through a
transaction-mode pooler, the pools count against the pooler's server
connections and only the listener connections against PostgreSQL's.

## Every instance shares the same signing key and configuration

The access log and the system chain are one hash chain each. Every instance
appends to them under a row lock on the chain head, and each instance seals
every append it makes to the access log in a signed checkpoint, in the
append's own transaction, when an OpenPGP key is configured ([version
signing](../signing/index.md)). Give every instance the same
`[record.signing]` key and the same configuration file, the same
`[audit.access_log] publish_directory` included. An instance with another
key writes checkpoints the others cannot vouch for.

Each instance compares itself with the others when it starts and every 30
seconds after. It writes the fingerprint of its signing key's certificate
(`digest` without one) and a SHA-256 of its configuration into the
`clinical.instance` table, and compares them with every instance that wrote
its row in the last 90 seconds. The configuration digest leaves out what is
each instance's own: `[server]`, `[telemetry]`, `[management]`,
`[usage_report]` and `[database]`; secrets are compared in their redacted form.
A difference is logged at start and shown in the health document under
`instances`, which reads `DEGRADED` and names each instance that runs another
key or another configuration, and the `instance_mismatches` metric counts
them; see [a signing key mismatch between
instances](../operations-access-path.md#a-signing-key-mismatch-between-instances).
During a rolling upgrade the old and the new
release can report a difference until the old instances stop; an instance
that stops removes its row.

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
  `EHR_ACCESS` settings row beside its own columns, in the same statement, and
  a verdict judged on another revision admits nothing;
- every commit reads that revision on its own transaction and judges the EHR
  again from the rows that transaction reads;
- a query decides every EHR in its one SQL statement from the settings row it
  reads there.

So a restriction, an opt-out or a withdrawn consent committed on one instance
decides the next read or write that reaches its data on every instance. A read
caught in the window answers as an EHR the caller may not see (`404`), even
when the change made the record more open, and the instance drops its copy so
the next request reads the settings again. The window lasts until the
notification arrives; while an instance's listener is down it reads the
settings on every request, so it has no window. No request adds a statement or
a transaction for this.

The copy also holds the EHR's subject, which decides care relationships. A
change of subject reaches another instance with its notification.

## The listeners hold connections of their own

Each instance runs four listeners, one per copy, and each holds one
connection for its `LISTEN`, apart from the request pools: three on the
clinical database and one on the party database. Each listener session
names itself `ferroehr policy listener` in `pg_stat_activity`.

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

While a listener is down, its copy reads the database on every request. The
instance keeps serving and every decision stays correct. The health document
shows the lost listener under `policy_listeners`, which reads `DEGRADED` and
names each listener until it connects again. A degraded indicator does not
withdraw readiness, so the instance stays in rotation. The metric
`policy_listener_up` reads `0` for the lost listener; see [a lost policy
listener](../operations-access-path.md#a-lost-policy-listener).

## Connection budget

Each instance opens three pools, one per domain (clinical, party and audit).
Each pool holds at most `[database] max_connections` connections, and the four
listeners hold one each beside them. With the default of 20, one instance uses
at most 3 × 20 + 4 = 64 connections:

| Instances | Connections at most | Fits PostgreSQL's default `max_connections = 100` |
|---|---|---|
| 1 | 64 | yes |
| 2 | 128 | no |
| 3 | 192 | no |

Keep `instances × (3 × max_connections + 4)` below PostgreSQL's
`max_connections` minus the connections your operators and tools need. For two
instances on a default PostgreSQL, set `[database] max_connections = 14`
((3 × 14 + 4) × 2 = 92 connections), or raise PostgreSQL's `max_connections`.
The Compose overlay above sets 10 on each instance, 68 in all. Through a
transaction-mode pooler, the pools count against the pooler's server
connections and only the listeners against PostgreSQL's.

## Every instance shares the same signing key and configuration

The access log and the system chain are one hash chain each. Every instance
appends to them under a row lock on the chain head, and signs each link when
an OpenPGP key is configured ([version signing](../signing/index.md)). Give
every instance the same `[record.signing]` key and the same configuration
file. An instance with another key writes links the others cannot vouch for.

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

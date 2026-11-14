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

## Every instance connects to the primary

Point every instance at the PostgreSQL primary. Each copy is kept fresh by a
`LISTEN` on a notification channel, and every commit that changes the policy
it copies notifies that channel when the commit lands. A hot standby accepts
no `LISTEN`, so an instance on a standby would never hear of a change. Every
write also needs the primary.

## The listeners need a direct or session-mode connection

Each of the four copies holds one connection of its pool for its `LISTEN`.
PgBouncer in transaction mode hands a connection back after each transaction
and drops the `LISTEN` with it. The listeners take their connections from the
same pools that serve requests, so connect FerroEHR to PostgreSQL directly or
through a session-mode pooler.

While a listener is down, its copy reads the database on every request. The
instance keeps serving and every decision stays correct. The health document
shows the lost listener under `policy_listeners`, which reads `DEGRADED` and
names each listener until it connects again. A degraded indicator does not
withdraw readiness, so the instance stays in rotation. The metric
`policy_listener_up` reads `0` for the lost listener; see [a lost policy
listener](../operations-access-path.md#a-lost-policy-listener).

## Connection budget

Each instance opens three pools, one per domain (clinical, party and audit).
Each pool holds at most `[database] max_connections` connections, and the
listeners take theirs from these pools. With the default of 20, one instance
uses at most 60 connections:

| Instances | Connections at most | Fits PostgreSQL's default `max_connections = 100` |
|---|---|---|
| 1 | 60 | yes |
| 2 | 120 | no |
| 3 | 180 | no |

Keep `instances × 3 × max_connections` below PostgreSQL's `max_connections`
minus the connections your operators and tools need. For two instances on a
default PostgreSQL, set `[database] max_connections = 15` (90 connections), or
raise PostgreSQL's `max_connections`.

## Every instance shares the same signing key and configuration

The access log is one hash chain per store. Every instance appends to it under
a row lock on the chain head, and signs each link when an OpenPGP key is
configured ([version signing](../signing/index.md)). Give every instance the
same `[record.signing]` key and the same configuration file. An instance with
another key writes links the others cannot vouch for, and the chain then fails
to verify; see [a signing key mismatch between
instances](../operations-access-path.md#a-signing-key-mismatch-between-instances).

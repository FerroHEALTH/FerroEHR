# Change events (AMQP)

When something is committed to the CDR, downstream systems often need to know:
an analytics pipeline, a care-coordination service, a cache invalidator. Rather
than have them poll, FerroEHR can publish a small event for every commit to an
AMQP 0.9.1 broker (RabbitMQ). The events are built so you can fan them out
broadly without leaking clinical data: they carry identifiers and metadata, never
the record content.

<!-- toc -->

## Delivery guarantees

The publisher is built on a **transactional outbox**, which is what gives it
properties you can design a consumer against:

- **At-least-once delivery.** Every commit writes its event row to an outbox
  table in the *same database transaction* as the change itself, so no commit
  without its event, no event without its commit. A background task drains the
  outbox and marks a row published only after the broker confirms. A crash or a
  retry may deliver a message more than once, so consumers deduplicate.
- **Ordered draining.** Rows are read in global sequence order, and the drainer
  stops the batch at the first publish failure rather than skipping ahead, so an
  earlier event for an EHR is not overtaken by a later one from the same drainer.
  Messages are published persistently to a durable exchange.
- **PHI-free envelopes.** The message body carries ids, version numbers, and
  metadata. To read the actual clinical content a consumer calls back through the
  authenticated REST API.
- **Commits never wait on the broker.** If the broker is down, events accumulate
  in the outbox and drain when it returns. Published rows are pruned after a
  retention window, and never past the cursor of an active reader registered in
  `event_outbox_reader`: the prune's floor is the lowest active reader cursor,
  read in the same statement.

```mermaid
flowchart LR
    commit["commit<br/>(composition / status / folder / party)"]
    tx[("same DB transaction")]
    node["clinical data"]
    outbox["event_outbox row<br/>(published_at = NULL)"]
    drain["outbox drainer<br/>(background task)"]
    broker["AMQP topic exchange<br/>ferroehr.events"]
    consumer["your consumer<br/>(bound queue)"]

    commit --> tx
    tx --> node
    tx --> outbox
    drain -->|"poll pending, publish, await confirm"| broker
    outbox -.->|"drained in seq order"| drain
    broker --> consumer
    consumer -.->|"fetch bodies via authenticated API"| commit
```

> [!NOTE]
> Several server replicas can drain the same outbox safely (each row is claimed
> exclusively and the others skip it) but that also means two replicas may have
> different rows in flight at once. Order your consumer on the `seq` field rather
> than on arrival order, and the guarantee holds however many replicas you run.

## The event envelope

Each published message is JSON. One contribution can touch several versioned
objects, and the publisher emits **one message per version**, each under its own
routing key. Every message carries the shared envelope:

| Field | Meaning |
|---|---|
| `contribution_id` | the contribution this change belongs to |
| `ehr_id` | the EHR; `null` for a demographic contribution, which has no EHR scope |
| `committed_at` | the commit instant |
| `versions[]` | one entry per changed versioned object |
| `seq` | the delivery sequence number (monotonic) |
| `version_index` | which entry in `versions` this message is for |

Each `versions[]` entry carries:

| Field | Meaning |
|---|---|
| `vo_id` | the versioned object's identifier |
| `kind` | the full RM type name: `COMPOSITION`, `EHR_STATUS`, `EHR_ACCESS`, `FOLDER`, or one of the demographic kinds (`PERSON`, `ORGANISATION`, `GROUP`, `AGENT`, `ROLE`, `PARTY_RELATIONSHIP`) |
| `sys_version` | the version ordinal |
| `version_tree_id` | the version-tree id, so a branch version is distinguishable from a trunk one |
| `change_type` | the numeric openEHR audit change-type code: `249` creation, `250` amendment, `251` modification, `523` deleted, `666` attestation, and the other members of that code group |
| `template_id` | the composition's operational template, or `null` |

The code, not its English rubric, is what travels: rubrics are display text and
change, the code is what the audit stores.

> [!TIP]
> Deduplicate on the pair `(contribution_id, version_index)` and process in `seq`
> order. That handles at-least-once redelivery and keeps ordering at the consumer
> regardless of how the server side is scaled.

## Routing keys and subscriptions

Messages are published to a **topic exchange** (default name `ferroehr.events`),
with a three-field routing key:

```text
<kind>.<change_type>.<template_id>
```

For example, `COMPOSITION.249.openEHR-EHR-COMPOSITION_encounter_v1`. AMQP topic
keys use `.` as the word separator, so a template id containing dots is
sanitised (every character outside `[A-Za-z0-9_-]` collapses to `_`) and the key
always has exactly three fields. When there is no template, the last field is `-`.

Bind a queue with the usual AMQP topic wildcards to select what you care about:
`COMPOSITION.*.*` for all composition changes, `*.523.*` for all deletions, `#`
for everything.

The server can also manage subscriptions for you. With the event-subscription
admin API enabled (`FERROEHR__EVENTS__ADMIN_API`), the CRUD routes under
`/admin/event_subscription` let you store subscription rows, and each **enabled**
row makes the server declare and bind a durable queue named `<exchange>.<name>`
(for the default exchange, `ferroehr.events.<name>`). Its binding key is built
from the row's `kind` / `change_type` / `template_id` predicates, with a wildcard
for any predicate left unset. Topology is (re)declared when the broker
connection is established or the enabled set changes, not on every poll, and
re-declaring is idempotent, so a broker replaced underneath the server gets its
queues back.

## Enabling it

Publishing is off by default. The keys live under `[events]`; the full table with
every default is on
[Integrations](../installation/config-integrations.md#events). The essentials:

| Environment variable | Default | Meaning |
|---|---|---|
| `FERROEHR__EVENTS__ENABLED` | `false` | master switch |
| `FERROEHR__EVENTS__URL` | a local development broker | broker connection URL (credentials are redacted from every rendering) |
| `FERROEHR__EVENTS__URL_FILE` | unset | read the broker URL from a mounted file instead |
| `FERROEHR__EVENTS__EXCHANGE` | `ferroehr.events` | topic exchange name, and the queue-name prefix |
| `FERROEHR__EVENTS__TLS` | `false` | upgrade an `amqp://` URL to `amqps://` |
| `FERROEHR__EVENTS__ADMIN_API` | `false` | mount the `/admin/event_subscription` routes |

Batch size, poll interval, publish retries, and the retention window for
published rows are tunable too, and their defaults are sensible for a normal
deployment. The retention window is a floor, not a schedule: a published row
that an active cursor reader has not reached survives the window until that
reader passes it, and a reader the configuration switches off is marked
inactive at boot so it holds nothing.

> [!WARNING]
> The broker URL carries credentials, so keep it in a secret (`url_file` reads it
> from a mounted file) not in a plain environment file. For anything beyond a
> local broker use TLS (`FERROEHR__EVENTS__TLS=true`, or an `amqps://` URL). Under
> `deployment_profile = "production"` the server refuses to start with the outbox
> on a plain `amqp://` broker unless `plaintext_broker` is accepted by name.

> [!NOTE]
> Eventing is also a **cargo feature** (`events`), on in the published images and
> any default build. A slim `--no-default-features` build contains none of the
> transport's code and **refuses to boot** with `events.enabled = true` rather
> than starting quietly without a publisher; see
> [From source → Build features](../installation/from-source.md#build-features).

## What a broker outage looks like

A broker the server cannot reach is a **degraded**, not a failed, deployment. The
`events` health indicator reports degraded with "event broker unavailable; outbox
buffering", and because it is not a required indicator, readiness still passes:
the CDR keeps accepting clinical writes and the outbox keeps growing. Watch the
indicator on the health surface rather than the broker alone: see
[Operations](../operations.md).

### On Kubernetes

The chart renders its `config` tree verbatim into the server's configuration
file, so every key above is reachable as `config.events.*`
([Any server setting is reachable](../installation/kubernetes.md#any-server-setting-is-reachable)).
The broker URL carries credentials, so it goes through `secrets.eventsUrl`,
which the chart mounts as a file and passes by path:

```yaml
# values.yaml
config:
  events:
    enabled: true
    exchange: ferroehr.events
    tls: true
secrets:
  eventsUrl: "amqps://user:pass@broker.example:5671/%2f"
```

**Before you enable it:** a reachable broker, a broker certificate the pod trusts
if you set `tls: true`, and (if the chart's default-deny egress policy is on) an
egress rule that admits the broker. **To turn it off**, set
`config.events.enabled: false` and upgrade; the outbox stops draining and nothing
else changes.

## Consuming events

A consumer binds a queue to the exchange and reads. In shell form with the
RabbitMQ tooling:

```bash
# bind a queue to every composition creation, then consume
rabbitmqadmin declare queue name=my-consumer durable=true
rabbitmqadmin declare binding source=ferroehr.events destination=my-consumer \
  routing_key='COMPOSITION.249.*'
```

The server declares the exchange itself (durable, topic) the first time it
publishes. If you bind before the server has ever published, declare the exchange
yourself with the same name and type, or the binding has nothing to attach to.

Each delivery is a JSON envelope as described above. Your consumer records the
`(contribution_id, version_index)` pairs it has seen, and for anything whose
content it needs, it calls the CDR's REST API (for example
`GET /ehr/{ehr_id}/composition/{vo_id}`) with its own credentials. The event
tells it *what* changed; the authenticated API is where it reads the *data*.

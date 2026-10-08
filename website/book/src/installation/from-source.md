# From source

You can build the `ferroehr` binary yourself: for a platform without a
published image, for local development, or to run the test suite. This chapter
covers the prerequisites and the build. Most operators should prefer the
published container images ([Docker Compose](compose.md),
[Kubernetes & Helm](kubernetes.md)); build from source when you need to.

## Prerequisites

- **The pinned Rust toolchain.** The repository pins Rust **1.98.1** (edition
  2024) via `rust-toolchain.toml`, so `rustup` installs and selects it
  automatically the first time you build in the checkout; you do not choose a
  version by hand.
- **Docker:** required only for the integration tests, which spin up a real
  PostgreSQL 18 in a container.
- **`xmllint`:** required only for the canonical-XML tests.

## Building

From the repository root:

```shell
cargo build --workspace
```

To build just the server binary in release mode (what the container image
ships):

```shell
cargo build --release --locked -p ferroehr-server
```

The binary crate is `ferroehr-server`; the executable it produces is named
`ferroehr`, so the resulting binary is `target/release/ferroehr`. It links a
pure-Rust TLS stack (no OpenSSL, no JVM, no runtime dependencies) so it
drops into a minimal base image or runs directly on the host.

## Running the tests

```shell
cargo nextest run --workspace
```

Every database-backed test takes its database from a shared harness: one
PostgreSQL 18 server, one migrated template database, and a fast clone per
test. By default the harness starts (or re-adopts) a single reusable `postgres:18` container, so
Docker must be running; reclaim it afterwards with
`docker rm -f ferroehr-testkit-pg18`. To use a PostgreSQL 18 server you already
run instead, point the suite at it and Docker is not needed at all:

```shell
export FERROEHR_TEST_PG_URL='postgres://user:password@localhost:5432/postgres'
cargo nextest run --workspace
```

The role in that DSN must be allowed to `CREATE DATABASE`.

## Running the binary

The binary is configured entirely through `FERROEHR_*` environment variables
(see the [configuration reference](configuration.md)). At minimum it needs a
database URL:

```shell
export FERROEHR__DB__URL='postgres://ferroehr:ferroehr@localhost:5432/ferroehr'
target/release/ferroehr
```

It runs its schema migrations at boot and then serves on the configured bind
address (default `0.0.0.0:8080`). That boot-time migration is a setting, not a
fixture: `db.migrate = "verify"` makes the server issue no DDL at all, for a
deployment whose database role has none of those rights (see
[Operations](../operations.md#applying-migrations)).

Two global flags and three subcommand groups round the CLI out:

- `--config <path>` points at a configuration file, overriding the search
  order (`FERROEHR_CONFIG`, `./ferroehr.toml`, `/etc/ferroehr/ferroehr.toml`);
- `--set <key>=<value>` is a repeatable dotted-path override with the highest
  precedence of all (`--set db.max_connections=40`);
- `ferroehr config check` validates the effective configuration and prints it
  redacted, exiting 0 when valid and 1 otherwise, the fastest way to test a
  deployment's configuration before starting it;
- `ferroehr config default` writes the annotated default configuration
  template to stdout, which is the reference every key in the
  [configuration reference](configuration.md) is drawn from;
- `ferroehr db migrate` applies the embedded migrations and exits, the
  out-of-band schema step, run under a `ferroehr_migrator` DSN;
- `ferroehr db verify` checks, without issuing any DDL, that the database
  carries exactly this build's migrations, exiting 0 when it does;
- `ferroehr healthcheck` (used by the container healthcheck and Kubernetes
  exec probes) probes the status endpoint and exits 0 or 1. It defaults to
  `http://127.0.0.1:8080/ferroehr/rest/status`; override with `--url` or
  `FERROEHR_HEALTHCHECK_URL`.

> [!NOTE]
> Building from source gives you the same binary the images use: the container
> Dockerfile pins its Rust version from the same `rust-toolchain.toml`, and CI
> cross-checks the two so they cannot drift.

## Build features

The server builds with three additive cargo features, all **on by default**:
`fhir` (the FHIR terminology providers, and the FHIR `AuditEvent` the audit
store, the audit feed and the ITI-81 retrieval carry), `events`
(contribution-outbox eventing and the AMQP transport), and `multimedia`
(`DV_MULTIMEDIA` externalization to S3-compatible object storage). Their
implementations live in a separate crate that the platform library pulls in
only when the matching feature is on, so a build without them contains none of
their code.

A slim build compiles them out entirely:

```bash
cargo build --release --locked -p ferroehr-server --no-default-features
```

A slim binary refuses loudly at boot if the configuration enables an
integration it was built without: `multimedia.enabled`, `events.enabled`,
`audit.store.enabled`, `audit.fhir_feed.enabled`, or a configured external
FHIR terminology provider. The syslog ATNA feed and the in-process terminology
bundle remain available in slim builds.

> [!WARNING]
> The local audit store is **on in the shipped defaults**, so a slim build
> refuses to start on an untouched configuration. To run one, disable it
> explicitly (`FERROEHR__AUDIT__STORE__ENABLED=false`) or configure the
> syslog feed as the audit sink; silently dropping the audit trail is not
> a boot mode.

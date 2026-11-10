# Installation

FerroEHR is one self-contained binary that connects to a PostgreSQL 18
database. There is no application server to install and no language runtime to
provision: the binary links a pure-Rust TLS stack and needs no OpenSSL and no
JVM, so all you choose is how to run it and where the database lives. This part
covers the three paths and the full configuration surface.

- **[Try it in Codespaces](codespaces.md):** one click boots the published
  images in a GitHub Codespace in your browser, with the Swagger UI and the
  viewer forwarded. Nothing to install. Made for evaluating FerroEHR; a real
  deployment uses one of the paths below.
- **[Docker Compose](compose.md):** the fastest way to run the server plus a
  preconfigured PostgreSQL 18, for development and evaluation. One downloadable
  file that pulls the published images and needs no configuration. Optional
  viewer, OIDC, and observability overlays.
- **[Kubernetes & Helm](kubernetes.md):** the production path, a hardened
  non-root workload that connects to an externally managed PostgreSQL 18. Its
  companion, [Cluster hardening](kubernetes-hardening.md), covers what the
  chart cannot do for you.
- **[From source](from-source.md):** building the binary yourself with the
  pinned Rust toolchain, and running the test suite.
- **[Configuration reference](configuration.md):** how configuration loads and
  what every key means, split by area: [server, database and
  telemetry](config-server.md), [authentication and access](config-auth.md),
  [integrations](config-integrations.md), [audit and subject
  proxy](config-audit.md), and the [CLI and production
  checklist](config-cli.md).

## What every path has in common

**One configuration file, with environment overrides on top.** The server reads
a single `ferroehr.toml` covering every subsystem; `FERROEHR__*` environment
variables override individual keys, and repeatable `--set key=value` flags
override those. An array of tables, such as `[[auth.issuer]]`, is set from the
environment by element index (`FERROEHR__AUTH__ISSUER__0__ISSUER`). `ferroehr config
default` writes an annotated template with every key at its default, and
`ferroehr config check` validates the result without touching the database.

**The schema is the binary's, and who applies it is a choice.** By default
(`database.migrations = "apply"`) the server applies its embedded migrations at boot, so
an empty database self-provisions. Setting `database.migrations = "verify"` makes the
server issue no DDL at all (it checks the schema and refuses to start if it is
not this build's) which lets the runtime role hold no DDL rights. Something
else then runs `ferroehr db migrate` under a migrator role first. That check
still reads every schema's migration state, so a least-privilege deployment
also sets `database.migrate_url` to the credential that can. Both postures are laid
out in [Operations → Applying migrations](../operations.md#applying-migrations).

**A deployment declares what it may hold.** The top-level
`deployment.profile` key is `sandbox` by default: the server starts whatever
separations are missing and names every one of them on the banner, in the log
and on `GET /management/status`. Set it to `production` and the server
refuses to start while a separation is open and not accepted by name in
`deployment.accepts`. Read
[`deployment.profile`](configuration.md#deploymentprofile) before you point a
deployment at real patient data.

## Choosing a specification generation

One top-level key, **`deployment.spec_profile`**, selects which openEHR specification
generation set the deployment runs: `development` (the default: Reference
Model 1.2.0 with BASE 1.3.0) or `stable` (the latest released generations:
Reference Model 1.1.0 with BASE 1.2.0). It is a
single coupled choice, and `stable → development` is always safe while the
reverse is not, so decide it before you commit clinical data. The full
semantics, defaults, and the direction contract are in
[`deployment.spec_profile`](configuration.md#deploymentspec_profile).

> [!NOTE]
> A Clinical Data Repository stores PHI. In production the database must be an
> externally managed, backed-up, point-in-time-recoverable PostgreSQL 18, never
> a throwaway sidecar. The Kubernetes chart deliberately ships **no** in-cluster
> database for this reason.

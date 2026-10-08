# Returning to the original state

This page resets a FerroEHR instance to the state its release shipped in: the
configuration the release ships, and an empty database carrying the schema
that release's migrations create. Nothing else survives a reset. EHRs,
compositions, templates, parties, the party-to-EHR map, the audit trail and
the instance id the [usage report](usage-report.md) sends are all gone, and
the next boot starts as a fresh install.

<!-- toc -->

## Before you reset

> [!WARNING]
> A reset deletes the access log along with the records it describes. If the
> instance has ever held real personal data, the audit trail is subject to the
> retention floor of every jurisdiction you operate in (three years in every
> EU Member State, five in the Netherlands, one in Switzerland; see
> [Audit](installation/config-audit.md)), and your duty to keep it does not end
> when the instance does. Take a backup of every domain first, the audit trail
> included ([Backup and point-in-time recovery](operations.md#backup-and-point-in-time-recovery)),
> and keep it for that horizon.

A reset does not reach anything outside the instance. Your logical dumps, WAL
archives, volume snapshots, log collector and any external Audit Record
Repository still hold what they held. Deleting those is a separate decision
with the same retention duties. To take the instance out of service for good
instead of starting it again, follow [Decommissioning](operations-decommissioning.md).

The reset has two halves, and you need both:

- **Configuration:** the server reads its built-in defaults, then a
  `ferroehr.toml`, then `FERROEHR__…` environment variables, then `--set`
  ([Configuration reference](installation/configuration.md)). The original
  state is the release's own file and no overrides.
- **Database:** drop the database and let the release's migrations build it
  again. Drop the whole database, never single schemas
  ([Recovering a partially wiped database](operations.md#recovering-a-partially-wiped-database)
  explains why).

The `deployment_profile` the release ships is `sandbox`, so a reset instance
says on its boot banner and on `GET /ferroehr/rest/status` that it must not
hold real patient data. Set `deployment_profile = "production"` again before
it does.

## Docker Compose

The quickstart keeps the database in the named volume `ferroehr-pgdata` (and,
with the `s3` profile, multimedia in `ferroehr-seaweedfs`), and its server
configuration inline in `docker-compose.yml`.

1. Stop the stack and delete its volumes:

   ```console
   docker compose down --volumes
   ```

   `--volumes` removes the named volumes the file declares
   ([docker compose down](https://docs.docker.com/reference/cli/docker/compose/down/)).
   Run it with the same `-f` overlay files and `--profile` flags you started
   the stack with, so it finds every service.

2. Restore the configuration. Replace `docker-compose.yml` with the copy
   attached to your release on
   [GitHub Releases](https://github.com/FerroHEALTH/FerroEHR/releases), and
   remove every `FERROEHR__…`, `DATABASE_URL` and `RUST_LOG` variable you set,
   from your shell and from any `.env` file next to the compose file:

   ```console
   env | grep -E '^(FERROEHR__|DATABASE_URL=|RUST_LOG=)'   # expect no output
   ```

3. Start it again:

   ```console
   docker compose up -d --wait
   ```

   The database image provisions the domain roles on the empty volume, and the
   server applies its migrations at boot.

4. Read the result back. The effective configuration should be the file's and
   nothing else:

   ```console
   docker compose exec ferroehr /usr/local/bin/ferroehr config check
   ```

## Kubernetes and Helm

The chart deploys no database, so the two halves are separate operations.

1. Remove the release, so no pod holds a connection:

   ```console
   helm uninstall ferroehr --namespace ferroehr
   ```

   This keeps the Secrets and PersistentVolumeClaims you created yourself,
   such as the DSN Secret and the backup claims.

2. Drop and recreate the database, as a role that may (the bootstrap superuser
   or the database owner), connected to a different database:

   ```sql
   DROP DATABASE ferroehr WITH (FORCE);
   CREATE DATABASE ferroehr OWNER ferroehr;
   ```

   `WITH (FORCE)` terminates the sessions still connected
   ([DROP DATABASE](https://www.postgresql.org/docs/18/sql-dropdatabase.html)).
   Use the owner your deployment provisioned the database with. If
   `[storage.party]`, `[storage.linkage]` or `[storage.audit]` name databases
   of their own, drop and recreate each of those too. The domain roles are
   cluster-wide objects, so they survive the drop, and the migrations reuse
   them.

3. Install the chart again with only the values every deployment must supply
   (the DSN Secret and an authentication mechanism), and none of your other
   overrides:

   ```console
   helm install ferroehr oci://ghcr.io/ferrohealth/charts/ferroehr \
     --version <your chart version> \
     --namespace ferroehr \
     -f minimal-values.yaml
   ```

   The schema is applied at boot (`config.db.migrate: apply`, the chart
   default), or by the migration Job when you enabled it.

## A single binary

1. Stop the server.
2. Drop and recreate the database as in step 2 of the Kubernetes procedure.
3. Restore the configuration: replace your `ferroehr.toml` with
   `ferroehr config default > ferroehr.toml` (or delete it, which leaves the
   built-in defaults), and unset every `FERROEHR__…`, `DATABASE_URL` and
   `RUST_LOG` variable except the database DSN.
4. Prepare the schema with `ferroehr db migrate`, or start the server with the
   default `db.migrate = "apply"`.
5. Check the effective configuration with `ferroehr config check`.

## How the procedure is tested

The deployment probe harness (`scripts/deploy-probe.sh`, its `original_state`
family) runs the Docker Compose procedure above. It writes an EHR, resets the
stack as steps 1 to 3 describe, and checks that the volume was deleted, that
the EHR is gone, and that the server reports the shipped configuration. The
Kubernetes and single-binary procedures are not exercised by any probe.

# Decommissioning

This page takes a FerroEHR instance out of service for good and removes what it
stored. `ferroehr db erase` deletes the multimedia blobs and every schema the
migrations created, which takes every EHR, composition, template, party, the
party-to-EHR map, the audit trail, the stored settings and the instance id with
it. The rest of the page lists what the command cannot reach and how you remove
it yourself.

To start over with an empty instance of the same release instead, follow
[Returning to the original state](operations-reset.md).

<!-- toc -->

## Before you erase

> [!WARNING]
> The erase cannot be undone. Nothing in FerroEHR keeps a copy.

You are the controller of what the instance holds, and the law may require you
to keep some of it after the instance is gone. Settle these before you run the
command:

- **Clinical records.** Medical-records law sets retention periods measured in
  years after the last treatment or after death. If the records must survive
  the instance, move them first: export each EHR over the API, or take a full
  dump through `{base}/admin/dump`
  ([dump and load](operations-admin-apis.md#dump-and-load)), and check the
  copy in its new home before you erase. The retention marks and what is
  already due are described in
  [Retention, restriction and objection](compliance/retention.md).
- **The audit trail.** Access logs have a retention floor of their own: three
  years from each access in every EU Member State, five in the Netherlands, one
  in Switzerland ([Audit](installation/config-audit.md)). The erase deletes the
  `audit` schema. Take a dump of it first and keep that dump for the longest
  period that applies to you
  ([Backup and point-in-time recovery](operations.md#backup-and-point-in-time-recovery)).
  A dump kept for this reason is still personal data and needs the same
  protection as the instance had.
- **Data subjects and processors.** If the instance served other organisations
  or fed other systems, tell them before the data goes.

## Stop every server

Stop every FerroEHR server and job that connects to the databases: the
replicas, the migration Job, the backup CronJobs and anything else that holds a
connection. A server that keeps running holds locks, and the erase gives up
after 30 seconds of waiting for one rather than hang. A server that keeps
running could also write a new blob after the blobs are deleted.

With the Helm chart, `helm uninstall` (see [Kubernetes](#kubernetes-and-helm)
below) stops all of them. With Docker Compose, stop the `ferroehr` service and
leave the database running.

## The dry run

Run the command once without `--confirm`. It erases nothing, prints what an
erase would delete, prints the value `--confirm` must carry, and exits with
status 1:

```console
$ ferroehr db erase
`ferroehr db erase --confirm` would permanently delete:
  12 multimedia blob(s) in bucket `openehr-multimedia`
  database `ferroehr`: schemas audit, linkage, party, clinical, ext
instance id: 0b5e3c1e-6f4a-4d1b-9a43-2b7c5f1d8e90
to erase, run: ferroehr db erase --confirm 0b5e3c1e-6f4a-4d1b-9a43-2b7c5f1d8e90
Error: dry run: nothing was erased
```

The command reads the same configuration the server does (`--config`,
`FERROEHR__…` and `--set`), so run it with the configuration of the instance
you mean to remove. When the domains live in several databases, it prints one
line per database.

The value to confirm with names the instance:

- **The instance id**, when the database stores one. The server creates it at
  the first start with the [usage report](usage-report.md) switched on.
- **The name of the database the clinical domain lives in**, when the database
  stores no instance id, because the usage report never ran.

Reading the value off the dry run is deliberate: you confirm the instance the
configuration actually reaches, not the one you think it reaches.

## The erase

```console
ferroehr db erase --confirm 0b5e3c1e-6f4a-4d1b-9a43-2b7c5f1d8e90
```

The command runs in this order:

1. It lists the blob store. A store that is configured but cannot be reached
   stops the command here, before any database is touched.
2. It checks the confirmation. A value that does not match stops the command
   with nothing erased.
3. It deletes every multimedia blob FerroEHR wrote to the bucket. These are the
   objects at the bucket root named by a 64-character lowercase hex SHA-256;
   any other object in the bucket is left alone.
4. For each database, in one transaction, it drops the schemas `ext`,
   `clinical`, `party`, `linkage` and `audit` that live there, with
   `DROP SCHEMA … CASCADE`. The cascade takes every table, row, function,
   partition and stored setting in them, and the `btree_gist` extension
   installed in `ext`. A database is either erased whole or left as it was.

It connects as `ferroehr db migrate` does: on `[db] migrate_url` (or
`[db] url` when that is unset) for every domain in that database, and on a
relocated domain's own DSN otherwise. That credential must own the schemas,
which the credential that ran the migrations does.

If a step fails, fix the cause and run the same command again. Blobs are
deleted before the schemas, so a failure while deleting blobs leaves the
database whole. A second run after a successful one finds nothing, says so, and
exits 0.

A build without the `multimedia` feature refuses to run against a
configuration that names a blob store, because it cannot delete the blobs.

## What the erase does not reach

The erase removes what FerroEHR stores in its databases and its bucket. The
following are outside them, and removing them is your job.

### Backups and dumps

Your `pg_dump` files, base backups and volume snapshots still hold everything.
With the chart's backup CronJobs (`backup.enabled`), the dumps are on the
PersistentVolumeClaims you named in `backup.clinical.persistentVolumeClaim`,
`backup.party.persistentVolumeClaim`, `backup.linkage.persistentVolumeClaim`
and, when the audit domain has its own database,
`backup.audit.persistentVolumeClaim`. The chart created none of these claims,
so `helm uninstall` leaves them. Keep the ones your retention duties require
and delete the rest.

### WAL archives, replicas and standbys

A WAL archive (pgBackRest, WAL-G, `archive_command`) holds every change the
database ever made, including the rows the erase dropped. A streaming replica
or standby replays the drop, but a delayed standby, a logical replica or a
replica you detached keeps its copy. Expire the archive and remove every
replica once your retention period allows.

### The databases and the roles

The erase drops schemas, not databases. Drop each database when nothing else
lives in it:

```sql
DROP DATABASE ferroehr WITH (FORCE);
```

Run it as the owner or a superuser, connected to another database
([DROP DATABASE](https://www.postgresql.org/docs/18/sql-dropdatabase.html)).
Repeat it for every database `[storage.party]`, `[storage.linkage]` or
`[storage.audit]` names.

The roles are cluster-wide, so neither command removes them: the migrations
create `ferroehr_migrator`, `ferroehr_clinical`, `ferroehr_clinical_reader`,
`ferroehr_party`, `ferroehr_party_reader` and `ferroehr_linkage`, and you
created the login roles your DSNs use. Drop them when no other database on the
cluster uses them:

```sql
DROP ROLE ferroehr_clinical, ferroehr_clinical_reader, ferroehr_party,
          ferroehr_party_reader, ferroehr_linkage, ferroehr_migrator;
```

[DROP ROLE](https://www.postgresql.org/docs/18/sql-droprole.html) refuses a
role that still owns objects or holds privileges in any database of the
cluster.

### Kubernetes and Helm

`helm uninstall` removes what the release created, including the Secret the
chart renders from `secrets.*` values:

```console
helm uninstall ferroehr --namespace ferroehr
```

It leaves what you created yourself: the Secrets named in the
`existingSecret` values (the DSNs, the migrator DSN, the backup DSNs) and the
backup claims. List them and delete each one you no longer need:

```console
kubectl get secret,pvc --namespace ferroehr
kubectl delete secret <name> --namespace ferroehr
kubectl delete pvc <name> --namespace ferroehr
```

Whether a deleted claim's volume is wiped or kept depends on its storage
class's reclaim policy
([Persistent Volumes](https://kubernetes.io/docs/concepts/storage/persistent-volumes/#reclaiming)).
A volume with the `Retain` policy survives the claim and must be deleted on its
own. Delete the namespace last if nothing else runs in it.

### Docker Compose

`docker compose down --volumes` removes the database volume `ferroehr-pgdata`
and, with the `s3` profile, the SeaweedFS volume `ferroehr-seaweedfs`. The
dumps the `backup` profile wrote are in `./backups/` next to the compose file.

### Configuration, keys and secrets

Delete the files the instance read: `ferroehr.toml`, the files the `*_file`
and `*_path` keys point at (DSNs, TLS keys, the OIDC secret, the signing key
and its passphrase, the national-identifier key, the multimedia secret key),
the licence token, and any `.env` file. Remove the `FERROEHR__…` variables from your service definitions
and your secret store. Revoke the credentials themselves where they live: the
database passwords, the object-store access key, the OIDC client and the
broker accounts.

### Systems FerroEHR sent data to

Change events on the message broker, records sent to an external Audit Record
Repository or a syslog collector, and the logs and traces your telemetry
backend received are held by those systems. Remove them there, under the same
retention duties.

### The physical media

Dropping a schema or deleting an object frees the storage; it does not
overwrite it, and a disk, a cloud volume or a bucket can still yield the bytes
until they are reused. Sanitising the media (cryptographic erasure of an
encrypted volume, the provider's deletion guarantees, or destroying the disks)
is outside what FerroEHR can do. Encrypting the volumes from the start
([TLS and database security](operations.md#tls-and-database-security)) is what
makes cryptographic erasure possible at the end.

## Removing a single party

The openEHR Admin API in ITS-REST defines deletion routes for EHRs only, so
FerroEHR has no REST route that physically deletes a party. The service
implements the specification's `physical_party_delete` operation, but no
endpoint exposes it. What you can do without writing code:

- `DELETE` on the demographic API removes a party logically: a new version
  marks it deleted, and its history stays.
- `ferroehr db erase` removes every party along with the rest of the instance.

EHRs have physical deletion over REST:
[Physical deletion](operations-admin-apis.md#physical-deletion).

## How the procedure is tested

The integration suite of the `ferroehr` crate writes an EHR with a composition,
a party, a party-to-EHR mapping, an audit record and an instance id into a
database, checks that a wrong confirmation changes nothing, erases, and checks
that none of the five schemas, and so no row, remains and that a second run is
a no-op. A second test deletes blobs from an in-memory store, and a third shows
that an unreachable store stops the erase before the database is touched. The
command has not been run against a real S3 bucket, a multi-database layout or a
Kubernetes cluster by any probe.

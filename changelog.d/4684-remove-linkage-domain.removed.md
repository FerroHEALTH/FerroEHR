- The empty linkage storage domain is gone: its schema, its `ferroehr_linkage`
  role, the `[storage.linkage]` and `[storage.migrate.linkage]` configuration
  tables, its pool, the `database.linkage` and `backup.linkage` chart values,
  the `ferroehr-backup-linkage` compose job and the `btree_gist` extension only
  its table needed. A configuration that still sets `[storage.linkage]` or
  `[storage.migrate.linkage]` is refused at boot as an unknown key. Cohort and
  AQL demographic join crossings still write their `linkage`-domain access
  records. **Upgrade note:** remove those keys and values before you upgrade.
  On a co-located database, migration `audit/0021_drop_linkage_domain.sql`
  drops the schema, the role and the extension at the first boot, and raises
  a `NOTICE` for any of them it cannot drop. When the linkage domain lived in
  a database of its own, `[storage.audit]` names another database, or the
  cluster holds more than one FerroEHR database, run in each such database, as
  a superuser or a member of `ferroehr_linkage`:
  `DROP SCHEMA IF EXISTS linkage CASCADE; REVOKE ALL ON SCHEMA ext FROM ferroehr_linkage; REVOKE ALL ON ALL FUNCTIONS IN SCHEMA ext FROM ferroehr_linkage; ALTER DEFAULT PRIVILEGES IN SCHEMA ext REVOKE EXECUTE ON FUNCTIONS FROM ferroehr_linkage; DROP OWNED BY ferroehr_linkage; DROP EXTENSION IF EXISTS btree_gist;`
  then `DROP ROLE ferroehr_linkage;` once per cluster. The released grant
  files still name `ferroehr_linkage`, so a first install whose migrator holds
  no `CREATEROLE` still provisions that role and drops it after the first boot.

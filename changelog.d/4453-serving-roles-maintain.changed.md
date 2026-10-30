- The serving roles may now analyze their store's partitioned tables: new
  migrations (clinical 0026, party 0018) grant `MAINTAIN` on `node`,
  `version` and `vo_attestation`, and on `clinical.ehr`, to
  `ferroehr_clinical` and `ferroehr_party`, so the hourly statistics
  refresh works under the role split. `MAINTAIN` also allows `VACUUM`,
  `CLUSTER`, `REINDEX` and `LOCK TABLE` on exactly those tables. A role
  that still lacks it gets the warning naming the tables, as before.

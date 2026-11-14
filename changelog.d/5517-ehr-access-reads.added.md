- The `EHR_ACCESS` reads, a FerroEHR extension under the extensions root:
  `GET /ehr/{ehr_id}/ehr_access` (current, at `version_at_time` or at a
  `version_uid`) and `GET /ehr/{ehr_id}/versioned_ehr_access` with its
  revision history and version reads, shaped on the ITS-REST `EHR_STATUS`
  reads, so another product can read the access directives a CONTRIBUTION
  wrote. The access evaluator judges each read, each read writes an
  access-log entry, and only the administrator, the record's person and the
  gate-keepers read the settings. A read that carries the settings is
  `application/json` only.
- Every write of `EHR_ACCESS` settings (CONTRIBUTION, the controller's
  commit, EHR Extract import and archive load) refuses with `422` a
  directive or a gate-keeper that names a party the demographic service
  does not hold, naming the directive or gate-keeper and the party.
  An import and an archive load check only the head `EHR_ACCESS` version,
  the one in force, so a party erased since never blocks a faithful restore.

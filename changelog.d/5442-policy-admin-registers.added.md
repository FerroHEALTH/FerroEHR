- The audit category map and the EHDS Art. 3(3) access delay are admin
  registers under `{ext}/admin/registers/{audit-categories,access-delay}`:
  `admin_role` reads one, replaces it whole over the version `If-Match` names (`412` on a
  stale write, the new version as the `ETag`) and lists its history. Every version is kept
  with its committer, every read and write is an access record, and a change applies on
  the next request on every replica with no restart. A document that does not hold is
  refused with `400`.

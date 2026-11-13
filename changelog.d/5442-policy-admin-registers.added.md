- The audit category map, the EHDS Art. 3(3) access delay and the Cedar policy set are
  admin registers under `{ext}/admin/registers/{audit-categories,access-delay,cedar-policies}`:
  `admin_role` reads one, replaces it whole over the version `If-Match` names (`412` on a
  stale write, the new version as the `ETag`) and lists its history. Every version is kept
  with its committer, every read and write is an access record, and a change applies on
  the next request on every replica with no restart. A document that does not hold is
  refused with `400`, so a Cedar policy set that does not parse or validate never replaces
  the set in force.

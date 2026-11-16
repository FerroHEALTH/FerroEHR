- The ITI-81 `AuditEvent` search counts every match only when the query asks with `_total=accurate`
  or `_total=estimate` (FHIR R4 Search §3.1.1.5.2); `_count=0` still returns the total alone. A
  page without `_total` carries no `Bundle.total`, its `next` link still says whether another page
  follows, and its cost no longer grows with the log. `_total` with another value is a `400`. The
  operator console asks for the total it draws.

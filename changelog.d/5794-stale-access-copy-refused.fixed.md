- An AQL query judged on a copy of the access settings older than the store no longer answers
  with no rows (a query pinned to one EHR) or without the EHRs of a settings class the copy does
  not hold (any other query). The statement fails instead; the server reads its copy again and
  answers from the settings in force, and when the copy has not caught up within 100 ms it
  refuses with `503`, `Retry-After` and the problem type `access-refreshing`. The new counter
  `aql_stale_access_total{copy, outcome}` counts each case.

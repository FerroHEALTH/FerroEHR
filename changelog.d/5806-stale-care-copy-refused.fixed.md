- A read or an AQL query that a care relationship admitted could still be admitted on another
  instance after the care relationship ended, the caller left the care team or the patient's
  `PERSON` was erased, until that instance heard of the change. A decision that care admits now
  first checks that the instance's copy of the care relationships holds the revision in the
  database. When the copy is older, the server reads it again and judges care once more; when it
  does not catch up within 100 ms, it refuses with `503`, `Retry-After` and the problem type
  `access-refreshing`. The new counter `access_care_stale_total{outcome}` counts each case.

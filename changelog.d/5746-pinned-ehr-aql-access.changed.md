- An AQL query whose rows all belong to one EHR (an `ehr_id` scope of one, or
  `e/ehr_id/value` held equal in the `WHERE` clause) is decided before its
  statement, from the replica's copy of that EHR's settings, as a direct read
  is. Its SQL keeps only the part of the verdict that reads each object and
  one check, run once, that the copy is the revision in force; a copy older
  than the store admits no row. The access settings are no longer probed
  once per answered row. The capacity test's single-EHR AQL reads EHRs of 20
  results (`aql_ehr`) and 100 results (`aql_chart`), and the capacity test
  and the performance gate count each statement's planning time.

- A clinical AQL query under the access evaluator reads each EHR's settings row once, by its key,
  instead of PostgreSQL hashing every settings row before the first result. A population query
  at 20 000 EHRs with settings no longer scans them all; rows, counts and pages are unchanged.

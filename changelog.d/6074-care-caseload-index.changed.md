- A clinical AQL query whose access policy admits no EHR outside the caller's care now starts
  from the caller's own care relationships, so its cost grows with the caller's caseload, whatever
  the size of the store; every EHR it reaches is still checked one by one. The care
  projection (`clinical.access_care`) records what each carer is and gains an index led by the
  carer and its role, and `clinical.ehr` gains an index on its subject digest (both in the v6
  baselines).
- The capacity test gains `aql_caseload`, the newest compositions of the clinician's whole
  caseload, and `care_commit`, care relationship commits as an ADT feed writes them, which
  `CAPACITY_CARE_WEIGHT` puts in the mix.

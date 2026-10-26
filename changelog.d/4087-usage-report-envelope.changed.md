- **Breaking: the usage report is sent by `ferrohealth-usage-report` 0.1.2**
  (#4087), the FerroHEALTH products' shared reporter, on the same envelope v2.
  The `status` object loses `spec_profile`, and `postgres_major` is left out
  when the database does not answer. The daily metrics lose the `aql` group
  and the `aql_executions` and `aql_slow` counters, and gain the
  `responses_4xx` counter. Routes outside the API families (the health
  family, the management surface, the status and discovery documents) are
  counted in no group. A replica adds its counts to the shared window at its
  own daily attempt instead of every five minutes, and a daily report that
  does not reach the collector keeps its counts for the retry an hour later
  where it dropped them. A failed or refused send is logged at WARN where it
  was DEBUG, and the boot line takes the family's wording. The clinical
  migration `0018_usage_report_envelope` drops the `slow` column and the
  `aql` rows and adds `usage_report_counter`.

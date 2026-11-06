- An AQL query scoped to one EHR now takes the streaming plan shape whatever
  its `ORDER BY`, `DISTINCT` or aggregates, whether the EHR comes from the
  `ehr_id` request parameter or from an `ehr_id/value` equality in the query.
  On a store of 100,000 compositions such a query used to spend 16 to 64 ms in
  PostgreSQL's planner for an execution of about 1 ms, on every new connection
  and every new statement text; it now plans in 1 to 3 ms.

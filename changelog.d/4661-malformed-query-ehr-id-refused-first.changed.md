- A query request whose `ehr_id`, as the query parameter or the
  `openehr-ehr-id` header, is not a UUID is refused with the same 400 and text
  as before, now before the AQL is parsed and planned. The refusal therefore
  comes ahead of the refusals planning raises, such as an EHR scope on a query
  rooted at a demographic class, and `aql_queries_total{outcome="analysis_error"}`
  no longer counts it.

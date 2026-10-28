- Every ITI-81 search parameter answers through an index again. The store
  takes the search tokens of a record's rendered `AuditEvent` when it writes
  the record: each distinct token once in `audit.audit_search_token`, the
  code-system tokens (`type`, `subtype`, `entity-type`, `entity-role`,
  `source.identifier`) as one shared set in `audit.audit_search_set`, and their
  ids on the record in `search_tokens` under a GIN index; `entity` reads a hash
  index on `resource_id`. A search on `address` or a token parameter no longer
  renders every record in its `date` window. The tamper chain covers
  `search_tokens`, and the verifier reports a changed token or set.

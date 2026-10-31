- An audit record no longer adds a row to `audit.audit_search_token` for its
  request id or for the version uid a create wrote. ITI-81 `entity.identifier`
  finds both from the record's own `request_id` and `resource_id` columns,
  through a new hash index on `request_id` (audit migration 0020), so the token
  table stays flat as requests are audited. Retention now deletes the search
  tokens that no remaining record names, among them the request id tokens of
  records written by 5.6.0-rc5 and earlier, and the database still refuses to
  delete a token a kept record names.

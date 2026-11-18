- An ITI-81 `AuditEvent` search whose parameters each carry one value now runs
  its count and its page as named statements, prepared once on each pooled
  connection, so PostgreSQL plans a page once per connection instead of on
  every request (about 0.65 ms of planning per page). A search with several
  values for one parameter still runs unnamed.

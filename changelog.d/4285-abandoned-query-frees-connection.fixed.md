- An AQL query (ad hoc or stored) whose client disconnects, or that the
  server's own `query.timeout_ms` or HTTP request timeout gives up on, no
  longer keeps its statement running and its pooled connection busy until the
  statement ends. The connection is closed instead of returned to the pool, so
  the slot frees at once, and every pooled session now sets PostgreSQL's
  `client_connection_check_interval` to one second, so the backend stops the
  abandoned statement within about a second. The composition-count
  statistics behave the same way.

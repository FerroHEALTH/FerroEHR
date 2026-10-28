- A request whose database connection a PostgreSQL restart, a crash or the
  network cuts now answers `503 Service Unavailable` with `Retry-After`, as a
  request that cannot get a connection already did, instead of `500`. SQLSTATE
  class `08` (except `08P01`), `57P01`, `57P02`, `57P03` and `57P05`, and a
  reset, aborted or closed socket on an open session count as such a cut.

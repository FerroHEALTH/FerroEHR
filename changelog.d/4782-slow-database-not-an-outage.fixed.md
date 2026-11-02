- A database that accepts connections slowly is no longer recorded as down. A
  probe connect still in progress after its two-second bound now leaves the
  acquire queueing on the pool within `acquire_timeout_secs`, so the request is
  served once the connect completes, or answers the pool-timeout `503` with
  `Retry-After`. Only a refused or reset connect, or a server shutting down
  (`57P01`, `57P02`, `57P03`), marks the database down and answers the outage
  `503` at once.

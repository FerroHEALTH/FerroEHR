- `server.max_concurrent_requests` is 256 by default and always on, where
  unset meant no bound: offered load past capacity is refused `503` with
  `Retry-After` and the health routes are never refused, so a server started
  from the defaults no longer grows until the kernel kills it. The value
  comes from `ferrohealth-server` 0.10.0; `0` is refused. The configuration
  reference states how to size it. Upgrade: a deployment that relied on no
  bound sets the ceiling it wants (#5743).

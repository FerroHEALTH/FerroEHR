- While PostgreSQL is down, requests that need the database answer `503` with
  `Retry-After` about a second after the first one starts waiting, and at once
  after that, instead of each queueing up to `db.acquire_timeout_secs`
  (30 seconds by default). An acquire that has waited a second tries one
  connection of its own; when that cannot reach the database, the outage is
  known until a fresh attempt connects, and requests are served again as soon
  as one does. A pool that is only busy keeps queueing as before.

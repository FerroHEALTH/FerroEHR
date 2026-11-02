- A database that resets connections is now answered as an outage at once on
  every request, the `503` with `Retry-After` a refused connect gets, instead
  of each request opening a connection of its own that the database resets. A
  connect of the pool's own that is refused, reset or turned away as shutting
  down (`57P01`, `57P02`, `57P03`) records the outage as the probe does, and
  a probe that connects again clears it. A connect that is only slow still
  queues.

- A request past `server.max_concurrent_requests` now waits for a slot
  before it is refused: at most `server.admission_wait` (100 ms by default),
  in a first-come queue of at most `server.max_waiting_requests` (256 by
  default; `0` keeps no queue). A full queue or an expired wait is still
  answered `503` with `Retry-After`, and the health routes are never refused
  or queued. A short stall, such as a WAL flush, no longer refuses requests
  the server answers a moment later. `/metrics` shows the waits, the time
  waited and the refusals by reason (ferrohealth-server 0.11.0, #5980).

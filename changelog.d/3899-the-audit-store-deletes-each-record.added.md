- **The audit store deletes each record at its own retention end**
  (#3899). Every record stores the first date it may be deleted on: an
  access record the end `ehds-logging` states for its categories and origins
  (Regulation (EU) 2025/327 Annex II 3.4), any other record the day after
  its date of access plus the `[audit.store]` horizon, and none for a record
  kept without end. The reaper deletes a record on or after that date and
  never before it, where it deleted by one store-wide horizon. Records
  written before this release keep the horizon. The end joins the chain
  digest (version 3), so moving one shows in the chain check.

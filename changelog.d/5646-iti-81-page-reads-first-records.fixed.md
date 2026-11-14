- The ITI-81 `AuditEvent` search reads a page from the first records of each
  chain through their `recorded_at` index. It no longer joins and sorts the
  whole access log, and the cost of a page no longer grows with the log. The
  exact `total` counts the access log on its search index alone. Pages, totals
  and the access record of the read are unchanged. (#5646)

- The syslog and FHIR Feed sinks read their next system record through the
  index range between their cursor and the delivery frontier, and a retried
  parked record by its key. A delivery round no longer reads the whole
  `audit.audit_event` table: on 500,000 records it touches 61 pages instead of
  about 21,000, and takes 1 ms instead of 120 ms.

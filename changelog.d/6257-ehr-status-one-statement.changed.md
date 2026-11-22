- An `EHR_STATUS` update writes its access record first and then commits in
  one statement that writes the version and the EHR's subject and status
  columns together, judged on one read of the EHR and its status, so it sends
  5 statements where it sent 14. Its answers stay as they were, the `409` for
  a subject another EHR holds and the `412` for a stale `If-Match` included.

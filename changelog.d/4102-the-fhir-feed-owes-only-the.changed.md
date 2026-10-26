- **The FHIR Feed owes only the records written while it is configured**
  (#4102). A record waiting for delivery is a row in
  `audit.fhir_feed_outbox`, which the store writes with the record while
  `[audit.fhir_feed]` is on and delivery removes. The two partial indexes
  over the unset delivery stamps, which held every record of a deployment
  without a sink (about 4.8 MB each at 428,000 records), are dropped; the
  syslog one served no query, since syslog delivers from its own spool.
  Records stored before this release and never delivered ship once a feed
  first runs.

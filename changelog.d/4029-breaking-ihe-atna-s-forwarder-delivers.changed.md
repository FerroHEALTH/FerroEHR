- **Breaking: `ihe-atna`'s forwarder delivers the audit trail, and
  `[audit.fhir_feed] batch_size` and `max_retries` are gone** (#4029). Syslog
  frames are stored in `ihe-atna`'s spool and delivered from it with
  backoff, retried until the repository takes them, so a failed send is no
  longer lost. The local audit store keeps only the FHIR document, so the
  spool, not the store, is syslog's outbox, and `delivered_syslog_at` is no
  longer written. New `[audit.syslog]` keys: `spool_dir` (a spool that
  survives a restart; unset, it is held in memory), `spool_max_messages`,
  `spool_max_bytes` and `retry_max_ms`. The FHIR Feed delivers from the store
  through an `ihe-atna` outbox, or with the store off from an in-memory
  spool. A record the repository refuses is set aside until the next start,
  and the records behind it flow on. `retry_max_ms` takes the place of
  `batch_size` and `max_retries`, which stop startup by name. A new
  `audit_delivery` health indicator reports each forwarder. Upgrade: remove
  `batch_size` and `max_retries` from `[audit.fhir_feed]`, and set
  `[audit.syslog] spool_dir` to a directory only the server can read.

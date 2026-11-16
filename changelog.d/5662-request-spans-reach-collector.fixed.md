- The OTLP trace export carries every request span again. The
  `ferrohealth-telemetry` 0.4.0 and 0.4.1 export filter skipped the next
  request span on a worker thread after a dependency checked a `log` record
  it then did not write, so most request traces never reached the
  collector while the console still logged each request. The pin moves to
  `ferrohealth-telemetry` 0.4.2, which re-evaluates the export filter for
  every span.

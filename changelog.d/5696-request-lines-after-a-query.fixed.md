- The console writes the request line of every request again under a
  `[telemetry] filter` that lets a target through above its default level,
  such as `info,ferrohealth_smart=debug`. A query's `log` check at `debug`
  left the console filter's "no" on the thread, and the line the request
  wrote next was dropped, for every request that ran a query. The server now
  installs its subscriber with the `log` bridge of ferrohealth-telemetry
  0.5.0, which leaves no decision behind.

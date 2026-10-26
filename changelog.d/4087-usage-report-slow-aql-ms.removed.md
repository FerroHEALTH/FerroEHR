- **Breaking: `[usage_report] slow_aql_ms` is removed** (#4087), with the AQL
  series it set the threshold of. A file or variable that still sets it
  refuses the start; remove the key. `state_dir`, the shared section's
  directory for the instance id, is refused: FerroEHR keeps the instance id
  and the counts in the clinical database. The endpoint is now refused when
  the configuration is read, with the same rule (`https`, or `http` to a
  loopback host, never with credentials).

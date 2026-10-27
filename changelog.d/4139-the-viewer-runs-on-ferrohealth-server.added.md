- **The viewer serves the family's health routes and commands**:
  `/health/liveness`, `/health/readiness` and `/health`, `ferroehr-viewer
  config check` (reads the configuration and builds the CDR and issuer
  clients without reaching either), `--config` and `--set key=value`, the
  `[server]` table with listener TLS, and the bounded drain on `SIGTERM`
  (`server.drain_delay_ms`, `server.shutdown_timeout_ms`). `ferroehr-viewer`
  with no subcommand still serves.

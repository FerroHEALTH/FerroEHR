- **FerroEHR's own `[server]` defaults again** (#4120): with no `listen` and
  no `body_limit_bytes` in the file or the environment, the server listens on
  `0.0.0.0:8080` and reads bodies up to 64 MiB, and a partial `[server]` table
  keeps both for the keys it does not name (`ferrohealth-config` 0.2.6
  `Loader::defaults`). The container image no longer sets `PORT=8080`, and
  the quickstart Compose file, the dev configuration and chart 19.0.1 no
  longer write the two keys. Upgrade: a bare binary that must stay on the
  loopback sets `listen = "127.0.0.1:8080"`.
- **The shared crates move to their newest releases** (#4120):
  `ferrohealth-smart` 0.8.3, `ferrohealth-authz` 0.2.2, `ihe-atna` and
  `ehds-logging` 0.3.1, `ihe-iti` 0.0.65, `ferrobridge-term` 0.1.140,
  `ferrohealth-server` 0.3.5 and `ferrohealth-config` 0.2.6.

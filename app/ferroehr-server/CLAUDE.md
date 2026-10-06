# `ferroehr-server` — the binary

The one bin crate, split per the Book ch12.3: `src/lib.rs` carries ALL the
wiring as the testable `run(Cli)` path — config → telemetry → PG pool →
migrations (`db::run_migrations`) → ATNA audit sender → `FerroEhrService` →
`ferroehr_rest::serve_full`, plus graceful shutdown (the audit queue drains
before exit), the detached usage-report task (started after the service is
assembled, aborted at shutdown) and the `healthcheck`/`config`/`db`/`usage-report`/`report`
subcommands; `src/main.rs` is a
thin `ferroehr_server::run(Cli::parse())` shell and must stay that way (a
bin-only crate is untestable by construction — Book ch11.3).

- **No logic lives here.** Anything beyond wiring belongs in `ferroehr`
  (platform) or `ferroehr-rest` (protocol adapter). `anyhow` is allowed here
  (the lib target is the binary's own logic half, not a consumable library);
  `thiserror` everywhere else.
- **`tests/it/` may test ONLY the wiring seam** — `Cli` parsing (incl. the
  `--set key=value` override parser), the subcommand shapes, the `run`
  branches that need no database, listener, or network (`config default`), and
  the **boot-installed POLICY posture**: what the shipped configuration
  compiles into and this crate installs on the service (`build_authz`, the
  `[privacy]` policy). That last one belongs here and nowhere else — the
  platform suites build with `FerroEhrService::new()`, which installs no
  collaborator, so a default posture is invisible to every other gate
  (#3190). Everything past that seam belongs to the crate that owns it:
  `ferroehr` API behaviour in `app/ferroehr/tests/it/`, the assembled
  `ferroehr-rest` router in `app/ferroehr-rest/tests/it/`. Parking either here
  made them invisible to the owning crate's gate — the four that had been
  (`persistence`, `telemetry`, `fhir_inbound`, `service_query`) were relocated
  to their owners. The dev-dependency set is scoped to that seam (`anyhow`,
  `assert_fs`, `clap`, `openehr-its`, `tokio`); a new dev-dep here is a signal
  the test belongs in another crate.
- The bin target is named `ferroehr` (`[[bin]] name = "ferroehr"`; container
  entrypoints/compose/Helm and `scripts/*` invoke that name — do not rename).
- Gates: `cargo clippy -p ferroehr-server --all-targets` +
  `cargo nextest run -p ferroehr-server`.

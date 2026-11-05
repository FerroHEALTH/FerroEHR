- RUSTSEC-2026-0440 (`astral-tokio-tar` is unmaintained, with no patched
  version) does not affect FerroEHR: `testcontainers` reaches the crate only
  in the test harness, and the server binary, the `ferroehr` image and the
  `ferroehr-viewer` image do not contain it. `deny.toml` accepts the advisory
  until a `testcontainers` release drops the crate
  (`security/vex/rust-advisories.openvex.json`).

- RUSTSEC-2026-0543 (`chrono` is unmaintained) and RUSTSEC-2026-0542
  (`humantime` is unmaintained) name no vulnerability and have no patched
  version. Both reach the server binary and the `ferroehr` image only through
  `object_store`, the multimedia blob backend, and FerroEHR code names neither.
  `sqlx` no longer enables its `chrono` feature. `deny.toml` accepts both
  advisories until the OpenDAL rewrite of blob storage removes `object_store`
  (`security/vex/rust-advisories.openvex.json`) (#5934).

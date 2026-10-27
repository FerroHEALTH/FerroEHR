- **Breaking: `openehr-sdt` 0.0.111 no longer carries `smart_scopes`.** The
  SMART on openEHR scope grammar moved, with its API and tests, to
  `ferrohealth-smart` 0.8.5 as `ferrohealth_smart::openehr::smart_scopes`, so
  a consumer of `openehr_sdt::smart_scopes` names that path. FerroEHR pins
  `ferrohealth-smart` 0.8.5, its scope gate and the viewer's scope previewer
  (both halves) parse with it, and the build resolves one `openehr-sdt`.
  Token checking and the scopes a token grants behave as before.

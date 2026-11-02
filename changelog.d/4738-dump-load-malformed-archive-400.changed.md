- **Breaking: `POST /admin/load` answers a malformed archive with `400`**
  (#4738). A container, `manifest.json`, segment or blob entry that is
  absent, truncated or does not parse as an export archive, and a manifest
  that declares an unknown logical format, are the caller's input, so the
  load refuses them with the new `archive-invalid` problem type instead of a
  `500`. The `detail` names the entry, and with
  `Prefer: return=representation` the `archive` member carries it as
  `{"entry": …}` or `{"format": …}`. A location with no archive, or one the
  server's filesystem refuses to read, still answers `500`.

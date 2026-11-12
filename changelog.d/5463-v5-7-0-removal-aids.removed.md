- **The v5.7.0 configuration removal aids are gone.** A key a release before v5.7.0
  read, in the server's `ferroehr.toml` or the viewer's configuration, is now an unknown
  key like any other: the start is refused naming it and the keys its table does read,
  where v5.7.0 named the key that replaced it. The v5.7.0 changelog still maps every old
  key to its replacement. `ferroehr.default.toml` keeps its 913 keys.

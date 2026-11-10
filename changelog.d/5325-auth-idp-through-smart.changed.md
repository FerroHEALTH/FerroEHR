- `[auth.idp.entra]` is read by `ferrohealth-smart`'s `[auth]` section, so an
  unknown key under it is refused with the crate's field list. The key path
  is unchanged. The effective configuration now shows `[auth.idp]` as an
  empty table when no adapter is configured.

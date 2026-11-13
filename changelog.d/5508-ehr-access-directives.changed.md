- `EHR_ACCESS.settings` now hold the `access_directives` scheme, an
  `ACCESS_DIRECTIVE_SETTINGS` with the default access, the gate-keepers, the
  subject's directives and the sensitivity labels, coded in FerroEHR's access
  control terminology. Every write is checked against the draft access control
  invariants and refused whole with `400` naming the first defect. A new
  version is committed only by a gate-keeper the current settings name or by
  `admin_role`; anyone else gets `403`. The settings are read only by those
  callers and the record's person: to anyone else they are absent from AQL,
  from an EHR Extract, and a CONTRIBUTION that commits them answers `404`.
  The default archetype `openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0` is
  stored at boot when absent. No configuration key is removed or added (#5508).

- `openehr-am`: `ArchetypeHrid` implements `Display`, writing the physical
  form `[ns::]publisher-package-class.concept.vMAJOR.MINOR.PATCH[-status[.build]]`
  that its `FromStr` reads back, so a consumer no longer rebuilds an archetype
  or template id from the fields. `openehr-adl` writes every identifier through
  it. `version_id` and `physical_id` no longer write a trailing `.` after a
  version modifier that carries no build count.

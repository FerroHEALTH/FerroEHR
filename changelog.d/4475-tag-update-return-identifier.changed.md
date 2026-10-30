- A tag update (`PUT …/tags` on EHR_STATUS, COMPOSITION and the five party
  kinds) sent with `Prefer: return=identifier` now answers `200` with the
  stored tag list and `Preference-Applied: return=identifier`. It answered
  `204`.

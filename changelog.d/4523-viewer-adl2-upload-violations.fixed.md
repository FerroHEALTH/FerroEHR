- The viewer's ADL 2 template upload names each violation the CDR found when
  it refuses the source: every AOM2 rule code with the CDR's words for it, so
  a template whose constituent archetype the CDR does not hold names that
  archetype (`VARXR`). The upload now asks for `Prefer: return=representation`,
  the preference under which ITS-REST lets a `4xx` carry its error details,
  and reads the stored template's id from the `ETag` of the `201`.

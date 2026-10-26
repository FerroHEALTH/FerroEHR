- **A party reference whose type is not the builder's default survives a FLAT
  or STRUCTURED round trip** (#4052). master05 gives `PARTY_REF.type` no
  row, so a `_health_care_facility` that is not an `ORGANISATION`, or a
  `_provider`, performer, composer, feeder-audit party or `PARTY_PROXY` datum
  that is not a `PERSON`, is now written as `|raw` (master04 §Raw canonical
  JSON) and read back whole; it came back with the default type.

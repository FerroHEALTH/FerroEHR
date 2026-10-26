- **An archetype HRID's version status reads as the `VERSION_STATUS` it
  names** (#4051), on the 0.0.102 crate line. AOM2
  `ARCHETYPE_HRID.version_status` maps `-rc`, `-alpha`, `-beta` and no
  modifier to `release_candidate`, `alpha`, `beta` and `released`, where `rc`
  and the empty modifier read as `VersionStatus::Other`. `openehr-adl` writes the
  HRID back with the modifier (`-rc`, never `-release_candidate`), so parse
  and write are inverse.

- **The `openehr-*` crates carry the APIs FerroBRIDGE kept local copies of**
  (#4047, #4048), on the 0.0.99 crate line. The FLAT builder takes a value
  whole under `|raw` (Simplified Formats master04 §Raw canonical JSON) on
  `ROLE.performer` (`<root>/performer|raw`, a `PARTY_REF`), on the COMPOSITION
  `language`, `territory` and `composer`, and on the `_feeder_audit`,
  `_link:i`, `_participation:i`, `_other_participation:i`, `_provider` and
  `_health_care_facility` families. A `ctx/` key still wins over a path
  spelling of the same attribute, and a raw value of the wrong class, or one
  beside another suffix, is refused as `InvalidRaw`. `openehr-am` implements
  `FromStr` for `ArchetypeHrid` (`openehr_adl::hrid::parse_hrid` now reads
  through it), `openehr-base` adds `TerminologyId::new(name, version_id)`,
  and `openehr-term` names the `audit_change_type` and
  `version_lifecycle_state` concepts (`codes::AuditChangeType`,
  `codes::VersionLifecycleState`), held equal to the embedded terminology.
  Until a `ferrohealth-smart` release requires 0.0.99, the build holds a
  registry copy of `openehr-sdt` 0.0.98 for it beside the workspace 0.0.99.

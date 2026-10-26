- **`TEMPLATE_ID` answers its BASE 1.3.0 HRID functions** (#4068), on the
  0.0.104 crate line. SPECBASE-42 gives `TEMPLATE_ID` `is_hrid`, `namespace`,
  `qualified_rm_class_name`, `concept_id`, `version_id` and the major, minor
  and patch versions (BASE base_types master05 §Template Identifiers). An
  identifier in the namespaced HRID form of the Artefact Identification
  reference expression (master03 §Formal Model) yields its parts; a legacy
  free-text template name stays valid and answers `false` and empty parts.

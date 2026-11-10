- The build compiles FHIR R4's resources alone: `fhir-types` is required with
  its `r4-resources` feature in place of the crate-wide `resources`, and the
  FerroBRIDGE crates (`fhir-types`, `fhir-search`, `ferrobridge-term`) are
  required by caret ranges in place of exact versions.

# Provenance: SMART App Launch 2.2.0, the Task launch profiles

- Source: the FHIR package `hl7.fhir.uv.smart-app-launch` 2.2.0 from the FHIR package
  registry, <https://packages.fhir.org/hl7.fhir.uv.smart-app-launch/2.2.0>, the package of
  <https://hl7.org/fhir/smart-app-launch/STU2.2/> (FHIR 4.0.1)
- Pin: the tarball's sha256 `0bb498dd09677684239f24d279b6da711cd814449f2f473c2970fbe881ec9c70`; the script refuses any
  other bytes
- Document: the Task profile for requesting a SMART launch
  (`task-ehr-launch`, `task-standalone-launch`), its two published
  examples and the `smart-codes` CodeSystem the profiles' codes come from;
  the package manifest is left out, as no FHIR manifest is committed
  (`scripts/checks/fhir-package-manifests.sh`)
- Retrieved: 2026-11-19
- Upstream licence: CC-BY-4.0, the `license` the package manifest declares:
  SMART App Launch 2.2.0 by HL7 International / FHIR Infrastructure,
  licensed CC BY 4.0, vendored unchanged. `REUSE.toml` declares the tree
  `CC-BY-4.0`.
- What is taken: the files below, each whole, at their path in the package
- Fetched by: `scripts/vendor/smart-app-launch.sh`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `package/StructureDefinition-task-ehr-launch.json` | `be11ce822660e9f06282372a18d9672bf458efce8c6e3c0cd34edac8752597e0` |
| `package/StructureDefinition-task-standalone-launch.json` | `6a21d0ddfedc3ef9bc06e37ccc65099a29e8647234800fbde24289e6f4dfc5ad` |
| `package/CodeSystem-smart-codes.json` | `64cc1466d906189e5ea0d14b6c3cf9427a958615ff538a85d7b0b4fc913ddf7e` |
| `package/example/Task-task-for-ehr-launch.json` | `cb1c0b9afa88183027ba65d85e5f268962358d1769ad863a5c4c2d8cb04fe13b` |
| `package/example/Task-task-for-standalone-launch.json` | `6e2b39e1aa7bbcc241abe1643a637153f6ad7809ace2aeb2f49d2c14c6626ad9` |

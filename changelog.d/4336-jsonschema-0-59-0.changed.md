- Canonical JSON validation against the openEHR ITS-JSON schemas runs on
  `jsonschema` 0.59.0, which refuses a value whose regex match the engine
  cannot finish where 0.58 accepted it under `not` or `patternProperties`.
  The `openehr-*` crates move to 0.0.113.

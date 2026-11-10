- The shipped `ferroehr.toml` template (`app/ferroehr/assets/ferroehr.default.toml`)
  is generated from the configuration types with `tools/config-reference`:
  every key, each under its description and set to its default, a secret or
  a key with no default as a `#?` placeholder line. The book's new page
  "Every configuration key" lists the same keys with their types, defaults
  and descriptions from the same schema. CI fails when either drifts from the
  types. No key changes: a `ferroehr.toml` that loads today loads unchanged.

- The shipped `ferroehr.toml` template (`app/ferroehr/assets/ferroehr.default.toml`)
  holds only the keys an operator usually sets, each at its default, in place
  of every key. The book's new page "Every configuration key" lists every key
  with its type, its default and its description. Both are generated from the
  configuration types with `tools/config-reference`, and CI fails when either
  drifts from them. No key changes: a `ferroehr.toml` that loads today loads
  unchanged.

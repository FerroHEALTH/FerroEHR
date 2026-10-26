- **An ordinal of an archetype read in the ADL 1.4 dialect lists its
  at-codes** (#4059). The `symbol` of each `[value, symbol]` row kept the
  verbatim `local::at0006` spelling, which the ADL 2 builder took for the
  code and its label; it now reads the at-code with its archetype rubric, as
  #4056 does for coded text. The nine `openehr-*` crates move to 0.0.101.

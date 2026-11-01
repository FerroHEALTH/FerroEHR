- The served Web Template of an ADL 2 operational template writes a coded
  input whose at-codes the template binds to a terminology as the bound
  terms: its `code` list carries each binding's code and label and its
  `terminology` names the bound terminology (ADL2
  `master08-terminology_integration` §"From Constraints to Concrete Codes in
  Data"). `COMPOSITION.category` bound `at1` to `openehr::433` reads
  `{"value": "433", "label": "event"}` with `terminology: "openehr"`, where
  it read the local `at1` before, and an `[acN@ttttt]` constraint takes the
  bound terms of the terminology it names. A FLAT or STRUCTURED composition
  without a category, and the example composition, now carry `openehr::433`.
  The `aqlPath` of every data value drops its `[idN]` predicate, so a
  constrained `category` and `context/setting` read `/category` and
  `/context/setting`.

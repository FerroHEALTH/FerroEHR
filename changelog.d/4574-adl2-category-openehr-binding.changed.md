- The served Web Template of an ADL 2 operational template writes a coded
  input whose at-codes the template binds to the `openehr` terminology as
  the bound openEHR terms, which the RM requires for `COMPOSITION.category`
  and `EVENT_CONTEXT.setting`: its `code` list carries each term's code and
  label and its `terminology` is `openehr`. `category` bound `at1` to
  `openehr::433` reads `{"value": "433", "label": "event"}` with
  `terminology: "openehr"`, where it read the local `at1` before. An
  `[atN@ttttt]` or `[acN@ttttt]` constraint takes the bound terms of the
  terminology it names; every other binding keeps the local at-codes (ADL2
  `master08-terminology_integration` §"From Constraints to Concrete Codes in
  Data"). A FLAT or STRUCTURED composition without a category, and the
  example composition, now carry `openehr::433`. The `aqlPath` of every data
  value drops its `[idN]` predicate, so a constrained `category` and
  `context/setting` read `/category` and `/context/setting`.

- A COMPOSITION committed against an ADL 2 operational template is validated
  against the template's leaf constraints, where it was stored with `201`
  whatever it held. An inlined archetype recorded by its complete identifier
  (`openEHR-EHR-OBSERVATION.x.v1.0.0`, BASE `master07-referencing`
  §Reconstitutability) now matches its template node, so the `C_INTEGER`,
  `C_REAL`, `C_STRING`, `C_BOOLEAN`, tuple, value-set and cardinality
  constraints below it answer `422` (AOM 2 `master04.5`). An archetype root the
  template does not inline is refused as a node the template does not define,
  and a `C_INTEGER` or `C_REAL` list (`{10, 15, 20}`) admits each of its values
  where only the first was read. Canonical, FLAT and STRUCTURED commits take
  the same check. The nine `openehr-*` crates step to 0.0.127.

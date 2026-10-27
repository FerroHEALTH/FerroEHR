- The OPT 1.4 writer refuses a redefined leaf constraint that admits a value
  the archetype's does not, as AOM2 Phase 2 validation (VPOV) requires: a
  string list or pattern, an integer or real list or range, a boolean, a date,
  time, date/time or duration range, pattern or timezone validity, a code
  phrase's terminology or code list, a quantity's property, units, magnitude
  or precision, and an ordinal list. Each refusal is a `WriteError::Widening`
  naming the archetype, the path and the constraint. The openehr crates move
  to 0.0.107.

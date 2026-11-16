- A population AQL query under the access evaluator probes each EHR's access settings once per
  row instead of once per row and per alias, compares the root archetype the object's head row
  now carries instead of reading the root node again, and for a caller whom every known settings
  class admits in full (an administrator) reads the EHRs of an unknown class once per query.
  Rows, counts and pages are unchanged. The clinical and party schemas gain
  `vo_head.root_archetype`, filled by migration.

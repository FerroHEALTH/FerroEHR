- An AQL query with an archetype predicate no longer builds the archetype
  index bitmap of the whole store once per version. The planner multiplied
  the selectivities of the archetype columns, which each archetype fixes
  together, and expected about 15 000 of 545 067 blood-pressure OBSERVATIONs;
  a query over one EHR then took 38.8 s and answered `408`. Clinical migration
  `0021_archetype_statistics.sql` gives the columns a statistics object, and
  the same query takes 0.3 s.

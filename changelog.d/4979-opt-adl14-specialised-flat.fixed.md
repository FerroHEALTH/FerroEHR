- `openehr-adl`'s `opt::create_opt` no longer flattens an ADL 1.4
  specialised archetype over its parent. An ADL 1.4 archetype is authored in
  flat form (ADL2 `master07.02` §File-naming Convention, AOM2 `master03`
  §Structural Variants), so overlaying a child such as CKM
  `openEHR-EHR-COMPOSITION.report-result.v1` on `report.v1` appended every
  object without a node id a second time: the OPT held two `EVENT_CONTEXT`
  objects and its Web Template two `/context` nodes. An artefact whose
  `adl_version` is 1.4 or earlier is now its own flat form, keeps its
  `parent_archetype_id`, and needs no parent in the repository; its 1.4 term
  constraints are still rewritten into the ADL2 form. ADL 2 differential
  flattening is unchanged.

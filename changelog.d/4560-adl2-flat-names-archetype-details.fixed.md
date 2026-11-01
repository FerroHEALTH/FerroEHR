- A COMPOSITION written from FLAT or STRUCTURED against an ADL 2 operational
  template carries what a canonical body built to that template carries. A
  node the template gives no term text, such as the HISTORY and ITEM_TREE of
  an OBSERVATION, is named for its RM type (`History`, `Tree`) instead of its
  bare id-code (`id2`). Every inlined archetype root records the complete
  identifier the template names (`…cnf_minimal.v1.0.0`, BASE
  `master07-referencing` §Reconstitutability) in its `archetype_node_id` and
  `archetype_details.archetype_id`, and `archetype_details.rm_version` is the
  template's `rm_release`. Before this change the root recorded the interface
  form (`…v1`) and the server's RM 1.2.0. Compositions built against an
  ADL 1.4 template keep their major-version archetype ids and RM 1.2.0.

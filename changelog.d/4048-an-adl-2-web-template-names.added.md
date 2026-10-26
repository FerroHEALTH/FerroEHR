- **An ADL 2 Web Template names a node under a template overlay by its
  parent archetype.** `openehr_adl::opt::create_opt` records each overlay's
  specialisation parent in the OPT's `other_meta_data`
  (`template_overlay_parents`), and the ADL 2 builder uses the parent's id
  for the node, its paths and its slot admissions, so data and mappings
  written against the published archetype match. An ADL 2 header now quotes
  a free metadata value that is not a bare token and reads a quoted one back.

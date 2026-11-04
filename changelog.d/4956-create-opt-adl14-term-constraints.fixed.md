- `openehr-adl`: `opt::create_opt` takes unconverted ADL 1.4 constituents
  again. It keeps their 1.4 node codes, so 1.4 template paths such as
  `/activities[at0001]/description[at0002]` still resolve, and rewrites each
  1.4 term constraint to the form ADL2 spells (`master04.5` §Formal
  Terminology Constraint), the way `adl14::convert` does. An external code
  such as `[openehr::433]` becomes a new at-code bound to
  `http://openehr.org/id/433`, and a code list such as
  `[local::at0168, at0169]` becomes an ac-code value set. The OPT2 then
  prints and parses back.

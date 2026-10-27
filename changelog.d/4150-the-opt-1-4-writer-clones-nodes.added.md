- **The OPT 1.4 writer clones nodes and narrows types**, on the 0.0.106
  crate line: `opt14_writer::Narrowing::Clone` copies an archetype node that
  may occur more than once under its own node id, told apart by a `name`
  constraint and carrying its own narrowings and slot fills, and refuses a
  clone of a node that occurs at most once, a name another copy carries, or
  copies the attribute's cardinality cannot hold (AOM2 VSONCO, VACMCU,
  VACMCO). `Narrowing::Redefine` narrows an attribute's children to
  conforming subtypes (a `DV_TEXT` value to a `DV_CODED_TEXT`) or drops
  alternatives, keeping the archetype's other constraints, and refuses a type
  that does not conform (AOM2 VSONCT). An attribute the template constrains
  can also hold archetypes placed by a slot fill. The writer now rebuilds 143
  of the 305 CKM operational templates to their upstream node tree, up from
  67.

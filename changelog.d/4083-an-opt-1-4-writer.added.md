- **`openehr-its` writes an OPT 1.4**, on the 0.0.105 crate line:
  `opt14_writer::build` and `write` assemble an operational template from a
  template definition (the root archetype, the archetypes placed in its slots
  or open attributes, and the occurrences, existences and attribute
  constraints it narrows) and the AM 1.4 archetypes it names. Internal
  references are inlined and deleted nodes removed (OPT2 raw flattening), a
  slot filler is checked against the slot's assertions and type, a widening is
  refused, and the output is byte-deterministic, valid against ITS-XML
  `Template.xsd` and accepted by the template upload.

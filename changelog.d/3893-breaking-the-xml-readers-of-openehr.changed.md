- **Breaking: the XML readers of `openehr-its` refuse a document outside
  the schema** (#3893), on the 0.0.103 crate line. A child element a type's
  closed `xs:sequence` does not declare, or a child element inside a text
  element, is refused with `XmlError::UndeclaredChild` (XML Schema 1.1 Part 1
  §3.4.4.2) where it was skipped as if absent, and `opt14::from_xml` and the
  AOM2 readers refuse a root other than the one the schema publishes
  (`<template>`, ITS-XML `components/AM/Release-1.4/Template.xsd`) with
  `XmlError::UnexpectedRoot`. One tool-written element no schema declares,
  `match_negated` on a `C_ATTRIBUTE`, is read when `false` and refused when
  `true` (`XmlError::UnsupportedExtension`): a negated match changes the
  constraint, and no schema says how. A CDR upload of a template outside the
  schema is now refused; the CNF Robot `valid_templates/validation/clinical_content_validation.opt`
  and two epSOS CKM templates are such documents (#4077, #4078).

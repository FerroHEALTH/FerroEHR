- **A composition commits when an element's value matches any one of the
  alternatives its template allows** (#4226), on the 0.0.110 crate line. ADL
  1.4 §"Single-valued Attributes" makes the object constraints under a
  single-valued attribute alternatives, only one of which the data needs to
  match. The free text of an open coded value-set (FLAT or STRUCTURED
  `|other`, a canonical `DV_TEXT`) was refused as `expected RM type
  conforming to DV_CODED_TEXT but found DV_TEXT` where the template allows
  `DV_TEXT` beside `DV_CODED_TEXT`, and a value that matched only a later
  alternative of the same type, such as a second `DV_QUANTITY` with other
  units, was checked against the first alone. When no alternative matches,
  the refusal names every alternative with what each one found, and FLAT
  writes a value under one of several same-type alternatives, not under each.

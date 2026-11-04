- The ADL 1.4 to ADL 2 conversion gives a terminology constraint that names
  no codes (`[openEHR::]`, `[local::]`) an ac-code per terminology, with no
  value set and no binding, and records the terminology in the conversion
  log. Before, every such constraint in an archetype shared one ac-code
  labelled as a synthesised value set, whatever terminology it named.

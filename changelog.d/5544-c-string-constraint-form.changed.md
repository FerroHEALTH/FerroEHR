- Archetype validation refuses a `C_STRING` constraint that mixes fixed
  Strings with a regular expression, or carries more than one regular
  expression, under the code `C_STRING.constraint` (SPECPR-172). An OPT 1.4
  string pattern now converts to a delimited regular expression, and a pattern
  beside a list is kept and refused instead of being dropped.

- A COMPOSITION committed against an ADL 2 operational template whose
  `C_INTEGER` or `C_REAL` constraint is a union of ranges (for example
  `magnitude matches {|0..5|, |10..15|}`) is refused with 422 when the value
  lies outside every range. Such a value was stored. The served Web Template
  does not change: its input `validation.range` holds one range, so a union
  leaves it out, as before, and the check runs on the commit.

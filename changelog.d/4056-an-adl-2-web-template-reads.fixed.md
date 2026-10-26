- **An ADL 2 Web Template reads the ADL 1.4 term constraints of an archetype
  read in the ADL 1.4 dialect** (#4056). `openehr-adl` keeps such a
  constraint verbatim (`openehr::433`, `local::at0003,at0004;at0003`) for its
  1.4 to 2 converter, and an at-coded ADL 2 template over unconverted
  archetypes carried it into the ADL 2 builder, which took the whole string
  for one local code. The builder now reads it as the OPT 1.4 builder reads a
  `C_CODE_PHRASE`: an `openehr` code carries its terminology and rubric,
  local codes their archetype rubrics, and the assumed code becomes the
  default. The nine `openehr-*` crates move to 0.0.100.

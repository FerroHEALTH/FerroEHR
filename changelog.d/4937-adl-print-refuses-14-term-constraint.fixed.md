- `openehr-adl`: `print::print` refuses a terminology constraint ADL2 has no
  syntax for, such as the ADL 1.4 `[openehr::433]` an unconverted 1.4
  archetype keeps, with `PrintError::TerminologyConstraint`, where it used to
  write text that `parse_artefact` then refused (ADL2 `master04.5`
  §Terminology Constraints). An ADL 1.4 archetype goes through
  `adl14::convert` before it enters an `ArchetypeRepository`; the OPT built
  from it then prints and parses back unchanged.

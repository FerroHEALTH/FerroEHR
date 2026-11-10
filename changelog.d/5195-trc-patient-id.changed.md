- A contact point's `patient.id` is read through `ferrohealth-smart`'s
  `TrcPatientId`, the family's one definition of the TRC patient. A value with
  whitespace in its system, or a value holding a control character, `|` or a
  CX delimiter (`^`, `&`, `~`, `\`), is refused as a missing binding, as the
  contact point that writes the claim refuses it.

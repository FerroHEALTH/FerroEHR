- **The BASE, AM, TERM and LANG specifications are re-vendored** (#4068,
  #4069, #4070, #4071) at BASE `93abaf9f`, AM `51aa07b2`, TERM `b2dd077d` and
  LANG `e61a7192`. The TERM `languages` code set restores `pt-pt`, adds
  `ro-md` and `ru-md`, and names the replacement of each deprecated code
  (SPECTERM-36), so the `openehr-term` code-set lookup accepts the two new codes. The
  BASE `Interval.has` post-condition gains its brackets, which the
  implementation already followed. AM's Archetype Identification document is
  superseded by BASE Artefact Identification (SPECAM-103), and the citations
  move with it. The nine `openehr-*` crates step to 0.0.104.

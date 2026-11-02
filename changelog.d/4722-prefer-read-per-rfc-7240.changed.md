- Every route now reads `Prefer` as RFC 7240 §2 defines it, as error
  responses already did: a preference in a second `Prefer` field, one that
  carries parameters after `;` (`return=representation; x=1`), and one with
  whitespace around `=` (`return = representation`) are honoured. Before,
  only the first `Prefer` field was read, and such a preference was ignored.

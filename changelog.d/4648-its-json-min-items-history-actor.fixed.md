- A composition whose HISTORY carries `events: []`, and an AGENT, GROUP,
  ORGANISATION or PERSON carrying `languages: []`, are refused with `400` and
  commit nothing. The ITS-JSON schemas give both arrays `minItems: 1`, and the
  canonical JSON reader now refuses the empty array as it does an empty
  FOLDER `folders`. The FLAT builder no longer writes an empty `events` for a
  HISTORY that has no event.

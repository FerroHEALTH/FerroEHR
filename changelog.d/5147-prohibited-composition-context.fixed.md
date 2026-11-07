- `openehr-sdt` builds no `/context` node, and no `start_time` or `setting`
  leaf, in the Web Template of an OPT 1.4 whose COMPOSITION prohibits
  `context` (existence `0..0`, or an `EVENT_CONTEXT` of occurrences `0..0`),
  and the template example of such a template carries no `context`. A
  template that leaves `context` open or optional keeps the node as before.

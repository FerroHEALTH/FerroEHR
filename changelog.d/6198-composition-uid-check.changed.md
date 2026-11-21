- A JSON COMPOSITION read decides whether the stored body carries the
  version's own root `uid` from the root members before it (`_type`, `name`,
  `archetype_node_id`) instead of searching the whole body: about 80 ns in
  place of 14 µs for a 128 KB IPS composition, with the same bytes served. A
  body whose only matching `uid` sits deeper than the root now takes the parse
  path and is served with its root `uid` injected.

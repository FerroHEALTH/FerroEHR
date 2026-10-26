- **Three AQL shapes no longer read every composition** (#4089, #4090),
  measured against a release build on one host holding about 51 000
  compositions. A population query ordered by `c/context/start_time/value`
  with a small `fetch` reads the compositions in start-time order from a new
  index and stops at the limit: 169 ms became 7 ms at p50. A
  `c/archetype_details/template_id/value` predicate on a `COMPOSITION` reads
  the indexed template id of the version: 142 ms became 2 ms for a template
  nothing matches, and 29 ms became 11 ms for one of thirteen templates. A
  `COMPOSITION`, `EHR_STATUS` or `EHR_ACCESS` root is bound at its root row,
  so a containment with `OR` probes one row per version where it read every
  node of it. Clinical migration 0017 adds the index and the planner
  statistics, and analyzes the node table once.

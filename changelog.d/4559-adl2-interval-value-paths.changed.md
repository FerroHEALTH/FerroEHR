- The served Web Template of an ADL 2 operational template writes the
  `aqlPath` of an ELEMENT's value and of a data value's parts without an
  `[idN]` predicate, since a data value carries no `archetype_node_id` (ADL2
  `master05-paths` §"Relationship with W3C Xpath"). A `DV_INTERVAL` and its
  parts read `…/value`, `…/value/lower` and `…/value/upper`, the form the
  ADL 1.4 Web Template writes, where they read `…/value[id6]/lower[id7]`
  before, and the parts' ids are `lower` and `upper`.

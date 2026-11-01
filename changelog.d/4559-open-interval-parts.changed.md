- The served Web Template gives every `DV_INTERVAL<T>` node its `lower` and
  `upper` children, each of type `T`, also when the operational template
  leaves them unconstrained (RM `data_types` §DV_INTERVAL). Such a node had no
  children before. An unconstrained part carries the open inputs of its type,
  and its `aqlPath` is `…/value/lower` or `…/value/upper`. This holds for ADL
  1.4 and ADL 2 templates.
- The template example fills such an interval: an unconstrained part takes
  the example value of its constrained sibling, so the limits stay
  consistent.

- An AQL `matches` over a value list now settles on PostgreSQL's generic
  plan. The list is bound as one array read through an initplan, so the
  statement is no longer planned again on every request when the planner's
  statistics favour its custom plans.

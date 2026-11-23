- A commit that changes a party's caller facts (a care relationship, a membership, a role, a
  record subject) stamps only that party with its transaction's id, so commits about different
  patients no longer queue on one row, and it sends no notification. Each replica holds the
  snapshot its copy of the facts was read under, and a request's one read of the party store's
  revisions asks `party.facts_behind` whether a stamp exists that snapshot did not see. In the
  v6 baselines, `party.binding_grant_revision` loses `facts_revision`,
  `party.binding_caller_fact_party` keys each party to `stamped_xid` (`xid8`), and
  `clinical.access_care_state` records the party snapshot the care projection was read under.

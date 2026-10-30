- A party update whose body carries no `uid` now checks each relationship
  `source` against the party the path names and refuses a foreign source with
  `422`, as RM `Relationships_validity` requires. The check ran only when the
  body carried a `uid`, so such an update committed a relationship sourced at
  another party.

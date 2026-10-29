- An AQL projection whose path ends on a list attribute inside a node's data
  (`c/feeder_audit/originating_system_item_ids`, `c/links`,
  `c/context/participations`, `c/composer/identifiers`, `.../mappings`)
  answers the list itself, `[{…}, {…}]`, and `null` when the list is absent.
  It answered the list wrapped in a second list, `[[{…}, {…}]]`; a client that
  unwrapped that outer array reads the column directly now. A path that
  continues below a list (`.../originating_system_item_ids/id`) still answers
  the array of its matches, `["…", "…"]`. `COUNT` over a path ending on a list
  counts its elements.

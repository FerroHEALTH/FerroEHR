- Two concurrent `POST /ehr/{ehr_id}/directory` requests no longer both
  commit when one's check for an existing directory runs before the other
  commits and its write runs after: the new hierarchy takes the rank after
  the highest its check found, so the later create answers `409`.

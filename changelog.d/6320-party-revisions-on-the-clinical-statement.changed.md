- Where the party domain shares the clinical database, a `GET` reads the
  party store's revisions in its own clinical statement, through the new
  `party_door.party_revisions()` function the clinical roles may run, in
  place of a statement of its own on the party pool: a COMPOSITION or EHR read
  is one statement and one transaction. A read decided on a party copy the
  store overtook runs once more and counts in `access_policy_reruns_total`.
  The boot gate refuses a `party_door` schema holding anything but that
  function.

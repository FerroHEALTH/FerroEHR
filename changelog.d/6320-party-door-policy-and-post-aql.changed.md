- Where the party domain shares the clinical database, an AQL query by `POST`
  reads the party store's revisions in its own statement, as a `GET` does, in
  place of a statement of its own on the party pool. A query decided on a
  party copy the store overtook runs once more with the same body and counts
  in `access_policy_reruns_total`. No other `POST` operation rides.
- A party read and a demographic query read the policy and definition
  revisions in their own party statement, through the new
  `clinical_door.policy_revisions()` function the party roles may run, in
  place of a read of the policy revision on the clinical pool. The boot gate
  refuses a `clinical_door` schema holding anything but that function, and an
  erasure drops it with the clinical schema.
- An AQL query whose served versions are not read after it carries the
  policy revisions on the first row it serves alone, in place of every row.

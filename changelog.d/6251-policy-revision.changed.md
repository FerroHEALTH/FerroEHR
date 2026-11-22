- A request confirms its operational-policy register and access-policy copies
  with one primary-key read of the new `policy_revision` row, which every
  register write and access policy version moves in its own transaction, in
  place of reading the newest version of every register and of the policy on
  each request. The newest versions are read only when the revision moved.

- The controller can exclude a named person from one record: an exclusion in
  the `exclusions` of the `EHR_ACCESS` settings refuses that person,
  a representative and a gate-keeper included, before every other step, as
  `403 refused-excluded` with neither the exclusion nor its reason in the
  answer, and the access log names it. Only an administrator reads and writes
  the exclusions: every other reader of an `EHR_ACCESS` gets none, and a
  version a gate-keeper or the person commits carries them forward unchanged.
  A restriction of the person binds a representative it names by party or by
  the `representative` relationship. A professional performing a `ROLE` on
  their own record, or one a `related_to` kinship joins to the person, is
  refused `refused-conflict` in a read and in a query unless a conflict rule
  of the access policy allows it, which the access log marks as a
  `conflict allowance` (#6077).

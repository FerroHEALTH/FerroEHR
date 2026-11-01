- Every write checks the committer against the login that authenticated the
  request where the server can compare them: a committer naming another login
  of the caller's token issuer or of this server, or a `PARTY_SELF` committer
  from a caller who is not the record's person, is refused with a **403**. A
  name only, or an identifier of another authority, is stored as sent. The
  direct writes (composition, EHR status, directory, EHR, demographic) now
  also refuse a committer that breaks its own rules with a **422**, as the
  contribution path already did.

- `ferroehr party-binding check` is the read-only pre-flight of a migration to
  party binding, and the one report of the provisional persons just-in-time
  creation left incomplete. It reads the party versions alone, so it runs
  before any boot fill, and writes nothing. Per key issuer it reports the
  `PERSON`s that carry a binding key, every selector value two `PERSON`s
  carry, the inactive and abandoned ones, the provisional ones older than
  `provisional_report_days`, the key-bearing ones that perform no
  professional `ROLE`, and every mapped Basic user with the `PERSON` it
  binds. For each key it names the committer of the version that introduced
  it, and fails unless that committer is a `[[scim.client]]`, an
  administrator named with `--admin` or `--admin-key`, a migration declared
  with `--migration`, or just-in-time creation on an issuer with `jit = true`.
  `--expected <file>` lists every login of the identity provider with no
  `PERSON`. The report is JSON on stdout, and the command exits non-zero on a
  duplicate, a missing person or a key that fails the provenance rule.

# `scaleseed`: the scale test's bulk seeder (tooling, not part of the app)

One binary that builds a store of millions of compositions in the shape the
commit path writes, checks it, and measures it (#5090). It is never shipped.

- **The commit path writes the shape.** `prototypes` commits a set of content
  variants, and their updates, through `FerroEhrService`. `seed` copies their
  rows with `COPY` under fresh identifiers minted by the server's stamp. It
  never builds a row of its own, so a change to the storage shows up in the
  clones without a change here.
- **The one recomputed value is the signature.** `version.signature` covers
  the identifiers, so it is recomputed over the prototype's served envelope
  with the identifiers swapped, by the crates' canonical form and the digest
  signer. Everything else is a text substitution of UUIDs.
- **`check` is the correctness gate.** It compares a sample of clones with
  their prototypes, row for row and as the service serves them, and the
  service verifies each clone's signature on read. A seed that `check` refuses
  is not measured. The test in `check.rs` proves it refuses a changed node row
  and a broken signature.
- **Resumable in steps.** Each chunk of EHRs is one transaction that records
  itself in `scaleseed.chunk`, so a stopped `seed` resumes. A larger target is
  a second `seed` with the larger numbers. The bookkeeping lives in its own
  `scaleseed` schema, beside the product's.
- **`lookup` times the login lookup (#5087).** It seeds the binding side
  table with bound persons, groups and memberships in the rows the commit path
  derives (keys derived from each ordinal, so a seeded store is asked for keys
  that exist), runs `VACUUM (ANALYZE)`, and times `LocalPartySource::read`,
  the lookup the server makes, against the 2 ms p99 budget. `--seed-only` seeds
  and times nothing; `scripts/perf/party-lookup.sh` uses it on a quickstart
  store. A store already holding the seed is reused, one of another size is
  refused.
- **`groups` weighs a SCIM Group at its cap (#5015).** It provisions the
  Users and builds one Group to `--members` (5 000, `[scim]
  max_group_members`) one member per change, through `provision_user` and
  `update_group`, the SCIM service the endpoint calls, then removes and adds
  members at the cap. A change, one version of the member's `PERSON` since
  the member holds its membership (#5220), is judged against 200 ms at p99 and
  2 MiB stored, the build against 12 GiB of database growth, and a change must
  write one side-table row; a miss fails the run. It refuses a store that
  already holds its Users. `scripts/perf/scim-groups.sh` runs it on a fresh
  quickstart database.
- **Keep each step's database removable.** Seed into a database of its own,
  and drop it when its numbers are recorded.
- **Gates:** `cargo clippy -p scaleseed --all-targets -- -D warnings`,
  `cargo nextest run -p scaleseed`.

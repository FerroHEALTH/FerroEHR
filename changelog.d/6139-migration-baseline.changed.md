- **BREAKING: v6 starts from a new migration baseline and needs a new, empty
  database.** The whole v5 migration history of the `ext`, `clinical`,
  `party` and `audit` schemas (188 files) is replaced by one baseline per
  schema that creates every object once, in its final shape: 33 files, and a
  fresh database is prepared in a fraction of the time. A database prepared
  by v5 is refused at boot with a message that names the schema it found and
  the way forward: start v6 against a new, empty database, dump the records
  with the v5 server's `POST {ext}/admin/dump` and load the archive with
  v6's `POST {ext}/admin/load` (Upgrades, "Coming from a 5.x release"). The
  schema a baseline builds equals the one the v5 history built, except where
  the history left an artefact: the `ferroehr_linkage` role is never created,
  `party.vo_head.access_restricted_at` (which nothing reads) is gone, the
  binding check of `audit.audit_event` is validated, and the NOT NULL
  constraints and identity sequences of tables the history renamed carry the
  table's own name. Once v6.0.0 is tagged the baseline is append-only again.

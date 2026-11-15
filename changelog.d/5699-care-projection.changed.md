- A clinical AQL query under the access evaluator no longer binds the subjects of every patient
  under the caller's care teams. A query pinned to the EHRs it names binds the ones in care, and
  any other query probes a care projection in the clinical database by the caller's team ids.
  The projection holds container UUIDs, care dates and keyed digests (HMAC-SHA-256 under the
  `[privacy.identifier_protection]` key) of subject references, and `ehr` gains the keyed
  digest of its subject (migration `clinical/0047_access_care.sql`). A replica reads it only
  while it is as recent as the care facts it holds, and erasing a `PERSON` removes its rows.
  Without identifier protection, queries bind the census as before.

- The access-log entry of each EHR an AQL query served now names the same
  grounds a read of that EHR alone names: the consent permit or delegation
  that admitted it, with the uid of its `EHR_ACCESS` version, the care
  relationship and the emergency override, beside the caller's roles and
  capabilities. The in-memory settings index the query's access condition is
  built from carries each EHR's subject and `EHR_ACCESS` version uid, so the
  record adds no database read and the condition's SQL is unchanged. No
  configuration key is added or removed.

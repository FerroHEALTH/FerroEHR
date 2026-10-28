- The audit store keeps each record's facts in columns and renders its FHIR R4
  `AuditEvent` from them when the record is read over ITI-81 or delivered by
  the FHIR feed, instead of storing the rendered document in every record. A
  record takes about a quarter of the space it took before. The configuration
  the rendering depends on (the audit source, the store horizon, the category
  map and the emergency purposes) is kept once per distinct configuration in
  `audit.audit_render_policy`, and every record names the policy it was
  written under, so it renders the same document after the configuration
  changes. The tamper chain hashes every column of the record in one digest
  and checks each render policy against its digest; the earlier digest
  versions are retired. Audit migration 0017 needs an empty audit store and
  refuses one that holds records written by an earlier release, naming the
  remedy. ITI-81 parameters on an element of the document (`address`, the
  `.identifier` parameters, `entity-type`, `entity-role`, `type`, `subtype`)
  are matched against the rendered document.

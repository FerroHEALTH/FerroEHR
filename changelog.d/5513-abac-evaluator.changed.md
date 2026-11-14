- A `restriction` or `opt_out` directive with `emergency_override` false is
  accepted and binds in every emergency, where it was refused with `400`. A
  restriction scoped by templates, archetypes, categories, origins or commit
  time covers only that data, where it covered the whole EHR, and its
  `applies_to`, `operations`, `purposes` and `exceptions` narrow it. A
  `restricted` EHR also admits a caller with a live care relationship to the
  subject or a consent permit, and `consent_required` a consent permit.

- The access evaluator decides every request that reaches an EHR by a fixed
  procedure over the person's access directives and labels, the caller's
  clearances, permitted purposes and care teams, and the controller's access
  policy: an opt-out and a restriction read as absent (`404`), and a consent
  deny, a sensitivity, a missing care relationship, a missing or unpermitted
  purpose and the default disposition answer `403` with their `refused-*`
  code in `detail` and the `refusal` problem member. A directive applies by
  its `applies_to`, `operations`, `purposes`, `time_validity`, `data` scopes
  and one level of `exceptions`, and an emergency lifts it only where its
  `emergency_override` holds. AQL applies the same decision as a condition of
  its one SQL statement, so `COUNT`, `LIMIT` and paging count only readable
  rows. No configuration key is added or removed.

- The access record names the grounds of every access decision: the
  access-log entry's `authorisations` carry each capability and role the
  caller held, a care relationship the decision read, each directive that
  admitted the access with the uid of its `EHR_ACCESS` version, the subject's
  own access, and each directive or step an emergency lifted, with the
  `ACCESS_POLICY` version a ground is defined in. Every refusal names its
  `refused-*` code as the entry's `outcome.reason` and the step with the
  directive or policy that decided it, the `404` of an opt-out or a
  restriction included, whose response says nothing. The EHDS `AuditEvent`
  projected from the entry writes the versions after the data permit at
  `agent[user].policy`. No configuration key is added or removed.

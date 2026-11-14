- **Breaking: the embedded Cedar engine is gone, with the whole
  `[authz.abac]` table.** The access evaluator decides every request that
  reaches an EHR, over the person's access directives and the controller's
  access policy, with no switch, engine or policy language. `enabled`,
  `engine`, `organization_claim` and `check_directory`, and the Cedar
  settings `[authz.abac.cedar]`, are unknown keys that refuse the start, from
  the file and from the environment. The patient binding moves to `[auth]`:
  `patient_claim` and `patient_namespace`, with no claim by default, where
  `[authz.abac] patient_claim` defaulted to `patient_id`; with a claim set,
  the patient scope holds every request to the patient it names, DIRECTORY
  operations included. The organisation an access record names is the
  provider of the caller's party binding; no token claim names it. The
  `cedar-policies` admin register and its routes are gone, and clinical
  migration 0041 deletes its stored versions. The shipped example policies,
  the `authz_cedar_decisions` metric and the `ferrohealth-authz` dependency
  are removed. The configuration reference lists 738 keys, down from 742.
  Helm chart 33.0.0 drops `config.authz.abac`: a values file that sets it
  renders a configuration the server refuses.

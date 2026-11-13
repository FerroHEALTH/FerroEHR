- An issuer written with a user name and password in its URL userinfo
  (RFC 3986 §3.2.1) showed that password wherever 5.7.0 renders its
  configuration: `ferroehr config check`, `GET /management/env`, `GET
  /ferroehr/rest/extensions/v1/admin/config`, `ferroehr report`, the binding
  posture and the refusals at start-up (GHSA-gwh2-7hjm-4pj4, CWE-532,
  CWE-200). The keys that held the issuer as a plain string were
  `[[authz.federation.agreement]] issuer`,
  `[[authz.party_binding.issuer]] issuer`, `[[authz.system_client]] issuer`,
  `[[auth.scim.client]] issuer`, `authz.rbac.role_issuers` and
  `auth.smart.patient_issuers`, and, through `ferrohealth-smart` 0.18,
  `[[auth.attributes]]`, `[[auth.federation]]`, `[[auth.party]]` and
  `auth.scopeless_subjects`. Each of these keys that 5.7.1 still reads holds
  the issuer as a `SecretUrl` and renders it as `https://***@host/...`, as
  `[[auth.issuer]]` already did; the others are removed with the move of
  policy into the CDR. Only a deployment that wrote credentials into an issuer
  identifier is affected: remove them from every key that names the issuer
  and rotate the credential.

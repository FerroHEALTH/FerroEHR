- Every FerroEHR key that names an `[[auth.issuer]]` holds the issuer as a
  `SecretUrl`: `[[authz.federation.agreement]] issuer`,
  `[[authz.party_binding.issuer]] issuer`, `[[authz.system_client]] issuer`,
  `[[auth.scim.client]] issuer`, `authz.rbac.role_issuers` and
  `auth.smart.patient_issuers` (GHSA-gwh2-7hjm-4pj4, CWE-532, CWE-200).
  Before this fix, an issuer written with a password in its userinfo showed
  that password in `ferroehr config check`, `GET /management/env`, `GET
  /ferroehr/rest/extensions/v1/admin/config`, `ferroehr report`, the binding
  posture and the refusals at start-up. These now render it as
  `https://***@host/...` (RFC 3986 §3.2.1), as `[[auth.issuer]]` already did.
  No configuration key changes. An operator who wrote credentials into an
  issuer identifier removes them and rotates the credential.

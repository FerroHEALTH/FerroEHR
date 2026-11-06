- The party binding's configuration, each table with its boot refusals:
  `[authz.party_binding]` (`provider` as one table or one table per tenant,
  `namespace`, `basic_user`), `[authz.import] trusted_systems`,
  `[[authz.system_client]]` (the provider, facilities, templates and origins
  of each client-credentials client that commits) and
  `[[authz.federation.agreement]]` (what a federation issuer's external
  persons or organisation may do, on which legal ground), with
  `[authz.rbac] role_issuers` and `import_role`. A template an agreement or
  a system client writes that the store does not hold refuses the start. The
  boot line and `GET /management/info` list the issuers with their kind and
  identifier issuer, the provider, and every agreement with its accessor and
  legal ground, and an agreement that admits an organisation without a named
  person is logged at `warn`.
- `[auth.idp.entra]`, the Microsoft Graph access of the `idp-entra` adapter
  of `ferrohealth-smart`, which resolves the groups of an Entra ID groups
  overage for an issuer whose profile sets `adapter = "idp-entra"`.
- `[auth.dpop]` and `[[auth.issuer]] sender_constraint`: tokens bound to a
  DPoP key (RFC 9449) or a mutual-TLS client certificate (RFC 8705), checked
  on every request. A DPoP proof is recorded by the replica that accepted it,
  so the Helm chart refuses a sender-constrained issuer on more than one
  replica.

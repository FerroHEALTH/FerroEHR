- The issuer of `[[auth.attributes]]`, `[[auth.federation]]`,
  `[[auth.party]]` and `auth.scopeless_subjects` renders with its userinfo
  masked (`https://***@host/...`) in `ferroehr config check`, `GET
  /management/env`, `GET /ferroehr/rest/extensions/v1/admin/config`,
  `ferroehr report` and every refusal that names it, as the
  `[[auth.issuer]]` itself already did, so a password written into one of
  these issuers reaches no rendering (RFC 3986 §3.2.1).

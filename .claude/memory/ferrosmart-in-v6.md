---
name: ferrosmart-in-v6
description: FerroSMART is the family's own SMART App Launch 2.x authorization server, and FerroEHR v6.0.0 depends on it (owner, 2026-11-16)
metadata:
  type: project
---

FerroSMART (FerroHEALTH/FerroSMART, its own OCI image, BUSL-1.1) is the
family's SMART App Launch 2.x authorization server. Keycloak, ZITADEL, Entra
ID and Okta are used only for login behind it. FerroEHR v6.0.0 depends on it.

**Why:** no IdP issues real SMART tokens: ZITADEL drops SMART scopes and
`launch` (#5827), and Keycloak has fixed scopes and no launch context.

**How to apply:** FerroEHR takes SMART tokens from FerroSMART; standards
only and vendor neutral on every interface; policy (role to scope, clients,
branding) is configuration, never code ([[cdr-mechanism-app-policy]]).
Related: [[ferroehr-v6-major]].

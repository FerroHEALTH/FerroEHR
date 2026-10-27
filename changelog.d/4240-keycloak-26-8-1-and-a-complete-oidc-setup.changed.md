- **The development and quickstart Keycloak moves to 26.8.1, and its realms
  form a complete OIDC example**: both compose files pin
  `quay.io/keycloak/keycloak:26.8.1` by digest, held together by
  `scripts/checks/compose-image-tags.sh` and watched weekly by the base-image
  watcher. In both realms the authorization-code flow requires PKCE (`S256`)
  and accepts only the viewer origins on port 3000 as redirect targets (the
  quickstart realm accepted any), and a service account holding `USER` lets a
  client-credentials token reach the clinical API.

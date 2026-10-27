- **The development and quickstart Keycloak moves to 26.8.1, and its realms
  form a complete OIDC example**: both compose files pin
  `quay.io/keycloak/keycloak:26.8.1` by digest, held together by
  `scripts/checks/compose-image-tags.sh` and watched weekly by the base-image
  watcher. The `ferroehr` client of both realms requires PKCE (`S256`) on the
  authorization-code flow, accepts only the viewer origins on port 3000 as
  redirect targets (the quickstart realm accepted any), and its service account
  holds `USER`, so a client-credentials token reaches the clinical API.

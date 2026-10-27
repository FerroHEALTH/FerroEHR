- **Breaking: the quickstart Keycloak realm gives the viewer and a backend
  service their own clients**: `ferroehr-viewer`
  (`ferroehr-viewer-quickstart-secret`) takes the authorization-code flow with
  PKCE and the password grant, and `ferroehr-service`
  (`ferroehr-service-quickstart-secret`) takes only the client-credentials
  grant, with the `system/` SMART on openEHR scopes and a `client_id` claim. Both
  put `ferroehr` in `aud`. The single `ferroehr` client and its
  `ferroehr-quickstart-secret` are gone, so a script that fetched a token with
  them moves to `ferroehr-viewer`. The overlay names `ferroehr-service` in
  `[smart] backend_clients`, so its `system/aql-*` grant runs AQL. The dev realm
  makes the same split and drops the unused Keycloak Authorization Services, and
  `scripts/ui-e2e.sh` keeps its scratch files under `target/ui-e2e/`.

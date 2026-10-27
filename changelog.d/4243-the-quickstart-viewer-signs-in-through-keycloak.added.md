- **The quickstart viewer signs in through the Keycloak overlay**: with
  `--profile viewer`, the viewer at `http://localhost:3000` takes the
  authorization-code flow with PKCE through the demo realm's
  `ferroehr-viewer` client and calls the server with the user's token. The
  issuer moves to `https://keycloak.localhost:8443`, a name the containers
  resolve through a network alias and a browser resolves to the loopback
  address with no hosts entry, and the development certificate names it.

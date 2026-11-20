- The FerroSMART overlay runs FerroSMART 0.2.0, pinned by the digest of its
  release index, and the hospital scenario runs the same image. The upstream
  provider is one `[[ferrosmart.upstream]]` entry with its OpenID Connect keys
  under `[ferrosmart.upstream.oidc]`, the form FerroSMART 0.2.0 reads: the
  Keycloak overlay's overrides are now `FERROSMART__FERROSMART__UPSTREAM__0__OIDC__*`
  and the ZITADEL bootstrap writes the new tables (#6100).

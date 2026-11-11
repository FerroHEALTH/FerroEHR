- The viewer moves to `ferrohealth-session` 0.13.0, which types the OpenID
  Provider `issuer`, `redirect_uri` and `post_logout_redirect_uri` of
  `[oidc]` as secret URLs, so a password written into one of them renders
  masked in the viewer's `config check`, its refusals and its configuration
  page.

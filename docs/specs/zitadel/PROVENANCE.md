# Provenance: ZITADEL v4.20.5 documentation and sources (excerpt)

- Source: <https://github.com/zitadel/zitadel>, the documentation under
  `apps/docs/content/`, the setup steps `cmd/setup/steps.yaml`, the official
  compose file `deploy/compose/docker-compose.yml` and five files of the
  OIDC server under `internal/api/oidc/`; and
  <https://github.com/zitadel/oidc>, the OIDC library ZITADEL's `go.mod`
  pins at `v3.47.5`, under `zitadel-oidc/`
- Version: ZITADEL v4.20.5, the newest stable release when retrieved; tag
  `v4.20.5` resolves to commit `59827a8ec6dd0c23acbba5d0ebdf41fdc857d66c` (2026-11-18T19:17:42Z). zitadel/oidc
  tag `v3.47.5` resolves to commit `fb9fbfe7d0d7c74e2e54ecc7356106dbd741dd7c`. The script
  fetches both commits, each file checked against the sha256 below
- Retrieved: 2026-11-24
- Upstream licences: the repository is AGPL-3.0-only (`LICENSE`, vendored).
  Its `LICENSING.md`, also vendored, reads: "The following files and
  directories, including their subdirectories, are licensed under the Apache
  License 2.0: proto/ apps/docs/", and names `apps/login/` as MIT. So the
  pages under `apps/docs/content/` are Apache-2.0, and `cmd/setup/steps.yaml`,
  `deploy/compose/docker-compose.yml` and the `internal/api/oidc/` files
  are AGPL-3.0-only, copied verbatim for citation and never built, linked or
  modified. zitadel/oidc is Apache-2.0, its `LICENSE` vendored under
  `zitadel-oidc/`. Read 2026-11-24.
- Images run, never vendored: `ghcr.io/zitadel/zitadel` (the server,
  AGPL-3.0-only) and `ghcr.io/zitadel/zitadel-login` (the Login v2 UI from
  `apps/login/`, MIT), both pinned by digest in `docs/VERSIONS.md` and run
  unmodified by the example
- Layout: the upstream paths, unchanged; zitadel/oidc under
  `zitadel-oidc/`. Text only: the figures under `apps/docs/public/` are
  not taken; fetch them from the repository at the pinned commit
- Fetched by: `scripts/vendor/zitadel-docs.sh`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `LICENSE` | `4df3c306dddaaf4baffdff5ca820cc679ac8cd6dc263c6a74517783e42fa7a3b` |
| `LICENSING.md` | `97a02685ee62bfd02b2e696580189807a193b81fcc37909818f9d19cef45394e` |
| `apps/docs/content/apis/openidoauth/claims.mdx` | `136f756c685021bff0a835cad7a0be151453176cc8f7ba4bcbaa8bcd94c1ccc2` |
| `apps/docs/content/apis/openidoauth/scopes.mdx` | `43d3649accd295538cf3169095c074de8398a47a8fb03421678ea8042fa707e2` |
| `apps/docs/content/apis/openidoauth/grant-types.mdx` | `0455350d47cc9989f874f4b64b9b359712b61503f77441801398fc8fcacd8cb9` |
| `apps/docs/content/apis/openidoauth/endpoints.mdx` | `da481abbf7d11d7fc3409baad0393a985f3bf5732e4062173f57af9b177a8323` |
| `apps/docs/content/apis/scim2.mdx` | `06e233f54bad6400bd52d003c14c8a983d7c41dd7ce41169744e5c3542d26273` |
| `apps/docs/content/apis/actions/complement-token.mdx` | `46850b32d4a11e54defb8fe35c414b60260a0eaaabade375ed1a7245fe0f5045` |
| `apps/docs/content/concepts/knowledge/opaque-tokens.mdx` | `82412a170f567962e72c75be4b0ec30e95b6bcd6f98a6a4db97ea521ba44ede5` |
| `apps/docs/content/concepts/features/actions_v2.mdx` | `ad1d175f7ed745b6d62c9ea2e5973f9e5b13afb21588cec28d5323b2b1aefa7e` |
| `apps/docs/content/guides/integrate/actions/usage.mdx` | `3de63ecba6aecd977d822c3f7b6aca3bf2cbb0b9d03dda509fe94078ba2b62c5` |
| `apps/docs/content/guides/integrate/actions/migrate-from-v1.mdx` | `aabc4f37ab766858b28ca196c51067e7330be56277a66d71749d6fb430150571` |
| `apps/docs/content/guides/integrate/retrieve-user-roles.mdx` | `ec7a5edc155bc14403700f791b5ea0482281ae6750f9265ec380bdbb3e5dd3a7` |
| `apps/docs/content/guides/integrate/service-accounts/authenticate-service-accounts.mdx` | `7fe2c4c4c7c415c46bcdf224e47b9622d1dd6ccfcd002391cfe1174953ae19da` |
| `apps/docs/content/guides/integrate/service-accounts/client-credentials.mdx` | `264b9fcf7025f58ce20ee9df1074d1b9374f4f333db66a0eb3f9fb6e86931fe9` |
| `apps/docs/content/guides/integrate/service-accounts/private-key-jwt.mdx` | `19e75f60128e481107d4508da28088a05e6d9938428d611ecb99825397892ef4` |
| `apps/docs/content/guides/integrate/service-accounts/personal-access-token.mdx` | `d82f343be5e192b454c25f6a39625e4c091bf55f8ab148137c8fdfe3b33beabf` |
| `apps/docs/content/guides/integrate/token-exchange.mdx` | `95e78bcb5b624bd6dabdbce8ce7d217d922d37491f36b585f9e307d3b6916474` |
| `apps/docs/content/guides/integrate/token-introspection/index.mdx` | `8d4b41e53c6fdbc3ba924fbf3fae4baabeed192d24141978cbf4bfb3972bc8e5` |
| `apps/docs/content/guides/integrate/token-introspection/basic-auth.mdx` | `4f1f0b40f24e70766859abcd223892ecb0f7fe9d53d8cc91da92ab2e44b2cefc` |
| `apps/docs/content/guides/integrate/token-introspection/private-key-jwt.mdx` | `926304ae1b9c11b8d8cce9b2cc5808ca9513fda84e9673b50ba007ed03b1de65` |
| `apps/docs/content/guides/integrate/login-ui/oidc-standard.mdx` | `6fb12712152b8d5f4bd7da5eec4d113e2e567f3cdd0e928d3f6b93fb7f45a8af` |
| `apps/docs/content/guides/integrate/login-ui/username-password.mdx` | `c315e0dbd896ef5273c4b9ba339976eaaac8070fd9178c5c511b4efb427d811e` |
| `apps/docs/content/guides/integrate/login-ui/login-app.mdx` | `6d42499886454351c2738b9851b0c45be24971c5a13f98f79bdccd38d83fd18b` |
| `apps/docs/content/guides/manage/terraform-provider.mdx` | `db504f9cbced179fafd8e674f89cf4d7d5a0d67f35b1f3f89628b7fdb00f67f6` |
| `apps/docs/content/self-hosting/deploy/compose.mdx` | `0066faeb74227850a2a281e4bf055fc6679c425b90145eb34ef3ffd393b0a915` |
| `apps/docs/content/self-hosting/manage/configure/configure.mdx` | `522056fc0fb4046e1c908b2328d42f1b17b1d7d737287dc9da51033cda31e467` |
| `apps/docs/content/self-hosting/manage/configure/example-zitadel-init-steps.yaml` | `308162892fd1754990022b73876c91691ee4516463abd91e89be89aa583ed1a4` |
| `apps/docs/content/self-hosting/manage/database/index.mdx` | `50d304cc16a259ad044d39d2f88c32f3a5ee31a8d407ed039bb3fc13dbf77f94` |
| `apps/docs/content/self-hosting/manage/database/_postgres.mdx` | `90acc1647f0a2b90c965de3ffe21ef3322d5787479e5806a9474edb89d4cb8de` |
| `apps/docs/content/self-hosting/manage/login-client.mdx` | `4e3d691dd3a1948a25e6882fb93463663578be8c1be5079c8961c5f6c5208423` |
| `apps/docs/content/self-hosting/manage/custom-domain.mdx` | `87ad8f42551f482ebbeb870c4836d7d1dbabfb3e9105e2c0b19b8f96a3265731` |
| `apps/docs/content/self-hosting/manage/tls_modes.mdx` | `6f71c71d7c8c8e812478f0207442df5a24303cdd703fec6afc120f0bfda12034` |
| `apps/docs/content/self-hosting/manage/productionchecklist.mdx` | `8e1143de122e88e7f19c76ae339aff9b5c9b76167032d9f7233d01d72c8a1431` |
| `apps/docs/content/self-hosting/manage/reverseproxy/reverse_proxy.mdx` | `829977e45208b86d4e18608ae14a2a798b7a440357c47a25851ac937600ee523` |
| `apps/docs/content/self-hosting/manage/reverseproxy/_proxy_guide_tls_mode.mdx` | `c73bf17bc13d9469067ed8a05f8699e12b671b0ba3b132fb43840982ecd49659` |
| `apps/docs/content/self-hosting/manage/reverseproxy/nginx/index.mdx` | `ef12ffec9e8a35db53bc1dcc93cb2821995499a6d8056deffeed84ac054779e5` |
| `apps/docs/content/self-hosting/manage/reverseproxy/nginx/nginx-enabled-tls.conf` | `44740a8c86d4a23e040609e5bb366aa29fbd89c2d6f9432ff1bba6fc6564c057` |
| `apps/docs/content/self-hosting/manage/reverseproxy/nginx/docker-compose.yaml` | `a6d7691d0b1a0311fff8723cea2dd6a06b905867941d2e9e51721cf07ca8f273` |
| `cmd/setup/steps.yaml` | `0fab79eb9a50251813f8404fc274194833d5768d92f3c82d91a7bb44b9744eb2` |
| `deploy/compose/docker-compose.yml` | `38221d76e2e86b01fa4b692b87ac2ecce53efe0a33836e69b9d34a120068e12c` |
| `internal/api/oidc/client_converter.go` | `5d616e2209d81f1d8b23494f98a90c23cd25c46ba1da068584c2674e368ab075` |
| `internal/api/oidc/token.go` | `0632199e2bff9968ffc93db06663fc3ff2611fd0ecdc4714e384e8faba56d4c5` |
| `internal/api/oidc/token_client_credentials.go` | `684ac5914fede959eedf5af758dab21564f6adf0a06d601efa025c1f0aa563de` |
| `internal/api/oidc/userinfo.go` | `bed6939b8596e1c8a43ec77f4547982ad5a04bac83e8ae2d428a30ea367a7a83` |
| `internal/api/oidc/sign/signer.go` | `86f97502616c64dfcd2e0e5da65d049def3e415e14ebdf09eaf17141adab3020` |
| `zitadel-oidc/LICENSE` | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` |
| `zitadel-oidc/pkg/oidc/token.go` | `b9a32f4960dbf8d6b001ad303a05f24d44c966d573d1a9891f87a461763b881d` |
| `zitadel-oidc/pkg/op/auth_request.go` | `1c657236f7d28bd73b7a144533c8066bfa6bc61b232a35c2bc91a3636f360f9a` |

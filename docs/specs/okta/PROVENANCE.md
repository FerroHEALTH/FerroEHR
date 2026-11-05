# Provenance: the Okta OpenAPI documents and the OAuth 2.0 API overview

- Source: <https://github.com/okta/okta-management-openapi-spec>, the OpenAPI documents Okta builds
  its management SDKs from, one directory per Okta release under `dist/`
- Pin: commit `ec20a8e9f2f204912d9b0f03a866ea5d181e69d5` (2026-11-05T05:34:03Z), the default branch's head when
  retrieved, release `2026.44.0`, the newest under `dist/` (identical to
  `dist/current/` at the commit); each file checked against the sha256 below
- Retrieved: 2026-11-05
- Documents: `dist/2026.44.0/oauth-minimal.yaml` ("Okta OpenID Connect &
  OAuth 2.0", the authorization server endpoints, their parameters and the
  token responses) and `dist/2026.44.0/management-minimal.yaml` ("Okta
  Admin Management API", the User and Application objects), each
  `info.version` 2026.44.0. The other variants under `dist/2026.44.0/`
  (the `noEnums`, `oneOfInheritance` and `noExamples` renderings of the same
  APIs, and the Governance, MyAccount, Access Gateway and Privileged Access
  documents) serve no reader here and are not taken.
- Upstream licence: the repository has no licence file and GitHub reports no
  licence for it. Three statements name Apache-2.0, read 2026-11-05: each
  document's `info.license` ("name: Apache-2.0", "url:
  https://www.apache.org/licenses/LICENSE-2.0.html"), the licence badge of
  the `README.md` ("License-Apache 2.0"), and the `license` field of the
  npm package `@okta/openapi` it names ("Apache-2.0"). Apache-2.0 permits
  redistribution with the licence text, which `LICENSES/Apache-2.0.txt`
  carries; the repository has no NOTICE file. The `README.md` is vendored
  as the record of the badge, and `REUSE.toml` declares the tree
  `Apache-2.0`. Each document's `termsOfService` points at
  <https://developer.okta.com/terms/>, which governs use of the Okta service.
- Layout: the upstream paths, unchanged
- Fetched by: `scripts/vendor/okta.sh`; never hand-edit, re-run the script
  to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `README.md` | `84b5118d547c6d3e88feb9a338630aa94dfd206d33f8d32e8a9f1994927ba8ab` |
| `dist/2026.44.0/oauth-minimal.yaml` | `36f06d3adcba39bd36f9bfd3a04f137b0c8980d6116707616281a936101b0a49` |
| `dist/2026.44.0/management-minimal.yaml` | `0e1ccf7a1c12f4e77e2e5cebf2d625f8869f7603746360a54e0360673644049f` |

## Cache only

`okta-oauth-overview.html` is the "OpenID Connect & OAuth 2.0" API
overview at <https://developer.okta.com/docs/api/openapi/okta-oauth/guides/overview/>, as served 2026-11-05, the page that lists the
reserved claims of an Okta access token: `uid` ("A unique identifier for
the user") and `cid` ("Client ID of the client that requested the access
token"). Its footer states "Copyright © 2026 Okta. All rights reserved.",
and <https://developer.okta.com/copyright/> grants no licence to copy it, so
the script fetches it into the git-ignored `.vendor-cache/okta/` and this
repository carries none of its content. The page is rebuilt without notice:
FerroHEALTH/FerroAUTH recorded another sha256 for it earlier the same day, so
a refused re-run means the page moved and the pin is moved after reading the
change.

| File | sha256 | Status |
|---|---|---|
| `okta-oauth-overview.html` | `c278d86c510d60d27ffe073a068d6ac8763358749a6017f449e9bc21f0d39ea9` | cache only |

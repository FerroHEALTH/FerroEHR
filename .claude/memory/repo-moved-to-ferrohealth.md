---
name: repo-moved-to-ferrohealth
description: 2026-10-01 the repo moved to FerroHEALTH/FerroEHR; images ghcr.io/ferrohealth (lowercase), board orgs/FerroHEALTH/projects/2; releases ≤ v4.3.1 stay signed as rubentalstra/FerroEHR
metadata:
  type: project
---

The repository was transferred from `rubentalstra/FerroEHR` to the `FerroHEALTH` organization on 2026-10-01 (#3516). The roadmap board was copied to <https://github.com/orgs/FerroHEALTH/projects/2> (Projects cannot change owner); the old user board #4 is retired.

- Image and chart refs are `ghcr.io/ferrohealth/...`, spelled as a literal: `github.repository_owner` is `FerroHEALTH`, and OCI refs must be lowercase.
- Org packages are listed under `/orgs/FerroHEALTH/packages`, not `/users/`.
- Still under the user account (do not rewrite): FerroTERM + `ghcr.io/rubentalstra/ferroterm`, Veredictum, FerroBRIDGE, `hetzner-deploy-action`, the Sonar org/key `rubentalstra_FerroEHR`, `urn:rubentalstra:ferroehr`.
- Releases up to v4.3.1 were signed as `rubentalstra/FerroEHR`; v4.3.2 (2026-10-01) is the first built under FerroHEALTH, and `website/book/src/verifying-releases.md` carries a note on verifying the older ones.

**Why:** owner moved the product line under one organization ([[sibling-products]]).
**How to apply:** verify a pre-move release with `-R rubentalstra/FerroEHR`; cosign signatures of v4.3.1 exist only at the `ghcr.io/rubentalstra` path.

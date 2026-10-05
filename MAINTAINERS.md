# Maintainers and access continuity

This file is the roster and the honest answer to the question an enterprise
procurement review asks about any software that holds patient data: *what
happens if the people who can ship a fix are unavailable?*

It is deliberately not aspirational. Everything below describes the project as
it is on the day you read it in git history, not a structure the project hopes
to grow into.

## Roster

| Person        | GitHub                                           | Role              | Since      |
|---------------|--------------------------------------------------|-------------------|------------|
| Ruben Talstra | [@rubentalstra](https://github.com/rubentalstra) | Maintainer (sole) | 2026-07-01 |

**The bus factor of this project is one.** There is exactly one person with
write access to the repository (`GET /repos/FerroHEALTH/FerroEHR/collaborators`
returns one login), one person who can publish a release, and one person who
can accept a pull request. No second maintainer exists.

[Cadasto B.V.](https://www.cadasto.com) is the Licensor and copyright holder,
and it handles the business side of the project, the commercial licence
included: write to [info@cadasto.com](mailto:info@cadasto.com) or use
<https://www.cadasto.com/contact/>. The maintainer handles the technical side:
code, review, releases, issues and security reports. Nothing in this file
assigns Cadasto B.V. a publishing identity, repository access or a duty to
continue the work.

Everything else in this file follows from the bus factor of one, and no wording
elsewhere in the repository should be read as softening it. The path out is in
[GOVERNANCE.md](GOVERNANCE.md): becoming a maintainer is a defined route,
and it is open.

## Publishing identities and where they live

These are the credentials and configured identities that can put bytes in front
of a user. Naming them is the point: an inventory nobody has written down is an
inventory nobody can hand over.

| Identity                                     | What it publishes                                             | Held by                                                                                                                                                                | Recovery if the holder is unavailable                                                                                                                                                         |
|----------------------------------------------|---------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| The GitHub organization `FerroHEALTH`        | everything: the repository, releases, issues, settings, and the GHCR images and chart from v4.3.2 on | the organization's owners, the maintainer among them | another owner of the organization can act; the repository is no longer tied to a single account |
| The GitHub account `rubentalstra`            | the GHCR images and charts published up to v4.3.1, and the `hetzner-deploy-action` the sandbox lane runs | the maintainer | none: those packages are user-owned, so GitHub's account-recovery process is the only route |
| The OpenPGP commit- and tag-signing key      | the verified signature on every commit and every release tag  | the maintainer, on his own hardware                                                                                                                                    | none: the private key is not escrowed. A successor would publish a new key and re-establish trust from a signed statement on the repository; historical signatures stay verifiable regardless |
| crates.io Trusted Publisher (OIDC)           | the nine `openehr-*` crates                                   | configured per crate on crates.io against this repository, the `crates-io` environment, and both publishing workflows (`release.yml` and `publish-crates.yml` — Trusted Publishing matches the workflow filename) — **no long-lived token exists anywhere** | the crates.io owner list is the recovery surface, and it is the maintainer's account. A successor with repository write access can run the lane, but only after crates.io ownership moves     |
| `GITHUB_TOKEN` (ephemeral, per workflow run) | the GHCR container images, the Helm chart, the GitHub release | GitHub, minted per run; nothing is stored                                                                                                                              | not applicable — there is no credential to lose                                                                                                                                               |
| The `FOSSA_API_KEY` repository secret        | nothing; it uploads a licence analysis                        | the repository                                                                                                                                                         | not applicable — a lost key costs a scan, not a release                                                                                                                                       |
| Zenodo                                       | the archived release deposit and its DOI                      | the Zenodo account linked to the GitHub account                                                                                                                        | tied to GitHub account recovery                                                                                                                                                               |
| The `ferroehr.eu` domain and GitHub Pages    | the documentation site                                        | the maintainer (registrar account)                                                                                                                                     | none: registrar account recovery only                                                                                                                                                         |

**The honest reading of that table:** with a single exception, every
publishing identity terminates at one person's GitHub account or one person's
hardware. Trusted Publishing removes the *stored secret* risk (there is no
crates.io token to leak), but it does not distribute the *authority*, which
is still one account's. That is the residual risk, and it is stated rather than
mitigated because no mitigation is currently available while one person holds
every technical role.

## If the maintainer is unavailable

There is no succession plan that a document can create. What exists instead:

- **Nothing already published disappears.** Releases are immutable and their
  assets stay downloadable; published crates cannot be unpublished (only
  yanked, which needs the owner anyway); published chart versions are never
  overwritten by policy and the lane refuses to; the Zenodo DOI is permanent.
  A deployment already running is not affected by maintainer availability.
- **Nothing new ships.** No release, no security fix, no chart version, no
  crate publish. The support window in [SECURITY.md](SECURITY.md) (only the
  newest release is supported) becomes, in that situation, no supported
  release at all.
- **The work is not lost.** The source is available under the Business Source
  License 1.1 and every version becomes Apache 2.0 four years after its
  release, the history is public, every gate is a committed script and every
  design decision is in the tree or on the tracker. A fork is a legitimate
  continuation within those terms, and the project's position is that it
  should be taken rather than waited on.
- **A vulnerability report has a fallback.** If a private report receives no
  acknowledgement within the window [SECURITY.md](SECURITY.md) commits to, the
  policy already tells you to escalate publicly, and publishing becomes your
  call. That path does not depend on the maintainer.

If you depend on this software in a clinical setting and that position is
not acceptable to you (it reasonably may not be), the mitigation is on your side
of the boundary: pin a version, keep a fork you can build, and budget for
maintaining it. That is a truthful answer, and it is more useful than a
continuity plan with nobody behind it.

## Adding a maintainer

The route is in [GOVERNANCE.md](GOVERNANCE.md). When someone takes it, this
file gains a row, [`.github/CODEOWNERS`](.github/CODEOWNERS) gains their handle
on the areas they own, and the table above gains a second holder wherever the
identity permits one. Those three edits are the whole mechanism.

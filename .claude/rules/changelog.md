---
paths: ["CHANGELOG.md", "Cargo.toml", "CITATION.cff", ".zenodo.json", "docker-compose.yml", "deploy/helm/**", ".github/workflows/release*.yml", ".github/workflows/*chart*.yml", ".github/workflows/publish-crates.yml", "scripts/release/**"]
---

# Changelog discipline (Keep a Changelog 1.1.0)

`CHANGELOG.md` at the repo root is the hand-curated release record
(https://keepachangelog.com/en/1.1.0/ + SemVer). It is enforced, not
optional:

- **Every PR that changes user-visible behaviour adds an entry under
  `## [Unreleased]` in the same PR.** User-visible = the REST surface, AQL
  semantics, validation behaviour, storage/migrations, configuration, CLI,
  container/compose/Helm artifacts, feature flags. The CI `changelog-guard`
  job fails the PR otherwise (escape hatch: the `no-changelog` label, only
  for genuinely invisible changes — internal refactors, CI, docs).
- Entries go under the standard subsections: `### Added` / `### Changed` /
  `### Deprecated` / `### Removed` / `### Fixed` / `### Security`. Write for
  the end user (what changed for them), not commit-message prose.
- **Releases are milestone-driven (owner 2026-07-20; milestones =
  releases):** the `vX.Y.Z` GitHub milestone collects the release's issues;
  the release is cut when the milestone reaches **zero open issues** (or
  the owner calls the cut and moves the stragglers to the next milestone).
- **Cutting a release** (on a `release/vX.Y.Z` branch): rename
  `[Unreleased]` → `## [X.Y.Z] - YYYY-MM-DD`, re-add an empty
  `[Unreleased]`, update the link references at the bottom, bump the
  workspace `version` in the root `Cargo.toml` (+ `Cargo.lock` via a
  `cargo check`, + `CITATION.cff` `version` and
  `date-released` (the `citation-guard` CI job enforces the version match),
  then re-render `.zenodo.json` (`bash scripts/render/zenodo-json.sh`, checked
  by the same job), + the ferroehr party statement's product version
  (`docs/conformance/party/ferroehr/statement.json`, regenerating the derived
  documents via `bash scripts/render/conformance-docs.sh` — the
  `statement-version.sh` step of `citation-guard` enforces the match) — it is GENERATED, never hand-edited, because Zenodo ignores
  `CITATION.cff` completely whenever a `.zenodo.json` exists
  (<https://help.zenodo.org/docs/github/describe-software/zenodo-json/>),
  + the default image tags in `docker-compose.yml` — the `ghcr.io/…:X.Y.Z`
  fallbacks the standalone quickstart pulls — guarded by
  `scripts/checks/compose-image-tags.sh` / the `compose-version-guard` CI
  job (the hosted sandbox carries NO release-cut step: its compose defaults
  track the `:latest` release pointer and the release pipeline's `sandbox` +
  `sandbox-reseed` legs redeploy + reseed the Hetzner box automatically after
  the tag's images apply — `deploy/hosted/README.md`, #2974), + the chart's `appVersion` and
  `artifacthub.io/images` tags (#2890, the same sweep — details in the Helm
  chart bullet below), + the book's pinned versions — `website/book/src/installation/
  kubernetes.md` pins the chart `--version` and `image.tag`, and
  `website/book/src/verifying-releases.md` pins the release tag and the image
  tags in its examples; `docs-claims` catches all of them, the chart/image pins
  since the v4.0.0 cut shipped them stale and the verifying-releases asset names
  and substitution note since v4.0.5 froze two stale attestation examples into
  `/docs/v4.0.5/` (#2779)), merge the release PR, then tag
  `vX.Y.Z` on the merge commit. The release workflow publishes the GitHub
  Release from the matching changelog section and **fails if the section or
  version match is missing**. Releases publish as OFFICIAL
  releases — `prerelease` is true only for an explicitly suffixed tag
  (`vX.Y.Z-rc1`, ...) — owner sign-off 2026-07-31.
- **The Helm chart, in the SAME release PR:** set `appVersion` and the
  `artifacthub.io/images` tags to X.Y.Z (the sweep step below), and bump the
  chart's own `version` — EVERY release, not only on chart diffs: every
  release ships a DISTINCT packaged chart (the changes annotation is injected
  at package time), and an unbumped version collides with refuse-overwrite at
  the tag (#2818 — the pipeline's
  `plan` refuses that tag before anything builds). Then
  regenerate whatever that change moved — the golden renders
  (`deploy/helm/validate.sh --update`) and the generated chart README
  (`helm-docs --chart-search-root deploy/helm/ferroehr --template-files
  README.md.gotmpl`). The chart version and `appVersion` are INDEPENDENT SemVer
  lines and stay so.

  **`appVersion` is an ordinary cut step again (#2890, the compose
  treatment):** the same release-PR sweep that bumps the docker-compose.yml
  image tags sets the chart's `appVersion` and the `artifacthub.io/images`
  tags to X.Y.Z, then regenerates the chart README. The committed value
  equals the workspace version at all times —
  `scripts/checks/chart-appversion.sh` enforces the equality (run by the
  `chart-appversion` CI job and again by the release pipeline's `plan` at the
  tagged commit, where plan's own tag-equals-workspace check makes it
  transitively `appVersion == ${TAG#v}`). The publish lane's package-time
  injection (`helm package --app-version ${TAG#v}` plus
  `deploy/helm/release-facts.sh`, #2779) stays as belt-and-braces and is what
  keeps the between-releases `publish-chart.yml` dispatch correct;
  `artifacthub.io/changes` remains inject-only (#2107). The accepted cost is
  the window docker-compose.yml has always accepted: between the release
  merge and the tag's Containers leg, the committed tags reference images not
  yet published.
- **The chart publishes as the release pipeline's `chart` leg** (`build-chart.yml`,
  called by `release.yml` after the scanned tags apply; `publish-chart.yml` is
  the dispatch-only dry-run/recovery lane between releases): it refuses to
  overwrite a published chart version, checks that the `appVersion` image
  accepts the chart's rendered defaults, injects that release's
  `artifacthub.io/changes` from this changelog, pushes to
  `oci://ghcr.io/ferrohealth/charts`, attests it through Sigstore, and
  re-pushes the Artifact Hub ownership tag. A chart-only fix between releases
  uses the same lane by `workflow_dispatch` with `publish: true`. **Never
  re-publish a chart version — bump it:** an OCI tag is MUTABLE, so `helm push`
  would silently replace it with a different digest, and the immutability the
  guards assume exists only because the lane enforces it.
- **A published release is IMMUTABLE.** GitHub release immutability is enabled:
  once a release is published, its assets and its tag cannot be modified. The
  release lane therefore creates the release as a **draft**, attaches the
  binaries, SBOMs and Sigstore bundles from the per-arch matrix, checks the
  expected asset set is complete, and only then publishes it — the freeze
  happens last, on purpose. The recovery path for a bad cut is **a new patch
  version**, never a retag.

  Do not confuse this with the chart lane one section above: a **GHCR OCI tag
  is still mutable**, which is exactly why `build-chart.yml` refuses to
  overwrite a published chart version. GitHub releases are immutable by the
  platform; container tags are immutable only because our lane enforces it.

- **The tag DOES produce an archived artifact — Zenodo is connected**
  (measured 2026-08-15: the v3.17.6 release archived as version DOI
  10.5281/zenodo.21940280 under concept DOI 10.5281/zenodo.21940279, the
  README badge's target). Two properties govern every cut: the deposit's
  FILES freeze at publication (record METADATA stays owner-editable in the
  Zenodo UI, files never), so `.zenodo.json` is verified before the tag and
  never after; and the metadata file only counts if Zenodo actually reads
  it — the v3.17.6 deposit IGNORED the then-committed InvenioRDM-record-shape
  file and archived GitHub's raw repo metadata (the full contributor list as
  creators), so `.zenodo.json` is rendered in the FLAT LEGACY DEPOSIT shape
  the help page documents
  (<https://help.zenodo.org/docs/github/describe-software/zenodo-json/>),
  generated from `CITATION.cff` by `scripts/render/zenodo-json.sh` and
  drift-checked by the `citation-guard` job. Whether a given deposit honoured
  the file is confirmed on the published record (its creators/title must be
  ours, not GitHub's), not assumed. The v3.17.6 record's own wrong metadata
  is only correctable by the owner in the Zenodo UI.
- **After the tag:** close the `vX.Y.Z` milestone (`gh api … milestones/N
  -f state=closed`) and make sure the NEXT milestone exists so triage always
  has a target. The milestone's closed-issue list + the changelog section
  together are the release's record. Then post a **status update on the
  public roadmap board** (`scripts/gh/project.sh update on-track "…"` —
  policy + honesty rules in `.claude/rules/project-board.md`): what the
  release shipped, what the next milestone targets, `--target` only from a
  real milestone due date. Give the next milestone its due date, then
  `scripts/gh/project.sh sync-dates` so the board's Roadmap timeline
  follows the re-milestoned issues.
- **Versioning split:** the product (workspace, `ferroehr-*`, tools, codegen
  tooling) follows the product SemVer (3.x line). The `openehr-*` **spec
  crates** publish to crates.io on their own independent SemVer line —
  permanently decoupled from the vendored spec versions (owner rulings
  2026-08-04, issue #1886 + same-day correction; the implemented spec pins
  live per generation — the emitted `Generation` enum (alone) in the
  generated crates, a literal crate-level `SPEC_VERSION` in the
  hand-written ones — full policy in
  `docs/VERSIONS.md` §Product and crate versioning); they release in
  lockstep and never with the product version.

- **Publishing the `openehr-*` crates (crates.io):** the crates ride the
  release cut. `release.yml` carries a `crates` leg (`needs` `plan` and the
  published GitHub release, real releases only, a leaf so a crates failure
  blocks nothing) that runs in the `crates-io` environment; that environment
  carries a **required reviewer**, so the pipeline PAUSES there and nothing
  reaches crates.io without an explicit approval. The leg is a green no-op on
  a cycle that changed no packaged crate content. Owner ruling 2026-08-27,
  issue #2836: the publish used to be a dispatch someone had to remember, and
  v4.0.6 shipped with the eight crates stepped to `0.0.42` and unpublished.
  `.github/workflows/publish-crates.yml` remains as the between-releases lane
  — the dry run (its default) and the recovery path when a release's leg fails
  — the same split `publish-chart.yml` has against `build-chart.yml`.
  **Trusted Publishing matches the top-level workflow FILENAME**, so each of
  the nine crates carries TWO publisher entries — one naming `release.yml`,
  one naming `publish-crates.yml` — both under repository
  `FerroHEALTH/FerroEHR` and environment `crates-io`; a missing entry is
  refused at the token exchange, and the release run's summary names that as
  the cause. Configuring the reviewer and the second entry set are owner
  clicks (crates.io, and Settings → Environments); until the reviewer exists
  the environment does not pause.

  Both lanes authenticate via **crates.io Trusted Publishing** (OIDC,
  `rust-lang/crates-io-auth-action`, `id-token: write`, the `crates-io`
  environment) and publish the nine `crates/*` members **one at a time in
  dependency order**, treating "already exists on crates.io index" as done —
  no long-lived crates.io token exists anywhere. The upload and its
  registry read-back are ONE implementation,
  `scripts/release/publish-crates.sh`, called from both; they cannot share a
  reusable WORKFLOW, because the OIDC `workflow_ref` claim names the calling
  workflow, so a shared one would hide which file each publisher entry must
  name while changing nothing about the identity. It is deliberately NOT
  `cargo publish --workspace`: that command is all-or-nothing at the START
  (it refuses the entire run if any member version already exists) while
  being non-atomic at the END, so a partial publish cannot be finished by
  re-running it. That combination stranded `openehr-its` and `openehr-adl` at
  `0.0.10` while six siblings reached `0.0.15` (issue #2211), and a split set
  is a broken graph for every consumer, not a cosmetic lag. The lane also
  reads the registry back before reporting success, so a publish that
  finished half the set fails instead of going green. Version bumps happen in the CONTENT PR, not at publish
  time: any PR changing packaged crate content bumps every crate's `version`
  and the internal `version =` requirements together (lockstep `0.0.x` —
  the `crate-version-guard` CI job and the local push hook enforce it; full
  rule in `.claude/rules/crates-publishing.md`), so a publish just ships the
  version already in the tree — verify locally with
  `cargo publish --workspace --dry-run` (the whole-workspace form is still
  the right DRY RUN: nothing is uploaded, so its all-or-nothing behaviour
  costs nothing and it checks every member together). The very first release
  of a crate cannot use OIDC (crates.io requires an existing crate to
  configure a Trusted Publisher) — it is pushed manually with a scoped API
  token, after which the crate's two Trusted Publisher entries are configured
  on crates.io as described above.

# Releases

A FerroEHR release is cut from a `release/vX.Y.Z` branch and published by
pushing the `vX.Y.Z` tag, which runs `.github/workflows/release.yml`. The cut
procedure (the changelog section, the version bumps, the chart, the per-release
reviews) is in `.claude/rules/changelog.md`; the post-market steps that follow a
release are in `post-market.md`. This page covers the check you run before the
tag.

## Preflight: the plan job, before the tag

The `plan` job verifies every release fact at the tagged commit before anything
is built or published. Its checks are one list, `scripts/release/plan.sh`, and
`scripts/release/preflight.sh` runs the same list on your machine:

```sh
bash scripts/release/preflight.sh v5.3.4
bash scripts/release/preflight.sh v5.3.4 --notes /tmp/notes.md   # keep the notes
```

Run it on the release branch once the release PR is ready, and again on `main`
after the merge, at the commit you are about to tag. It exits 0 when the plan
job would pass, names every check that refuses, and prints the path of the
release notes it composed so you can read them before they are published.

The checks, in the order the plan job runs them:

| Check | Script |
|---|---|
| The milestone titled as the tag holds no open item | `scripts/release/milestone.sh check` |
| The tag equals the workspace version | `plan.sh` |
| The changelog has only Keep a Changelog types, each once per section | `scripts/checks/changelog-structure.sh` |
| The section being cut opens its `### Security` with the reason when it mixes security and functional changes, and every Security entry names a GHSA id or a `security/vex/` path | `scripts/checks/changelog-security.sh --section X.Y.Z` |
| No fragment is left under `changelog.d/`: the cut assembled every one (`scripts/release/changelog.sh --assemble X.Y.Z YYYY-MM-DD`). At a pre-release tag (`vX.Y.Z-rcN`) the fragments stay for the stable cut, and the section must carry each of them. CI runs this check (`plan.sh fragments vX.Y.Z`) in the merge queue on every change that moves the workspace version, so a fragment merged ahead of the release PR fails the PR before the tag | `plan.sh`, `scripts/release/changelog.sh --check`, `--covered` |
| The technical documentation and the CRA risk assessment follow their paths since the previous release | `scripts/checks/technical-documentation.sh`, `scripts/checks/cra-risk-assessment.sh` |
| The changelog section becomes the notes, with the support period and the CRA user-information link | `plan.sh notes`, `scripts/checks/release-notes.sh` |
| The chart's `appVersion`, its unpublished version, the party statement, `CITATION.cff`, `.zenodo.json`, the compose image tags and the documented keys | the scripts `plan.sh` names |

The previous release is resolved the way the plan job resolves it: from
`git ls-remote --tags origin 'v*'`, by version order. A tag that was pushed but
whose release never published still counts, and ancestry plays no part
(`bash scripts/release/plan.sh prev-tag vX.Y.Z` prints it).

You need `gh` logged in (the milestone check reads the tracker) and `helm` (the
chart check reads the published chart versions). A missing tool refuses its
check; it is never skipped.

To add a release fact, add a function and a line to the list in
`scripts/release/plan.sh`. The plan job and the preflight both pick it up.

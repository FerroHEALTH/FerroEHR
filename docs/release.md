# Releases

A FerroEHR release is cut from a `release/vX.Y.Z` branch and published by
pushing the `vX.Y.Z` tag, which runs `.github/workflows/release.yml`. The cut
procedure (the changelog section, the version bumps, the chart, the per-release
reviews) is in `.claude/rules/changelog.md`; the post-market steps that follow a
release are in `post-market.md`. This page covers the check you run before the
tag and the recovery of the chart leg after it.

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
| No fragment is left under `changelog.d/`: the cut assembled every one (`scripts/release/changelog.sh --assemble X.Y.Z YYYY-MM-DD`). At a pre-release tag (`vX.Y.Z-rcN`) the fragments stay for the stable cut, and the section must carry each of them. CI runs this check (`plan.sh fragments vX.Y.Z`) in the merge queue on every change that moves the workspace version, so a fragment merged ahead of the release PR fails the PR before the tag, and again on the push to `main`, so a release commit merged past the queue turns `main` red before the tag | `plan.sh`, `scripts/release/changelog.sh --check`, `--covered` |
| The technical documentation and the CRA risk assessment follow their paths since the previous release | `scripts/checks/technical-documentation.sh`, `scripts/checks/cra-risk-assessment.sh` |
| The changelog section becomes the notes, with the support period and the CRA user-information link | `plan.sh notes`, `scripts/checks/release-notes.sh` |
| The chart's `appVersion`, its unpublished version, the party statement, `CITATION.cff`, `.zenodo.json`, the compose image tags and the documented keys | the scripts `plan.sh` names |

The previous release is resolved the way the plan job resolves it: from
`git ls-remote --tags origin 'v*'`, by version order. A tag that was pushed but
whose release never published still counts, and ancestry plays no part
(`bash scripts/release/plan.sh prev-tag vX.Y.Z` prints it). The change-control
checks compare against that tag.

The performance guard needs the previous release's published images, so it
takes its baseline differently: the newest release below the tag, by version
order, whose GitHub release is published, neither a draft nor a pre-release
(`bash scripts/release/plan.sh published-tag vX.Y.Z` prints it). A tag whose
release run stopped before `finalize-release` has no published images and is
skipped. After v5.6.1 and v5.6.2 were tagged and never published, the baseline
of v5.6.3 is v5.6.0.

You need `gh` logged in (the milestone check reads the tracker) and `helm` (the
chart check reads the published chart versions). A missing tool refuses its
check; it is never skipped.

To add a release fact, add a function and a line to the list in
`scripts/release/plan.sh`. The plan job and the preflight both pick it up.

The quickstart has one copy. On `main`, `docker-compose.yml` pulls the
FerroEHR images at `:main` and carries the configuration `main` reads, and
`compose-version-guard` refuses any FerroEHR release tag in it. The
`quickstart-config` job checks its configuration with the server image built
from the change under test. A release never edits the file: the `github-release`
job writes the tag's version in place of `:main` in the copy it attaches
(`scripts/release/quickstart-compose.sh`), and refuses the copy if one FerroEHR
image still names `:main` or another version. The plan job runs the same
rewrite on a scratch copy, so a tag whose quickstart cannot be pinned is
refused before anything is published.

## Recovering the chart leg

A chart version in the registry is never pushed twice. When the chart leg
(`.github/workflows/build-chart.yml`) finds its version published already, it
unpacks both packages and compares their files
(`scripts/release/chart-published.sh`), since `helm package` stamps the
packaging time into the tarball. The same files skip the push, and the
signature, the provenance and SBOM attestations and the Artifact Hub ownership
tag are made again against the published digest. Other files refuse the
publish.

A chart leg that pushed and then failed after the push, on a Sigstore outage
for example, is therefore finished by re-running the failed jobs of the release
run. The signing identity stays `build-chart.yml`, so the verification commands
in the book's supply-chain page apply unchanged.

A release runs its workflows from the tag's tree, so a re-run of a tag cut
before this recovery existed runs the old leg, which refuses the published
version. Recover such a tag with the `Publish chart` workflow dispatched from
`main`, which packages the chart of the tag's tree as the release did:

```sh
gh workflow run publish-chart.yml --repo FerroHEALTH/FerroEHR --ref main \
  -f release-tag=vX.Y.Z -f publish=true
```

Without `-f publish=true` the dispatch is a dry run that reports whether the
published chart holds the same files. After a recovery, close the milestone the
release run left open:
`GITHUB_REPOSITORY=FerroHEALTH/FerroEHR bash scripts/release/milestone.sh close vX.Y.Z`.

## The performance gate

Every pull request that touches the server's build, its images or the gate
runs the `perf` job in `ci.yml` (#5081, #5326). `scripts/perf/gate.sh ab`
starts two servers side by side on one runner, each in a container with its
own FerroEHR PostgreSQL container:

- **The head:** the image this tree builds, with this tree's gate
  configuration.
- **The base:** the change's base commit, the first parent of the commit
  under test. The `perf-base-binary` job builds its server exactly as
  `ui-e2e-binary` builds the head's (the same container, toolchain and `e2e`
  profile), so the arms differ in the change alone. The `perf` job packages
  it with the base tree's own Dockerfile and PostgreSQL image, and the gate
  runs it under that tree's `perf/ferroehr.toml`, professional templates,
  SCIM client and seed data, so each arm meets the configuration contract of
  its own tree.

It runs `ferrohealth-perf ab` (FerroSYS #249, pinned in `docs/VERSIONS.md`)
over this tree's `perf/plan.toml`, with each arm's seeded ids:

- **Idle:** 60 seconds with no requests to the head. The head's CPU stays
  under 2 % of one core, and its database commits and rolls back fewer than 5
  transactions a second, whatever the background workers do. FerroPIX #490
  ran about 8,000.
- **Load, in pairs:** the request set, sent at a fixed rate to both servers
  at once, four pairs, the base leading in the first and the head in the
  next. It covers EHR create and read, composition commit and read, the
  lab-results AQL query with and without `ORDER BY`, a template upload, and two
  demographic reads. Each group's p50, p95 and p99 latency, the throughput,
  the peak memory, the CPU time per request and the transactions per request
  of the head are judged against the base's.

Runners of one CPU model differ by 25 to 45 % in latency and CPU per
request, so the change is judged against its base on the same runner in the
same job, never against a stored number (#5095). The release's
`performance` job runs `gate.sh ab` the same way, with the release image as
the head and the previous published release as the base: its images, pulled
from GHCR, run under the tree of its tag's commit. No baseline is stored
anywhere.

The job first runs `gate.sh self-test`. It polls a table with no wait during
the idle phase, and the self-test passes only when the gate fails that run on
`idle.transactions_per_second`.

### Reading the report

The job summary and the `perf-report` artifact (`target/perf/summary.md`,
`latest.json`) open with the verdict. A failed run lists its breaches first.
Each one names the metric, the base, the head, their ratio, the tolerance,
the noise floor and in how many pairs the head breached. A load metric fails
when the head's median is worse than the base's by more than the larger of
the plan's tolerance and the noise floor measured within the two arms, and
the head breaches in most of the pairs. A run that cannot be judged exits 2
and says why: a phase that stalled, a generator that lagged, a window that
was not steady, or a base that fell behind the schedule. A request the head
answered with a status other than the plan's expected one fails the run
too. `target/perf/base-rev` names the base commit. Locally, `PERF_SERVER`,
`PERF_DATABASE` and `PERF_TARGET` point `gate.sh self-test` at a server you
started yourself (see the script's header).

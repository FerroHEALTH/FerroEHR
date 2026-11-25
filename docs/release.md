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
| The chart's `appVersion`, its unpublished version, the party statement, `CITATION.cff`, `.zenodo.json`, the compose image tags, the default demographic definitions' release assets and the documented keys | the scripts `plan.sh` names |
| The information sheet and the instructions for use at the commit `docs/ehds/system-documents.env` pins name only configuration keys this release reads (EHDS Art. 28(b)): every `[table]`, `[table] key`, `FERROEHR__` form, dotted key and `toml` block in a section whose heading names FerroEHR resolves in the shipped default configuration, and a paragraph that names a key as unknown names one this release refuses. A refused key fails the plan, and the fix is a corrected page in the FerroHEALTH book and the pin moved to it. `accompanying-documents.sh` refuses the same pages when it attaches them | `scripts/checks/ifu-config-keys.sh` |

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

The default demographic definitions under `deploy/definitions/` ride every
release as well, one asset per `.adls` file with a `.sha256sum` beside it
(`scripts/release/definition-assets.sh`). The `github-release` job stages
them, the plan job stages them on a scratch directory first, and
`finalize-release` refuses to publish a draft that lacks one of them.

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

The `perf` job in `nightly.yml` runs every night on `main`, against the commit
the previous night measured, and on a pull request labelled `ci:perf` or
`ci:heavy`, against its base (#5081, #5326, #6517). It reads a clock on a
shared runner, so it never gates a merge (`docs/ci-cd.md`). `scripts/perf/gate.sh ab`
starts two servers side by side on one runner, each in a container with its
own FerroEHR PostgreSQL container:

- **The head:** the image this tree builds, with this tree's gate
  configuration.
- **The base:** the change's base commit, the first parent of the commit
  under test, or the previous night's commit on a schedule. The
  `perf-base-binary` job builds its server exactly as `server-binary` builds
  the head's (the same container, toolchain and `e2e`
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
  lab-results AQL query with and without `ORDER BY`, a population query
  (`SELECT e/ehr_id/value FROM EHR e LIMIT 20`), a template upload, two
  demographic reads, an export of the read EHR and an access-log read
  through the ITI-81 retrieval. Each group's p50, p95 and p99 latency, the throughput,
  the peak memory, the CPU time per request and the transactions per request
  of the head are judged against the base's.

Runners of one CPU model differ by 25 to 45 % in latency and CPU per
request, so the change is judged against its base on the same runner in the
same job, never against a stored number (#5095). The release's
`performance` job runs `gate.sh ab` the same way, with the release image as
the head and the previous published release as the base: its images, pulled
from GHCR, run under the tree of its tag's commit. No baseline is stored
anywhere.

Each arm's seed adds `PERF_POPULATION` EHRs through its own server (2 000 in
the pull-request gate, 20 000 in the release's), so the population query
serves its whole `LIMIT` from a populated store and writes one access-log
entry per EHR it served. After the load the gate counts those entries in each
arm's store and adds a table to the summary: the statements, the per-EHR
entries and the entries per statement of the head and the base, beside the
group's latency, CPU and transactions per request (#5564). A base whose store
has no access log reads `none`. A second table gives each arm's access-log
entries and the bytes they take on disk.

An arm whose tree configures the `clinician` Basic user runs with the access
path in force (#5590, `scripts/perf/access-seed.sh`): the administrator
commits an access policy that requires a care relationship, makes the
clinician a member of a care team in a coded role that a controller rule of
the policy lets read HIV data, and places the subject of
the read EHR, the write EHR and every population EHR under that team's care,
each EHR with five directives. The clinical groups (EHR read, composition
commit and read, the AQL queries, the export) are sent as the clinician, so
the evaluator judges every one by its care relationship, role and
directives; an older base sends them as its administrator. Every arm signs
its access-log links with an OpenPGP key the gate generates for the run: an
ML-DSA-65+Ed25519 key (RFC 9980 algorithm ID 30) from `ferrohealth-sop`, or,
for a base whose binary predates that algorithm, a classical key from `gpg`,
Ed25519 by default and RSA-3072 with `PERF_SIGNING_KEY=rsa3072`.

A gate measures a change in the code, so it compares like with like: when
the base binary accepts this tree's gate configuration (`ferroehr config
check`), the base arm runs under it too, with the same signing key, access
seed and credentials. Only a base that refuses it, such as an earlier release
without the access path, runs under its own tree's configuration; that
comparison shows the whole cost of the access path and the signed access log,
which `docs/benchmarks/access-path/record.json` records as the accepted cost
of the EHDS access log.

`gate.sh replicas <image> <pg-image> [<instances>...]` measures the reload
load of one, two and three instances on one PostgreSQL: after each change the
copies listen for (a grant, a care relationship, the access policy, an EHR's
directives) it counts the transactions and statements of the window and reads
each instance's grants refreshes. The recorded runs of both are in
`docs/benchmarks/access-path/record.json`, rendered into the book's
performance page.

The job first runs `gate.sh self-test`. It polls a table with no wait during
the idle phase, and the self-test passes only when the gate fails that run on
`idle.transactions_per_second`.

### A cost the owner accepted

Some changes cost something the gate measures and are worth it: an exact
access decision that reads the store per request costs transactions a
notification-timed copy did not (#5891). The `perf` job measures and fails
only a run that could not be judged; on a labelled pull request the
`perf-verdict` job judges a breach (`scripts/perf/accepted.sh`, #5924). A
scheduled run fails on any breach, and `nightly-report.yml` files it. A
breach passes only when all three hold:

- The pull request carries the `perf-accepted` label.
- Its description names every breached metric on a line of its own,
  `Perf-accepted: #<issue> <metric> [<metric>...]`, with the metric as the
  Breaches table names it (`wal.flushes_per_request` and
  `wal.bytes_per_request` for the WAL judge).
- That issue carries a comment by the code owner of the base branch (the `*`
  line of `.github/CODEOWNERS`, read through the API) that says "accepted".

The breach is then reported as accepted: a warning per metric, and an
"Accepted breaches" table with the issue and the comment's link after the
full report in the `perf-verdict` job's summary and log. A run that could not
be judged is never accepted. No openEHR spec governs this: our own design.

The release's `performance` job runs the same gate against the previous
published release and honours the same acceptances
(`scripts/perf/accepted.sh release`, #5927). A breach passes there only when
a pull request merged into the milestone titled as the tag carries the
`perf-accepted` label and a `Perf-accepted:` line naming that metric, and the
named issue carries the "accepted" comment of the code owner of the default
branch. The job verifies each acceptance again when it runs, with the same
check `perf-verdict` uses, so a label removed, a description edited, or a
comment written by anyone else no longer counts. A closed, unmerged pull
request claims nothing. Each honoured acceptance is listed with its pull
request, its issue and the comment's link in an "Accepted breaches" table
after the gate's report, in the job summary and the `perf-guard` artifact,
with a warning per metric. A breach no acceptance covers, or a run that
could not be judged, still refuses the release. The job accepts nothing on
its own and widens no tolerance.

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
too. Below the verdict, each arm's ten statements by PostgreSQL time over
the run name their calls and their mean planning and execution time:
PostgreSQL counts planning beside execution (`pg_stat_statements.track_planning`),
and an AQL statement is planned for its values on every call (#5746).
Each arm's PostgreSQL keeps its cluster in memory (tmpfs): every request
commits before it answers, and a hosted runner's WAL fsync time varies
16-fold between runs, which would tail every route on both arms (#5866). A
commit still flushes the WAL, so the gate judges the durable work by count:
the last table of the summary gives each arm's WAL bytes and WAL flushes per
request over the run, and the run fails when the head flushes more than 0.10
times per request more than the base (a commit added to every request is a
whole one) or writes more than 10 % more WAL bytes per request. Two arms of
one build differ by up to 4 % in bytes and 0.04 in flushes. The gate does
not measure what a flush costs on a disk: the capacity test
(`scripts/perf/capacity.sh`, the Capacity test workflow) runs its PostgreSQL
on the runner's disk and measures the absolute fsync cost.
`target/perf/base-rev` names the base commit. Locally, `PERF_SERVER`,
`PERF_DATABASE` and `PERF_TARGET` point `gate.sh self-test` at a server you
started yourself (see the script's header).

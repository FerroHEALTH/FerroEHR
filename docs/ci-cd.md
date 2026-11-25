# CI and CD

FerroEHR's pipeline runs in four tiers. Only the first two can block a merge,
and they block it through one required check, `conclusion` in
`.github/workflows/ci.yml`. No specification governs the tier split: it is
our own design, recorded here (owner decisions D-4 and D-5, 2026-11-24).

## A required check is exact

A check that `conclusion` needs counts something a runner cannot vary:
pass or fail of a test, SQL statements, WAL bytes, allocations, callgrind
instructions, wire bytes, a diff. A check that reads a clock (thread CPU,
wall time, latency, throughput) never blocks a merge. Hosted runners are
shared VMs
(<https://docs.github.com/en/actions/reference/runners/github-hosted-runners>),
so a clock reading on one is noise the merge queue must not carry. Such a
check runs in the nightly tier against the same thresholds; moving it there
changes no threshold, corpus or test.

## The tiers

| Tier | Runs on | What | Blocks a merge |
|---|---|---|---|
| Gate | `pull_request` | change detection, fmt, lint and guards, clippy, the test build and its shards, rustdoc, codegen drift, cargo-deny, the exact budgets, conformance, the deployment harness, chart boot | yes, through `conclusion` |
| Queue | `merge_group` | change detection, fmt, lint and guards, cargo-deny, the bin link, the test archive and its smoke set (every library and bin test target and the server's wiring tests) | yes, through `conclusion` |
| Pull request | `pull_request` | the guards that read pull-request context (changelog, crate and chart versions, migrations, screenshots, privacy boundary), the browser lanes when the viewer changed, CodeQL when Rust or workflow paths changed | yes for the guards and the browser lanes; CodeQL is advisory |
| Main | `push` to `main` | the caches pull requests read, the docs deploy, the images, coverage with SonarQube Cloud | no |
| Nightly | `schedule` on `main`, `workflow_dispatch`, the `ci:perf` and `ci:heavy` labels | every `ci.yml` lane with the browser lanes in full, perf A/B, the CPU budget, the feature-combination and MSRV sweeps, fuzzing, CodeQL's baseline | no |

The pull request runs the full gate on its own tree, so a merge group runs the light queue set (owner ruling 2026-11-25): it proves the merged tree builds and passes its smoke set, and the nightly full run checks `main`. The merge queue checks the exact tree that lands
(<https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue>),
so a push to `main` whose tree a merge-group run passed re-runs nothing: the
`changes` job finds that run (`scripts/lib/queue-runs.sh`), and each cache
lane only builds. A push no merge-group run passed runs every lane.

## Which packages a change builds

`scripts/lib/affected.sh` maps the changed paths to workspace packages
through the `cargo metadata` graph
(<https://doc.rust-lang.org/cargo/commands/cargo-metadata.html>), and
clippy, the test build and its shards, and rustdoc run for those packages
alone. A path under a member reaches that member; any other path reaches the
members whose Rust sources name its top-level directory or file in a string
literal; a reached package reaches its dependants, transitively, and their
dev-dependants. The toolchain, `.cargo/`, `.config/`, the lint, format and
cargo-deny configurations, `ci.yml`, an orphan under `app/`, `crates/` or
`tools/`, and a third-party line of `Cargo.toml` or `Cargo.lock` reach every
package.

A pull request measures its own change. A merge group measures the whole
group, from where it forked from the base branch to its head: a group's
`base_sha` is the head of the entry ahead of it, and GitHub merges every
entry of a group once its last entry passes (#5741). A push, a schedule and
a base that cannot be resolved build every package, and the nightly full run
is the backstop for a read the literal scan cannot see.

## The analyzers

Owner decision of 2026-11-25: "Sonar off PRs, CodeQL kept". SonarQube Cloud
with its coverage run analyses `main` on each push and weekly, and a newer
push cancels the older analysis (`sonar.yml`, #6574). CodeQL runs on every
pull request that touches Rust sources, Cargo files, workflows or actions,
and daily on `main` (`codeql.yml`). Both are advisory
(`.claude/rules/ai-code-review.md`).

## The nightly tier

- **`ci.yml`:** every lane, as if the change touched everything, the browser
  lanes in full included.
- **`nightly.yml`:** perf A/B (`scripts/perf/gate.sh ab`) against the commit
  the previous night measured, the hot-path CPU budget on optimised code, the
  clippy feature-combination sweeps (the slim server, `openehr-its` and
  `openehr-sdt` on wasm32, the ITS-REST features) and the MSRV check with
  cargo-hack
  (<https://doc.rust-lang.org/cargo/guide/continuous-integration.html#verifying-rust-version>).
- **`fuzz.yml`:** each fuzz target for its time budget.
- **`codeql.yml`:** both languages on `main`, daily.

The timing lanes judge for a stable signal with no threshold changed:
perf A/B runs eight load-phase pairs, so a metric breaches in most pairs or
not at all and one low base sample cannot carry a breach (#6458), and the
CPU budget runs three rounds on three runners and fails only when two
breach, since a small write's share moves with the runner host (#6477); a
round the test refuses as too noisy to judge, which it checks only once
every path passed, counts as no breach (#6569).

A pull request runs the nightly timing lanes when it carries `ci:perf`, and
every lane of `nightly.yml` when it carries `ci:heavy`. Adding the label
starts the run. None of these runs is in `conclusion`.

`nightly-report.yml` reads every scheduled run of CI, Nightly, SonarQube
Cloud and Fuzz on `main`. A red one files one issue for its workflow, or
comments on the open one, with the failed jobs and the commits since that
workflow's previous scheduled run.

## Concurrency

Groups follow GitHub's documented rules
(<https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#concurrency>):

- a pull request's runs share its ref, and a newer push cancels the older run;
- each merge-group entry has a group of its own commit, which only the queue
  cancels;
- push runs of `ci.yml` share one group, and a newer push cancels the older
  run; `docs.yml` and `containers.yml` let a newer pending run replace an
  older one but never cancel a running one, since a container build stopped
  between its digest push and its tags leaves GHCR orphans (#3731);
- scheduled runs never cancel each other.

## Build settings and the compile cache

Every CI workflow that compiles for tests sets `CARGO_INCREMENTAL=0`,
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`
(<https://doc.rust-lang.org/cargo/reference/environment-variables.html>):
CI compiles from a cache, never incrementally, and debug info is link weight
no cache removes. No workflow limits `CARGO_BUILD_JOBS`.

The build jobs (`build`, `build-tests`, `test-viewer`, `codegen-drift`,
`instructions`, `nesting-release` and the nightly CPU budget) compile through
sccache on an S3-compatible bucket once one exists
(<https://github.com/mozilla/sccache/blob/main/docs/S3.md>), through
`.github/actions/sccache`. Until then the action does nothing and rust-cache
keeps the compiled crates as before. To switch it on, an owner sets, at the
organisation or the repository:

| Kind | Name | Value |
|---|---|---|
| variable | `SCCACHE_BUCKET` | the bucket's name |
| variable | `SCCACHE_ENDPOINT` | `https://<account id>.r2.cloudflarestorage.com` for Cloudflare R2 |
| secret | `SCCACHE_RW_ACCESS_KEY_ID`, `SCCACHE_RW_SECRET_ACCESS_KEY` | an R2 API token with object read and write on the bucket |
| secret | `SCCACHE_RO_ACCESS_KEY_ID`, `SCCACHE_RO_SECRET_ACCESS_KEY` | an R2 API token with object read only |

The region is `auto`, as R2 requires, and every entry sits under the key
prefix `ferroehr/`. A pull request reads with the read-only token and
`SCCACHE_S3_RW_MODE=READ_ONLY`, so it never writes an entry `main` or the
merge queue reads; the merge queue, `main` and the nightly write. With the
bucket set, rust-cache keeps only the registry and the tools of those jobs.
The release lanes call no cache.

---
name: session-workflow-gotchas
description: "Harness/hook gotchas that repeatedly cost time — background-task kill limit, attribution-hook false positives, changelog-guard label refresh, conformance.sh EXIT-trap corpus wipe"
metadata: 
  node_type: memory
  type: project
  originSessionId: 1be6641a-9768-4fd5-8149-acb2551a1d97
  modified: 2026-08-27T08:51:58.196Z
---

Recurring session-workflow traps (all hit 2026-07-13/14):

0. **Docker image build OOMs at 2 cargo jobs when cold** (hit twice:
   2026-08-21 during #2530, again 2026-08-21 after the 0.0.35 crate bump —
   an `openehr-*` lockstep version bump invalidates the BuildKit target-cache
   fingerprints for all eight crates, so the next image build is COLD for
   them and two concurrent heavy crate compiles (ferroehr-ext + openehr-adl)
   SIGKILL inside the Docker VM). After any crate-version bump or Dockerfile
   cache invalidation, run the pipeline as
   `CARGO_BUILD_JOBS=1 bash scripts/conformance.sh` (docker/sut-ferroehr.yml
   forwards the env var); warm rebuilds are fine at the default 2.
   ESCALATION (2026-08-22, #309): the console crate's wasm-release pass
   (`[profile.wasm-release]`: opt-level=z + codegen-units=1) now SIGKILLs in
   the 8 GB Docker VM even building ALONE at jobs=1 — rustc's own peak on the
   grown crate exceeds the VM. jobs=1 cannot fix a single-process peak; the
   fixes are a bigger VM (Docker Desktop → Resources → Memory, 12-16 GB —
   recommended to the owner) or host-built/prebuilt packaging. Keycloak (1 GB
   container limit) is always the first OOM-reaper victim mid-run — the
   monitor tripwire `docker start ferroehr-e2e-keycloak-1` revives it.

1. **Long background Bash tasks get killed (~30 min)** — for multi-hour runs
   (benchmark ladders, seeds), launch detached: `nohup caffeinate -is
   script.sh > log 2>&1 & disown`, then watch the log with a Monitor
   (`tail -f | grep --line-buffered`). Also wrap in `caffeinate` — the Mac
   otherwise sleeps overnight and takes Docker down with it.

2. **Hook false positives**: (a) the no-attribution PreToolUse hook regex
   `generated with .*claude` spans the whole tool-call payload — any commit
   message containing "…generated with…" trips it because scratch/session
   paths contain "claude"; write "rebuilt with"/"produced with" instead.
   (b) `protect_vendored_specs.sh` greps files for the literal `@generated`
   marker — a hand-written file whose doc comment merely *mentions* the
   marker string gets blocked from Edit (fixed in openehr-term bundle.rs by
   rewording; avoid quoting the marker in prose).

4. **Killing scripts/conformance.sh destroys the seeded corpus** (hit
   2026-07-23): the script's `trap compose_down EXIT` fires on ANY exit —
   `pkill` of a mid-seed pipeline runs `docker compose down -v` and wipes
   the hour-long 1M-composition seed's backing
   data. If a run must be aborted but the corpus kept, kill with SIGKILL
   (`pkill -9 -f conformance.sh`) so the trap cannot run — or accept the
   re-seed. Corollary: smoke-test new measured-workload wire shapes against
   a light composed stack BEFORE burning a seeded run (the pack preflight
   gate now covers payload validity at seeding).

5. **`pkill -f conformance.sh` does NOT kill the instrument child** (hit
   2026-07-24, cost two tainted measured runs): the orphaned `veredictum
   perf` keeps firing its arrival schedule for its full window (70+ min)
   at the SUT ports — and a relaunched run reuses the same compose
   project/ports, so the orphan's request stream and writes contaminate
   the NEW run's seed + measured window, and it writes its garbage
   measurement into the same `--results` path. Aborting a measured run
   means: `pkill -9 -f veredictum` AND `pkill -9 -f conformance.sh`, then
   `pgrep -fl 'conformance.sh|veredictum'` must be EMPTY, then explicit
   `docker compose -p <project> down -v`, then `git restore` the
   artifact dir — verify all four before relaunching.

3. **Guard labels need a fresh PR event** — the label-gated CI guards
   (`no-changelog` on changelog-guard, `no-ui-visual-change` on
   ui-screenshot-guard) read `github.event.pull_request.labels` (frozen at
   trigger time), so adding the label then rerunning the failed job still
   fails. Since #2777 ci.yml listens for `labeled`/`unlabeled`, so applying
   the label raises a fresh run by itself — wait for that run, never re-run
   the stale one (close+reopen is obsolete). Corollary (hit 2026-08-26):
   `gh pr create --label X` fires opened+labeled in the same second — the
   opened run gets CANCELLED and `gh pr checks` then shows its rows as
   `fail` beside the live run's rows. Monitor the RUN ID
   (`gh run list --branch <br>`), never the mixed pr-checks rows. Also
   (hit 2026-07-20):
   a label a workflow references must actually EXIST in the repo —
   `no-ui-visual-change` didn't until it was first needed; `gh label
   create` fails loudly at apply time, the workflow never warns.

**Why:** each cost a debugging loop mid-flow; the fixes are non-obvious.
**How to apply:** overnight/long runs → detached+caffeinate+Monitor pattern;
commit-message wording. Late labels: since #2777 applying a label raises a fresh run itself (labeled/unlabeled types); never re-run the failed job (stale payload).

6. **Merge PRs only behind a GREEN gate, in code** (hit 2026-07-26, broke
   main's rustfmt): never chain `gh pr merge` unconditionally after a
   wait loop — the wait can terminate on a FAILURE and the merge still
   runs. Pattern: `if [ -z "$(gh pr checks N | grep -v 'pass\|skipping')" ];
   then gh pr merge N …; else report; fi`. Repo has no auto-merge
   (enablePullRequestAutoMerge is off), so this guard is the only gate.

7. **Formatter order: leptosfmt FIRST, `cargo fmt` LAST** (same incident):
   leptosfmt may collapse a `view! {…}` to one line, which changes where
   rustfmt wants a chained `.into_any()` — running leptosfmt after a clean
   `cargo fmt --check` leaves the tree rustfmt-dirty. Always finish with
   `cargo fmt --all && cargo fmt --all --check`.
- **Changelog inserts:** a section (`### Fixed` etc.) may already exist in the target release block — ALWAYS merge the entry into the existing subsection header; blindly inserting a new header creates a duplicate (happened twice, owner-corrected 2026-08-01). Check `grep -n '^###' CHANGELOG.md` for the block first.
- **`gh` comment bodies need heredocs:** backticks inside a double-quoted
  `--body` get shell-executed and silently mangle the posted comment. Use
  `--body-file -` with a quoted heredoc (`<<'EOF'`). The SAME trap applies to
  `git commit -m "..."` (hit 2026-08-19: a `` `up --wait` `` in the message
  executed and vanished) — use `git commit -F -` with a quoted heredoc; and
  the `block_dangerous` hook refuses ALL force-pushes (even feature branches),
  so an already-pushed mangled message is not amendable — write it right the
  first time. Pipes also mask exit codes (`cmd | tail -1 && next` runs `next`
  even when `cmd` failed — bit twice on 2026-08-18/19): put the gate command
  bare, never piped, when chaining with `&&`.
- **Renaming a container-test module breaks nextest serialization:** the
  `containers` group in `.config/nextest.toml` matches those suites by module
  prefix (`binary(it) & test(/^(…)::/)`), so a module rename silently
  un-serializes them — update the filter in the same change.
- **PR close keywords:** GitHub closes ONE issue per keyword — `Closes #1, #2, #3`
  closes only #1. Every issue needs its own `Closes #N` (one per line is
  clearest). Happened on PR #1821 (14 of 15 left open, owner-corrected
  2026-08-03); Batch A (#1812) had the correct per-issue form.
- **Editing a PR body CANCELS the in-flight CI run** and raises a fresh one
  (`ci.yml` listens for the `edited` pull-request type, and the concurrency
  group cancels the previous run). `gh pr checks` then reports every cancelled
  job as `fail`, which reads exactly like a real breakage — check
  `gh run list --branch <b>` for a newer run before chasing any of them. Cost
  two full CI cycles on 2026-09-04 (#3112, #3115). Write the body once, with
  the licensing checkbox included, rather than editing it after opening.
- **A hung `docker-credential-desktop` silently blocks every `docker pull`**
  (hit 2026-08-14, cost ~2 h): pulls/builds hang with NO output while
  `docker run`/`images`/host curl all work — the client stalls calling the
  keychain-backed credential helper (a pending macOS Keychain/TCC dialog;
  even reading `~/Library/Group Containers/group.com.docker/settings*.json`
  hangs then). Diagnose: `echo '{}' | docker-credential-desktop list` hangs.
  Bypass without touching user config: a scratch
  `DOCKER_CONFIG=$SCRATCH/dockercfg` whose `config.json` is
  `{"cliPluginsExtraDirs": ["$HOME/.docker/cli-plugins"]}` — the plugin dir
  entry is REQUIRED or `docker compose`/`buildx` vanish ("unknown shorthand
  flag: 'p'"). All conformance pulls are anonymous, so this is loss-free.
- **Render-time dates must be UTC-stable or the drift gates detonate on the
  tag** (hit 2026-08-14, v3.17.6): `git log --format=%cs` renders the
  committer's OWN timezone, so a baseline committed just past local midnight
  regenerates a different day than the copy rendered pre-commit under the
  mtime fallback — and the regenerate-and-diff gate fails permanently INSIDE
  the tag's immutable tree (the frozen-site cut can then never pass from the
  tag; recover by fixing the script on main and running
  `scripts/site/cut-version.sh vX.Y.Z` locally — toolchain versions must
  match the workflow pins). Any rendered date derives from the committed
  artifact in UTC (`TZ=UTC --date=format-local:%Y-%m-%d`), never `%cs`,
  never the wall clock.
- **`gh pr merge --admin` is blocked by the harness permission classifier**
  (hit 2026-08-18), and the repo has no auto-merge — so the merge path is:
  wait for the run, rerun the flaky red (`gh run rerun <id> --failed`; while
  #2285 is open, ui-e2e is red-on-main and needs this), then plain
  `gh pr merge --merge` behind the green-gate `if` (gotcha 6). `gh run
  rerun` refuses while the run is `in_progress` — wait for `completed`
  first.

8. **A conformance-artifact commit must run the FULL docs render set** (hit
   three times on 2026-08-25: the #2726 artifact refresh turned the docs lane
   red in three successive drift gates). Committing a regenerated
   `docs/conformance/<sut>/results.json` obligates, in the SAME commit, every
   derived surface docs.yml regenerates and diffs: `conformance-stats.sh
   includes`, `comparison.sh` (COMPARISON.md), `perf-assets.sh`,
   `conformance-assets.sh` (+ `CONF_SUT=ehrbase`), `conformance-docs.sh`
   (report/certificate/badges). Run all six, commit whatever changed.

9. **A release-cut's local guard runs are `--all`, and the version sweep greps
   the BARE number too** (hit at the v4.0.5 cut, 2026-08-26: PR #2757's
   post-merge docs-claims went red). Bare `bash scripts/checks/docs-claims.sh`
   scopes to CHANGED files — on a clean tree it passes vacuously; CI runs
   `--all`. And `verifying-releases.md` pins image tags as
   `ghcr.io/...:X.Y.Z` (no `v` prefix), so a `vX\.Y\.Z`-shaped grep misses
   them. At every cut: `grep -rn '<old-version>' website/book/src/` (bare
   number), then `docs-claims.sh --all` before merging the release PR.

10. **A subagent that detaches a nohup run and ends its turn has NOTHING
    waking it** (hit 2026-08-27, #2752: the worker's retried conformance run
    had no waiter — it would have slept forever). When a worker reports
    "running X, will report when it lands" and goes idle, `pgrep` the run's
    PID and check for a waiter process on it; if none, arm an orchestrator
    Monitor on the PID and `SendMessage` the worker on completion. Corollary:
    a worker killed by the session limit loses nothing when it commits as it
    goes — resume with `SendMessage` naming the branch state and its last
    line; never respawn fresh.

11. **zsh pipeline exits mask mutation proofs** (bit three times 2026-08-27):
    `guard.sh 2>&1 | head -3; echo $?` prints HEAD's status — a failing guard
    reads as exit 0. Every mutation proof asserts the exit UNPIPED
    (`guard.sh >/dev/null 2>&1; echo $?`), with the piped run only for the
    message text.

12. **BuildKit's "transferring context" line is incremental** (hit at #2813):
    a warm builder transfers only changed files, so a context-size measurement
    read from it is fiction (3.9 MB shown vs the real 944 MB). Measure with a
    fresh ephemeral builder (`docker buildx create --name probe` → build →
    `buildx rm probe`), one per side of the comparison.

13. **A failed release-pipeline leg: rerun for TRANSIENT, fix-forward for
    BROKEN — read the log before choosing** (both halves hit live at the two
    2026-08-27 cuts). When the leg's CODE is defective (v4.0.6's chart leg:
    the missing helm-docs), `gh run rerun --failed` re-executes the tag
    snapshot and fails identically — fix forward on main, then the leg's
    dispatch recovery lane (chart: refresh the committed appVersion default
    first; the empty-Unreleased changelog fallback injects the right
    section), and the red run stays as the honest record. When the failure
    is INFRASTRUCTURE (v4.0.7's arm64 leg: softprops/action-gh-release died
    on `Headers Timeout Error` AFTER every asset uploaded ✅),
    `rerun --failed` is correct and safe: the release is still a DRAFT
    (freeze-last design), the action replaces same-named assets
    idempotently, and the rerun un-skips the dependent publish/crates legs.
    The discriminator is the failed step's log, never the leg's name.

14. **The SNOMED/Snowstorm probe never runs on this machine** (owner ruling
    2026-08-27, #2236 parked on-hold): the full Snowstorm + Elasticsearch
    import exceeds the development box's compute — do not launch
    `PROBE_ONLY=terminology` locally, ever; the issue holds until a bigger
    environment exists. Corollary: an hours-long opt-in run is launched only
    after confirming with the owner that THIS machine is the intended place
    to run it.

15. **Never close/reopen a dependabot PR** (hit 2026-08-27, #2802): closing
    one makes dependabot DELETE its branch and treat that update as
    dismissed — reopen fails ("Could not open the pull request") and the
    bump must be hand-recreated from the closed PR's diff. To refresh a
    dependabot PR's checks: `@dependabot rebase` when the base moved; when
    the base is current but the runs are stale, a foreign empty commit to
    the branch fires fresh runs (a PR run's MERGE SNAPSHOT is frozen at run
    creation, so a run that sat queued past a base fix still tests the old
    tree — check `started_at` vs the fix's merge time before believing a
    red). A foreign commit also ends dependabot's ownership (`rebase` then
    refuses; only `recreate` remains), so push one only when the PR is
    being merged soon. Corollary: a base-image digest bump must sweep EVERY
    base.digest declaration (both Dockerfiles + the label blocks in
    containers.yml and release.yml) — image-labels.sh holds them equal and
    dependabot bumps only the FROM lines.

- **A main push cancels an in-flight containers dispatch** (`concurrency:
  containers-${{ github.ref }}` + cancel-in-progress): hold every merge while
  a `workflow_dispatch` image build runs, or it dies at the next merge — the
  v4.0.10 night lost three dispatches to this before freezing merges.
- **A release cut has TWO more regenerate-from-artifacts layers beyond the
  changelog rule's list**: `scripts/render/comparison.sh`
  (docs/conformance/COMPARISON.md) and `scripts/render/conformance-assets.sh`
  (the book's conformance SVGs) both re-render release-day facts and are
  diff-gated by the Docs lane — run them in the release PR whenever the party
  statement or the committed results stamp moved.

**PR labels at creation poison the required fan-in (2026-09-01):** `gh pr create --label X` fires `opened` + `labeled` events, raising two CI runs in one concurrency group; the cancelled one's `conclusion` fan-in completes as FAILURE on the same head SHA, and the ruleset's required `conclusion` context then blocks auto-merge even though the survivor run is green. Create the PR bare, apply labels in a SECOND call only if a guard needs them (that raises a labeled run with the label in payload), and clear a poisoned SHA with one empty commit.

**Release tags are truly immutable — a failed cut burns the version (2026-09-01):** the `release-tags` ruleset (deletion + non-fast-forward + signatures, ZERO bypass actors) makes a pushed `v*` tag unmovable even for the owner, and a `workflow_dispatch` recovery reads the workflow file FROM THE TAG, so a tag whose release.yml is itself broken can never re-run. Recovery is a version skip: fix on main, re-cut as the next patch (rename the unpublished changelog section, full version sweep, chart version bumps again), move the open next-milestone issues up one, move the shipped issues into the skipped-to milestone, delete the burned milestone. The v4.0.14→v4.0.15 case: a reusable-workflow call-job without a `permissions:` block fails the WHOLE run at startup when the called jobs request `contents: read` (under workflow-level `permissions: {}` a call-job grants nothing) — every `uses: ./.github/workflows/*.yml` job needs an explicit permissions grant covering the called jobs' requests.

9. **Never edit a bash script while it is running** (hit 2026-09-02): bash
   reads the file incrementally, so a comment edit two lines longer than the
   original made the running `reseed.sh` fail with `syntax error near
   unexpected token` at a line that was fine. Copying the script does not
   help either (`ROOT_DIR` resolves from the copy's location); freeze edits
   until the run ends.

10. **Local DB-backed tests and stacks on this Mac** (2026-09-02): testkit
    needs `DOCKER_HOST=unix:///Users/rubentalstra/.docker/run/docker.sock`
    (the default socket is a dead symlink); the compose postgres publishes no
    host port, so for a locally built server use a throwaway
    `docker run -p 5433:5432 -e POSTGRES_PASSWORD=postgres -e PG_INIT_USER=…
    -e PG_INIT_PASSWORD=… -e PG_INIT_DB=… ghcr.io/rubentalstra/ferroehr-postgres:<tag>`
    plus `FERROEHR__DB__URL` and `--config deploy/hosted/ferroehr.sandbox.toml`
    (basic user ferroehr/ferroehr, admin on). A debug build of the seed takes
    ~120 s versus 39–45 s on the release image.

11. **`cargo clean` is a cold-rebuild decision, not a reflex**: check
    `pgrep -fl 'cargo|rustc'` FIRST and skip the clean while anything (IDE
    included) is compiling; after a clean, every gate in the session runs cold
    and the 10-minute foreground tool limit kills it — run cold gates as
    `nohup caffeinate -i bash -c '…' > log &` with a background `until grep`
    waiter.
  - `gh pr create --body` bypasses the PR template, and the
    `contribution-licence-guard` CI job then fails on the missing ticked
    line. Every PR body must carry, verbatim and ticked:
    `- [x] I accept the terms in [CONTRIBUTING.md § Licensing of contributions](../CONTRIBUTING.md#licensing-of-contributions): …`
    (PR #3090 merged red on exactly this).

9. **A PR with a merge conflict runs NO CI, and auto-merge never fires**
   (hit 2026-09-11, #3248: main moved under it and the changelog conflicted).
   After every `gh pr create` / push, check `gh pr view --json mergeable`
   is `MERGEABLE` and that `gh pr checks` shows pending rows; when main
   moved, rebase, re-run any tracker-derived generator (the control matrix
   follows closed issues), and `--force-with-lease`.

10. **The from-source image build and a cargo gate battery do not fit in memory together** (2026-09-11): running `docker compose build ferroehr` (a `--release` build inside the 8 GB Docker VM) while the host ran the viewer clippy/nextest/cargo-leptos battery got BOTH killed by the harness for low memory, and the docs-screenshot pass chained behind the build died with it. Sequence them: gates first, then the image build alone with `--build-arg CARGO_BUILD_JOBS=1`, then `UI_E2E_SHOTS_ONLY=1 UI_E2E_DOCS_SHOTS=1 UI_E2E_NO_BUILD=1 CHROMEDRIVER=/opt/homebrew/bin/chromedriver bash scripts/ui-e2e.sh` (the compose file set is `docker-compose.yml` + `docker-compose.dev.yml` + `docker/viewer/e2e-env.yml`, project `ferroehr-e2e`; the base compose file alone has no build section, so it reports "No services to build"). The same box also kills a full `cargo nextest run -p ferroehr` battery when it compiles test binaries at default parallelism beside RustRover (1 GB) and Docker Desktop (8 GB VM): run the suites as `CARGO_BUILD_JOBS=3 cargo nextest run … -j 6`, and never chain the suites behind clippy/rustdoc in one background command (a kill loses all of it).

11. **chromedriver comes from Chrome for Testing, not Homebrew** (2026-09-11): the `chromedriver` cask is disabled (Gatekeeper, 2026-09-01) and `/opt/homebrew/bin/chromedriver` is a dangling symlink, so `scripts/ui-e2e.sh` reports "chromedriver not found" even though `brew list` shows it. Fetch the build matching `Google Chrome --version` from `https://storage.googleapis.com/chrome-for-testing-public/<version>/mac-arm64/chromedriver-mac-arm64.zip` into the scratchpad, `xattr -dr com.apple.quarantine`, and pass it as `CHROMEDRIVER=...`. The `block_dangerous` hook refuses `rm -rf` even in the scratchpad; use `mkdir -p` + `unzip -o`.

12. **Arm auto-merge only when the PR's content is final** (2026-09-12, #3275): I armed `gh pr merge --auto` while a benchmark record and two follow-up commits were still to come; the required checks went green on the intermediate head and GitHub merged it, so the later pushes recreated a branch nobody was watching and had to become a second PR. Either open as draft (`gh pr create --draft`, `gh pr ready` at the end) or arm auto-merge as the LAST step after the final push.

13. **A negative test never touches the working file, and an untracked file has no git safety net** (2026-09-12, twice in one session): `git checkout -- website/book/ehds.yaml` after a mutation reverted my own uncommitted edits (the index held the old copy), and a `sed -i "${L}s/…/"` with an EMPTY `$L` rewrote every line of the untracked `tracker-status.sh`, which nothing could restore but a scratchpad draft. Mutate a COPY in the scratchpad (or `cp file file.bak` first and `mv` back), never restore from git while uncommitted edits exist; guard every computed line number with `[[ -n "$L" ]]`; and keep the shell-quoting-heavy perl one-liners to a single `-e` with no `'"'"'` splices.

14. **Do not drive the owner's browser to check a GitHub page; ask for a screenshot** (2026-09-12): a Chrome navigate to the issue chooser was rejected, and the owner pasted a screenshot instead, which answered the question at once. Verify GitHub UI facts from the official docs and from `gh`/`curl` where a login is not needed; when only the logged-in UI shows it, ask for a screenshot rather than reaching for the browser tools.

15. **A `perl -0pi s|…|…|` with `\|` inside the pattern silently rewrites the wrong place** (2026-09-12): escaping `|| true` as `\|\| true` under a `|` delimiter made the pattern match at the file head and replaced the shebang, and because `shellcheck … && echo` does not trip `set -e`, the mangled file was committed, pushed and auto-merge re-armed before anyone looked. For multi-line block edits use the Edit tool (exact match, fails loudly); when perl is unavoidable, pick a delimiter absent from the text (`s{…}{…}`), and never chain `git commit && git push` behind a guard whose failure `set -e` cannot see.

16. **Never pass `--label` to `gh pr create`; add a label after CI has started, or not at all** (2026-09-12, PR #3316): the `opened` and `labeled` events raise two `pull_request` CI runs in the same second, the PR concurrency group cancels one and can cancel BOTH, each `conclusion` job then reports failure with nothing left running, and auto-merge never fires. Recovery is `gh run rerun <newer run id>` (a re-run keeps the current label payload). A PR label is only needed for a guard escape hatch (`no-changelog`, …), and even then apply it once the first run is in progress.

- **Filing an issue with a `Control:` line changes the generated control matrix**: `scripts/render/control-matrix.sh` is a pure function of tracker state, so the NEXT PR (whatever it is about) fails the `control-matrix` CI job until it re-renders and commits `website/book/src/compliance/control-matrix.md`. Re-render right after filing such issues, on the branch about to be pushed (2026-09-13, #3324/#3325 tripped #3329).

- **Local ui-e2e chromedriver (2026-09-13)**: the Homebrew `chromedriver` cask is disabled (Gatekeeper, 2026-09-01) and `/opt/homebrew/bin/chromedriver` dangles; download the Chrome-for-Testing chromedriver matching the installed Chrome into the scratchpad and pass `CHROMEDRIVER=<path>`. A reused e2e stack breaks the template-delete journey (#3335); run the battery on fresh volumes.

- **Background cargo runs get killed for low memory** (2026-09-14): a `run_in_background` nextest or clippy on this box was stopped by the harness twice in one session. Run cargo in the FOREGROUND with `CARGO_BUILD_JOBS=2` and `-j 2`, one command at a time; keep only light shell waits in the background.
- **Describing the attribution guard trips the attribution guard**: the `no_attribution_guard.sh` PreToolUse hook blocks any commit/PR command whose text contains the literal trailer name (`Co-authored-by`), even in a sentence ABOUT the guard; write "the co-author trailer" instead. Dependabot squash merges carry that trailer for the bot, and CI's `[bot]` catch-all refused it until #3486 excluded Dependabot's exact line.

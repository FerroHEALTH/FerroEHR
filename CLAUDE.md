# CLAUDE.md

A pure-Rust, **openEHR-spec-conformant** CDR (ITS-REST 1.1.0 + AQL 1.1) in a single root Cargo workspace, with greenfield PG18-native internals. The openEHR spec + serialization + REST-contract layer is **generated** from the official machine-readable specs; the application is **modern idiomatic Rust of our own design on top of the generated crates** (own storage, versioning and AQL engine), validated by the openEHR CNF conformance suite. EHRbase is prior art, never an oracle.

**The tracker is GitHub Issues** (Issue workflow below); the public FerroEHR Roadmap board is a view over it (its readme carries the direction themes); deep working plans live in `docs/plans/`. Issues, PRs, `CHANGELOG.md` and git history are the authoritative record. **There is no ADR/design-doc layer:** decisions live in THIS file, `docs/architecture.md` and the code; a plan/design markdown is DELETED in the PR that implements it; the vendored specs are the only doc oracle.

**Read on demand, not loaded at launch:** `docs/architecture.md` (the current design: storage, AQL engine, access layer, SM component map — read before any design or storage work), `docs/VERSIONS.md` (every pin + the spec version policy), `docs/postgres-features.md` (which PG 17/18 features the code uses and why not the others). Path-scoped rules in `.claude/rules/` load when a matching file is read; the tracker rules (`issue-workflow.md`, `issue-relationships.md`, `project-board.md`) are scoped to `scripts/gh/**`, so READ them before any tracker write beyond the commands named here.

## Repo map

Single workspace, `members = ["crates/*", "app/*", "tools/*"]`. **Crate naming:** `openehr-*` = the openEHR **specification** (generated from the vendored BMM/XSD/OAS — treat as `// @generated`); `ferroehr-*` = the **application**.

- `crates/openehr-base`, `openehr-rm`, `openehr-am`, `openehr-term`, `openehr-lang` — **generated** spec crates. `openehr-its` — canonical JSON + generated XML (`emit-xml`) + generated ITS-REST contract (`emit-rest`) + hand-written runtimes (Apache-2.0). `openehr-sdt` — hand-written Simplified Formats (`flat`), RM-instance validation (`rm_instance`), SMART scope grammar (`smart_scopes`) (BUSL-1.1). `openehr-query` — hand-written AQL lexer/parser/AST. `openehr-adl` — hand-written ADL 2.4 engine over `openehr-am::v2_4`. **The `openehr-*` crates are PUBLISHED on crates.io** on their own lockstep SemVer line, decoupled from spec versions: a PR changing packaged crate content bumps the version in the same PR (`.claude/rules/crates-publishing.md`; `crate-version-guard` CI + `crate_version_bump_guard.sh` hook).
- `app/*` — five crates: `ferroehr` (the platform **library**: storage, service layer [one module per SM chapter, concrete `FerroEhrService` methods, no traits], AQL engine, versioning + signing, config, telemetry, `privacy`, `system_log`, `licence`), `ferroehr-rest` (ITS-REST adapter + auth + `access` authz, calls the service directly), `ferroehr-server` (wiring-only binary named `ferroehr`), `ferroehr-ext` (feature-gated optional integrations: FHIR, events, multimedia), `ferroehr-viewer` (Leptos SSR viewer, own OCI image, consumes the CDR STRICTLY over ITS-REST — may depend on `crates/openehr-*`, NEVER on `app/ferroehr`/`app/ferroehr-rest`; gates via `/ui-gates`, rules in `.claude/rules/leptos-ui.md`).
- **Zero re-exports: import every name from its defining module.**
- `tools/*` — not part of the app: `openehr-codegen` (BMM/XSD/OAS→Rust) and `testkit` (one PG18 server + template-database cloning; every DB-backed test uses `testkit::db()`, never a per-test container).
- **The acceptance instrument is NOT in this workspace:** [Veredictum](https://github.com/rubentalstra/Veredictum) (Apache-2.0: CNF 2.0 runner, catalogue, ambiguity register), pinned in `scripts/lib/veredictum.sh`. The shared vendored test corpus is `corpus/`.
- `docs/` — plans, VERSIONS, architecture, postgres-features, conformance artifacts. **Endpoint call chains are read from the code (router → handler → service → SQL), never from a standing document.** `docs/specs/openehr/` = the vendored spec text (the oracle) + the CNF test schedule (a stalled structural guide, never the correctness authority); see its README + `/spec-lookup`.
- `.claude/` — rules, skills, hooks, agents, **`memory/`** (persistent agent memory, in-repo; the harness memory dir under `~/.claude/projects/` is a symlink to it — never break that link). Agents: `spec-researcher`, `spec-conformance-reviewer`, `cnf-triage`, `compliance-researcher`, `compliance-implementer`, `implementer`, `ui-implementer`, `docs-implementer`, `leptos-reviewer`, `k8s-reviewer`. Website/book + markdown prose goes to `docs-implementer` (never `ui-implementer`); regulation questions to `compliance-researcher`, compliance deliverables to `compliance-implementer` (law corpus `docs/law/`, rule `law-corpus.md`) — kept SEPARATE from the openEHR instruments.

**Every crate carries its own `CLAUDE.md`** (crate-local role, generated-vs-hand-written split, never-do rules, gates). Nested files load on demand and are not re-injected after `/compact` until the next read, so repo-wide hard rules live ONLY in this root file; never move a global rule down. Update a crate's `CLAUDE.md` in the same change that alters its reality.

## Code generation — READ THIS FIRST

**The openEHR spec crates are GENERATED from the vendored BMM meta-model.** Never hand-transcribe or hand-edit RM/BASE/AM classes. Full rules: `.claude/rules/codegen.md` (incl. the generation-modules design).

- **Pipeline:** vendored `*.bmm.json` (`tools/openehr-codegen/vendor/bmm/`) → `openehr-lang` loader → `openehr-codegen` → the spec crates. Canonical-JSON `_type` (de)serialization is **emitted MANUAL serde impls** (`emit-json` → each crate's `src/json_serde.rs`, over `openehr_base::serde_support`); spec types carry NO serde derives or attributes (strict reader: undeclared/duplicate keys refused). Entry points: `openehr_its::json::{to_canonical_json,from_canonical_json,from_canonical_value}`.
- **Regenerate:** `cargo run -p openehr-codegen -- emit` · `-- emit-xml` · `-- emit-rest` · `-- check`/`check-xsd`. `/regen-codegen` runs all + the drift check; the `codegen-drift` CI job guards it.
- Generated type files start with `// @generated … DO NOT EDIT`. To change output, edit the emitter (`tools/openehr-codegen/src/render/emit.rs`) or its override map and regenerate. Hand-written spec behaviour lives in sibling `*_impl.rs` files.
- **Generation modules:** each generated BMM crate carries its generations as version-named modules (`openehr_base::v1_2`/`v1_3`, `openehr_rm::v1_1`/`v1_2`, `openehr_lang::v1_0`/`v1_1`, `openehr_am::v1_4`/`v2_4`, `openehr_term::v3_1`), driven by `tools/openehr-codegen/src/plan/composition.rs`; an emitted `Generation` enum is the ONLY pin authority; the prelude re-exports the current generation only. Details in `codegen.md`.
- **Partly generated:** `openehr-its` (`src/xml/generated/` + `src/rest/generated/`). **Hand-written:** `openehr-lang`, `openehr-codegen`, `openehr-term`, `openehr-query`, `openehr-adl`, `openehr-sdt`, all `ferroehr-*`.
- **Pinned spec versions:** RM 1.2.0, BASE 1.3.0, TERM 3.1.0, AM 1.4.0 + 2.4.0 (`docs/VERSIONS.md`, incl. the pin-honesty column and the spec version policy). ITS-REST is single-version, always the latest RELEASED API.

## Issue workflow (the loop)

**The open issue list IS the worklist.** Issue state is edited only via `gh` and the `scripts/gh/*` helpers; never track work only in chat.

1. **Orient:** the SessionStart hook injects the open issues with `<Type/Priority>`, `{k/n}`, `child-of`, `BLOCKED-by`/`blocks`. Pick the pinned issue (max 3) or the one the user names, else the highest Priority and, within it, the oldest; **skip an issue `BLOCKED-by` an open issue**; prefer the next open child of a parent. Read with `gh issue view <n> --json title,body,comments` (NEVER `--comments`: it prints nothing for an issue without comments) and `scripts/gh/rel.sh tree <n>`.
2. **The body is the contract:** a plain opening summary (what + why, rulings, spec citations), `## Acceptance criteria` checklist, optional `## Tasks`. New work found en route gets its own issue via `scripts/gh/fields.sh new <type> <priority> <effort> …` (never bare `gh issue create`), then is **linked** with `scripts/gh/rel.sh` (sub-issue or `blocked-by`/`blocking`), never a prose "see also". **Within an audit/QA program, every self-filed issue is fixed and merged before the next unit starts; filing is the record, never permission to move on.**
3. **Do the work:** at pickup set any missing type/priority/effort, then `scripts/gh/project.sh status <n> in-progress`. **Read the governing spec sections first** (`/spec-lookup`). Spec layer: change the generator and regenerate. Application: idiomatic Rust on the generated crates, specs as the authority. Compiling, tested increments.
4. **Record progress on the issue:** tick verified criteria (`gh issue edit`), post decisions as comments.
5. **Commit** on a conventional-type branch; the PR body declares `Closes #<n>` (one keyword per issue) so the merge closes it — never close by hand.
6. **Close:** `/phase-done` verifies, writes the close narrative into the PR description, posts the handoff comment, DELETES the implemented plan file in that PR.

**Taxonomy** (full policy `.claude/rules/issue-workflow.md`): type = the native issue type (`Bug`↔fix, `Feature`↔feat, `Task` + exactly ONE work-kind label `documentation`/`chore`/`refactor`/`perf`/`test`/`ci`); priority = the org `Priority` field (`Urgent`/`High`/`Medium`/`Low`); effort = the org `Effort` field (never reorders the worklist). All set with `scripts/gh/fields.sh`. Domain labels: `spec:<comp>`, `spec-update`, `spec-impact:*`, `viewer`, `upstream-report` (an OUTBOUND spec-defect report; its body shape and the TERMINAL verification lifecycle with `upstream-confirmed` are in `.claude/rules/cnf-triage.md`). PR escape-hatch labels: `no-changelog`, `no-ui-visual-change`, `no-crate-bump`, `no-conformance-run` (applying one raises a fresh CI run; a re-run of the failed job reuses its stale payload). **Milestones = releases** (`vX.Y.Z`, a delivery promise; a `blocked-upstream` issue carries none); a release is cut at zero open issues (`.claude/rules/changelog.md`).

**Relationships** (full policy + the ONLY sanctioned commands: `.claude/rules/issue-relationships.md`): set every edge with `scripts/gh/rel.sh` (never raw `gh api` — writes need the database id). Sub-issues decompose; milestones stay the release spine (no per-release epic parents); `blocked-by` is real in-repo sequencing; an upstream wait is the `blocked-upstream` label. **An edge lives ONLY in its native panel, never in the issue body**, and a parent's acceptance criteria are outcomes, not a roll-call of children.

**Board** (full policy: `.claude/rules/project-board.md`): a VIEW, never a second tracker. Status is the only board-managed datum; the one manual move is pickup → `scripts/gh/project.sh status <n> in-progress`; park abandoned work to `todo`; never hand-set `Done`, never archive/delete, never raw `gh project item-edit`; an issue leaves the repo ONLY via `scripts/gh/project.sh transfer`. New milestones get a due date. The roadmap lives ONLY in the board readme.

## Model orchestration (workflows & subagents)

**When the session runs on Fable 5 (effort `high`), Fable orchestrates; it does not implement bulk work.** It owns the issue loop, architecture, spec-conformance judgement and the hard bespoke logic (AQL IR→SQL, versioning, validation, the node codec) in-context, and fans file-heavy implementation out to subagents (`Agent` with `model: 'opus'`, or a `Workflow`). The win is context isolation and parallelism, not intelligence, so delegate by the nature of the work, never reflexively.

| model     | cost | intelligence | taste |
|-----------|------|--------------|-------|
| fable-5   | 2    | 9            | 9     |
| opus-5    | 4    | 8            | 8     |

- Only these two models. Never pass Sonnet or Haiku to `Agent`/`Workflow`.
- **Opus workers** take bulk implementation on a clear spec (handlers, DTOs, migrations, sea-query building, test scaffolding) and file-heavy investigation. **Max 2 implementation workers at once** (owner cap). Prefer one worker holding a whole subsystem over splitting a coherent task. Tell workers NOT to spawn subagents; drop "double-check your work" scaffolding (Opus self-verifies).
- **Fable subagents** for delegated work that still needs top intelligence/taste.
- **Reviews and research:** `spec-conformance-reviewer` before committing a spec-facing subsystem; `spec-researcher` for spec questions; `implementer` for bounded code; **every red CNF run goes to `cnf-triage`**; regulation work to the compliance agents (`/law-lookup`, `/compliance-audit`). Hand every agent the governing `docs/specs/openehr/...` (or `docs/law/...`) paths.
- Effort: Fable on `high`; `effort: 'low'` for mechanical worker stages.
- Subagents obey every hard rule below; delegate with a tight spec and verify the result.

## Tech stack

Rust stable **1.98** (1.98.1), MSRV 1.97, **edition 2024**, resolver v3 (`rust-toolchain.toml`). **PostgreSQL 18** (18.6+), extension `btree_gist` only. Core crates: `axum` 0.8 + `tower-http`, `tokio`, `sqlx` 0.9 + `sea-query` 1.0 via `sea-query-sqlx` (never sea-orm), `jiff` (the one time library), `uuid` v7, `thiserror` (libs) / `anyhow` (bins), `tracing` + opentelemetry, `utoipa`, `urlencoding` (ALL percent encoding — never hand-roll a percent codec). **Never hand-roll anything a vetted crate provides** (auth, status codes, OpenAPI, …). The full pinned set is the root `Cargo.toml` `[workspace.dependencies]` (every entry consumed by a member; the manifest wins on any discrepancy); the vetted menu is `.claude/rules/dependencies.md`. Add a crate with `dep.workspace = true`, version verified on crates.io first.

**Served OpenAPI:** the vendored ITS-REST OpenAPI is codegen input for `emit-rest` ONLY and a subordinate wire source (the docs text at `docs/specs/openehr/ITS-REST/` wins; where it is silent the released OAS grounds the expectation; both-silent goes to the ambiguity register). It is NEVER imported or served by `ferroehr-rest`, which serves ONLY its own `#[utoipa::path]`-generated document (composed in `ferroehr-rest::extensions::openapi`). Any surface change updates those declarations in the same PR.

## Build and test

```bash
cargo build --workspace
cargo nextest run --workspace
# clippy = the EXACT CI lanes (viewer excluded from --all-features: hydrate/ssr are mutually exclusive):
cargo clippy --workspace --exclude ferroehr-viewer --all-targets --all-features -- -D warnings
cargo clippy -p ferroehr-viewer --all-targets --features ssr -- -D warnings
cargo clippy -p ferroehr-viewer --target wasm32-unknown-unknown --features hydrate -- -D warnings
cargo fmt --all
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --exclude ferroehr-viewer --all-features --no-deps --document-private-items
RUSTDOCFLAGS="-D warnings" cargo doc -p ferroehr-viewer --features ssr --no-deps
cargo deny check   # subsumes cargo-audit
bash scripts/conformance.sh   # the CNF 2.0 acceptance run → docs/conformance/<sut>/ (baseline numbers live ONLY there)
# perf (hour-plus, exclusive SUT): CONF_PERF_CLASS=POC|S|L|R [CONF_PERF_HOURS=1..12] bash scripts/conformance.sh
# stress / aql-probe (exploration only, never a record): source scripts/lib/veredictum.sh; "$(veredictum_bin)" stress|aql-probe --root "$(veredictum_artifacts)" --ixit docs/conformance/party/<sut>/ixit.json --out <file>
# published perf SVGs regenerate from committed artifacts: bash scripts/render/perf-assets.sh
# CPU profiling: the /flamegraph skill
# viewer gates: /ui-gates · browser journeys: bash scripts/ui-e2e.sh
bash scripts/deploy-probe.sh  # deployment conformance at the far end, each integration off/working/broken; its report names what it did NOT exercise (silence is never coverage);
                              # record = the JSON under docs/conformance/deployment/ (gitignored): read a named CI run's artifact or your own run, never a stale file
```

### Target-dir & warm-build discipline

A cold build is expensive and `target/` can grow to hundreds of GB.

- **ONE `./target` for ALL cargo, the IDE included.** No `CARGO_TARGET_DIR` override, no second target dir anywhere (scratchpad, `/tmp`, worktree). Cargo's lock serializes builds; waiting on it is intended. Subagents do not run cargo in parallel; the orchestrator builds once at convergence. Never `pkill -9 rustc`.
- **Iterate scoped, gate wide:** `cargo clippy -p <crate> --all-targets` + `cargo nextest run -p <crate>` while working; the `--workspace` gates once before commit.
- **Never vary `RUSTFLAGS`, features or profile between runs** — any change rebuilds the world.
- **Disk hygiene:** `du -sh target` at the start of a heavy session and after rewrite-scale changes; above ~30 GB, check `pgrep -fl 'cargo|rustc'` then `cargo clean`.

The spec layer and the application both compile and are clippy-clean; keep every crate you touch green (generated crates by fixing the emitter). The CDR is shipped; the conformance baseline lives in `docs/conformance/ferroehr/`; remaining work is on GitHub Issues.

## Conventions

- **Spell the product `FerroEHR`** wherever a human reads it as a name (UI, titles, headings, display metadata, diagram labels, prose). Lowercase `ferroehr` only for technical identifiers (crate/binary/image names, URLs, the REST base path, `FERROEHR__` env, `ferroehr.toml`, k8s/compose names, DB names, `urn:` forms, the conformance `sut` key, storage keys).
- Dependencies point downward: app (`ferroehr-*`) → spec (`openehr-*`), never the reverse. The app consumes the generated types directly as its domain model — never re-model the RM or re-serialize.
- **Two disciplines:** `openehr-*` = generated, wire + semantic + invariant parity, change the emitter; `ferroehr-*` = idiomatic Rust of our own design on proper crates, verified at the REST/AQL surface by CNF + corpus tests.
- Settled emission choices (do not re-litigate per class): closed subtype sets → untagged `enum`s; recursion → `Box`; `_type` via emitted manual serde; strong types where unambiguous; back-references in `*_impl.rs` use `Weak` or an index, never an owning reference.
- `thiserror` in libraries, `anyhow` only in binaries. No `unwrap`/`expect` outside tests.
- **No `use X as Y` import renaming** (a `use Trait as _;` is fine); a genuine collision gets a qualified path.
- Async-first idiomatic tokio/axum.

## IMPORTANT hard rules

- **The vendored spec text is the oracle.** Before implementing or reviewing spec-facing behaviour (RM semantics, invariants, REST wire, AQL, canonical JSON/XML, templates, terminology) read the section under `docs/specs/openehr/` (`/spec-lookup`) and cross-check the CNF schedule. Never resolve a spec question from memory or EHRbase behaviour; cite file + section.
- **CNF red-run triage is spec-adjudicated** (`.claude/rules/cnf-triage.md`, agent `cnf-triage`). Every red row is attributed BEFORE any fix to exactly one of {application, Veredictum runner `src`, Veredictum catalogue `artifacts`} by three-way comparison of spec-required vs catalogue-expected vs SUT-observed, with the spec citation and the wire exchange as evidence. The spec is never a suspect; **the application is never presumed correct** (it is the most common culprit); never adjust an expectation to match observed behaviour; the SUT response is evidence, NEVER the reference. Spec silence goes to the ambiguity register. Runner/catalogue fixes land in Veredictum and arrive here as a pin bump. **The catalogue's job is TOTAL wire coverage** (`.claude/rules/testing.md` §CNF coverage).
- **Cite ONLY durable references:** the vendored specs (file + section) or official external docs (PostgreSQL docs, Rust book/reference, docs.rs of pinned crates). NEVER an internal markdown as a design authority (a guard naming the `.claude/rules/*.md` it enforces is fine). Where the openEHR specs are SILENT, say so: "no openEHR spec governs this — our own design/extension".
- **Delete a plan/design markdown in the PR that implements it.** The durable record is closed issues, PR descriptions, `CHANGELOG.md`, git history and the living reference docs.
- **Never hand-edit a `// @generated` file.** Edit the emitter or the `*_impl.rs` sibling and regenerate.
- **The generator emits the COMPLETE model; never trim, prune, narrow or suppress output** to quiet a diff or dodge a build error. A generated shape that is wrong or insufficient versus the spec is fixed in `openehr-codegen` + regeneration, NEVER by a shadow type, duplicate model, adapter layer, placeholder or "temporary" local representation in a consumer. A large emitter fix gets an issue; the workaround stays forbidden; an existing workaround gets a removal issue. Full text: `.claude/rules/codegen.md` §The three hard rules.
- **Comments: official Rust conventions (RFC 505 + RFC 1574) with hard budgets, no essays in code** (`.claude/rules/comments.md`, enforced by `scripts/checks/comment-style.sh`). Line comments only; pending work is ONLY `// TODO(#NNNN): <what is missing>`; `// NOTE:` is a settled decision as citation + ONE sentence (max 3 lines); a plain `//` run max 8 lines; `// SAFETY:` reserved for `unsafe` (which is forbidden); no other markers, no phase/plan markers, no prose deferrals. Docs: one-sentence summary, blank line, detail, `# Errors`/`# Panics` sections.
- **Branches:** `<type>/<kebab-slug>`, type ∈ `feat|fix|chore|docs|refactor|perf|test|ci|build|release`. Never force-push `main`. Never delete `docs/plans/WORKLIST.md` or `docs/plans/README.md`.
- **NEVER add AI/Claude attribution to commits, PRs, issues or code — absolute, no exceptions.** No `Co-Authored-By` trailer of any kind, no "Generated with Claude Code" line, emoji or footer, in any commit message, PR title/description/comment, issue or code comment. Commit and PR text describe only the change.
- **Keep the changelog** (`CHANGELOG.md`, Keep a Changelog 1.1.0): every PR with user-visible changes adds an `[Unreleased]` entry in the same PR (`changelog-guard`; rule `.claude/rules/changelog.md`).
- **User docs track the product:** a PR changing the REST surface, `FERROEHR_*` config, the CLI, deployment artifacts or other user-visible behaviour updates the matching `website/book/src` page in the same PR (`.claude/rules/docs-website.md`). The site hosts no OpenAPI reference; API links point at the hosted sandbox's Swagger UI.
- **Never weaken, skip or delete a test** to make a build pass, and never edit a test to route around a bug it exposes.
- **A fuzz crash is fixed in the crate, never in the harness** (`.claude/rules/fuzzing.md`); each fixed crash gets a regression test. `fuzz/` is its own nightly workspace, never a root member.
- **Machine review (SonarQube Cloud) is a second opinion, never authority** (`.claude/rules/ai-code-review.md`): it never outranks the specs, these rules or the local gates, gates no merge, and nothing is applied through a UI that attributes a commit to a bot.
- **Every external corpus is fetched by a committed `scripts/vendor/*.sh` script**, vendored verbatim, provenance-stamped, exercised 100% with adjudicated skips only (`.claude/rules/vendored-corpora.md`, which also holds the CKM paging trap and the ADL 1.4-only fact). Never hand-download or hand-edit a vendored file.
- **Reliability rules are machine-enforced** (`.claude/rules/reliability.md`): every safety rule pairs with the lint or CI check that fails on violation.
- **Record progress on the tracker and commit before ending a session** (a `Stop` hook enforces it).
- **Every application change builds as compiling, tested increments** on the generated crates.
- **The instance is single-tenant:** no tenant column, row policy or session GUC; isolation between organisations is one instance per organisation. RBAC/ABAC (`ferroehr-rest::access`) is shipped. Capabilities not yet built are ordinary tracker work of our own design; upstream EHRbase is read-only prior art at most.

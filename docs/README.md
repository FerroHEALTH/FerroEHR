# docs/ — map of the documentation tree

A pure-Rust, openEHR-spec-conformant CDR (ITS-REST 1.1.0 + AQL 1.1) with
greenfield PG18-native internals. This file says what lives where and which
document is authoritative for what. When two docs disagree, the **newer
decision wins**. The vendored specs are the only doc oracle; live behaviour
is documented in the code + the user website.

## Start here

| Path | What it is | Authoritative for |
|---|---|---|
| The [FerroEHR Roadmap board](https://github.com/FerroHEALTH/FerroEHR/projects) | The public direction + live-status view over the tracker (`.claude/rules/project-board.md`) | where the product goes next |
| GitHub Issues (`gh issue list --state open`) | The open-items tracker (root `CLAUDE.md` §Issue workflow) | what's open + what's active (pinned = current focus) |
| Closed issues + PR descriptions + `CHANGELOG.md` | The build record | the historical record of what shipped |
| The code itself (router → handler → service → SQL) | Per-endpoint call chains — there is no standing endpoint map (a standing map goes stale) | navigation + optimization |

## Decisions

**There is no ADR layer** (owner ruling 2026-07-17). Architectural decisions
live inline, in the durable record: the living reference docs
(`architecture.md`, this tree, `VERSIONS.md`), the closed issues + PR
descriptions, `CHANGELOG.md`, and git history. **No document reads, writes,
or cites an ADR; the only citable references are the vendored specs and
official external documentation.** (The current design in brief: the spec +
ITS layer is generated from the vendored machine-readable specs by
`openehr-codegen`; the application is idiomatic Rust of our own design on
those crates, with its own PG18-native storage and typed AQL engine, four
app crates with zero re-exports, an SM-aligned service layer, and enterprise
capabilities — eventing, multi-tenancy, FHIR connectors, multimedia
externalization; acceptance is the openEHR conformance suite. See
`architecture.md`.)

## The specs (the oracle — do not edit)

- **`specs/openehr/`** — the vendored normative openEHR spec text + the CNF
  Platform Conformance Test Schedule. **The conformance oracle.** Never
  hand-edit; refreshed only by `scripts/vendor/spec-docs.sh`. Use `/spec-lookup`
  / `/spec-audit`. Pins recorded in `VERSIONS.md`.

## Plans

- `plans/` — one deep working plan per open tracker issue that needs one,
  plus `WORKLIST.md` (a delete-protected pointer stub to the tracker).
  Completed plan files are pruned in the PR that lands them, with the close
  recorded in the PR description + issue handoff comment (all in git
  history). See `plans/README.md`.

## Platform + verification

- `architecture.md` — how the system is built and why, including the SM
  Platform Service Model component map.
- `VERSIONS.md` — the single source of truth for every version pin (language,
  PG, openEHR spec components) except third-party Rust crates (root `Cargo.toml`
  wins for those).
- `postgres-features.md` — the PG 17/18 feature delta this CDR exploits.
- `conformance/` — CNF 2.0 pipeline artifacts, one directory per SUT
  (`ferroehr/`: `results.json`, `verdicts.json`). `ehrbase/` +
  `COMPARISON.md` are frozen comparison data until the CNF pipeline re-bases
  the public comparison. Numbers live ONLY in these committed artifacts.
Statement-level profiling evidence is not produced here. `veredictum aql-probe`
seeds a class-scale corpus, fires the measurement machinery's AQL set against
the composed stack, and attributes the database-side cost per statement through
`pg_stat_statements`; its report is exploration evidence and never a
conformance record.

- `benchmarks/` — benchmark records, which are deliberately NOT under
  `conformance/`: conformance is the CNF 2.0 suite
  [Veredictum](https://github.com/rubentalstra/Veredictum) runs.
  `benchmarks/storage/<schema-generation>/record.json` is the storage
  harness's record (`cargo bench -p ferroehr --bench storage`): per-operation
  latencies for the write path, the point reads, AQL CONTAINS, archive/restore
  and prune, with the database-side facts beside them. Records are gitignored
  except a deliberately committed baseline; the shape is documented in
  `benchmarks/storage/README.md`. `benchmarks/cohort/record.json` is the
  cohort-query record — how the demographic predicate and the clinical
  statement behave as the cohort grows — produced by the ignored `cohort_bench`
  test (`COHORT_BENCH_N=100000 cargo nextest run -p ferroehr -E 'test(cohort_bench)'
  --run-ignored all`); the book's AQL page includes the table
  `scripts/render/cohort-bench.sh` renders from it.

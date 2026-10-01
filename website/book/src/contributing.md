# Contributing

FerroEHR is source-available (the Business Source License 1.1 for the
application, Apache 2.0 for the five generated `openehr-*` model crates, and
vendored third-party material under its upstream terms; see
[Licensing & legal](licensing.md)) and welcomes contributions. This
chapter is a short orientation for anyone who wants to file an issue, report a
vulnerability, or open a pull request; the authoritative documents live in the
repository and are linked below. You keep your copyright, and there is no
separate agreement to sign; the terms a contribution lands under are set out
in
[CONTRIBUTING.md](https://github.com/FerroHEALTH/FerroEHR/blob/main/CONTRIBUTING.md)
§ Licensing of contributions.
[Why FerroEHR exists](why-ferroehr.md) explains what the project offers
the organisations that run it and build on it, and what it asks in return.

A bug report with a reproducing request, a
conformance case for behaviour nothing covers yet, a specification ambiguity you
had to resolve in your own integration, a documentation correction, or a
measurement from your own hardware all count.

<!-- toc -->

## Where to start

The three governing documents are kept in the repository root:

- [CONTRIBUTING](https://github.com/FerroHEALTH/FerroEHR/blob/main/CONTRIBUTING.md)
  — the practical rules for setup, the required checks, and pull requests.
- [Code of conduct](https://github.com/FerroHEALTH/FerroEHR/blob/main/CODE_OF_CONDUCT.md)
  — the Contributor Covenant (v2.1) the community follows.
- [Security policy](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md)
  — how to report a vulnerability privately.

## Setting up

The Rust toolchain is pinned by the repository's `rust-toolchain.toml` (stable
1.98.1), so `rustup` installs the right version automatically on your first
build. The declared minimum supported version is lower (Rust 1.97) and CI
verifies it independently with `cargo hack`, so do not reach for a language
feature newer than that. The edition is 2024.

Two extra tools are needed for the full test suite:

- **A PostgreSQL 18 server** for the database-backed tests. The shared test
  harness starts (or re-adopts) one reusable container if Docker is running;
  otherwise point it at a server you already run with `FERROEHR_TEST_PG_URL`
  (its role must be able to `CREATE DATABASE`). See
  [From source → Running the tests](installation/from-source.md#running-the-tests).
- **`xmllint`** (from `libxml2`), used by the canonical-XML parity tests.

Install the shared git hooks once with `bash scripts/install-hooks.sh`.

## The checks every pull request must pass

CI runs the same set of gates locally and on every pull request; none of them
are advisory:

```bash
cargo build --workspace
cargo nextest run --workspace          # unit + integration (real PostgreSQL 18)
cargo fmt --all --check

# clippy is three lanes: the viewer's `hydrate` and `ssr` features are
# mutually exclusive, so it is excluded from the workspace lane and linted
# per-feature on both of its targets.
cargo clippy --workspace --exclude ferroehr-viewer --all-targets --all-features -- -D warnings
cargo clippy -p ferroehr-viewer --all-targets --features ssr -- -D warnings
cargo clippy -p ferroehr-viewer --target wasm32-unknown-unknown --features hydrate -- -D warnings

# rustdoc lints + doctests (the rustdoc lint table is inert without a doc run)
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --exclude ferroehr-viewer \
  --all-features --no-deps --document-private-items
RUSTDOCFLAGS='-D warnings' cargo doc -p ferroehr-viewer --features ssr --no-deps
cargo test --workspace --doc

cargo deny check                       # subsumes cargo-audit: same RustSec DB, plus yanked/licenses/bans/sources
cargo machete                          # unused dependencies
cargo hack check --rust-version --workspace   # the declared MSRV really builds
bash scripts/checks/codegen-drift.sh    # generated layer matches the vendored specs
```

Beyond those, a family of small single-purpose scripts under `scripts/checks/`
runs on every pull request: comment and doc-comment style, default values
declared inline in their struct's `Default` impl, HTTP statuses compared as
types rather than as numbers, licensing declarations and SPDX headers, no Python
anywhere in the tooling, and the documentation-claim gates for this site. Each
is a plain `bash` script you can run yourself, and the failure message names
what to fix.

CI adds a few gates that need more than a checkout: a container smoke test that
composes the built server image against the database image, the browser
end-to-end battery for the viewer (`bash scripts/ui-e2e.sh`), the Helm
chart render and boot lanes, and the changelog, crate-version, and attribution
guards. Viewer-only work has its own local battery; see the repository's
`CONTRIBUTING.md`.

> [!IMPORTANT]
> Two rules are absolute. Never hand-edit a generated file: anything under a
> `// @generated … DO NOT EDIT` header is produced by the code generator; change
> the generator and regenerate instead. And never weaken, skip, or delete a test
> to make a build pass, or edit a test to route around a bug it exposes.

A few more conventions worth knowing before you open a pull request:

- Branch from `main`, and target your pull request at `main`. Branch names
  are `<type>/<slug>` with the conventional-commit types (`feat/…`, `fix/…`,
  `docs/…`, `chore/…`, and so on), and commit subjects use the same types.
- Keep changes focused, and describe **what** changed and **why**. For anything
  that touches openEHR behaviour, cite the relevant specification section.
- Behaviour changes come with tests. Snapshot changes must be reviewed, not
  blindly accepted.
- Any user-visible change (the REST surface, AQL, validation, configuration, the
  CLI, or the deployment artifacts) adds an entry to the changelog **and**
  updates the matching page of this documentation, both in the same pull
  request. CI guards enforce both.

## Personal data

The server keeps clinical content, the identities it belongs to and the map
between them in three separate schemas, reachable by separate database roles. A
change can move that boundary without meaning to, so four rules hold everywhere
in the repository.

- **Synthetic data only:** tests, fixtures, seeds, examples and screenshots use
  invented values. A real name, national identifier, address, phone number,
  email or date of birth never goes into the repository, an issue, or a pull
  request body.
- **No identifiers in telemetry:** logs, traces, metric labels and `Debug`
  output carry record identifiers (an EHR id, a version uid, a template id) and
  shapes, never the content of a subject's data. A `Debug` impl on a type
  holding personal data prints field names rather than field values.
- **No grant across the domains:** a database role reaches one of the three
  pseudonymisation domains, never a second. A migration granting across them
  rejoins the identities to the records the split exists to separate.
- **A review step at the boundary:** a change touching the demographic or
  linkage migrations, the demographic or linkage services, the identifier
  scanner, the access-event model or an outbox payload builder describes its
  data flow, names the roles involved, and ticks the "Privacy boundary"
  checklist in the pull request template.

The `privacy-boundary-guard` job in CI enforces the last rule: a pull request
touching those paths without a ticked checklist fails. The same job reads the
added lines of every diff and fails on a value shaped like a real Dutch
identifier, a nine-digit number passing the BSN eleven-test or a postcode
followed by a house number. A synthetic value that still has that shape carries
`privacy-allow: <reason>` on the same line, and the job proves its own detectors
on every run before it judges a diff.

These are design-time rules because
[GDPR Art. 25](https://eur-lex.europa.eu/eli/reg/2016/679/oj) places data
protection by design in the design phase, before a line of it is deployed.

## Review

Two things review a pull request. The maintainers, who decide; and SonarQube
Cloud, which analyzes every pull request (with CodeQL as the security
scanner beside it).

The analysis is a second opinion, and deliberately nothing more. Its check
is not required and it blocks no merge; if one of its findings is right, the
change is written by hand. A finding that contradicts the vendored openEHR
specification text, the repository's own rules, or a local gate is wrong by
construction, and saying so on the thread is the correct response.

## Profiling: finding where the time goes

Four flamegraph instruments, all built on established crates (the sampling is
[`pprof`](https://docs.rs/pprof/latest/pprof/), the rendering is
[`inferno`](https://docs.rs/inferno/latest/inferno/)). Pick by situation:

- **A running server** (composed stack, staging, production): the
  `GET /management/flamegraph` endpoint; see
  [Operations → Profiling](operations.md#profiling-the-on-demand-cpu-flamegraph).
- **A code path in isolation**: the criterion benches carry a pprof profiler, so
  any bench emits a flamegraph under `--profile-time`:

  ```bash
  cargo bench -p ferroehr --bench aql -- --profile-time 10
  # → target/criterion/<bench>/profile/flamegraph.svg
  ```

- **Async attribution** (a sampled stack under tokio often blames the executor's
  poll loop; a span flame blames the instrumented operation): set
  `telemetry.flame_file = "/tmp/ferroehr.folded"`; the
  [`tracing-flame`](https://docs.rs/tracing-flame/latest/tracing_flame/) layer
  captures span timings as folded stacks, rendered offline:

  ```bash
  cargo install inferno
  inferno-flamegraph < /tmp/ferroehr.folded > span-flame.svg
  ```

- **A whole local binary run** (no code changes needed):
  [`cargo flamegraph`](https://crates.io/crates/flamegraph), a dev tool, not a
  dependency (`cargo install flamegraph`):

  ```bash
  cargo flamegraph --bin ferroehr            # Linux: perf; add -F 999 for finer sampling
  cargo flamegraph --bench aql -- --bench    # profile a bench run end to end
  ```

  On macOS it uses `dtrace`, which needs elevated permissions: run with
  `sudo cargo flamegraph …` or grant your terminal Developer-Tools access; on
  Linux you may need `perf` installed and `kernel.perf_event_paranoid` ≤ 2.

## Reporting issues and vulnerabilities

Use the GitHub issue tracker for bugs and feature requests.

> [!WARNING]
> Do **not** open a public issue for a suspected security vulnerability. Report
> it privately through
> [GitHub's private vulnerability reporting](https://github.com/FerroHEALTH/FerroEHR/security/advisories/new)
> ("Report a vulnerability" on the repository's Security tab). Because the server
> handles PHI-class data by design, reports about data exposure through the API,
> AQL, telemetry, or the audit trail are in scope even when they look like "just
> configuration". Coordinated disclosure is preferred; please allow a reasonable
> window for a fix before publishing details.

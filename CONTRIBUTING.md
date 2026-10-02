# Contributing to FerroEHR

Thank you for your interest in contributing. This document covers the practical
rules; the architectural ground rules live in
[`docs/architecture.md`](docs/architecture.md) and the root `CLAUDE.md`, and
the project's position (why it exists, and what it offers the organisations
that run it and build on it) is
[Why FerroEHR exists](https://ferroehr.eu/docs/latest/why-ferroehr.html).

You keep your copyright, and there is no separate agreement to sign; the terms
your contribution lands under are set out in
[Licensing of contributions](#licensing-of-contributions) below. Contributions
are not only code: a bug report with a reproducing request, a conformance case
for uncovered behaviour, a specification ambiguity you had to resolve, a
documentation correction, or measurement from your own hardware all count.

## Before you start

- **Read [`docs/architecture.md`](docs/architecture.md) first.** It explains
  the two-layer split (generated `openehr-*` spec crates, hand-written
  `ferroehr-*` application) every change must respect.
- For anything spec-facing (RM semantics, REST wire behaviour, AQL, canonical
  JSON/XML, templates, terminology), **the vendored openEHR specification text at
  [`docs/specs/openehr/`](docs/specs/openehr/) is the authority.** Cite the spec
  file and section in your PR description. EHRbase and other CDRs are prior art,
  not oracles.

## Setup

- The Rust toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml);
  `rustup` installs it automatically on first build.
- Docker is required for the PostgreSQL 18 integration tests (testcontainers).
- `xmllint` (`libxml2-utils` on Debian/Ubuntu, part of libxml2 on macOS) is
  required by the canonical-XML parity tests.
- Install the shared git hooks once: `bash scripts/install-hooks.sh`.

## The gates (every PR must pass all of them)

```shell
cargo build --workspace
cargo nextest run --workspace          # unit + integration (real PG 18 via Docker)
cargo test --workspace --doc
# clippy is THREE lanes: the viewer's hydrate/ssr features are mutually
# exclusive (compile_error!-guarded), so it is excluded from --all-features
# and linted per feature + per target:
cargo clippy --workspace --exclude ferroehr-viewer --all-targets --all-features -- -D warnings
cargo clippy -p ferroehr-viewer --all-targets --features ssr -- -D warnings
cargo clippy -p ferroehr-viewer --target wasm32-unknown-unknown --features hydrate -- -D warnings
cargo fmt --all --check
cargo deny check && cargo machete      # deny subsumes cargo-audit (same RustSec DB + yanked/licenses/bans/sources)
cargo hack check --rust-version --workspace  # the declared MSRV actually builds
bash scripts/checks/codegen-drift.sh    # generated layer matches the vendored specs
```

CI runs the same set, plus the `scripts/checks/*` guard family (comment
style, inline `Default` values, typed HTTP status comparisons, SPDX headers
+ licensing declarations, docs claims, no-Python) and the Helm
render/boot lanes; nothing is advisory.

## Hard rules

- **Never hand-edit a generated file.** Everything under a `// @generated … DO NOT
  EDIT` header (the `openehr-base`/`openehr-rm`/`openehr-am` crates, the generated
  parts of `openehr-its`) is produced by `openehr-codegen`. Change the emitter (or
  a sibling `*_impl.rs`) and regenerate:
  `cargo run -p openehr-codegen -- emit` / `emit-xml` / `emit-rest`.
- **Never weaken, skip, or delete a test** to make a build pass, and never edit a
  test to route around a bug it exposes.
- Dependencies are added only from the root `Cargo.toml`
  `[workspace.dependencies]` (`dep.workspace = true`); don't hand-pin versions in
  member crates, and don't hand-roll what a pinned crate already provides.
- `thiserror` in library crates, `anyhow` only in the binary; no
  `unwrap`/`expect` outside tests.
- Application crates (`ferroehr-*`) consume the generated `openehr-*` types
  directly — never re-model the RM or re-serialize.

## Personal data

FerroEHR keeps clinical content, the identities it belongs to and the map
between them in three separate schemas, and a change can move that boundary
without meaning to. Four rules hold everywhere in this repository:

- **Synthetic data only:** tests, fixtures, seeds, examples and screenshots use
  invented values. Never put a real name, national identifier, address, phone
  number, email or date of birth into the repository, an issue, or a pull
  request body.
- **No identifiers in telemetry:** logs, traces, metric labels and `Debug`
  output carry record identifiers (an EHR id, a version uid, a template id) and
  shapes. They never carry the content of a subject's data, so a `Debug` impl on
  a type holding personal data prints field names rather than field values.
- **No grant across the domains:** a database role reaches one of the three
  pseudonymisation domains, never a second. A migration that grants across them
  rejoins the identities to the records the split exists to separate.
- **A review step at the boundary:** a change touching the demographic or
  linkage migrations, `service::demographic`, `service::linkage`, the identifier
  scanner, the access-event model or an outbox payload builder describes its
  data flow, names the roles involved, and ticks the "Privacy boundary"
  checklist in the pull request template.

The `privacy-boundary-guard` CI job enforces the last rule and refuses a pull
request whose checklist is missing or unticked. It also reads the added lines of
every diff and fails on a value shaped like a real Dutch identifier, so a
deliberately synthetic value that still has that shape carries
`privacy-allow: <reason>` on the same line. These are design-time rules
because GDPR Art. 25 places data protection by design in the design phase
([Regulation (EU) 2016/679](https://eur-lex.europa.eu/eli/reg/2016/679/oj)).
The same rules bind the agents that work in this repository
(`.claude/rules/personal-data.md`).

## Pull requests

- Branch from `main`; PRs target `main`.
- Keep PRs focused; describe **what** changed and **why**, citing spec sections
  for conformance-relevant behaviour.
- Tests accompany behaviour changes. Snapshot changes (`insta`) must be reviewed,
  not blindly accepted.
- Commit messages describe the change itself (conventional-commit style subjects
  like `feat: …`, `fix: …`, `docs: …` are used throughout the history).

## Licensing of contributions

FerroEHR's own code is licensed under the Business Source License 1.1 ([`LICENSE`](LICENSE)). By submitting a contribution you:

1. certify that you wrote it, or otherwise have the right to submit it under these terms;
2. license it under the Business Source License 1.1 as applied to the version it lands in, including that version's Change License, so it becomes Apache 2.0 with the rest of that version. A contribution to one of the five generated model crates (`openehr-base`, `openehr-rm`, `openehr-am`, `openehr-lang`, `openehr-term`) is licensed under that crate's Apache License 2.0 instead; and
3. grant the Licensor named in `LICENSE` a perpetual, irrevocable, worldwide, royalty-free, transferable right to use, reproduce, modify, distribute, sublicense and relicense the contribution as part of the Licensed Work under any terms, including commercial licences.

You keep your copyright. Point 3 is what lets the Licensed Work stay one work with one licensor: a commercial licence, a change of the licence parameters, or a transfer of the project can then cover every line, not only the maintainer's own. There is no separate agreement to sign: the pull request template carries a checkbox recording your acceptance of these terms, and a pull request from a person does not merge without it (the `contribution-licence-guard` CI job).

## Reporting issues

Use the GitHub issue tracker; the chooser offers a form per kind (bug, enhancement,
documentation defect, regulation or jurisdiction, task), each form sets the issue's type
(Bug, Feature or Task), and each asks for what triage needs first. Maintainers set the
priority and the effort at triage. A question is a Discussion, not an issue. For suspected security
vulnerabilities, **do not open a public issue**; see [SECURITY.md](SECURITY.md).

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).

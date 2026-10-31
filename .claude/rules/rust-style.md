---
paths: ["crates/**/*.rs", "app/**/*.rs", "tools/**/*.rs"]
---

# Rust style — idiomatic application code

Applies to hand-written `.rs`: the application (`ferroehr`, `ferroehr-rest`,
`ferroehr-server`, `ferroehr-viewer`), the tools (`testkit`,
`openehr-codegen`), the hand-written spec
crates (`openehr-its`, `openehr-term`, `openehr-query`, the
tooling crates), and `*_impl.rs` behaviour files. **The application is modern idiomatic Rust of our own design, built on the
generated `openehr-*` crates**. The openEHR specifications are
the authority; EHRbase and other CDRs are prior art only.

**Generated files are off-limits.** The openEHR spec/ITS crates (`openehr-base`,
`openehr-rm`, `openehr-am`, and the generated code in `openehr-its`) are produced
by `openehr-codegen`; every `// @generated` file **must never be hand-edited** —
change the emitter and regenerate (`cargo run -p openehr-codegen -- emit`/
`emit-xml`/`emit-rest`).

## Build idiomatic, compiling, tested code

- **Consume the generated crates directly** as the domain model (`openehr-rm`,
  `openehr-am`, `openehr-term`, `openehr-query`, `openehr-its`). Never re-model
  the RM or re-serialize.
- **Use proper crates; don't hand-roll** what the ecosystem provides — `axum`/
  `tower-http`, `sqlx`+`sea-query`, `ferrohealth-smart`/`openidconnect`/
  `argon2`, `utoipa`, `moka`, `jiff`, `uuid`. Add deps only from
  `Cargo.toml [workspace.dependencies]` (`dep.workspace = true`).
- **Every crate you touch compiles + is clippy-clean + tested before you move
  on.**
- **Read the vendored spec first for any spec-facing behaviour** — the
  normative text + CNF schedule live at `docs/specs/openehr/` (use
  `/spec-lookup`; full rule in `spec-adherence.md`).
- **The specs are the authority; design the bespoke logic ourselves** (AQL
  engine, versioning, validation, node codec), verified by the CNF
  conformance suite + corpus tests. Consult prior art (upstream EHRbase, other
  CDRs) when useful; never port it blindly.

## Comments & documentation

The comment/doc-comment discipline lives in **`comments.md`** (RFC 505 +
RFC 1574): line comments only, `// TODO(#NNNN):` / `// NOTE:` / `// SAFETY:`
as the only markers, NOTE = citation + one sentence (≤3 lines), `//` runs
≤8 lines, doc-comment summary-line + section conventions. Enforced by
`scripts/checks/comment-style.sh` (hook + CI) and
`clippy::too_long_first_doc_paragraph`.

## Default values live in the struct's `Default` impl (owner directive 2026-08-06)

The shape is RFC 3681's
(<https://rust-lang.github.io/rfcs/3681-default-field-values.html>), written by
hand: **one** `impl Default` per struct, every default value inline, and
container-level `#[serde(default)]` so serde fills omitted fields from it.

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OidcConfig {
    pub issuer: String,
    pub clock_skew_leeway_seconds: u64,
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            clock_skew_leeway_seconds: 60,
        }
    }
}
```

The RFC's own syntax (`clock_skew_leeway_seconds: u64 = 60`) is **nightly-only**
— feature `default_field_values`, tracking issue
<https://github.com/rust-lang/rust/issues/132162>, implemented with no
stabilization PR — and this project pins stable (`docs/VERSIONS.md`), so the
expansion above is the form we write. When the feature stabilizes, the
declaration replaces the `impl` block and the guard's rules stop mattering; do
not adopt it before then, and do not reach for nightly to get it.

Three forms are banned, because each one puts a field's default value somewhere
other than the field's own struct:

- **`#[serde(default = "path")]`** — the per-field path form. The default then
  lives in a function, so `Default::default()` and a deserialized value can
  silently disagree about the same field. Container-level `#[serde(default)]`
  (no path) is the required form and reads the one `Default` impl.
- **`fn default_x() -> T`** — a zero-argument constructor that exists to be one
  field's default. (A function that takes arguments and happens to start with
  the word — `default_provider(&self)`, `default_committer(&self)` — is an
  ordinary domain function and is fine.)
- **`const DEFAULT_X` with a single reader** — a constant that nothing shares is
  a default value spelled far from its struct.

A `const` with MORE THAN ONE consumer stays a constant and may be referenced
from inside the `Default` impl: a spec-fixed value with several readers
(`service::DEFAULT_SYSTEM_ID`) is a single source of truth, which is the
opposite of the problem above.

Enforcement (tier 4): `scripts/checks/default-style.sh` — per-edit via the
`rust_fmt_clippy.sh` hook, per-PR via the `default-style` CI job (`--all`).

## HTTP statuses are compared as types (owner directive 2026-08-06)

An HTTP status is a `StatusCode`, and it is compared as one:

```rust
if status == StatusCode::OK { … }          // yes
if status.as_u16() == 200 { … }            // no
```

`http::StatusCode` names every registered code
(<https://docs.rs/http/latest/http/status/struct.StatusCode.html>), so there is
always a constant. A numeric comparison throws away the type the crate exists to
provide, and a bare literal tells a reader nothing about which member of a family
was meant — `403` versus `404` is a one-character typo the compiler cannot catch.

Rendering the number stays legal, because that is not a comparison: a log field, a
metric label, a recorded wire outcome, a `/ 100` class bucket. Only comparison
against a numeric literal is refused.

Enforcement (tier 4): `scripts/checks/typed-status.sh` — per-edit via the
`rust_fmt_clippy.sh` hook, per-PR via the `default-style` CI job (`--all`, since
the tree has no violations).

## RFC-grounded conventions and where each one is enforced

Surveyed 2026-08-06 against the accepted-RFC corpus (<https://github.com/rust-lang/rfcs/tree/master/text>),
each item's stabilization status verified first-hand. The point of the table is
that a convention with no check is labelled as such rather than assumed.

| RFC | The rule | Enforcement |
|---|---|---|
| 0505 + 1574 | comment/doc conventions | `comments.md` + `scripts/checks/comment-style.sh` + `too_long_first_doc_paragraph` |
| 3681 | a field's default lives inline in its `Default` impl | §Default values above + `scripts/checks/default-style.sh` |
| 3107 | `#[derive(Default)]` + `#[default]` where the default is a VARIANT | `clippy::derivable_impls` (deny) — hand-write `impl Default` only for VALUES |
| 0199 / 0344 / 0430 | `as_`/`to_`/`into_` cost conventions, naming | `clippy::wrong_self_convention` (`clippy::all`, deny) + rustc naming lints |
| 1940 | `#[must_use]` on functions whose result is the point | `clippy::must_use_candidate` (pedantic → deny) |
| 2383 | every suppression carries `reason = "…"` | `clippy::allow_attributes_without_reason` (deny) |
| 1946 | intra-doc links over bare paths | `rustdoc::broken_intra_doc_links` (deny) + the CI doc job |
| 3013 | no typo'd `cfg` predicates | `unexpected_cfgs` (deny) |
| 3373 | no `impl` inside a fn body | `non_local_definitions` (deny) |
| 3389 | lints configured in the manifest, not `#![allow]` sprinkles | the `[workspace.lints]` table IS this |
| — | zero re-exports: every import names its defining module | `clippy::pub_use` (deny) — the generated crates' preludes carry an emitter-stamped `#![expect]` |
| 0201 + 0236 | **an error carries its cause** (`Error::source`), and prints it once: the cause is the source and stays out of the variant's own message, or the variant is `#[error(transparent)]` | `scripts/checks/error-source-chain.sh` — a per-tree RATCHET on flattened causes (`map_err(\|e\| …to_string())`, counts may only fall) and a HARD ZERO on double prints (an `#[error]` format string naming a `#[from]`/`#[source]`/`source` field), whose detector proves itself with its `--self-test` on every run |
| 2294 | `if let` guards on match arms (stable 1.95.0, one release under our MSRV) | review-only — a genuine wish, labelled as one |

**Rejected on merit, so nobody re-files them as gaps:**

- `clippy::self_named_module_files` (RFC 2126 module conventions) — 25 files
  match, ALL generated. `composition/composition.rs` is the emitter mirroring
  the BMM package `composition` and its class `COMPOSITION`; converting it to
  `mod.rs` would merge the class module into the package module and change
  published paths (`openehr_rm::v1_2::composition::composition::Composition`).
  The spec-mirroring layout wins.
- `clippy::exhaustive_enums` (RFC 2008) and RFC 0356 — both adjudicated in
  `reliability.md` §Recorded deviations.

**The one real gap: error chaining.** RFC 0201 makes the cause chain part of
the `Error` contract, and 109 `map_err(|e| Variant(e.to_string()))` sites
flatten it against 48 that carry `#[source]`/`#[from]`. A stringified cause
cannot be walked, matched, or logged structurally — the same silent-context-loss
class `reliability.md` legislates for `Result → Option`, except this one CAN be
checked. New code carries the source; the sweep is tracked.

## Type and error conventions

- `thiserror` error enums in library crates; `anyhow` only in the `ferroehr`
  binary. No `unwrap`/`expect` outside `#[cfg(test)]`; `todo!()` is denied
  workspace-wide (owner rule 2026-07-12) — an unready dependency gets a typed
  error or real code, never a panic placeholder.
- Closed openEHR subtype sets are already Rust `enum`s in `openehr-rm`; consume
  them. Trait objects only for genuinely open, runtime polymorphism.
- Back-references use `Weak<..>` or an index, never an owning reference; recursive
  containment is boxed (the generated crates already do this).
- `std::sync::LazyLock` (edition 2024) for statics, not `once_cell`.
- **No `use X as Y` import renaming (owner hard rule, 2026-07-11).** Import
  types under their direct names. An alias papers over a naming problem —
  if the name is bad, FIX THE NAME at its definition; if two imports
  genuinely collide, qualify one at the use site (full path) instead of
  renaming. Only alias in highly exceptional cases where no other solution
  exists, with a comment saying why. (Trait imports as `use Trait as _;`
  are not renames and are fine.)
- Edition 2024, resolver v3, MSRV 1.98. `cargo fmt` clean; run `cargo clippy` on
  the crate you touched before considering it done.
- **Suppressions are `#[expect(lint, reason = "…")]`**, scoped to the smallest
  item; `#[allow(lint, reason = "…")]` only for cfg/feature-conditional fire
  (full policy: reliability.md §When a lint fights a legitimate case —
  `allow_attributes_without_reason` is deny). The one sanctioned
  logically-impossible-`Err` escape is also there (Book ch9 shape:
  should-phrased message + inspection-proving reason).

## Documentation (missing_docs is enforced workspace-wide)

- Every public item carries a doc comment (rustc `missing_docs`; generated
  crates get theirs from the emitter — never hand-edit `// @generated`).
  Shape, sections, and summary-line rules: `comments.md` (RFC 1574).
- Intra-doc links (`[`Type`]`) resolve in the scope of the module where the
  item is DEFINED — which under the zero-re-exports rule is also where
  readers import it from. `rustdoc::broken_intra_doc_links` is deny; the CI
  doc job is the gate. Literal square brackets in prose are escaped `\[…\]`.
- `#[doc(alias = "EHR_STATUS")]`-style aliases on spec-named types are
  encouraged — rustdoc search then finds the Rust type from the openEHR
  spelling.

## Edition-2024 standing guidance (behaviour that compiles fine and differs)

- **`if let` scrutinee temporaries drop before `else`** — the guard rule
  lives in reliability.md; rewrite as `match` when a guard must span arms.
- **Never-type fallback**: `f()?;` on a fn generic over the `Ok` type can
  now infer `!` — annotate the turbofish/binding type at such call sites
  instead of leaning on inference
  (https://doc.rust-lang.org/edition-guide/rust-2024/never-type-fallback.html).
- **RPIT captures every in-scope lifetime by default** — when a returned
  `impl Trait` must NOT capture one, say so with precise capturing
  (`use<…>`); the old `Captures<..>` trick is obsolete
  (https://doc.rust-lang.org/edition-guide/rust-2024/rpit-lifetime-capture.html).
- **`Future`/`IntoFuture` are in the prelude** — a collision with a local
  trait method is resolved with fully-qualified syntax
  (`<T as MyTrait>::poll(…)`), never an import rename (the no-alias rule
  stands).
- The `unsafe_*` 2024 items are all moot under `unsafe_code = "forbid"`;
  `static_mut_refs` is compiler-enforced and the `LazyLock` convention
  already complies. Do not re-litigate them.

## Code is written once

No specification governs this: our own design (#4454, ported from
VernumBOEK's rule of the same name).

- **Share knowledge and decisions, never text that only looks alike.** Two
  copies that change for the same reason (one business rule, one wire rule,
  one security rule) move to one owner in the same change. Two pieces with
  different reasons to change stay apart, however alike they read.
- **The owner is the module that owns the concern.** Across crates it is the
  crate lower in the layering (an `openehr-*` crate below the `ferroehr-*`
  crates, `ferroehr` below `ferroehr-rest` and `ferroehr-viewer`). Within a
  crate it is the module whose part of the domain the code serves. It is
  never a `util`, `utils`, `helpers` or `common` module, whose name says
  nothing about what it holds, and never a function that takes a flag to
  choose between two behaviours.
- **One copy is fine, two are looked at, three are extracted.**
- **The ratchet measures it.** `scripts/checks/duplication.sh` runs jscpd
  (pinned by version and image digest in `docs/VERSIONS.md`, CI tools) over
  `app/*/src/`, `crates/*/src/` and `tools/*/src/` in the `duplication` CI
  job and holds the total of duplicated lines to the count in
  `scripts/checks/duplication.count`. A rise fails and names both places of
  each new block. A fall fails too, until you run
  `scripts/checks/duplication.sh --update` and commit the lower count with
  `scripts/checks/duplication-baseline.txt`, the guard's own key of every
  counted block: its two paths and the sha256 of the lines at each, with no
  line number, so a change that leaves a block's lines alone leaves its key
  alone. `--update` never raises the count; a rise that has to stay is a
  hand edit of the file in a change that says why. Locally the guard takes a
  `jscpd` on `PATH` of the pinned version, then the pinned image through
  docker or podman, then the release binary
  `scripts/checks/duplication.sh --install` puts under `target/tools/`.
- **The threshold is 50 tokens over at least 5 lines**, jscpd's own
  defaults. The scan leaves out the `use` declarations, a file that opens
  with `// @generated`, a file from its first column-one `#[cfg(test)]` line
  on (the inline test module), and `tests/`. Measured on 2026-10-31 at
  `80d1dfdd` with those exclusions, over 650 files and 341,621 lines:

  | Threshold | Blocks | Duplicated lines | Blocks across two files | Blocks in an OpenAPI `responses(...)` list |
  | --- | --- | --- | --- | --- |
  | 50 tokens, 5 lines | 1,284 | 20,731 | 348 | 258 |
  | 70 tokens, 5 lines | 724 | 16,089 | 138 | 203 |
  | 100 tokens, 5 lines | 386 | 12,413 | 58 | 158 |

  The last column counts the blocks that carry a `(status = NNN` line: the
  response list of a `#[utoipa::path]` handler in `ferroehr-rest`, written
  out per handler. Raising the threshold drops the short blocks (two
  getters, two `format!` helpers, two match arms written side by side) and
  keeps the response lists (158 of 386 blocks at 100 tokens) and whole
  functions, yet each short
  block is code written twice as well, so 50 it is. Raising the threshold
  is a change to this section and to the guard together, with the table
  measured again.
- **A hand-written file carries at most 800 lines**, its inline test module
  included. `scripts/checks/file-size.sh` measures every tracked `.rs` file
  under `app/*/src/`, `crates/*/src/` and `tools/*/src/` that does not open
  with `// @generated`, in the `file-size` CI job and per edit through
  `.claude/hooks/rust_fmt_clippy.sh`. The 113 files over the budget when the
  guard landed stand in `scripts/checks/file-size.list` with their length;
  a listed file may shrink and never grow, and a shrink fails until
  `--update` lowers its entry, so the list only tightens. A file over the
  budget is split by concern.

## No Python, anywhere (owner hard rule, 2026-08-10)

The tooling languages are **bash and Rust**. Python is banned across the
repository — standalone scripts, and especially embedded in shell.

Embedded is worse than either language alone, and not as a matter of taste: a
heredoc nested inside a command substitution makes bash scan the *Python* for
quote pairs, so a single apostrophe in a Python comment breaks the whole
script with an error pointing at the wrong line. That is not hypothetical — it
happened while writing `scripts/render/zenodo-json.sh`, which is now bash and
`jq` only.

What to reach for instead:

| Job | Tool |
|---|---|
| JSON read/build | `jq` |
| Line scans, field extraction | `awk`, `sed`, `grep` |
| Anything with real data structures | Rust, in `tools/` |

Enforcement (tier 4): `scripts/checks/no-python.sh`, per-PR via the
`no-python` CI job. It fails on any `python`/`python3`/`pipx`/`pip` invocation
and on any `.py` file under `scripts/`, `.github/workflows/`,
`.claude/hooks/`, `deploy/` and `docker/`; prose *about* not using Python is
allowed, since several scripts carry exactly that comment. Mutation-proven in
both directions. (`pipx`/`pip` joined the pattern in #2791: `pipx run
cffconvert` validated CITATION.cff for months — Python with no `python` token
to see. The validation is now the vendored CFF 1.2.0 schema +
`yq -o=json` + the pinned Rust `jsonschema-cli`, same schema file cffconvert
used, in the citation-guard job.)

**There are no exemptions.** The last two were the Helm selector-immutability
and restricted-profile gates, which parsed multi-document YAML; they are now
`yq -o=json | jq` gate programs under `deploy/helm/gates/`, with every
assertion re-proven against the same mutations the Python caught.

## What not to do

- Do not port JVM plumbing (classloaders, Spring context internals, PF4J) —
  design the idiomatic Rust equivalent (tower middleware, axum state, a Rust
  plugin model) or register it as its own tracker issue with a `// TODO(#NNNN):`.
- Do not hand-edit generated code; do not re-model what `openehr-*` provides.

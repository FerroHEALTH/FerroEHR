# Fuzzing the parsers that read untrusted bytes

libFuzzer harnesses (via [cargo-fuzz]) for every parser in this repository that
reads bytes an attacker controls off the network. Each harness is a **pure parse
of `&[u8]`** — no I/O, no database, no network, no global mutable state — so a
finding is always reproducible from the recorded input alone.

Why it exists rather than only property tests: `proptest` explores the *valid*
space and proves round-trip and structural invariants. A missing nesting bound in
the XML reader lived in the *malformed* space — a deeply nested document recursed
the generated `FromXml` impls off the stack, and a Rust stack overflow **aborts**
the process instead of unwinding, so the `tower-http` catch-panic layer that
renders this server's clean `500` could not intercept it. One request would have
taken the process down for every caller. That is the class this directory exists
to catch.

## The targets

| target | reads | entry points |
|---|---|---|
| `canonical_json` | every REST write body | `openehr_its::json` typed reader, the negotiation wire door, the RM/terminology validators |
| `canonical_xml` | the same writes via content negotiation, plus archetype XML | `openehr_its::xml::runtime::XmlReader`, `from_canonical_xml`, `openehr_its::aom2`/`aom2_model` |
| `aql_query` | arbitrary query text from any authenticated caller | `openehr_query::parser::parse_str`, plus the printer's documented `parse(to_aql(ast)) == ast` invariant |
| `simplified_formats` | FLAT / STRUCTURED composition bodies | `openehr_sdt::flat::sim`, `openehr_sdt::flat::convert` |
| `adl2_source` | archetype uploads, both dialects | `openehr_adl::source::parse_source` |
| `opt14_template` | operational-template uploads | `openehr_its::opt14::from_xml`, then `build_web_template` |
| `identifiers` | the `{version_uid}` PATH parameter, reached before any body, negotiation or authorization | `ObjectVersionId`/`HierObjectId`/`ArchetypeId`/`VersionTreeId` `from_str`, plus the composite's recomposition contract |

## Seeds

`fuzz/seeds.sh` builds `fuzz/seeds/<target>/` from corpora that are **already
committed and provenance-stamped**, as symlinks — the archetype and template
packs are around 100 MB and are never copied. Selection is size-bounded and
deterministic, because libFuzzer re-reads every seed on each run and derives its
default input length from the largest one.

```sh
fuzz/seeds.sh                 # all targets
fuzz/seeds.sh canonical_xml   # one target
```

The script fails loud if a source directory has moved, so a renamed corpus can
never quietly degrade into an empty seed set. Two AQL sources are extracted
rather than linked: the official worked-example queries inside the AsciiDoc
listing blocks of the vendored QUERY spec examples (the same extraction the
`openehr-query` corpus test performs), and the query text of the CNF catalogue's
cases.

## Recorded regressions

`fuzz/seeds/` is generated output and is **gitignored**; `fuzz/regressions/` is
the tracked half. Every input that reproduced a real crash, leak or timeout is
committed under `fuzz/regressions/<target>/` and linked into that target's seed
directory by `seeds.sh`, after the wipe — so a finding is re-checked by every
run, forever, instead of living on one machine until the next `seeds.sh`.

The corpus is not a substitute for a test: a fixed defect is also pinned by a
regression test in the crate that owns the code
(`.claude/rules/fuzzing.md`). Reproducing one artifact directly — name the
FILE, which libFuzzer runs once and exits (`FUZZ_NIGHTLY` is the pinned nightly,
set as in [Running](#running)):

```sh
cargo +"$FUZZ_NIGHTLY" fuzz run -s none aql_query \
  fuzz/regressions/aql_query/timeout_2758_nested_predicates.aql
```

Never pass `fuzz/regressions/<target>/` as the first corpus argument: libFuzzer
treats its first corpus directory as **writable** and fills it with generated
inputs, which is exactly what this directory must not accumulate. The seed
directory reaches it read-only, as the second corpus argument (below).

## Running

`cargo-fuzz` needs a **nightly** toolchain, and `fuzz/` is therefore its own
workspace: no ordinary `cargo build`, `cargo clippy` or `cargo nextest` run over
`crates/*`, `app/*` or `tools/*` ever sees this package, and the CDR workspace
stays on its pinned stable toolchain (the `Rust toolchain` row of
`docs/VERSIONS.md`).

Use the dated nightly that CI pins, never the moving `nightly` channel: a regression
CI was pinned away from would otherwise reach your local run. The pin is the
`Fuzz toolchain` row of `docs/VERSIONS.md`, the `FUZZ_NIGHTLY` value of
`.github/workflows/fuzz.yml`, and the commands below read it from there:

```sh
FUZZ_NIGHTLY=$(sed -nE 's/^  FUZZ_NIGHTLY: (nightly-[0-9-]+)$/\1/p' .github/workflows/fuzz.yml)
rustup toolchain install "$FUZZ_NIGHTLY"
cargo install cargo-fuzz --locked

fuzz/seeds.sh

# A short run, the shape CI uses.
cargo +"$FUZZ_NIGHTLY" fuzz run canonical_xml fuzz/corpus/canonical_xml fuzz/seeds/canonical_xml \
  -- -max_total_time=300 -max_len=32768 -timeout=25

# A long local campaign: several cores, no time limit, until you stop it.
cargo +"$FUZZ_NIGHTLY" fuzz run --jobs 8 canonical_xml \
  fuzz/corpus/canonical_xml fuzz/seeds/canonical_xml \
  -- -max_len=32768 -timeout=25
```

`fuzz/seeds.sh` also creates `fuzz/corpus/<target>/`, because libFuzzer refuses
to start when a corpus directory it was given does not exist.

The seed directory is passed as a **second** corpus argument on purpose:
libFuzzer writes only to the first one, so the committed corpora stay read-only
inputs and `fuzz/corpus/<target>/` accumulates everything new.

Coverage of a corpus, to see what a campaign actually reached:

```sh
cargo +"$FUZZ_NIGHTLY" fuzz coverage canonical_xml fuzz/corpus/canonical_xml
```

## When a target crashes

A crash writes its input to `fuzz/artifacts/<target>/`. The sequence is fixed:

1. **Minimize** it — `cargo +"$FUZZ_NIGHTLY" fuzz tmin <target> fuzz/artifacts/<target>/<input>`.
2. **Fix the defect** in the crate that owns the parser. For a `// @generated`
   file that means an `openehr-codegen` emitter change plus regeneration, never a
   hand-edit.
3. **Pin the refusal** as an asserted regression test in that crate's normal test
   tree, the way `crates/openehr-its/tests/it/xml_hostile_input.rs` pins the
   DOCTYPE and depth refusals. A fuzz finding that is only fixed, not pinned,
   comes back.

Step 3 is the one that matters: the crash artifact itself is not a test, it is a
scratch file, and `fuzz/artifacts/` is not tracked.

## CI

`.github/workflows/fuzz.yml` runs a bounded campaign per target on a nightly
schedule (and on `workflow_dispatch`, where the per-target duration is an input),
with each corpus persisted in the Actions cache between runs so coverage
accumulates. It is deliberately not on the pull-request path.

## Working in an IDE

`fuzz/` is a separate Cargo workspace, so an IDE that only knows the root
workspace reports "file does not belong to a known Cargo project" for the
harnesses. Attach `fuzz/Cargo.toml` as a second Cargo project (in RustRover:
*File → Link Cargo Project*). Nothing in the repository needs to change.

[cargo-fuzz]: https://rust-fuzz.github.io/book/cargo-fuzz.html

# Rust crates

FerroEHR's openEHR specification layer is published on crates.io as nine
standalone Rust crates, usable without the CDR. They are the same crates the
server itself is built on: the spec types are generated deterministically from
the official openEHR machine-readable artifacts (BMM/XSD/OpenAPI), and the
engines (ADL, AQL, Simplified Formats, serialization runtimes) are hand-written against the
vendored specification text.

| Crate | Implements | What it gives you |
|---|---|---|
| [`openehr-base`](https://crates.io/crates/openehr-base) | BASE 1.2.0 + 1.3.0 | Foundation + base types (identification, intervals, ISO-8601 types) |
| [`openehr-rm`](https://crates.io/crates/openehr-rm) | RM 1.1.0 + 1.2.0 | The Reference Model: `COMPOSITION`, data structures/types, change control, generated invariant validation, the static RM attribute model |
| [`openehr-am`](https://crates.io/crates/openehr-am) | AM 1.4.0 + 2.4.0 | The Archetype Object Model, both majors side by side |
| [`openehr-adl`](https://crates.io/crates/openehr-adl) | ADL 2.4.0 | ADL2/cADL/ODIN parser, AOM2 validation, flattener, OPT2, ADL 1.4→2 conversion |
| [`openehr-term`](https://crates.io/crates/openehr-term) | TERM 3.1.0 | Terminology model + the embedded official openEHR terminology (five languages: `en`, `es`, `ja`, `pt`, `zh`) |
| [`openehr-lang`](https://crates.io/crates/openehr-lang) | LANG 1.0.0 + 1.1.0 | The BMM meta-model and its P_BMM schema form, plus hand-written ODIN, BEL and Expression-Language readers |
| [`openehr-query`](https://crates.io/crates/openehr-query) | QUERY 1.1.0 | AQL lexer, parser, typed AST with source spans, visitors, parameter binding, and canonical printer; the `federation` feature for the `FROM ENDPOINT` directive |
| [`openehr-its`](https://crates.io/crates/openehr-its) | ITS-JSON, ITS-XML, ITS-REST 1.1.0 | Canonical JSON + XML codecs, the generated ITS-REST contract (axum server router, client, and an operation matcher for intermediaries), OPT 1.4 and AOM2 archetype XML |
| [`openehr-sdt`](https://crates.io/crates/openehr-sdt) | ITS-REST 1.1.0 Simplified Formats + SMART App Launch | Simplified Formats (FLAT/STRUCTURED/Web Template, TDD import), template-independent RM-instance validation, the SMART on openEHR scope grammar |

```toml
[dependencies]
openehr-rm = "0.0.71"
openehr-its = "0.0.71"
```

All nine are **edition 2024** with an MSRV of **Rust 1.97**, and all nine
inherit the workspace lint table, including `unsafe_code = "forbid"`, which no
attribute anywhere in the crate can relax. There is no `unsafe` block in the
published specification layer.

<!-- toc -->

## Generations: reaching more than one specification version

A crate generated from more than one openEHR generation exposes each as a
**version-named module**: `openehr_rm::v1_1` and `openehr_rm::v1_2`,
`openehr_base::v1_2` / `v1_3`, `openehr_am::v1_4` / `v2_4`,
`openehr_lang::v1_0` / `v1_1`, `openehr_term::v3_1`. The crate's `prelude`
re-exports the **current** generation, so ordinary code needs no module path;
an older generation is reached by naming its module in full:

```rust,no_run
// The current generation (RM 1.2.0), via the crate prelude:
use openehr_rm::prelude::Composition;

// The released generation (RM 1.1.0), via its own module path. Import
// renaming is not used anywhere in this project; where both generations
// appear in one file, give one of them a type alias:
type Rm110Composition = openehr_rm::v1_1::composition::composition::Composition;
```

Each generation mirrors its specification's own package structure, so the path
after the generation module reads the same in both. Every generation module
also carries its own `prelude`; no name from one generation is ever mixed into
another.

`openehr-lang` is the one crate whose generations also differ in *what they
contain*, because upstream publishes two BMM meta-models side by side. Its
`v1_1` generation carries them as sibling specification units: the stable,
tool-implemented BMM v2.x model (`bmm`, its persistence form
`bmm_persistence`, and the `beom` expression model) is on the generation's
prelude, while the paused BMM3 model (`bmm3`) is reachable only by full module
path. They cannot be merged (a set of class names, `BmmClass`, `BmmModel` and
`BmmPackage` among them, occurs in both units with materially different
shapes) so the prelude carries the stable units and the choice stays explicit
at the use site. The older `v1_0` generation is what its release actually
defines: the BMM model plus an ODIN reader, with no BEL or Expression-Language
notation.

## Versioning

The package version is the crates' **own independent SemVer line**: it tracks
this implementation's code and moves freely with fixes and improvements, never
with the vendored openEHR specification versions. While the line is `0.0.x`,
expect breaking changes between releases, which always ship in lockstep across
all nine crates.

The implemented specification version is therefore a **separate datum, per
generation**. Each generated crate emits a `Generation` enum that is the only
authority for it: one variant per generation module, `Default` marking the
current one, each variant carrying its specification version as a `const fn`,
and `Display`/`FromStr` round-tripping the module token:

```rust,no_run
assert_eq!(openehr_rm::Generation::default().spec_version(), "1.2.0");
assert_eq!(openehr_rm::Generation::V1_1.spec_version(), "1.1.0");
assert_eq!(openehr_rm::Generation::default().as_str(), "v1_2");
```

There is deliberately **no crate-level `SPEC_VERSION` constant in the generated
crates**: a single constant would contradict a caller using a non-current
generation. Exactly four crates implement one specification each and do expose
one: `openehr_its::SPEC_VERSION`, `openehr_sdt::SPEC_VERSION`,
`openehr_query::SPEC_VERSION`, `openehr_adl::SPEC_VERSION`.

## Taking only the part of `openehr-its` and `openehr-sdt` you need

`openehr-its` layers its features so a browser or embedded consumer does not
have to compile an HTTP server to read a template. The parsing spine is
`json` → `xml` → `opt14`, each pulling in the one below it. Beside the spine
sit `schema-validation` (validation against the compiled-in ITS-JSON RM
schema, its only `jsonschema` user) and three ITS-REST features. `rest` is the
generated contract (DTOs, parameter structs, route tables and the `ApiError`
type, plus the `routes::lookup` operation matcher) over serde and `http`, with no HTTP engine. `rest-server` adds the
per-group server traits, their success enums and a generated `router(api)` per group, which is what brings
in `axum`. `rest-client` adds a generated client per API group over a runtime
that sends through `reqwest`: each operation answers one outcome variant per
status the OpenAPI documents, and an undocumented status is an error. A
consumer that calls a CDR takes `rest-client` and never compiles `axum`. The
default feature `full` is all seven, so an existing dependency line keeps the
crate it had. With `default-features = false` and no feature at all,
`openehr-its` compiles empty.

`openehr-sdt` sits on top: its `flat` feature pulls `openehr-its` with `opt14`
and adds the Simplified Formats and RM-instance validation, and `cache` adds
the async Web Template cache (the only `moka` user). Its default `full` is
both. `opt14` and `flat` build for `wasm32-unknown-unknown`, so a browser
consumer can call `flat::webtemplate::builder::build_web_template` or read an
OPT 1.4 template:

```toml
[dependencies]
openehr-its = { version = "0.0.71", default-features = false, features = ["opt14"] }
openehr-sdt = { version = "0.0.71", default-features = false, features = ["flat"] }
```

With `default-features = false` and no feature at all, `openehr-sdt` compiles
to the SMART App Launch scope grammar alone: std-only, with no dependency of
any kind, so a REST client can parse scope strings with the grammar the CDR
enforces instead of carrying a second one.

## Releases

The nine crates publish as the last leg of the release pipeline, gated on a
human: the leg runs in a protected environment with a required reviewer, so
the run pauses there and nothing reaches crates.io without an explicit
approval. A separate dispatch lane covers a publish between releases and the
recovery path when the release leg fails.

Both lanes authenticate with **crates.io Trusted Publishing** (OIDC, no
long-lived token anywhere) and publish the crates **one at a time in
dependency order**, treating "already exists on the index" as done, so a run
interrupted halfway can be re-run to finish the set. The lane then reads the
registry back and refuses to report success unless all nine resolve at the
same version, because while the line is `0.0.x` a straggler makes its
siblings' internal requirements unresolvable for every consumer.

## Licensing

The nine crates fall into two groups.

The six generated-or-wire crates (`openehr-base`, `openehr-rm`, `openehr-am`,
`openehr-lang`, `openehr-term`, `openehr-its`) are **`Apache-2.0`**, the
licence of the openEHR machine-readable artifacts they are generated from, so
any Rust project can use them, in proprietary and hosted products included.
They embed material derived from those artifacts (generated types carrying
specification documentation text, codecs and a REST contract generated from
the XSD, OpenAPI and BMM files), which is Apache-2.0 as well, and their files
name the openEHR Foundation as a second copyright holder. `openehr-its` also
carries the hand-written runtimes, canonical JSON entry points and
wire-validation dispatcher the generated code cannot ship without, and it
packages the vendored ITS-JSON schema as bytes, so its README carries that
file's attribution (upstream repository, the exact vendored commit, and the
licence) inside the package, where it travels with any redistribution.

The three hand-written engines, `openehr-query`, `openehr-adl` and
`openehr-sdt`, are under the **Business Source License 1.1**, the licence of
the FerroEHR application. Each declares **`BUSL-1.1`** and ships only its own
Rust sources, the README and its `LICENSE` with the parameters. Non-production
use of these three is free, production use is free for Non-Commercial
Purposes, and any other production use, hosting for third parties or
distribution for a fee needs a commercial licence, exactly as for the
application.

`openehr-term` carries a third term, because it embeds a different kind of
openEHR material: the official terminology XML (the five language bundles, the
external-terminology index, and the property/unit data) is **CC-BY-SA 3.0**,
redistributed verbatim with attribution. Its manifest declares
**`Apache-2.0 AND CC-BY-SA-3.0`** and the package ships both license texts, so the declaration a consumer reads names every license the
crate's own bytes are under. If you redistribute the crate, the terminology data
travels under CC-BY-SA 3.0.

Versions 0.0.56 and earlier were published under the MIT terms in force at the
time (MIT AND Apache-2.0 for the crates that embed openEHR material) and keep
them; 0.0.57 was never published; 0.0.58 and 0.0.59 are Apache-2.0 for all
eight crates of that time. From 0.0.60 `openehr-query` and `openehr-adl` are
BUSL-1.1, and `openehr-its` was published as `BUSL-1.1 AND Apache-2.0` from
0.0.60 to 0.0.67; its 0.0.68 was never published, and from 0.0.69 it is
Apache-2.0 again. Its hand-written Simplified Formats, RM-instance validation
and scope grammar moved into the new `openehr-sdt`, first published alone at
0.0.68 under BUSL-1.1 against the 0.0.67 siblings and in lockstep from 0.0.69.
Published
versions keep the licence they were published with. The full picture, including
the vendored material that never reaches a published package, is in
[Licensing](licensing.md).

The openEHR specifications themselves are © the openEHR Foundation.

# Pinned Version Matrix

The single source of truth for every version pin in this project. If this file
and a config file (`Cargo.toml`, `rust-toolchain.toml`, CI workflows) disagree,
fix the drift — do not let either silently win. For third-party Rust crate
versions specifically, the root `Cargo.toml` `[workspace.dependencies]` table
is authoritative (see "Rust dependency pins" below); this file records the
platform, language, database, and openEHR specification pins.

## Product and crate versioning

The **product** version (the workspace `version` in the root `Cargo.toml`,
inherited by the `ferroehr-*` application crates, the tools, and the codegen
tooling) follows its own SemVer line, starting at **3.0.0** (this
project releases as the successor of the EHRbase 2.x line). Releases are
changelog-driven (`CHANGELOG.md`, Keep a Changelog 1.1.0) and published as
official releases (owner sign-off 2026-07-31); only an explicitly suffixed
tag (`vX.Y.Z-rc1`, ...) publishes as a pre-release.

The `openehr-*` **spec crates** are published on crates.io on their own
**independent SemVer line** (owner rulings 2026-08-04, issues #1886 + the
same-day correction: the package version is PERMANENTLY decoupled from the
vendored spec versions — it tracks this implementation's code and moves
freely with every fix and improvement, while the spec pins move only on a
re-vendoring; the crates never adopt a spec version as their package
version). The implemented spec version is a separate datum, per GENERATION
(owner ruling 2026-08-05, #1942: a multi-generation crate has no single
crate-level pin — a fixed one would contradict a configured non-current
generation): the generated crates carry an emitted `Generation` enum
(per-variant `spec_version()` — a `const fn`, so const contexts need no
constant — with derived `Default` marking the current generation, from the
codegen composition table); no version CONSTANT exists anywhere in them; the hand-written single-spec crates keep a literal
crate-level `SPEC_VERSION`. The crates release in lockstep — while the line is
`0.0.x`, a bump bumps all nine plus their internal version requirements
(cargo treats every `0.0.x` as its own compatibility set). Spec-crate
releases never ride the product version.

## Language and runtime

| Item | Pin |
|---|---|
| Rust toolchain | stable 1.98 (1.98.1) |
| MSRV | 1.97 |  <!-- deliberately behind the toolchain: see the MSRV bump policy below -->
| Edition | 2024 |
| Cargo resolver | v3 |

Rationale: the deliverable is a binary/service, not a published library, so
MSRV can track current stable rather than trailing it. A crate later split out
for standalone publication may relax to 1.94/1.95 at that time.

MSRV bump policy (the documented policy the Cargo book asks for — an MSRV
change is "possibly-breaking", semver class `env-new-rust`,
https://doc.rust-lang.org/cargo/reference/semver.html#env-new-rust): the
workspace `rust-version` and `rust-toolchain.toml` bump together, in their
own PR, with a `CHANGELOG.md` `### Changed` entry; the declared MSRV is
verified in CI by the `msrv` job (`cargo hack check --rust-version` — the
book's named tool), not just by `clippy::incompatible_msrv`. Consumers can
escape with `cargo build --ignore-rust-version`.

## Database

| Item | Pin |
|---|---|
| PostgreSQL | 18, target 18.6 or newer (CI runs `postgres:18.6`) |
| Required extension | `btree_gist`, installed by the server at boot |

PostgreSQL is the only component in the stack with a meaningful version delta
(upstream EHRbase targets PG 15/16; we target PG 18). PG 18 brings asynchronous I/O,
native `uuidv7()`, B-tree skip scan, temporal `PRIMARY KEY`/`UNIQUE`/`FOREIGN
KEY`, `RETURNING OLD/NEW`, virtual generated columns, self-join elimination, and
OR-to-`ANY` planning, plus `JSON_TABLE()` + the SQL/JSON functions inherited
from PG 17. **See `docs/postgres-features.md`** for the full PG 17+18 feature
delta over PG 16 and where each app phase (P09/P12/P16/P11/P20) uses it —
"the best possible system" means exploiting these. Minor releases (18.1–18.6,
17.x) are bugfix/security only (no new features); pin the latest patch (18.6,
2026-08-13 — a security release fixing 28 CVEs; 18.5 was never released, a
regression was found post-wrap). Managed providers may lag on `io_uring`; still
target PG 18 and let the AIO benefit follow.

## openEHR specification matrix

There is no umbrella "openEHR Release" version; each component versions
independently. **The spec crates are generated from the vendored BMM
meta-model, so the version pins below are load-bearing** — they are the
actual codegen input, not just documentation. Pins bumped 2026-07-03 (the
versions available as clean `*.bmm.json`); the release-ladder columns were
verified against specifications.openehr.org on 2026-07-20.

**Pin honesty:** several pins are development-generation snapshots AHEAD of
the latest official release — the "Our pin" column says which. That is safe
by openEHR's own release strategy (see "Spec version policy" below): within
a major line every release is a compatible superset, so a newer-generation
pin accepts every valid older-minor instance.

| Component | Our pin | Latest official release | Upstream WIP | Notes |
|---|---|---|---|---|
| BASE (Foundation + Base Types) | **1.3.0** (pre-release generation; BMM refreshed 2026-07-25 @ `e4879576` — SPECBASE-48 invariants, SPECAM-82 CODE_PHRASE move, SPECPR-426/386/460; text @ `45caad1b` since 2026-10-14, a README-only delta with an unchanged BMM) **+ 1.2.0 released, emitted side by side** (#1942: generation module `v1_2` beside the current `v1_3`) | 1.2.0 (09-Apr-2021) | 1.3.0 | generated → `openehr-base` (foundation + base types; `openehr-foundation` folded in) |
| RM (Reference Model) | **1.2.0** (development generation; BMM and text @ `654296bb` since 2026-10-19 — SPECPR-453 typos, the TBD notes removed, and SPECRM-103 rewording the `INTERVAL_EVENT.time` documentation (a documentation-string change in the generated `v1_2` types only; `interval_start_time()` already computed time minus width)) **+ 1.1.0 released, emitted side by side** (#1942: generation module `v1_1` beside the current `v1_2`; RM 1.1.0's own BMM `includes` names `openehr_base_1.2.0`, so `v1_1` resolves against BASE `v1_2` — the released pairing. PIN-HONESTY CAVEAT: the published 1.1.0 BMM carries post-release content — ITEM_TAG/the tags package post-date the release per the amendment record (reported upstream, #2537; the in-repo stable-profile adjudication is #2538) — so `v1_1` is a faithful serialization of the FILE, not of the release TEXT) | 1.1.0 (29-Sep-2020) | dev | generated → `openehr-rm` |
| AM (Archetype Model) | **1.4 + 2.4.0** (1.4 released; 2.4.0 WIP generation; text @ `2635f42c`, 2026-10-19: the TBD notes removed and SPECPR-366's AOM2 text naming `AUTHORED_ARCHETYPE.build_uid` mandatory, which the generated `v2_4` type already is) | 2.3.0 (20-Mar-2024) | 2.4.0, **3.0.0** | generated → `openehr-am`, both majors side by side as the generation modules `v1_4` (ADL 1.4) + `v2_4` (ADL 2) — the spec-mandated dual-generation case |
| QUERY (AQL) | 1.1.0 (= the release) | 1.1.0 (14-May-2021) | 1.2.0 | `openehr-query`; grammar-driven (AqlLexer/Parser `.g4`), not BMM |
| LANG (BMM / ODIN / EL) | master snapshot beyond 1.0.0 (development toward 1.1.0; text @ `f1a98436`, 2026-10-19: SPECLANG-13's BEL amendment record entry and the TBD notes removed, editorial only); **both extant BMM generations vendored** — the stable v2.x BMM (`openehr_lang_1.1.0.bmm.json`) and the v3 development line (`openehr_lang_1.1.0-bmm3.bmm.json`) | 1.0.0 (11-May-2020) | dev | `openehr-lang` — the ODIN + BMM reader that feeds codegen; pins are per generation. THREE BMM files are vendored: the released `openehr_lang_1.0.0.bmm.json` plus the two 1.1.0-line snapshots. The released 1.0.0 BMM emits the `v1_0` generation FAITHFULLY, defects verbatim (#1946, reversing the #1942 refusal — the file's defects: no `includes` at all, `BMM_CLASS`/`BMM_PACKAGE` lack `name`, an `org.openehr.lang.obsolete-elom` package; the defect class stays reported in upstream-report #1927). `v1_0` carries ONLY what Release-1.0.0 defines: the BMM model, the ODIN reader and an ODIN-only lexer over the release's own grammar set (`crates/openehr-lang/vendor/grammar/v1_0/`) — 1.0.0 EL is DEVELOPMENT prose with no grammar, BEL first appears in 1.1.0. **The two BMM generations are emitted side by side** (the second live dual-generation case beside AM's `v1_4`/`v2_4`): 18 class names occur in both files with materially different shapes, so both emit inside the ONE `v1_1` component-version generation module as sibling SPECIFICATION UNITS — the v2.x model (`bmm` + `bmm_persistence` + `beom`, on the prelude) and the BMM3 model (`bmm3`, full-path only). Upstream formalised the split in SPECLANG-14 (`LANG/docs/bmm3/master00-amendment_record.adoc`) and keeps v2.x as "the normative, tool-implemented version" (`LANG/docs/bmm/master01-preface.adoc` §History). The prelude carries the version's STABLE units only, per the published LANG index (BMM v2.x STABLE, "the form implemented by current tooling"; BMM3 PAUSED); `v1_1::bmm3` types are reachable by full module path |
| TERM (Terminology) | **3.1.0** (WIP generation; text, computable assets and BMM @ `d2b05afd`, 2026-10-19: SPECTERM-31 adds `interoperability` (672) to the `term_mapping_purpose` group in all five languages; before it, amendment 3.1.4 / SPECPR-450 added properties 764 + 765 with their units and concepts 761–765 to the `property` group) | 3.0.0 (26-Jun-2023) | 3.1.0 | `openehr-term` — **hand-written** (BMM has only interface classes; bundle/assets/logic are not derivable) |
| ITS-XML (XSDs) | **both published lineages vendored**: tag `Release-1.0.2v2` @ `f7a93777` (namespace `…/v1`, the STABLE bundle) + tag `Release-2.0.0v2` @ `de8b37ba` (namespace `…/v2`, TRIAL upstream) | 2.0.0 TRIAL (26-Apr-2021) | 2.1.0 | canonical XML in `openehr-its`, one generated codec for both (the lineages differ only in the root `xmlns`); bundles at `crates/openehr-its/schemas/xml/`. **Served wire = v2 by default, v1 on request** (owner ruling 2026-08-03, issue #1666 — the v1 bundle cannot describe 50 RM 1.2.0 classes the server emits, register AMB-185): a client selects the lineage with the `version` media-type parameter on `application/xml` — our own extension, no openEHR spec governs namespace selection on the REST wire. OPT 1.4 template XML is always v1. |
| ITS-REST (REST API) | **Release-1.1.0** @ `24058992d` | 1.1.0 (19-Jul-2026) | 1.2.0 | policy: single version, always the latest released; spec text at `docs/specs/openehr/ITS-REST/` and the OAS at `crates/openehr-its/vendor/rest-oas/` are the **same commit** (tag Release-1.1.0). Per-API lifecycle within the release: Overview/System/EHR/Query/Definition/Formats **STABLE**; Demographic/Admin/SMART **DEVELOPMENT** (the OAS bundle artifacts are marked TRIAL) — all 7 API groups vendored |
| ITS-JSON (JSON Schemas) | development: schemas and docs text @ `14387ce9` (2026-10-14; the schemas are byte-identical to the earlier `5acae056` pin, and `scripts/vendor/codegen-inputs.sh` refuses the two pins drifting apart) | (1.0.0 itself still WIP) | dev | validation oracle for the fidelity gate; `openehr_rm_1.1.0_all.json` vendored at `crates/openehr-its/schemas/json/` |
| ITS-BMM (BMM meta-model, JSON) | `37b31739` + three component-repo overlays | per-schema | — | **the codegen input**; vendored `*.bmm.json` at `tools/openehr-codegen/vendor/bmm/` with provenance, fetched by `scripts/vendor/codegen-inputs.sh`. Overlays pending ITS-BMM republication: BASE 1.3.0 json from `specifications-BASE` @ `e4879576`, RM 1.2.0 json from `specifications-RM` @ `654296bb`, TERM 3.1.0 json from `specifications-TERM` @ `d2b05afd` — see that PROVENANCE.md |
| SM (Service Model) | master snapshot | 1.1.0 | dev | vendored spec text only (the service layer's design authority) |
| CNF (Conformance) | master snapshot @ `239dfb4e` (2026-10-14) | 1.0.0 | dev | vendored spec text + CNF test schedule (the conformance oracle) |

**Spec text vendored in-repo:** the normative documentation for every
component above — plus SM (platform service model / SDT) and **CNF (the
conformance guide + Platform Conformance Test Schedule + Robot suite)** — is
vendored at `docs/specs/openehr/` by `scripts/vendor/spec-docs.sh`, pinned to
these same versions (exact commits in each component's `PROVENANCE.md` and in
the script). It is the read/conformance oracle; codegen still consumes only
`tools/openehr-codegen/vendor/**`, `crates/openehr-its/schemas/**` and
`crates/openehr-its/vendor/rest-oas/`, which `scripts/vendor/codegen-inputs.sh`
fetches at their own pins (`--check` reproduces the committed trees byte for
byte). A BMM overlay and its `docs/specs/openehr/<C>/computable/BMM/` twin stay
byte-identical.

## Conformance instrument

The acceptance instrument is [Veredictum](https://github.com/Vernum-Projecten/Veredictum),
an independent Apache-2.0 project (split from this repository, #2789/#2811).
Its pin — the `veredictum` crates.io version the pipeline installs and the
matching tag whose catalogue/spec data is checked out — lives in EXACTLY one
place, `scripts/lib/veredictum.sh` (`VEREDICTUM_VERSION`); this file records
no copy of it, per the never-copy-a-pin rule this file opens with. Bumping
the instrument is a one-line change there plus a fresh
`bash scripts/conformance.sh` acceptance run.

## Spec version policy (owner rulings 2026-07-20, superseded in part 2026-08-05)

Grounded in the official openEHR release strategy
(specifications.openehr.org/governance/release_strategy, read 2026-07-20):
patch = "error corrections and minor additions that do not change the
semantics"; minor = "significant additions that do not change the semantics
of the existing part of the release"; major = "changes to the semantics or
large changes", incompatible and "most likely requiring software upgrade and
possibly data migration".

- **TWO generations per BASE/RM/LANG component — the released one and the
  development one, both emitted, selected at the application level (owner
  ruling 2026-08-05, issue #1936; this supersedes the 2026-07-20 single-pin
  rule for those components).** The development pins are pre-release
  generations, so a deployment must be able to run on RELEASED spec text:
  every future re-vendor keeps the previous generation emitted and
  selectable, and dropping a generation is an owner decision, never a bump
  side effect. Selection is the ONE coupled `spec_profile` configuration key
  (`development` = RM 1.2.0 + BASE 1.3.0 + LANG 1.1.0, the default;
  `stable` = RM 1.1.0 + BASE 1.2.0 + LANG 1.0.0) — per-component free choice
  is rejected because the generations are modelled against each other
  (RM 1.1.0's own BMM `includes` names BASE 1.2.0), so incoherent
  combinations stay unrepresentable. Runtime behaviour: one typed core on
  the development generations (within-major supersets make every
  stable-generation instance valid there); the profile is the ACCEPTANCE
  boundary — surface the released generation does not define is a typed,
  profile-naming refusal (the AQL planning gate), and the exact model
  delta between the RM and BASE generation sets — the two profile-coupled
  components whose types can appear in stored or served content — is
  MACHINE-PINNED (`profile_generation_delta_is_pinned`, the emitter
  invariants), so a re-vendor that changes it fails until the boundary is
  extended. The LANG generations are deliberately OUTSIDE that pin (#2390):
  LANG types are code-generation input only — the application binds nothing
  but the `openehr_lang::Generation` token — so no LANG delta entry can
  change what the boundary accepts or serves, and the `v1_1` generation's
  side-by-side `bmm`/`bmm3` specification units (18 class names in both,
  materially different shapes) make a name-keyed cross-generation diff
  ill-defined by construction.
  Direction contract: `stable → development` is always safe (minor releases
  are additive); `development → stable` is supported only for data that
  never used development-only constructs — anything else is refused loudly
  at read, never silently down-converted. The mechanism is a **commit-time
  stamp read at serve time**: every accepted commit records in
  `version.stable_compatible` whether the RELEASED generation's own reader
  can express the body (`true` by construction under `stable`; the extra
  in-memory parse runs only under `development`), and the one seam every
  stored version body leaves storage through
  (`ferroehr::versioning::read::version_read`) refuses a `false`-stamped
  version under the `stable` profile with a `409` naming the profile, the
  version and the remedy. A `NULL` stamp — a row committed before the column
  existed, or written by a verbatim-replay path (EHR-Extract import, archive
  load) — is assessed on the fly at read, with no write-back. **AQL takes the
  same stamp wherever it serves a version BODY**: a whole-object (or subtree)
  projection is gated at RESULT ASSEMBLY, so a page reaching a version the
  released generations cannot express refuses the whole query with that same
  `409` naming the version — never a row silently elided from the result set,
  which the `RESULT_SET` has no per-row diagnostic channel to explain. Leaf
  projections over the identical rows stay UNGATED: they serve data values, not
  version bodies, over paths the planning gate has already bounded to the
  released generation's declared surface. Under `development` the assembly gate
  costs nothing — it returns before it touches the database. No openEHR spec
  governs runtime version selection — our own design/extension.
- **Single pin still holds for TERM/QUERY/ITS-REST/ITS-XML** (the components
  the 2026-08-05 ruling did not touch): within a major line every release is
  a compatible superset, so the newest-generation pin accepts every valid
  older-minor instance. Proven in practice: the fidelity gate validates our
  canonical wire against the 1.1.0-era ITS-JSON schema while the types are
  RM 1.2.0-generation, and stock-EHRbase's RM 1.1.0-era wire round-trips
  the corpus.
- **Across major boundaries, dual generations are decided per component when
  a major actually releases.** There are two live cases, both mandated by
  upstream keeping two generations side by side:
  - **AM** — the BASE Architecture Overview
    (`master05-package_structure.adoc`) mandates ADL 1.4 and ADL 2
    "maintained side by side", and `openehr-am` ships both as
    `v1_4`/`v2_4`. A future major (AM 3.0.0 is in WIP upstream) gets the same
    treatment only if the ecosystem runs both generations; otherwise
    cutover.
  - **LANG's BMM** — upstream formalised the v2/v3 split in SPECLANG-14
    (`LANG/docs/bmm3/master00-amendment_record.adoc`), publishes the two as
    separate specifications, and keeps v2.x as "the normative,
    tool-implemented version" (`LANG/docs/bmm/master01-preface.adoc`
    §History) while the v3 line adds the expression/statement meta-model. The
    two are vendored as separate BMM files and emitted as sibling
    SPECIFICATION UNITS inside the one `v1_1` component-version generation —
    the v2.x packages (`bmm` + `bmm_persistence` + `beom`) beside the BMM3
    package (`bmm3`) — because 18 class names occur in both with materially
    different attribute sets, so merging them into one class map would
    discard one unit's shapes. The prelude carries the version's STABLE
    units only (BMM3 is PAUSED upstream; in-repo hold record #1920) —
    `v1_1::bmm3` types are reached by full module path.
- **ITS-REST is single-version: always the latest RELEASED API** — no legacy
  REST surface. The 1.1.0 adoption is tracked on issue #178; future releases
  are detected automatically (the spec-update/release watcher workflows).
- **ITS-XML is the one place a lineage is negotiated, and it is not a second
  generation** (owner ruling 2026-07-28). The `Release-2.0.0` restructure
  changed the schemas' target namespace, and every element name and `xsi:type`
  a document uses is spelled identically in both lineages, so there is exactly
  ONE generated codec: the two bundles merge into one emission closure and the
  lineage is a serialize-time choice of root `xmlns`. Upstream marks 2.0.0
  TRIAL while directing stable consumers to `Release-1.0.2`. **The two bundles
  are NOT interchangeable as validators, though** (corrected 2026-08-02): the
  same release also re-packaged the schemas per component and per RM release,
  and the flat `Release-1.0.2v2` bundle was never re-issued against a newer RM
  — it declares no `Ehr.xsd`/`Demographic.xsd` (50 concrete RM 1.2.0 classes
  absent) and omits 23 attributes on 17 more, `FOLDER.details` among them. So
  a correct RM 1.2.0 document can be invalid against the v1 schemas and valid
  against the v2 ones. That is a bundle-content fact, not a second generation:
  no second model, crate module, or impl set exists. The sweep is pinned by
  `crates/openehr-its/tests/it/xml_xsd_validity.rs` and adjudicated in
  the conformance instrument's `artifacts/registers/ambiguities.yaml` AMB-185.
  The CDR serves v2 by default (owner ruling 2026-08-03, issue #1666: the
  default a schema-validating client receives must actually validate this
  server's RM 1.2.0 output, and only the v2 bundle can — correctness over the
  STABLE label) and v1 when a request asks for it via the `version`
  media-type parameter on `application/xml`. That parameter is our own extension — no openEHR spec
  governs namespace selection on the REST wire — and it is not the
  dual-generation exception the AM rule above describes, because no second
  model, crate module, or impl set exists.
- The acceptance instrument is the openEHR CNF conformance schedule, **not**
  an EHRbase parity harness; EHRbase is prior art. Stock EHRbase/`archie`
  emits an RM 1.1.0-era wire format — compatible under this policy, and the
  fidelity gate is where any divergence surfaces.

## EHRbase reference point

This is a fork; its import point is recorded so provenance is unambiguous:

| Item | Value |
|---|---|
| Original reasoning authored against | EHRbase v2.31.0 |
| Tree actually imported at (Phase 0) | EHRbase v2.33.0 |
| Enterprise-capability prior art | upstream EHRbase tag v0.32.0 (last pre-v2) |
| How the import is recorded | the root commit of `main`, `Import: EHRbase as the fork point` |

The original design reasoning was authored against v2.31.0. By the time this
fork's Phase 0 reorganization ran, upstream had advanced to v2.33.0, and that
is the tree actually imported into the Cargo workspace. EHRbase v2.33.0 is the
*prior-art* reference, consulted via its upstream repo — never an in-tree copy
and never a parity oracle (acceptance is the openEHR CNF suite).

The inherited commit history is not part of `main`. `main` begins at a single
labelled import commit carrying the imported tree, and every commit after it is
this project's own work (history rewritten 2026-08-31, issue #2962). The
upstream commit history that produced the imported tree stays reachable in this
repository through the release tags `v3.0.0` and later, whose ancestry predates
that rewrite, so the provenance remains checkable first-hand rather than only
asserted here.

Work on enterprise capabilities not yet built (the plugin system and peers)
may consult the upstream EHRbase v0.32.0 tag — the last release before the
v1→v2 architectural break — read-only, in the public upstream repository.
Everything built ships as our own design, with the openEHR specifications as
the only authority.

## Rust dependency pins

The authoritative, fully-pinned third-party Rust dependency set lives in the
root `Cargo.toml` under `[workspace.dependencies]`. `CLAUDE.md` carries a
categorized narrative summary for orientation. On any discrepancy between this
file, `CLAUDE.md`, and the manifest, **the manifest wins**. Add a dependency to
a crate with `dep.workspace = true`; do not hand-pin a version at the crate
level.

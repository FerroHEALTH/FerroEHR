# Licensing & legal

This page is the complete licensing picture for FerroEHR: what the project's own
code is licensed under, which third-party material ships inside the repository
and the container images, and the trademark and lineage acknowledgments. It is a
summary for evaluators and deployers, not legal advice.

<!-- toc -->

## Do you need a commercial licence?

FerroEHR is source-available under the Business Source License 1.1, which is
not an OSI-approved open-source licence.
[`LICENSE`](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSE) is the
authority and names the Licensor, the Licensed Work, the Additional Use Grant
and the Change Date; this table is the same boundary in the order people ask
about it, and the sections below give the full text of each rule.

| What you are doing | What you need | Why |
|---|---|---|
| Reading, building, modifying or redistributing the source | Free | The licence grants it without a fee and without asking anyone. |
| Development, testing, evaluation, prototyping | Free | All non-production use is granted. |
| Production use for Non-Commercial Purposes | Free | Personal use, academic or scientific research, teaching, and use by a non-profit organisation or public body that is not in the course of a business, does not deliver a service for payment, and is not for commercial advantage. |
| A hospital, clinic or care provider running it for its patients | Commercial licence | Delivering health care, or any other service for payment, is production use outside the grant. |
| A vendor or integrator, or any company running it in production | Commercial licence | Production use in the course of a business is outside the grant. |
| Offering it, or a work derived from it, to third parties as a hosted, managed or embedded service | Commercial licence | Excluded from the grant in every case, whoever you are. |
| Selling, sublicensing or otherwise distributing it for a fee | Commercial licence | Excluded from the grant in every case, whoever you are. |

The last two rows hold whatever else you are: they need a commercial licence
even for an organisation the rows above would otherwise leave free.

### Installing a commercial licence

A commercial licence is a signed token file the licensor issues to you. Place
it where the server can read it and point `[licence] file` (or
`FERROEHR__LICENCE__FILE`) at it; `GET /ferroehr/rest/status` then reports
`licence.use = "commercial"` with your organisation as `licensee`. Without a
token the same field reports the `non-commercial` grant every build embeds.
The server behaves identically under either; identifiers it mints carry a
few bits derived from the licence id in place of random bits, so a record
states which grant it was written under, and nothing else.

Each version becomes Apache License 2.0 four years after that version is
published. A commercial licence starts with a short conversation with the
maintainer named in
[`MAINTAINERS.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/MAINTAINERS.md).

The nine `openehr-*` crates on crates.io are a separate question. The five
generated model crates and the `openehr-its` wire layer are Apache-2.0, so any
Rust project can use them without a licence conversation; the three
hand-written engines carry the same Business Source License as the
application. Full detail is under
[FerroEHR's own code](#ferroehrs-own-code-the-business-source-license-11)
below.

## FerroEHR's own code: the Business Source License 1.1

Everything written for this project (the server and application crates, the
code generator and tooling, the viewer, and the three hand-written
specification engines `openehr-query`, `openehr-adl` and `openehr-sdt`) is licensed under the
[Business Source License 1.1](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSE),
SPDX identifier `BUSL-1.1`. The Licensor is Vernum Projecten B.V. The copyright
holder is stated as *Vernum Projecten B.V.*, identically in `LICENSE`, in
`REUSE.toml`, and in every first-party file header; a CI gate compares those
sources with this page and with the `README.md` of each hand-written engine
crate, so they cannot drift apart. Ruben Talstra remains the maintainer, which
is a credit rather than a holder statement.

FerroEHR is source-available. The Business Source License 1.1 is not an
OSI-approved open-source licence, and this project does not claim that it is.

**What the licence allows without asking anyone.** You can read the source,
build it, modify it, and redistribute it without a fee. All non-production use
is permitted: development, testing, evaluation and prototyping. Production use
is permitted for Non-Commercial Purposes, which the licence defines as
personal use, academic or scientific research, teaching, and use by a
non-profit organisation or public body that is not in the course of a
business, does not deliver a service for payment, and is not for commercial
advantage.

**What needs a commercial licence from the Licensor.** Any other production
use, including the delivery of health care or any other service for payment.
A hospital, clinic or care provider running FerroEHR for its patients needs a
commercial licence, and so does a vendor, integrator or any company running it
in production. Two uses need one in every case, whoever you are:

- offering FerroEHR, or a work derived from it, to third parties as a hosted,
  managed or embedded service, meaning a service through which anyone other
  than you and your affiliates stores, manages or queries health data held by
  it;
- selling, sublicensing or otherwise distributing FerroEHR for a fee, on its
  own or as a component of another product.

Companies and care providers building on FerroEHR are wanted here, and the
commercial licence is the normal path for them. It starts with a short
conversation with the maintainer named in
[`MAINTAINERS.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/MAINTAINERS.md):
Ruben Talstra, `@rubentalstra` on GitHub.

**The Change Date.** Each version becomes available under the Apache License
2.0, its Change License, four years after that version is published.

**What the change does not affect.** Releases v3.0.0 through v4.0.17 stay under
the MIT terms they were published with, and so do the `openehr-*` crate
versions 0.0.56 and earlier on crates.io (MIT AND Apache-2.0 for the crates
that embed openEHR-derived material); `openehr-query`, `openehr-adl` and
`openehr-its` 0.0.58 and 0.0.59 stay Apache-2.0, and `openehr-its` 0.0.60 to
0.0.67 stay `BUSL-1.1 AND Apache-2.0`. Relicensing changes future versions
only. `LICENSES/MIT.txt` reproduces that grant as those versions carry
it, Ruben Talstra's copyright line included, because a published grant is a
record of what was handed out and is not rewritten afterwards.

The conformance instrument is no longer part of this repository. It is
[Veredictum](https://github.com/rubentalstra/Veredictum), an independent
project under Apache-2.0, and the conformance pipeline here consumes it at a
pinned version. The vendored test corpora this repository still carries keep
their upstream terms exactly as the table below states.

**Six of the nine published `openehr-*` spec crates are the exception: they
are Apache-2.0, not BUSL-1.1.** `openehr-base`, `openehr-rm`, `openehr-am`,
`openehr-lang` and `openehr-term` are the generated openEHR model, published on
crates.io under the licence of the openEHR machine-readable artifacts they are
generated from, so any Rust project can use them, in proprietary and hosted
products included, with no commercial licence involved. They embed
openEHR-derived material (specification documentation text in the generated doc
comments), which is Apache-2.0 as well; their generated files name the openEHR
Foundation as a second copyright holder. `openehr-term` also embeds the official
openEHR terminology XML, which is CC-BY-SA 3.0 (see the table below) and is
redistributed verbatim with attribution, so it declares
`Apache-2.0 AND CC-BY-SA-3.0` and ships both texts.

`openehr-its`, the ITS wire layer, is the sixth. Its canonical XML codecs,
`_type` dispatch, REST contract and archetype XML codecs are generated from the
Apache-2.0 openEHR XSD, OpenAPI and BMM artifacts, and it embeds the vendored
ITS-JSON schema. The hand-written runtimes, canonical JSON entry points and
wire-validation dispatcher are Apache-2.0 with it, because the generated code
cannot ship without them. From 0.0.69 it declares `Apache-2.0`, and every
file names the openEHR Foundation as the second copyright holder.

The three hand-written engines, `openehr-query` (the AQL parser), `openehr-adl`
(the ADL engine) and `openehr-sdt` (the Simplified Formats, RM-instance
validation and the SMART scope grammar), are the project's own engineering and
carry the Business Source License 1.1 like the application, each with its own
`LICENSE` naming the crate as the Licensed Work and each declaring `BUSL-1.1`.
See [Rust crates](crates.md#licensing).

## Vendored third-party material

The repository vendors external material verbatim: machine-readable
specification artifacts the code generator consumes, the openEHR specification
text used as the conformance oracle, and real-world clinical models and fixtures
used as test corpora. Each family keeps its upstream license:

| Material | Source | License |
|---|---|---|
| openEHR machine-readable artifacts (BMM meta-models, XML Schemas, OpenAPI documents, JSON Schemas) | the openEHR `specifications-ITS-*` repositories | [Apache-2.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/Apache-2.0.txt) |
| The normative ADL, cADL, ODIN, BEL and Expression-Language ANTLR grammars | `openEHR/adl-antlr`, `openEHR/openEHR-antlr4` | [Apache-2.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/Apache-2.0.txt) |
| openEHR specification text (the conformance reference) | the openEHR `specifications-*` repositories | [CC-BY-SA 3.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-SA-3.0.txt) |
| The AQL grammar and the computable terminology assets (the terminology XML the server embeds, and its schemas) | `specifications-QUERY`, `specifications-TERM` | [CC-BY-SA 3.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-SA-3.0.txt) |
| Clinical models (archetypes and templates) from the openEHR Clinical Knowledge Manager | ckm.openehr.org | per-file `licence` metadata — a **mix** of [CC-BY-SA 4.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-SA-4.0.txt) and [CC-BY-SA 3.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-SA-3.0.txt) |
| The ADL 2 archetype library, with its ADL 1.4 twins (the shared corpus) | `openEHR/adl-archetypes` | **no stated licence** — see below |
| The ADL 2 validator-regression library (the `openehr-adl` corpus) | `openEHR/adl-archetypes`, a different subtree | mixed: mostly **unstated**, else [CC-BY-SA 3.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-SA-3.0.txt), [CC-BY 4.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-4.0.txt) or [CC-BY 3.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-3.0.txt) |
| Test corpora (archie fixtures and reference models, Better `web-template-tests`, EHRbase SDK canonical-JSON data) | Nedap, Better Ltd, vitasystems | Apache-2.0 |
| Three ISO 13606 / rejected-extract BMM reference models inside the archie corpus | offered by their authors under MPL 1.1 / GPL 2.0 / LGPL 2.1 | taken under [MPL 1.1](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/MPL-1.1.txt) — see the election below |
| One terminology schema file, `PropertyUnitData.xsd` | ADL Designer / ADL2-tools, via the openEHR TERM assets | [AGPL-3.0-only](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/AGPL-3.0-only.txt) — see the contradiction below |
| The self-hosted KaTeX stylesheet and fonts this documentation site renders maths with | KaTeX contributors | MIT |
| The Citation File Format 1.2.0 JSON Schema (validates `CITATION.cff` in CI) | `citation-file-format/citation-file-format` | [CC-BY 4.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-4.0.txt) |
| EU legal acts vendored for the compliance citations (GDPR, EHDS, NIS2, CRA, MDR), under `docs/law/eu/` | the EU Publications Office (EUR-Lex) | [`LicenseRef-EUR-Lex-Reuse`](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/LicenseRef-EUR-Lex-Reuse.txt) (Commission Decision 2011/833/EU); the consolidated texts additionally [CC-BY 4.0](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/CC-BY-4.0.txt) |
| EDPB Guidelines 01/2025 on pseudonymisation | European Data Protection Board | [`LicenseRef-EDPB-Reuse`](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/LicenseRef-EDPB-Reuse.txt) |
| Dutch acts and decrees (UAVG, Wabvpz, BW Boek 7, the logging-retention decree, Begz), under `docs/law/nl/` | wetten.overheid.nl | [`LicenseRef-Auteurswet-Art11-Public-Domain`](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/LicenseRef-Auteurswet-Art11-Public-Domain.txt): no copyright subsists in Dutch legislation |
| German federal law (BDSG, SGB V, GDNG, StGB; the DigiG as its Bundesgesetzblatt issue), under `docs/law/de/` | gesetze-im-internet.de and recht.bund.de | [`LicenseRef-UrhG-Para5-Amtliche-Werke`](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/LicenseRef-UrhG-Para5-Amtliche-Werke.txt): no copyright protection for German legislation (§ 5 Abs. 1 UrhG)
| Swiss federal law (DSG/FADP, DSV/DPO, EPDG, EPDV, EPDV-EDI; German, plus the non-binding English where Fedlex publishes it), under `docs/law/ch/` | Fedlex (the Federal Chancellery) | [`LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke`](https://github.com/FerroHEALTH/FerroEHR/blob/main/LICENSES/LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke.txt): Swiss legislation and its official translations are not protected by copyright (Art. 5 URG)

The NEN 7510, 7512 and 7513 standards the compliance pages cite are sold under
copyright and are not in the tree; `docs/law/nl/nen-75xx/` holds a record of the
clauses cited and why the text is absent.

Every vendored tree in the repository carries a provenance note naming its exact
upstream source and pinned revision, with the upstream `LICENSE` file vendored
alongside where the source publishes one; the specification and corpus trees
name their license there as well, and `REUSE.toml` below is the authority for
all of them. The fuzzing seed corpus is a copy of several of those trees, so it
is declared under the union of their licenses rather than guessing each seed's
origin from its filename.

**The CKM clinical-model corpus is mixed, and the table says so on purpose.** A
first-hand count over the vendored CKM material finds both CC-BY-SA 4.0 (the
majority) and CC-BY-SA 3.0 (several hundred files), so no single version is a
true statement about the tree. Each of those archetypes carries its own
`licence` field inside its `description` block, and that per-file metadata is
the authority for any individual file, which also means licensing for this
material already survives being copied out of the repository.

**The ADL 2 archetype library states no licence at all**, and this is the one
tree here that carries no grant. Measured on 2026-09-10 over the vendored copy
(`openEHR/adl-archetypes` at the pinned commit): of 652 archetypes, exactly one
states a licence — and it states CC-BY 4.0, not CC-BY-SA — while the other 651
carry `copyright = <"© openEHR Foundation">` and nothing more. Upstream states
nothing either: no `LICENSE` file at that commit, and a four-line `README.md`.

The validator-regression library from the same upstream repository is a
different subtree and measures differently: of its 302 archetypes, 111 state a
licence and 191 state none, and where one is stated it is predominantly
CC-BY-SA 3.0 (100 of the 111), with 10 CC-BY 4.0 and 1 CC-BY 3.0. **No file in
it states CC-BY-SA 4.0**, which is why its declaration names what is actually
there rather than inheriting the CKM expression.

An unstated licence is not a permissive one, so this material is declared
`LicenseRef-openEHR-unstated`, whose text in `LICENSES/` records the
measurement and says plainly that it is not a licence. If you lift a file out
of that tree, you are doing so without a stated permission from the copyright
holder, and that is your call to make rather than one this repository has made
for you. The tree is here because it is the only independent source of paired
ADL 1.4 and ADL 2 forms of the same archetype: generating the pairs with this
project's own converter would validate that converter against its own output.
Whether it stays committed is tracked on the issue this correction came from.

**Two positions worth stating explicitly**, because both are the kind of thing a
compliance review finds and a summary table hides:

- **The MPL election.** Three BMM reference models in the archie corpus
  (`cen_EN13606_0.95.bmm`, `cen_ts14796_0.90.bmm`,
  `openehr_ehr_extract_999.bmm`) are offered by their authors under a
  tri-license: MPL 1.1, GPL 2.0, or LGPL 2.1. This project **takes them under
  MPL 1.1**, a file-scoped weak copyleft. The election is recorded in that
  corpus's `PROVENANCE.md`, so **no GPL or LGPL obligation attaches to anything
  here**.
- **One upstream contradiction, not resolved by us.**
  `crates/openehr-term/assets/schema/PropertyUnitData.xsd` carries an ADL
  Designer / ADL2-tools header offering it under the GNU Affero General Public
  License, inside an upstream repository whose own `LICENSE` is CC-BY-SA 3.0.
  Both cannot be right, the contradiction is upstream's, and re-licensing
  someone else's file is not ours to do, so it is declared at the **more
  restrictive** of the two readings, the one the file's own text asserts. No
  obligation reaches a consumer: the terminology schemas are excluded from the
  published crate by that crate's `include` list, so the file ships in nothing.

## Machine-readable licensing (REUSE 3.3)

The `PROVENANCE.md` arrangement above is accurate, and it stays. What it does
not do is survive a file leaving this repository: someone who lifts a single
archetype out of a test corpus takes a CC-BY-SA file bearing no marking they
copied. People build on this project and redistribute it, so downstream
file-level redistribution is the expected case.

So licensing is **also** published in the machine-readable form the
[REUSE Specification 3.3](https://reuse.software/spec-3.3/) defines:

- **[`LICENSES/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/LICENSES)**
  holds the full text of every license any file in the tree is offered under,
  named by SPDX identifier: `BUSL-1.1`, `MIT`, `Apache-2.0`, `CC-BY-SA-3.0`,
  `CC-BY-SA-4.0`, `CC-BY-4.0`, `CC-BY-3.0`, `MPL-1.1`, `AGPL-3.0-only` — plus
  `LicenseRef-openEHR-unstated`, which is not a licence but the record that
  one tree carries none.
- **[`REUSE.toml`](https://github.com/FerroHEALTH/FerroEHR/blob/main/REUSE.toml)**
  declares, by glob, which files are offered under which, including the two
  positions above, represented rather than flattened.
- **Every first-party source file carries the header inside itself:** an
  `SPDX-FileCopyrightText` line and an `SPDX-License-Identifier` line stating the
  same position `REUSE.toml` declares for it, so a file copied out of this
  repository takes its licensing along. Rust files of the five generated spec
  crates and of `openehr-its` state `Apache-2.0`, and every other first-party
  Rust, shell, SQL and YAML file, the `openehr-query`, `openehr-adl` and
  `openehr-sdt` sources included, states `BUSL-1.1`. A copied migration or
  script arrives licensed, which is the whole point.

The vendored trees are glob-declared rather than headered for a reason that is
not convenience: **no vendored file may be edited**, so a header sweep over
third-party material was never available. `REUSE.toml` is the mechanism that
makes the declaration complete without touching one. The generated spec-crate
sources are the mirror case: a hand-written header there would be erased by the
next code-generation run, so they receive theirs from the code generator, which
stamps every file it writes.

All of it is gated in CI, and by more than one check, because each one can only
see part of the picture:

- `reuse lint` proves the declarations are **complete**: every file carries
  licensing information and no license text is orphaned.
- A second check fails the build if the set of licenses declared in `REUSE.toml`,
  the texts present in `LICENSES/`, and the licenses named **on this page** ever
  stop agreeing. A license cannot enter the tree without this chapter acquiring
  it.
- Two header checks fail the build if a first-party file loses its header or
  states a license other than the one declared for its path: one for Rust, one
  for shell, SQL and YAML.
- One check fails if the copyright holder is stated differently in `LICENSE`,
  `REUSE.toml`, the file headers, this page, or an engine crate's `README.md`.
- One check refuses copyleft license **text** inside this project's own source,
  which is a conflict no reviewer reliably catches by eye.

What none of them can check is whether the *prose* on this page is correct. No
tool can judge that, which is why any change to what the repository
redistributes updates this chapter in the same pull request.

The CC-BY-SA specification text and clinical models are redistributed
**verbatim, with attribution**: they are reference and test material, not part
of the compiled server. The FerroEHR binary you deploy is built from this
project's own BUSL-1.1 code, plus the Apache-2.0 machine-readable inputs the
generated crates carry, plus the CC-BY-SA 3.0 openEHR terminology bundle
`openehr-term` compiles in (five languages, the external-terminology index,
and the property/unit data). No
copyleft obligation beyond those attribution-and-share-alike terms on the
verbatim data attaches to anything shipped.

## Rust dependencies

All third-party Rust crates are pinned in the workspace manifest and
license-gated in CI with `cargo deny`, which checks licenses, security
advisories, bans, and sources on every change. The allow-list is permissive
(MIT, Apache-2.0, the BSD family, ISC, Zlib, BSL-1.0, Unicode-3.0, CC0-1.0,
MIT-0, CDLA-Permissive-2.0) and deliberately admits two **file-scoped weak
copyleft** licenses: MPL 2.0, and CDDL 1.0 as a single crate-scoped exception for
the flamegraph renderer the profiling instruments use. Obligations under both
attach to those crates' own files, which are consumed unmodified. No strong
copyleft (GPL, LGPL, AGPL, SSPL) is admitted, and a new dependency carrying one
fails the build.

A separate FOSSA lane publishes dependency and license analysis from the
committed CLI configuration for review. It is analysis-only by design and gates
no merge; `cargo deny` is the gate.

## Trademarks and lineage

- **openEHR®** is the registered trademark of the
  [openEHR Foundation](https://www.openehr.org/). FerroEHR is an independent
  implementation of the openEHR specifications and is **not affiliated with or
  endorsed by** the openEHR Foundation.
- FerroEHR began as a fork of **EHRbase**, developed by
  [vitasystems GmbH](https://www.vitagroup.ag/) and the
  [Peter L. Reichertz Institute](https://www.plri.de/), and records that lineage
  in the labelled import commit at the root of its history. EHRbase itself
  remains Apache-2.0; no code from it is present
  in this tree, and it is consulted as prior art only. FerroEHR is not affiliated
  with or endorsed by the EHRbase project. The measured
  [comparison](comparison.md) between the two is published in both directions.

## Questions

If you need a clarification for a compliance review, open a
[GitHub discussion or issue](https://github.com/FerroHEALTH/FerroEHR/issues);
provenance questions can usually be answered by pointing at the exact
`PROVENANCE.md` and upstream pin.

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL 1.4 → ADL 2 conversion (the `adl14` 1.4→2 upgrade pipeline).
//!
//! NOTE: **no openEHR spec governs the 1.4 → 2 conversion ALGORITHM — the
//! entire `adl14` module is our own design/extension.**
//!
//! The released text establishes only that the conversion must exist and names
//! two of its outcomes: `ADL2/master01-preface.adoc` §ADL 2.0 → §Backward
//! Compatibility ("require conversion of ADL 1.4 archetypes to ADL 2 form …
//! the changes have been carefully designed to allow this conversion to be
//! implementable"), `ADL2/master07.04-adl_basics.adoc` §Node Identifier Codes
//! ("In ADL 1.4 at-codes were used as node identifiers. For id-coded ADL2
//! archetypes, these are converted to id-codes by ADL 1.4 to ADL 2
//! converters"), and `ADL2/master07.05-adl_identification.adoc` §Human Readable
//! Archetype Identifier (convert a single-number 1.4 version part to `v1.0.0`
//! or another Knowledge-Identification-conformant identifier). Every other rule
//! below is ours, pinned by the paired
//! `tests/corpus/adl2-reference/upgrade/upgrade_from_14/**` fixtures (each 1.4
//! `.adl` source paired with its expected ADL2 `.adls`) — which are therefore
//! the converter's only oracle.
//!
//! Pipeline:
//! 0. **cADL front end** ([`lower`] + [`domain`]) — the ADL 1.4-only cADL
//!    productions the `Dialect::Adl14` parse dispatches into: the
//!    qualified/listed terminology constraints, the pipe-ordinal shorthand,
//!    and the inline dADL `C_DV_QUANTITY`/`C_DV_ORDINAL`/`C_CODE_PHRASE`
//!    domain blocks. They are the WRITE side of the converter-internal
//!    encoding that step 2 reads back
//!    ([`convert::convert_constraint`](convert)).
//! 1. **Front end** — [`crate::assemble::parse_artefact`] in
//!    [`crate::parse::Dialect::Adl14`] parses a 1.4 `.adl` into a 1.4-SHAPED
//!    `openehr_am::v2_4` [`Archetype`](openehr_am::v2_4::aom2::archetype::archetype::Archetype):
//!    at-code node ids, terminology constraints preserved verbatim in the
//!    `C_TERMINOLOGY_CODE.constraint` string, inline dADL domain blocks lowered
//!    to their `DV_*` types with an attribute tuple. The converter core takes
//!    the assembled `Archetype`, not raw text, so a stored 1.4 operational
//!    template converts through it too once an application-side front end has
//!    decomposed the flattened OPT into one source `Archetype` per embedded
//!    root.
//! 2. **Converter core** ([`convert`]) — node-id renumbering (the `+1`
//!    first-segment shift, separate id-/at-code spaces, `0.`-prefixed
//!    new-at-level codes kept, missing ids synthesised in document order),
//!    terminology-constraint conversion (local single → at-code, local list →
//!    synthesised `ac` value set, external code(s) → synthesised at-code(s) +
//!    term-binding URIs) and the terminology rebuild. Its three
//!    converter-state-free stages are siblings: `walk` (the read-only
//!    definition traversals the code planning consumes), `multiplicity` (the
//!    1.4 default occurrences materialisation + RM-default elision) and
//!    `metadata` (the description / meta-data / version transform).
//! 3. **Differ** ([`differ`]) — for a specialised 1.4 source, re-differentialise
//!    the converted child against its converted+flattened parent (strip
//!    inherited-unchanged nodes).
//! 4. **Conversion log** ([`log`]) — records every synthesised code/value set so
//!    a re-conversion consulting the same log yields identical codes.

pub mod convert;
pub mod differ;
pub mod domain;
pub mod log;
pub mod lower;
mod metadata;
mod multiplicity;
mod walk;

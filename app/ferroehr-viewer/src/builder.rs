// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Query Builder core.
//!
//! The component-free state model the UI edits and its lowering into the
//! `openehr_query` AST (rendered via `openehr_query::printer::to_aql`) — **AQL
//! is never string-concatenated**. Grammar/semantics authority:
//! `docs/specs/openehr/QUERY/docs/AQL/`. The per-datatype criterion catalog
//! follows the RM data types the Web Template exposes per node (`inputs`),
//! executed against the CDR Query API. Compiles on both targets; the UI
//! manipulates this state in WASM and the BFF validates through the same code.
//!
//! The lowering has an inverse: [`lift`] recognizes exactly the query shape
//! [`lower`] emits and lifts a stored query's AQL back into the editable
//! state, refusing anything outside that envelope rather than loading it
//! lossily.

pub mod catalog;
pub mod lift;
pub mod lower;
pub mod model;

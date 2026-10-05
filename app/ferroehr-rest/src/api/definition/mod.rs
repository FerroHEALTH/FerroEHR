// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **definition API** (Release-1.1.0, STABLE).
//!
//! The surface is generated into `openehr_its::rest` from the
//! `definition-*.openapi.yaml` OAS group.
//!
//! Governing spec: `docs/specs/openehr/ITS-REST/specifications/docs/definition/`.
//! The group is split along the three spec resources the OAS `tags` name —
//! `ADL1.4`, `ADL2`, and `Query` — one module each, with `dispatch` as the
//! operation-id `match` that fans out to them:
//!
//! - `template_adl14` — `definition_template_adl1.4_{list,upload,get,example_get}`.
//! - `template_adl2` — `definition_template_adl2_{list,upload,get,example_get,version_get}`.
//! - `stored_query` — `definition_query_{list,store,version_get,version_store}`.
//!
//! `archetype` sits beside them as **our own extension** — the ADL 1.4 /
//! ADL 2 archetype + artefact routes the release never surfaced; it is
//! governed by no ITS-REST operation and carries its
//! own spec-silence flag.
//!
//! Each module implements the generated operation contract over the
//! `ferroehr-sm` native API (`DefinitionAdl14Service` / `DefinitionAdl2Service` /
//! `DefinitionQueryService` + the wire-shaped `DefinitionAdapter` extension).

pub(crate) mod archetype;
pub(crate) mod dispatch;
pub(crate) mod openapi_routes;
mod stored_query;
mod template_adl14;
mod template_adl2;

// The `adl1.4/{id}/example` handler negotiates the Simplified-Formats
// (FLAT/STRUCTURED) representations through the shared `crate::formats::dispatch`
// adapter, called by its full path (no module alias).

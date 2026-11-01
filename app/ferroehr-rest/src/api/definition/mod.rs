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

/// Renders an example party (the example of a demographic template) as
/// canonical JSON or XML, or `None` when `example` is not a party.
///
/// The ITS-REST example operation answers the five party classes beside
/// COMPOSITION (`responses/200_Template_example_retrieved.yaml`). The
/// Simplified Formats RM mapping (`simplified_formats/master05-rm_mapping.adoc`)
/// maps no demographic party class, so a FLAT or STRUCTURED `Accept` on a party
/// example is the `406` the canonical negotiation answers.
#[expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the example is the canonical value the \
              generator produced"
)]
fn party_example_response(
    headers: &http::HeaderMap,
    example: &serde_json::Value,
) -> Option<axum::response::Response> {
    use crate::negotiate::respond_rm;
    use openehr_rm::prelude::{Agent, Group, Organisation, Person, Role};
    let ok = http::StatusCode::OK;
    let response = match example.get("_type").and_then(serde_json::Value::as_str)? {
        "PERSON" => respond_rm::<Person>(headers, ok, example, openehr_its::xml::LOCATABLE_ROOT),
        "ORGANISATION" => {
            respond_rm::<Organisation>(headers, ok, example, openehr_its::xml::LOCATABLE_ROOT)
        }
        "GROUP" => respond_rm::<Group>(headers, ok, example, openehr_its::xml::LOCATABLE_ROOT),
        "AGENT" => respond_rm::<Agent>(headers, ok, example, openehr_its::xml::LOCATABLE_ROOT),
        "ROLE" => respond_rm::<Role>(headers, ok, example, openehr_its::xml::LOCATABLE_ROOT),
        _ => return None,
    };
    Some(response)
}

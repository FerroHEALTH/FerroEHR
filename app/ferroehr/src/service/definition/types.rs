// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DEFINITION data shapes shared with the REST adapter: the wire
//! template-list filter and the SM stored-query descriptor.

/// The template-list filter carried by the ITS-REST
/// `definition_template_*_list` operations.
///
/// All three are optional query parameters the wire decodes but the SM
/// `I_DEFINITION_*` list interfaces (which return plain `List<UUID>`) do not
/// express — so they ride on the wire-shaped list methods alongside the SM
/// cursor [`Page`](crate::service::list::Page).
///
/// - `template_id`: glob pattern matching `template_id`, `*` wildcard
///   (`parameters/query/filter_template_id.yaml`, "supports wildcards `*`").
/// - `concept`: glob pattern matching `concept`, `*` wildcard
///   (`parameters/query/concept.yaml`).
/// - `version`: version filter taken from `template_id` (e.g. `1.2.*`, or `*`
///   for all); absent → latest version only
///   (`parameters/query/filter_version.yaml`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateListFilter {
    /// Glob pattern for `template_id` (`*` wildcard); `None` = match all.
    pub template_id: Option<String>,
    /// Glob pattern for `concept` (`*` wildcard); `None` = match all.
    pub concept: Option<String>,
    /// Version filter (e.g. `1.2.*`, `*`); `None` = latest version only.
    pub version: Option<String>,
}

/// One served ADL2 operational template.
///
/// Pairs the artefact's **resolved** `ARCHETYPE_HRID` (the addressed
/// `template_id` may be a partial that selects the latest matching version)
/// with the rendered representation.
///
/// The wire needs the resolved id as well as the payload, because the `ETag` of
/// a template response identifies the served artefact — ITS-REST
/// `Requests_and_responses.md` §"`ETag` and Last-Modified": both headers
/// "SHOULD be included in responses for VERSION, `VERSIONED_OBJECT`, or other
/// resources that have versioning or unique state identifiers", and an ADL2
/// artefact's unique state identifier is its versioned HRID (AM `v2_4`
/// `ARCHETYPE_HRID`, whose `release_version` is the artefact's SEMVER).
/// Returning the addressed string instead would keep one `ETag` across two
/// different served versions.
#[derive(Debug, Clone)]
pub struct Adl2Template {
    /// The resolved full `ARCHETYPE_HRID` of the served artefact.
    pub hrid: String,
    /// The rendered representation (ADL2 source, or the
    /// `OperationalTemplateV2` canonical JSON).
    pub payload: String,
}

/// The SM `QUERY_DESCRIPTOR` (`query_descriptor.adoc`) — the registration
/// record `I_DEFINITION_QUERY.store_query` / `list_queries` return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDescriptor {
    /// `qualified_query_name [1]` — "Unique qualified name of query.
    /// Qualified names follow patterns such as `<namespace>::<query_name>`,
    /// e.g. `ehr::all_over_50_women`."
    pub qualified_query_name: String,
    /// `version [0..1]` — "Query semver.org version number."
    pub version: Option<String>,
    /// `registration_time [1]` — "Time query was registered in the service"
    /// (ISO-8601).
    pub registration_time: String,
    /// `formalism [1]` — "Formalism of the query, matching one of: 'aql';
    /// any other string value."
    pub formalism: String,
    /// `source [0..1]` — "Source query text to be executed (prior to
    /// parameter substitution)."
    pub source: Option<String>,
}

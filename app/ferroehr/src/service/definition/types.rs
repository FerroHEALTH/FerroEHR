// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DEFINITION data shapes shared with the REST adapter: the wire
//! template-list filter and the served ADL2 template.

/// The template-list filter carried by the ITS-REST
/// `definition_template_*_list` operations.
///
/// All three are optional query parameters the wire decodes, carried on the
/// wire-shaped list methods alongside the cursor
/// [`Page`](crate::service::list::Page).
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

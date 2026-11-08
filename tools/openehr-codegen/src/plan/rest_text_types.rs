// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST text values `emit-rest` types with a hand-written grammar.
//!
//! The OAS gives these as `type: string`, while the ITS-REST docs text gives
//! each a grammar, so the generated contract carries the type that reads it
//! (`openehr_its::rest::stored_query`) and a value outside it is refused where
//! it enters.

/// One text value and the type that reads it.
pub(crate) struct RestTextType {
    /// The `#/components/schemas/` or `#/components/parameters/` name.
    pub component: &'static str,
    /// For a parameter, the operations whose use of it the grammar governs;
    /// empty for a schema, which carries one meaning wherever it is used.
    pub operations: &'static [&'static str],
    /// The Rust type the generated code names.
    pub rust_type: &'static str,
    /// The ITS-REST text that gives the grammar.
    pub citation: &'static str,
}

/// The component schemas typed by a grammar.
pub(crate) const REST_TEXT_SCHEMAS: &[RestTextType] = &[
    RestTextType {
        component: "QueryName",
        operations: &[],
        rust_type: "crate::rest::stored_query::QualifiedQueryName",
        citation: "docs/query/Qualified_query_name.md: `[{namespace}::]{query-name}`, the name \
                   over `[a-zA-Z0-9_.-]` and never `aql` (case-insensitive)",
    },
    RestTextType {
        component: "QueryVersion",
        operations: &[],
        rust_type: "crate::rest::stored_query::QueryVersion",
        citation: "docs/query/Qualified_query_name.md: \"SEMVER style (i.e. \
                   `major.minor.patch`)\"",
    },
];

/// The parameter components typed by a grammar, in the operations named.
///
/// The `version` path parameter also addresses an ADL 2 template, whose
/// version carries a pre-release modifier (ADL 2
/// `master07.05-adl_identification.adoc` §Version Identifiers), so only the
/// stored-query operations read it as a query version pattern.
pub(crate) const REST_TEXT_PARAMETERS: &[RestTextType] = &[RestTextType {
    component: "version",
    operations: &[
        "definition_query_version_get",
        "definition_query_version_store.yaml",
        "query_execute_stored_query_version",
        "query_execute_stored_query_version_body",
    ],
    rust_type: "crate::rest::stored_query::VersionPattern",
    citation: "parameters/path/version.yaml: \"an exact version (e.g. `1.7.1`), or a pattern as \
               partial prefix, in a form of `{major}` or `{major}.{minor}`\"",
}];

/// The type a component schema is read as, when a grammar types it.
pub(crate) fn schema_type(component: &str) -> Option<&'static RestTextType> {
    REST_TEXT_SCHEMAS
        .iter()
        .find(|row| row.component == component)
}

/// The type a parameter component is read as in `operation`, when a grammar
/// types it there.
pub(crate) fn parameter_type(
    component: Option<&str>,
    operation: &str,
) -> Option<&'static RestTextType> {
    let component = component?;
    REST_TEXT_PARAMETERS
        .iter()
        .find(|row| row.component == component && row.operations.contains(&operation))
}

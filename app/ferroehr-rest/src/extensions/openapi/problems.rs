// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The problem details body of every error answer the served document
//! declares (RFC 9457).

/// The media type of a problem details document (RFC 9457 §3).
pub(super) const PROBLEM_JSON: &str = "application/problem+json";

/// Declares the problem details schema as the body of every `4xx` and `5xx`
/// answer in `doc`, except under the FHIR base, whose errors are
/// `OperationOutcome` (FHIR R4 `http.html`), and on the health routes, whose
/// `503` is the health document.
pub(super) fn declare_problems(
    doc: &mut utoipa::openapi::OpenApi,
    fhir_base: &str,
    health: &[String],
) {
    use utoipa::openapi::{Content, Ref, RefOr};
    doc.components
        .get_or_insert_with(Default::default)
        .schemas
        .extend(
            utoipa::openapi::ComponentsBuilder::new()
                .schema_from::<crate::overview::problem::ProblemDocument>()
                .schema_from::<crate::overview::problem::Violation>()
                .schema_from::<crate::overview::problem::Position>()
                .build()
                .schemas,
        );
    for (path, item) in &mut doc.paths.paths {
        let fhir = path
            .strip_prefix(fhir_base)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        if fhir || health.iter().any(|route| route == path) {
            continue;
        }
        for op in [
            item.get.as_mut(),
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.options.as_mut(),
            item.head.as_mut(),
            item.patch.as_mut(),
            item.trace.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            for (status, response) in &mut op.responses.responses {
                if !(status.starts_with('4') || status.starts_with('5')) {
                    continue;
                }
                if let RefOr::T(response) = response {
                    response.content.clear();
                    response.content.insert(
                        PROBLEM_JSON.to_owned(),
                        RefOr::T(Content::new(Some(Ref::from_schema_name("Problem")))),
                    );
                }
            }
        }
    }
}

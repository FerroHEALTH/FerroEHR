// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What Upload a template answers a post with, for either family, and the
//! refusal it draws for the violations the CDR found. No openEHR spec
//! governs the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::refusal::{Issue, Refusal, Severity, refusal};

use crate::adl2::{TemplateFamily, split_hrid};
use crate::bff::templates::{TemplateUpload, Violation, upload_adl2_template, upload_template};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::definitions::DefinitionsMsg;
use crate::locale::{current, tr, tr_with};
use crate::pages::common::count;

use super::{adl2_href, template_href};

/// What Upload read of the post it answered.
#[derive(Debug, Clone)]
pub(super) enum Uploaded {
    /// No upload was posted.
    Fresh,
    /// The CDR refused the upload.
    Refused(ViewerError),
    /// The CDR refused the source `400` for the violations it found.
    Invalid(Vec<Violation>),
    /// The upload succeeded and the browser was sent on.
    Left,
}

/// Answers an upload posted to Upload a template, of `family`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
pub(super) async fn load_upload(
    // The family uploaded to.
    family: TemplateFamily,
) -> Result<Uploaded, ViewerError> {
    crate::session::require_session()?;
    let Some(posted) = crate::posted::posted_for("save") else {
        return Ok(Uploaded::Fresh);
    };
    let source = posted.get("source-text").unwrap_or_default();
    let answered = match family {
        TemplateFamily::Adl14 => upload_template(source).await,
        TemplateFamily::Adl2 => upload_adl2_template(source).await,
    };
    Ok(match answered {
        Ok(TemplateUpload::Stored(id)) => {
            redirect_with(
                &stored_href(family, id.as_deref()),
                &Done::ok(tr(DefinitionsMsg::TemplateUploaded)),
            );
            Uploaded::Left
        }
        Ok(TemplateUpload::Invalid(found)) => Uploaded::Invalid(found),
        Err(error) => Uploaded::Refused(error),
    })
}

/// The page of the template of `family` stored under `id`, else the list of
/// its family when the CDR's answer named no id.
fn stored_href(family: TemplateFamily, id: Option<&str>) -> String {
    match (family, id) {
        (TemplateFamily::Adl14, Some(id)) => template_href(id),
        (TemplateFamily::Adl2, Some(id)) => {
            let (stem, release) = split_hrid(id);
            adl2_href(stem, release)
        }
        (_, None) => format!("/templates?family={}", family.as_query()),
    }
}

/// The refusal of a template source for the violations `found`: one issue
/// for each, its rule code and the CDR's words for it.
#[must_use]
pub(super) fn violations_refused(found: &[Violation]) -> Html {
    refusal(
        &current(),
        &Refusal {
            status: Some(http::StatusCode::BAD_REQUEST.as_u16()),
            summary: tr_with(
                DefinitionsMsg::TemplateUploadInvalid,
                &[("count", count(found.len()))],
            ),
            issues: found
                .iter()
                .map(|violation| {
                    Issue::new(Severity::Error, violation.rule.clone())
                        .text(violation.detail.clone())
                })
                .collect(),
            request: None,
            evidence: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{stored_href, violations_refused};
    use crate::adl2::TemplateFamily;
    use crate::bff::templates::Violation;

    #[test]
    fn a_refused_upload_draws_each_violation_with_its_rule_and_detail() {
        let found = [
            Violation {
                rule: "VARXR".to_owned(),
                detail: "external reference \"openEHR-EHR-OBSERVATION.cnf_constituent.v1\" \
                         does not resolve to an archetype in the repository (at id2)"
                    .to_owned(),
            },
            Violation {
                rule: "/definition/attributes[1]".to_owned(),
                detail: "the node id is not in the ontology".to_owned(),
            },
        ];
        let html = violations_refused(&found).as_str().to_owned();
        for violation in &found {
            assert!(html.contains(&violation.rule), "{html}");
        }
        assert!(
            html.contains("openEHR-EHR-OBSERVATION.cnf_constituent.v1"),
            "{html}"
        );
        assert!(
            html.contains("the node id is not in the ontology"),
            "{html}"
        );
        assert!(html.contains("it breaks 2 rules"), "{html}");
        assert!(!html.contains("\"errors\""), "no raw JSON is drawn: {html}");
    }

    #[test]
    fn a_stored_upload_goes_to_its_template_or_else_to_its_family_list() {
        assert_eq!(
            stored_href(TemplateFamily::Adl14, Some("cnf.minimal v1")),
            "/templates/cnf.minimal%20v1"
        );
        assert_eq!(
            stored_href(TemplateFamily::Adl14, None),
            format!("/templates?family={}", TemplateFamily::Adl14.as_query())
        );
        assert_eq!(
            stored_href(TemplateFamily::Adl2, None),
            format!("/templates?family={}", TemplateFamily::Adl2.as_query())
        );
    }
}

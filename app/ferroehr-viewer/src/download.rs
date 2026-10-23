// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A template's Download: the source the CDR stores, as a file.
//!
//! The browser holds no CDR credential, so it cannot link to the CDR's
//! `definition/template` resources itself. This route reads the template
//! with the session's credential, as the template's Source tab does, and
//! answers it as an attachment: an ADL 1.4 operational template as
//! `application/xml` (`docs/specs/openehr/ITS-REST/` definition API, Get an
//! ADL 1.4 template), an ADL2 template as the `text/plain` artefact. The
//! route enforces the viewer session like every server fn (the public
//! endpoint rule). No openEHR spec governs the viewer: our own design.

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::adl2::TemplateFamily;
use crate::error::ViewerError;

/// The route a template's Download reads from, followed by the family and
/// the template id: `/download/template/adl14/{id}`.
pub const TEMPLATE_DOWNLOAD_ROUTE: &str = "/download/template";

/// The address of the Download of the template `template_id` of `family`.
#[must_use]
pub fn template_href(family: TemplateFamily, template_id: &str) -> String {
    format!(
        "{TEMPLATE_DOWNLOAD_ROUTE}/{}/{}",
        family.as_query(),
        urlencoding::encode(template_id)
    )
}

/// What a download of `family` is read and served as: the media type asked
/// of the CDR, and the file name's extension.
const fn served_as(family: TemplateFamily) -> (&'static str, &'static str) {
    match family {
        TemplateFamily::Adl14 => ("application/xml", "opt"),
        TemplateFamily::Adl2 => ("text/plain", "adlt"),
    }
}

/// The file name of the download of `template_id`, every character outside
/// the safe set of a quoted `filename` replaced with `_`.
fn file_name(template_id: &str, extension: &str) -> String {
    let stem: String = template_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{stem}.{extension}")
}

/// `GET /download/template/{family}/{id}`: the template as the CDR stores
/// it, as a file.
///
/// A request with no viewer session is sent to `/login`; an unknown family
/// is a `404`; a CDR refusal answers `502` with the CDR's words.
pub async fn download_template(
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    axum::extract::Path((family, template_id)): axum::extract::Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let session = crate::session::unseal(
        &state.session_keys,
        &headers,
        state.config.session.idle_minutes,
    );
    let Some(viewer) = session.get::<crate::session::ViewerSession>(crate::session::SESSION_KEY)
    else {
        return axum::response::Redirect::to("/login").into_response();
    };
    let family = match family.as_str() {
        "adl14" => TemplateFamily::Adl14,
        "adl2" => TemplateFamily::Adl2,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let (accept, extension) = served_as(family);
    let path = match family {
        TemplateFamily::Adl14 => format!(
            "definition/template/adl1.4/{}",
            urlencoding::encode(&template_id)
        ),
        TemplateFamily::Adl2 => crate::adl2::template_path(&template_id, None),
    };
    let url = state.cdr.rest_v1(&path);
    let answer = match state.cdr.get(&viewer.credential, &url, accept).await {
        Ok(response) => crate::cdr::CdrClient::expect_success(response),
        Err(error) => Err(error),
    };
    match answer {
        Ok(response) => {
            let disposition = format!(
                "attachment; filename=\"{}\"",
                file_name(&template_id, extension)
            );
            match HeaderValue::from_str(&disposition) {
                Ok(disposition) => (
                    [
                        (header::CONTENT_TYPE, HeaderValue::from_static(accept)),
                        (header::CONTENT_DISPOSITION, disposition),
                    ],
                    response.body,
                )
                    .into_response(),
                Err(_invalid) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Err(ViewerError::Unauthenticated) => axum::response::Redirect::to("/login").into_response(),
        Err(error) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{file_name, template_href};
    use crate::adl2::TemplateFamily;

    #[test]
    fn a_download_is_addressed_by_family_and_encoded_id() {
        assert_eq!(
            template_href(TemplateFamily::Adl14, "temperatur °C"),
            "/download/template/adl14/temperatur%20%C2%B0C"
        );
        assert_eq!(
            template_href(TemplateFamily::Adl2, "openEHR-EHR-COMPOSITION.cnf.v1.0.0"),
            "/download/template/adl2/openEHR-EHR-COMPOSITION.cnf.v1.0.0"
        );
    }

    #[test]
    fn a_file_name_keeps_only_safe_characters() {
        assert_eq!(
            file_name("minimal_evaluation.en.v1", "opt"),
            "minimal_evaluation.en.v1.opt"
        );
        assert_eq!(file_name("a\"b/c °", "adlt"), "a_b_c__.adlt");
    }
}

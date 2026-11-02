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
//! route enforces the viewer session like every page (the public
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

/// The template id `raw` when it can only name a template: not empty, at
/// most 255 characters, not a dot segment, and holding no slash, backslash
/// or control character, so no id reaches another CDR path once the
/// client resolves the URL (RFC 3986 §5.2.4, removing dot segments).
fn template_id(raw: &str) -> Option<&str> {
    let named = !raw.is_empty()
        && raw.chars().count() <= 255
        && raw != "."
        && raw != ".."
        && !raw.chars().any(|c| c == '/' || c == '\\' || c.is_control());
    named.then_some(raw)
}

/// The family and the CDR path a download of `template_id` of `family`
/// reads, the id percent-encoded as one path segment; `None` when either
/// part names no template, which refuses the request before any CDR call.
fn cdr_path(family: &str, template_id: &str) -> Option<(TemplateFamily, String)> {
    let family = match family {
        "adl14" => TemplateFamily::Adl14,
        "adl2" => TemplateFamily::Adl2,
        _ => return None,
    };
    let id = self::template_id(template_id)?;
    let path = match family {
        TemplateFamily::Adl14 => format!("definition/template/adl1.4/{}", urlencoding::encode(id)),
        TemplateFamily::Adl2 => crate::adl2::template_path(id, None),
    };
    Some((family, path))
}

/// `GET /download/template/{family}/{id}`: the template as the CDR stores
/// it, as a file.
///
/// A request with no viewer session is sent to `/login`; a family or an id
/// that names no template is a `404` before any CDR call; a CDR refusal
/// answers `502` with the CDR's words.
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
    let Some((family, path)) = cdr_path(&family, &template_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (accept, extension) = served_as(family);
    let url = state.cdr.rest_v1(&path);
    let answer = state
        .cdr
        .call(http::Method::GET, &url, &viewer.credential)
        .accept(accept)
        .ok()
        .await;
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
    use super::{cdr_path, file_name, template_href};
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
    fn only_a_template_id_reaches_the_cdr() {
        for id in ["..", ".", "", "../ehr", "..%2Fehr", "a/b", "a\\b", "a\nb"] {
            let decoded = urlencoding::decode(id)
                .map_or_else(|_| id.to_owned(), std::borrow::Cow::into_owned);
            assert_eq!(cdr_path("adl14", &decoded), None, "{id:?}");
            assert_eq!(cdr_path("adl2", &decoded), None, "{id:?}");
        }
        assert_eq!(cdr_path("adl14", &"x".repeat(256)), None, "an overlong id");
        assert_eq!(cdr_path("../adl1.4", "minimal"), None, "an unknown family");
        assert_eq!(cdr_path("ADL14", "minimal"), None, "the family is exact");
        assert_eq!(
            cdr_path("adl14", "temperatur °C..v1").map(|(_, path)| path),
            Some("definition/template/adl1.4/temperatur%20%C2%B0C..v1".to_owned()),
            "a dot inside an id stays in its one segment"
        );
        assert_eq!(
            cdr_path("adl2", "openEHR-EHR-COMPOSITION.cnf.v1.0.0").map(|(_, path)| path),
            Some("definition/template/adl2/openEHR-EHR-COMPOSITION.cnf.v1.0.0".to_owned())
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

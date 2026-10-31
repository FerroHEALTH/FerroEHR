// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The diagnostic a refused CDR answer carries, read out of its body.
#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use super::CdrResponse;

/// The `errors` entries of a problem, one `path: detail` line each.
///
/// FerroEHR's `validation-failed` problem lists every violation as
/// `{path, detail}` in its `errors` member, the additional detail a request
/// with `Prefer: return=representation` receives, so a `422` can name every
/// offending path instead of one generic line.
fn problem_errors(value: &serde_json::Value) -> Vec<String> {
    value
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let detail = entry.get("detail").and_then(serde_json::Value::as_str)?;
                    Some(
                        match entry.get("path").and_then(serde_json::Value::as_str) {
                            Some(path) if !path.is_empty() => format!("{path}: {detail}"),
                            _ => detail.to_owned(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `message` without a leading `word` (and the `: ` after it), which the
/// error's own display already says, so a refusal never reads "forbidden:
/// forbidden"; a message that is the word alone becomes the refusal's plain
/// reason.
pub(super) fn without_lead(message: &str, word: &str) -> String {
    let trimmed = message.trim();
    let rest = trimmed
        .get(..word.len())
        .filter(|head| head.eq_ignore_ascii_case(word))
        .and_then(|_| trimmed.get(word.len()..))
        .map(|rest| rest.trim_start_matches([':', ' ']));
    match rest {
        Some("") => "the CDR refused this session".to_owned(),
        Some(rest) => rest.to_owned(),
        None => trimmed.to_owned(),
    }
}

/// Pull the human diagnostic out of a refused body, whatever its vocabulary:
/// RFC 9457 problem details (`detail`, else `title`, then one line per
/// entry of `errors`) or a FHIR
/// `OperationOutcome` (the connector authors its refusals as outcomes — their
/// `issue[].diagnostics` join into one line), falling back to the raw body or
/// the bare status. ONE reader for both vocabularies, so no screen ever hands
/// raw JSON to a toast.
pub(super) fn diagnostic_of(response: &CdrResponse) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.body) {
        if value
            .get("resourceType")
            .and_then(serde_json::Value::as_str)
            == Some("OperationOutcome")
        {
            let joined = value
                .get("issue")
                .and_then(serde_json::Value::as_array)
                .map(|issues| {
                    issues
                        .iter()
                        .filter_map(|issue| {
                            issue
                                .get("diagnostics")
                                .or_else(|| issue.get("code"))
                                .and_then(serde_json::Value::as_str)
                                .filter(|text| !text.is_empty())
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            if !joined.is_empty() {
                return joined;
            }
        }
        let details = problem_errors(&value);
        for key in ["detail", "title"] {
            if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
                return std::iter::once(text.to_owned())
                    .chain(details)
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        }
        if !details.is_empty() {
            return details.join("\n");
        }
    }
    if response.body.is_empty() {
        format!("HTTP {}", response.status.as_u16())
    } else {
        let mut text = response.body.clone();
        text.truncate(500);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::super::CdrResponse;
    use super::diagnostic_of;

    fn response(status: http::StatusCode, body: &str) -> CdrResponse {
        CdrResponse {
            status,
            content_type: Some("application/json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: body.to_owned(),
        }
    }

    #[test]
    fn an_operation_outcome_refusal_yields_its_diagnostics_never_raw_json() {
        // #2581: the shared reader speaks the FHIR refusal vocabulary too.
        let response = CdrResponse {
            status: http::StatusCode::BAD_REQUEST,
            content_type: Some("application/fhir+json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: r#"{"resourceType":"OperationOutcome","issue":[
                {"severity":"error","code":"invalid","diagnostics":"missing field `template_id`"},
                {"severity":"error","code":"structure"}]}"#
                .to_owned(),
        };
        let text = diagnostic_of(&response);
        assert_eq!(text, "missing field `template_id`; structure");
        assert!(!text.contains("resourceType"), "never raw JSON: {text}");
    }

    #[test]
    fn diagnostic_prefers_the_problem_detail() {
        let r = response(
            http::StatusCode::BAD_REQUEST,
            r#"{"type":"https://ferroehr.eu/docs/latest/using-the-api/problems.html#aql-invalid",
                "title":"The AQL query is not valid","status":400,
                "detail":"AQL syntax error at line 1"}"#,
        );
        assert_eq!(diagnostic_of(&r), "AQL syntax error at line 1");
    }

    #[test]
    fn diagnostic_falls_back_to_title_then_raw_then_status() {
        assert_eq!(
            diagnostic_of(&response(
                http::StatusCode::CONFLICT,
                r#"{"type":"about:blank","title":"Conflict","status":409}"#
            )),
            "Conflict"
        );
        assert_eq!(
            diagnostic_of(&response(http::StatusCode::IM_A_TEAPOT, "teapot")),
            "teapot"
        );
        assert_eq!(
            diagnostic_of(&response(http::StatusCode::BAD_GATEWAY, "")),
            "HTTP 502"
        );
    }

    #[test]
    fn every_validation_error_gets_its_own_line_under_the_detail() {
        // The `validation-failed` problem lists each violation in `errors`: a
        // 422 names each offending path, so the reader renders all of them.
        let r = response(
            http::StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"type":"about:blank","status":422,"detail":"Validation failed","errors":[
                {"path":"/content[0]/data","detail":"missing mandatory element"},
                {"path":"/content[0]/items[1]","detail":"cardinality 1..1 violated"},
                {"path":"/context/start_time","detail":"value is not a DV_DATE_TIME"}]}"#,
        );
        assert_eq!(
            diagnostic_of(&r),
            "Validation failed\n/content[0]/data: missing mandatory element\n/content[0]/items[1]: \
             cardinality 1..1 violated\n/context/start_time: value is not a DV_DATE_TIME"
        );
    }

    #[test]
    fn validation_errors_render_even_without_a_detail_member() {
        let r = response(
            http::StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"status":422,"errors":[{"path":"/content","detail":"missing"}]}"#,
        );
        assert_eq!(diagnostic_of(&r), "/content: missing");
        // An empty list leaves the detail exactly as it was.
        let plain = response(
            http::StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"status":422,"detail":"Validation failed","errors":[]}"#,
        );
        assert_eq!(diagnostic_of(&plain), "Validation failed");
    }
}

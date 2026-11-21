// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The challenge of a refused bearer token: one status and one error code,
//! with the refusing check's reason code as `error_description` (RFC 6750 §3).

use super::*;

/// RFC 9068 §4 rejects a token "in case of any failure in the validation
/// checks" with one `invalid_token` answer (RFC 6750 §3.1): the status and
/// the error code never vary with the reason, and the challenge names the
/// refusing check by its stable reason code as `error_description` (RFC
/// 6750 §3), never a claim value or any part of the token.
#[tokio::test]
async fn every_invalid_token_renders_one_status_and_names_its_reason_code() {
    let auth = bearer_only().await;
    let prefix = r#"Bearer realm="ferroehr", error="invalid_token", error_description=""#;
    for (case, token) in invalid_tokens() {
        let err = auth
            .authenticate(&headers(&format!("Bearer {token}")))
            .await
            .expect_err(case);
        assert!(matches!(err, AuthError::InvalidToken(_)), "{case}: {err:?}");
        assert_eq!(
            err.to_api_error().status(),
            StatusCode::UNAUTHORIZED,
            "{case}"
        );
        let challenge = auth.challenge(Some(&err));
        let challenge = challenge.to_str().expect("an ASCII challenge");
        let code = challenge
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_else(|| panic!("{case}: {challenge}"));
        assert!(
            !code.is_empty()
                && code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{case}: the reason is a code: {challenge}"
        );
    }
    let reason = |token: &str| {
        let token = token.to_owned();
        async move {
            let err = auth
                .authenticate(&headers(&format!("Bearer {token}")))
                .await
                .expect_err("refused");
            auth.challenge(Some(&err))
                .to_str()
                .expect("ASCII")
                .to_owned()
        }
    };
    let cases = invalid_tokens();
    let named = |case: &str| {
        cases
            .iter()
            .find(|(name, _)| *name == case)
            .map(|(_, token)| token.clone())
            .expect("the case exists")
    };
    assert!(
        reason(&named("another audience"))
            .await
            .ends_with(r#"error_description="audience-mismatch""#)
    );
}

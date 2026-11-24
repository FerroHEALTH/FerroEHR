// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caseload members of a query execution body, `caseload`,
//! `caseload_roles` and `caseload_episodes`, **our own extension**.
//!
//! The draft access control ITS-REST page, Queries and the Caseload, lets a
//! query request carry the three members beside `q`, so the AQL text stays
//! portable. They are read from the JSON body of `POST /query/aql` and of a
//! stored query's `POST` execution and taken out of it before the body is
//! decoded as the ITS-REST schema, which does not declare them. A `GET`
//! execution carries no body, and every member of its query string stays an
//! AQL parameter bind as ITS-REST reads it (`Request.md` §Query parameters).
//! No openEHR specification defines the members.

#![expect(
    clippy::disallowed_types,
    reason = "the three members are read from the request's JSON body before the ITS-REST schema \
              decodes the rest of it (#1694 boundary class: wire edge)"
)]

use ferroehr::access_eval::caseload::{
    self, Ask, CASELOAD, CASELOAD_EPISODES, CASELOAD_ROLES, EPISODES_FORM, ROLES_FORM,
};
use openehr_its::rest::runtime::ApiError;
use serde_json::Value;

use crate::overview::error::RestError;

/// The caseload `body` asks for, and the body without its three members; a
/// body that is no JSON object is handed on as it is, for the schema decode
/// to refuse.
///
/// # Errors
/// `400` naming the member for an unknown value, roles that are no
/// non-empty list of codes, episodes that are no non-empty list of
/// `issuer|id` tokens, and roles or episodes without a caseload.
pub(super) fn split(body: &bytes::Bytes) -> Result<(bytes::Bytes, Option<Ask>), RestError> {
    // NOTE: our own design, a body that does not read as a JSON object is handed on whole,
    // and the ITS-REST decode answers it as it answers every malformed body.
    let Ok(Value::Object(mut members)) = serde_json::from_slice::<Value>(body) else {
        return Ok((body.clone(), None));
    };
    if ![CASELOAD, CASELOAD_ROLES, CASELOAD_EPISODES]
        .iter()
        .any(|member| members.contains_key(*member))
    {
        return Ok((body.clone(), None));
    }
    let refused =
        |refused: caseload::Refused| RestError::from(ApiError::BadRequest(refused.to_string()));
    let path = match members.remove(CASELOAD) {
        None => None,
        Some(Value::String(text)) => Some(text),
        Some(other) => return Err(refused(caseload::Refused::Value(other.to_string()))),
    };
    let roles = texts(
        members.remove(CASELOAD_ROLES),
        caseload::Refused::Roles(ROLES_FORM),
    )
    .map_err(refused)?;
    let episodes = texts(
        members.remove(CASELOAD_EPISODES),
        caseload::Refused::Episodes(EPISODES_FORM),
    )
    .map_err(refused)?;
    let ask =
        caseload::read(path.as_deref(), roles.as_deref(), episodes.as_deref()).map_err(refused)?;
    let rest = serde_json::to_vec(&Value::Object(members))
        .map_err(|e| RestError::from(ApiError::BadRequest(format!("invalid JSON body: {e}"))))?;
    Ok((rest.into(), ask))
}

/// The texts the list `member` holds; `None` when it is absent.
///
/// # Errors
/// `form` when it is no list of texts.
fn texts(
    member: Option<Value>,
    form: caseload::Refused,
) -> Result<Option<Vec<String>>, caseload::Refused> {
    match member {
        None => Ok(None),
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|item| match item {
                Value::String(text) => Ok(text),
                _ => Err(form.clone()),
            })
            .collect::<Result<Vec<String>, _>>()
            .map(Some),
        Some(_) => Err(form),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_members_leave_the_body_and_a_bad_value_is_a_400() {
        let body = bytes::Bytes::from_static(
            br#"{ "q": "SELECT e FROM EHR e", "caseload": "team", "caseload_roles": ["RESP"] }"#,
        );
        let (rest, ask) = split(&body).expect("the members read");
        assert_eq!(
            serde_json::from_slice::<Value>(&rest).expect("json"),
            serde_json::json!({ "q": "SELECT e FROM EHR e" })
        );
        assert_eq!(ask.expect("a caseload").roles.len(), 1);
        let plain = bytes::Bytes::from_static(br#"{ "q": "SELECT e FROM EHR e" }"#);
        assert_eq!(split(&plain).expect("no members").0, plain);
        let bad = bytes::Bytes::from_static(br#"{ "q": "x", "caseload": "indirect" }"#);
        let refused = split(&bad).expect_err("an unknown value");
        assert!(format!("{refused:?}").contains("caseload"), "{refused:?}");
    }
}

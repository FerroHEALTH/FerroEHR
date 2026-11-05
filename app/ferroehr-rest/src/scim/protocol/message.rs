// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The members of a SCIM message (RFC 7644 §3.1), whose attribute names are
//! case-insensitive like every SCIM attribute name (RFC 7643 §2.1).

use serde_json::{Map, Value};

use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::urn;

/// Returns the value of each of `names` in `object`, in that order, matched
/// case-insensitively.
///
/// # Errors
///
/// Returns a `400` with `invalidSyntax` when `object` carries a member that
/// is none of `names`, or one of them twice; `message` names the message.
pub(crate) fn members<'a, const N: usize>(
    object: &'a Map<String, Value>,
    names: [&str; N],
    message: &str,
) -> Result<[Option<&'a Value>; N], ScimError> {
    let mut found = [None; N];
    for (key, value) in object {
        let Some(index) = names.iter().position(|name| name.eq_ignore_ascii_case(key)) else {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                format!("'{key}' is not a member of a {message}."),
            ));
        };
        let slot = found.get_mut(index).ok_or_else(|| {
            ScimError::bad_request(
                ScimType::InvalidSyntax,
                format!("'{key}' is not a member of a {message}."),
            )
        })?;
        if slot.replace(value).is_some() {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                format!("'{key}' appears twice in a {message}."),
            ));
        }
    }
    Ok(found)
}

/// Refuses a message whose `schemas` does not list `schema` (RFC 7644 §3.1:
/// a message carries "a "schemas" attribute with a URI").
///
/// # Errors
///
/// Returns a `400` with `invalidSyntax`.
pub(crate) fn require_schema(
    schemas: Option<&Value>,
    schema: &str,
    message: &str,
) -> Result<(), ScimError> {
    let listed = schemas.and_then(Value::as_array).is_some_and(|schemas| {
        schemas
            .iter()
            .filter_map(Value::as_str)
            .any(|uri| urn::same(uri, schema))
    });
    if listed {
        Ok(())
    } else {
        Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            format!("A {message} carries 'schemas' with {schema}."),
        ))
    }
}

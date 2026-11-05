// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One attribute value against its definition: the JSON type of each data
//! type (RFC 7643 §2.3), the array of a multi-valued attribute (§2.4, RFC 7644
//! §3.8), the sub-attributes of a complex one (§2.3.8) and the unassigned
//! state (§2.5).

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Map, Value};

use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::schema::{AttributeDefinition, AttributeType, Mutability};

/// Whose value is read: a client's request, whose `readOnly` values are
/// ignored (RFC 7644 §3.3, §3.5.1), or the provider's own representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A request body or a PATCH value.
    Request,
    /// A representation the provider holds.
    Representation,
}

/// Returns `value` checked against `definition`, with every name in its
/// schema's spelling, or `None` when the value is unassigned (RFC 7643 §2.5:
/// "Unassigned attributes, the null value, or an empty array ... SHALL be
/// considered to be equivalent in "state"").
///
/// `label` names the attribute in a refusal.
pub(crate) fn normalize(
    definition: &AttributeDefinition,
    value: &Value,
    origin: Origin,
    label: &str,
) -> Result<Option<Value>, ScimError> {
    if value.is_null() {
        return Ok(None);
    }
    if !definition.multi_valued {
        return single(definition, value, origin, label);
    }
    let Value::Array(items) = value else {
        return Err(ScimError::bad_request(
            ScimType::InvalidValue,
            format!("Attribute '{label}' is multi-valued and takes an array."),
        ));
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        if item.is_null() {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                format!("Attribute '{label}' holds a null value."),
            ));
        }
        if let Some(value) = single(definition, item, origin, label)? {
            values.push(value);
        }
    }
    if values.iter().filter(|value| is_primary(value)).count() > 1 {
        // NOTE: RFC 7643 §2.4: "The primary attribute value "true" MUST appear no more than once."
        return Err(ScimError::bad_request(
            ScimType::InvalidValue,
            format!("Attribute '{label}' marks more than one value primary."),
        ));
    }
    Ok((!values.is_empty()).then_some(Value::Array(values)))
}

/// Returns whether a value of a multi-valued attribute is marked primary.
pub(crate) fn is_primary(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|members| {
            members
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("primary"))
        })
        .is_some_and(|(_, primary)| primary == &Value::Bool(true))
}

pub(crate) fn single(
    definition: &AttributeDefinition,
    value: &Value,
    origin: Origin,
    label: &str,
) -> Result<Option<Value>, ScimError> {
    let valid = match definition.kind {
        AttributeType::Complex => return complex(definition, value, origin, label),
        AttributeType::String | AttributeType::Reference => value.is_string(),
        AttributeType::Boolean => value.is_boolean(),
        AttributeType::Decimal => value.is_number(),
        // NOTE: RFC 7643 §2.3.4: an integer "MUST NOT contain fractional or exponent parts".
        AttributeType::Integer => value.is_i64() || value.is_u64(),
        AttributeType::DateTime => value.as_str().and_then(instant).is_some(),
        AttributeType::Binary => value.as_str().is_some_and(is_base64),
    };
    if valid {
        Ok(Some(value.clone()))
    } else {
        Err(ScimError::bad_request(
            ScimType::InvalidValue,
            format!(
                "Attribute '{label}' takes a value of type {}.",
                type_name(definition.kind)
            ),
        ))
    }
}

fn complex(
    definition: &AttributeDefinition,
    value: &Value,
    origin: Origin,
    label: &str,
) -> Result<Option<Value>, ScimError> {
    let Value::Object(members) = value else {
        return Err(ScimError::bad_request(
            ScimType::InvalidValue,
            format!("Attribute '{label}' is complex and takes an object."),
        ));
    };
    let mut out = Map::new();
    for (key, member) in members {
        let Some(sub) = definition.sub_attribute(key) else {
            if is_unassigned(member) {
                continue;
            }
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                format!("Sub-attribute '{key}' is not defined for '{label}'."),
            ));
        };
        if out.contains_key(&sub.name) {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                format!("Sub-attribute '{}' of '{label}' appears twice.", sub.name),
            ));
        }
        if origin == Origin::Request && sub.mutability == Mutability::ReadOnly {
            continue;
        }
        if let Some(value) = normalize(sub, member, origin, &format!("{label}.{}", sub.name))? {
            out.insert(sub.name.clone(), value);
        }
    }
    Ok((!out.is_empty()).then_some(Value::Object(out)))
}

/// Returns whether `value` is the unassigned state: null or an empty array.
pub(crate) fn is_unassigned(value: &Value) -> bool {
    value.is_null() || value.as_array().is_some_and(Vec::is_empty)
}

/// Returns the instant an `xsd:dateTime` with a date and a time names (RFC
/// 7643 §2.3.5); one without a time zone offset is read in UTC, a choice no
/// specification governs (our own design).
pub(crate) fn instant(text: &str) -> Option<Timestamp> {
    // NOTE: a text that is not a dateTime is legitimately no instant; callers refuse or skip it.
    if let Ok(timestamp) = text.parse::<Timestamp>() {
        return Some(timestamp);
    }
    let civil = text
        .parse::<jiff::civil::DateTime>()
        .ok()
        .filter(|_civil| text.contains('T'))?;
    civil
        .to_zoned(TimeZone::UTC)
        .ok()
        .map(|zoned| zoned.timestamp())
}

/// Returns whether `text` is base64, standard or URL-safe, with its trailing
/// padding optional (RFC 7643 §2.3.6: "trailing padding characters MAY be omitted").
fn is_base64(text: &str) -> bool {
    let config =
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);
    let standard = GeneralPurpose::new(&base64::alphabet::STANDARD, config);
    let url_safe = GeneralPurpose::new(&base64::alphabet::URL_SAFE, config);
    standard.decode(text).is_ok() || url_safe.decode(text).is_ok()
}

const fn type_name(kind: AttributeType) -> &'static str {
    match kind {
        AttributeType::String => "string",
        AttributeType::Boolean => "boolean",
        AttributeType::Decimal => "decimal",
        AttributeType::Integer => "integer",
        AttributeType::DateTime => "dateTime",
        AttributeType::Binary => "binary",
        AttributeType::Reference => "reference",
        AttributeType::Complex => "complex",
    }
}

#[cfg(test)]
mod tests {
    use super::{instant, is_base64};

    #[test]
    fn date_times_carry_a_date_and_a_time() {
        for good in [
            "2008-01-23T04:56:22Z",
            "2011-08-01T21:32:44.882Z",
            "2008-01-23T04:56:22+01:00",
            "2008-01-23T04:56:22",
        ] {
            assert!(instant(good).is_some(), "{good}");
        }
        for bad in ["2008-01-23", "yesterday", "04:56:22"] {
            assert!(instant(bad).is_none(), "{bad}");
        }
        assert_eq!(
            instant("2008-01-23T05:56:22+01:00"),
            instant("2008-01-23T04:56:22Z")
        );
    }

    #[test]
    fn base64_admits_both_alphabets_and_no_whitespace() {
        assert!(is_base64("AA=="));
        assert!(is_base64("AA"));
        assert!(is_base64("-_8"));
        assert!(!is_base64("AA AA"));
        assert!(!is_base64("not base64!"));
    }
}

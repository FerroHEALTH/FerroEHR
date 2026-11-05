// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Resource versioning (RFC 7644 §3.14): entity tags (RFC 9110 §8.8.3) and
//! the `If-Match` and `If-None-Match` preconditions (RFC 9110 §13.1.1,
//! §13.1.2).
//!
//! RFC 7644 §3.14 makes weak tags "the preferred mechanism" and sends one in
//! `If-Match` (`If-Match: W/"e180ee84f0671b1"`), which the strong comparison
//! RFC 9110 §13.1.1 prescribes for `If-Match` could never match. SCIM is the
//! profile in play, so [`if_match`] compares weakly; a request without the
//! header is accepted, since §3.14 lets a client send one.

use std::fmt;
use std::str::FromStr;

use http::header::{IF_MATCH, IF_NONE_MATCH};
use http::{HeaderMap, StatusCode};

use crate::scim::protocol::error::ScimError;

/// An entity tag, `[ weak ] opaque-tag` (RFC 9110 §8.8.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntityTag {
    weak: bool,
    opaque: String,
}

/// A text that is not an entity tag, or an opaque value no tag can carry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{0}' is not an entity tag (RFC 9110 §8.8.3)")]
pub struct EntityTagError(String);

impl EntityTag {
    /// Creates a weak tag over `opaque`, the form a SCIM version takes (RFC
    /// 7643 §3.1: a version that is not a strong validator "MUST [be marked]
    /// as weak").
    ///
    /// # Errors
    ///
    /// Returns [`EntityTagError`] when `opaque` holds a character `etagc`
    /// excludes: a double quote, a space or a control character.
    pub fn weak(opaque: impl Into<String>) -> Result<Self, EntityTagError> {
        let opaque = opaque.into();
        if opaque.chars().all(is_etagc) {
            Ok(Self { weak: true, opaque })
        } else {
            Err(EntityTagError(opaque))
        }
    }

    /// Creates a strong tag over `opaque`.
    ///
    /// # Errors
    ///
    /// Returns [`EntityTagError`] as [`EntityTag::weak`] does.
    pub fn strong(opaque: impl Into<String>) -> Result<Self, EntityTagError> {
        let opaque = opaque.into();
        if opaque.chars().all(is_etagc) {
            Ok(Self {
                weak: false,
                opaque,
            })
        } else {
            Err(EntityTagError(opaque))
        }
    }

    /// Returns whether the tag is weak.
    #[must_use]
    pub const fn is_weak(&self) -> bool {
        self.weak
    }

    /// Returns the opaque value, without quotes.
    #[must_use]
    pub fn opaque(&self) -> &str {
        &self.opaque
    }

    /// Returns whether both tags are strong and their opaque values match
    /// (RFC 9110 §8.8.3.2, "Strong comparison").
    #[must_use]
    pub fn strong_eq(&self, other: &Self) -> bool {
        !self.weak && !other.weak && self.opaque == other.opaque
    }

    /// Returns whether the opaque values match, weak or not (RFC 9110
    /// §8.8.3.2, "Weak comparison").
    #[must_use]
    pub fn weak_eq(&self, other: &Self) -> bool {
        self.opaque == other.opaque
    }
}

/// `etagc = %x21 / %x23-7E / obs-text` (RFC 9110 §8.8.3).
const fn is_etagc(c: char) -> bool {
    matches!(c, '\u{21}' | '\u{23}'..='\u{7e}' | '\u{80}'..='\u{ff}')
}

impl FromStr for EntityTag {
    type Err = EntityTagError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || EntityTagError(text.to_owned());
        let (weak, quoted) = match text.strip_prefix("W/") {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let opaque = quoted
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .ok_or_else(invalid)?;
        if !opaque.chars().all(is_etagc) {
            return Err(invalid());
        }
        Ok(Self {
            weak,
            opaque: opaque.to_owned(),
        })
    }
}

impl fmt::Display for EntityTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.weak {
            f.write_str("W/")?;
        }
        write!(f, "\"{}\"", self.opaque)
    }
}

/// A precondition header's value: `"*" / #entity-tag` (RFC 9110 §13.1.1, §13.1.2).
enum Condition {
    /// `*`.
    Any,
    /// The listed tags.
    Tags(Vec<EntityTag>),
    /// A value of neither form.
    Malformed,
}

/// Reads every field of the header `name` as one list (RFC 9110 §5.3).
fn condition(headers: &HeaderMap, name: &http::HeaderName) -> Option<Condition> {
    let mut fields = headers.get_all(name).iter().peekable();
    fields.peek()?;
    let mut members = Vec::new();
    for field in fields {
        let Ok(text) = field.to_str() else {
            return Some(Condition::Malformed);
        };
        members.extend(
            text.split(',')
                .map(str::trim)
                .filter(|member| !member.is_empty()),
        );
    }
    // NOTE: RFC 9110 §13.1.1: a list holding "*" and other values "is syntactically invalid".
    if members == ["*"] {
        return Some(Condition::Any);
    }
    let tags: Result<Vec<EntityTag>, _> = members
        .iter()
        .map(|member| member.parse::<EntityTag>())
        .collect();
    Some(tags.map_or(Condition::Malformed, Condition::Tags))
}

/// Evaluates `If-Match` against the resource's current tag.
///
/// The condition holds for `*`, or when a listed tag matches `current` by
/// the weak comparison (see the module documentation). A request without
/// `If-Match` passes.
///
/// # Errors
///
/// Returns a `412 Precondition Failed` when the condition is false, a
/// malformed value included (RFC 9110 §13.1.1: "Otherwise, the condition is
/// false").
pub fn if_match(headers: &HeaderMap, current: &EntityTag) -> Result<(), ScimError> {
    let holds = match condition(headers, &IF_MATCH) {
        None | Some(Condition::Any) => true,
        Some(Condition::Tags(tags)) => tags.iter().any(|tag| tag.weak_eq(current)),
        Some(Condition::Malformed) => false,
    };
    if holds {
        Ok(())
    } else {
        Err(ScimError::new(
            StatusCode::PRECONDITION_FAILED,
            format!("The resource has changed; its version is {current}."),
        ))
    }
}

/// What a read does after `If-None-Match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPrecondition {
    /// Serve the representation.
    Proceed,
    /// Answer `304 Not Modified` (RFC 9110 §13.1.2; RFC 7644 §3.14: "an
    /// empty body with a 304 (Not Modified) response code").
    NotModified,
}

/// Evaluates `If-None-Match` on a read against the current tag, with the
/// weak comparison RFC 9110 §13.1.2 prescribes; a malformed value leaves the
/// condition true ("Otherwise, the condition is true").
#[must_use]
pub fn if_none_match_read(headers: &HeaderMap, current: &EntityTag) -> ReadPrecondition {
    if none_match_holds(headers, current) {
        ReadPrecondition::Proceed
    } else {
        ReadPrecondition::NotModified
    }
}

/// Evaluates `If-None-Match` on a write against the current tag.
///
/// # Errors
///
/// Returns a `412 Precondition Failed` when the condition is false, as RFC
/// 9110 §13.1.2 requires "for all other request methods".
pub fn if_none_match_write(headers: &HeaderMap, current: &EntityTag) -> Result<(), ScimError> {
    if none_match_holds(headers, current) {
        Ok(())
    } else {
        Err(ScimError::new(
            StatusCode::PRECONDITION_FAILED,
            format!("The resource is at version {current}."),
        ))
    }
}

fn none_match_holds(headers: &HeaderMap, current: &EntityTag) -> bool {
    match condition(headers, &IF_NONE_MATCH) {
        None | Some(Condition::Malformed) => true,
        Some(Condition::Any) => false,
        Some(Condition::Tags(tags)) => !tags.iter().any(|tag| tag.weak_eq(current)),
    }
}

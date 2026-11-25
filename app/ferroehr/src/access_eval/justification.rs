// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Justified access: a caller admitted outside its care relationships on a
//! justification of the controller's policy (`ACCESS_POLICY.justifications`),
//! by a declared purpose of use and a reason it states.
//!
//! A request invokes the first justification whose `purposes` hold the
//! declared purpose (HL7 v3 `ActReason`, such as `TREAT`, `ETREAT` or `BTG`)
//! and whose `applies_to` names the caller. An emergency justification lifts
//! what EHDS Art. 10(2) and 11(5) let an emergency lift; any other stands in
//! for a live care relationship and lifts nothing the subject set (EHDS
//! Art. 8, GDPR Art. 18). The reason is the text FHIR R4
//! `AuditEvent.purposeOfEvent.text` carries. Which justification a request
//! invokes, and the bound of the reason, are our own design (#6587).

use std::fmt;

use openehr_rm::draft::access_control::access_control::access_justification::AccessJustification;
use openehr_rm::draft::access_control::terminology::CallerKind;
use openehr_rm::draft::access_log::access_log::access_authorisation::AccessAuthorisation;

use crate::access_eval::directive::Reader;
use crate::access_eval::scope::Pred;

/// The justification a request invokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invoked {
    /// Its position in the policy's `justifications`.
    pub at: usize,
    /// Whether it is an emergency justification.
    pub emergency: bool,
    /// Whether the request states a reason.
    pub reasoned: bool,
}

impl Invoked {
    /// Whether the invocation lifts what an emergency lifts: an emergency
    /// justification with a reason.
    #[must_use]
    pub const fn lifts(self) -> bool {
        self.emergency && self.reasoned
    }
}

/// The justification an access rested on, with the data it admitted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Justified {
    /// Its position in the policy's `justifications`.
    pub at: usize,
    /// Whether it is an emergency justification.
    pub emergency: bool,
    /// The data it admitted: every datum for an emergency, and for any other
    /// justification the data no care relationship of the caller covers.
    pub cover: Pred,
}

/// The justification the request of `read` invokes; `None` when it declares
/// no purpose of one, or no justification names the caller. The subject and
/// a representative act inside their own standing and never invoke one.
pub(super) fn invoked(read: &Reader<'_, '_>) -> Option<Invoked> {
    let judged = read.judged;
    let kind = judged.caller.kind;
    if matches!(kind, CallerKind::Subject | CallerKind::Representative) {
        return None;
    }
    let purpose = judged.asked.purpose?;
    let names = |justification: &AccessJustification| match justification.applies_to.as_deref() {
        None => kind == CallerKind::Professional,
        scopes => read.allows(scopes),
    };
    judged
        .policy
        .justifications
        .iter()
        .enumerate()
        .find(|(_, justification)| {
            justification
                .purposes
                .iter()
                .any(|coded| coded.defining_code.code_string == purpose)
                && names(justification)
        })
        .map(|(at, justification)| Invoked {
            at,
            emergency: justification.emergency,
            reasoned: judged.asked.reasoned,
        })
}

/// A justified access as its record names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Justification {
    /// Whether it rested on an emergency justification (EHDS Art. 11(5)).
    pub emergency: bool,
    /// The reason the caller stated.
    pub reason: Reason,
}

impl Justification {
    /// The justification a stored entry's `authorisations` name: the ground
    /// of kind `justification` or `emergency` that carries a reason; `None`
    /// when none does.
    #[must_use]
    pub fn of_grounds(grounds: &[AccessAuthorisation]) -> Option<Self> {
        grounds.iter().find_map(|ground| {
            let emergency = match ground.kind.defining_code.code_string.as_str() {
                "emergency" => true,
                "justification" => false,
                _ => return None,
            };
            let reason = Reason::new(ground.reason.as_deref()?).ok()?;
            Some(Self { emergency, reason })
        })
    }

    /// The code the record stores the justification's kind under: the
    /// `access authorisation` code of its ground.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        if self.emergency {
            "emergency"
        } else {
            "justification"
        }
    }
}

/// The reason a caller states for a justified access, in its own words.
///
/// It may name clinical facts, so it never reaches a log or a `Debug`
/// output; the access record alone keeps it.
#[derive(Clone, PartialEq, Eq)]
pub struct Reason(String);

/// The most characters a reason holds: our own bound (#6587).
pub const REASON_MAX_CHARS: usize = 1000;

impl Reason {
    /// The reason a request header states: UTF-8, percent-encoded (RFC 3986
    /// §2.1), and after decoding and trimming 1 to [`REASON_MAX_CHARS`]
    /// characters with no control character.
    ///
    /// # Errors
    ///
    /// [`ReasonError`] for a value that is not percent-encoded UTF-8, is
    /// empty, holds a control character or is longer than the bound.
    pub fn from_header(raw: &[u8]) -> Result<Self, ReasonError> {
        let ascii = std::str::from_utf8(raw).map_err(|_not_utf8| ReasonError::Encoding)?;
        let decoded = urlencoding::decode(ascii).map_err(|_not_utf8| ReasonError::Encoding)?;
        Self::new(&decoded)
    }

    /// The reason `text` states, trimmed.
    ///
    /// # Errors
    ///
    /// [`ReasonError`] for a text that is empty, holds a control character or
    /// is longer than [`REASON_MAX_CHARS`] characters.
    pub fn new(text: &str) -> Result<Self, ReasonError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ReasonError::Empty);
        }
        if text.chars().any(char::is_control) {
            return Err(ReasonError::Control);
        }
        if text.chars().count() > REASON_MAX_CHARS {
            return Err(ReasonError::TooLong);
        }
        Ok(Self(text.to_owned()))
    }

    /// The reason's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Reason(..)")
    }
}

/// Why a stated reason is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReasonError {
    /// The value is not percent-encoded UTF-8.
    #[error("the reason is not percent-encoded UTF-8")]
    Encoding,
    /// The value is empty.
    #[error("the reason is empty")]
    Empty,
    /// The value holds a control character.
    #[error("the reason holds a control character")]
    Control,
    /// The value is longer than the bound.
    #[error("the reason is longer than {REASON_MAX_CHARS} characters")]
    TooLong,
}

#[cfg(test)]
mod tests {
    use super::{REASON_MAX_CHARS, Reason, ReasonError};

    #[test]
    fn a_percent_encoded_reason_decodes_to_its_text() {
        let reason = Reason::from_header(b"Second%20opinion%20f%C3%BCr%20Dr.%20M%C3%BCller")
            .expect("a well-formed reason");
        assert_eq!(reason.as_str(), "Second opinion für Dr. Müller");
    }

    #[test]
    fn a_reason_at_the_bound_holds_and_one_over_it_is_refused() {
        let at = "é".repeat(REASON_MAX_CHARS);
        assert!(Reason::new(&at).is_ok());
        let over = "é".repeat(REASON_MAX_CHARS + 1);
        assert_eq!(Reason::new(&over), Err(ReasonError::TooLong));
    }

    #[test]
    fn an_empty_a_control_and_a_malformed_reason_are_refused() {
        assert_eq!(Reason::new("   "), Err(ReasonError::Empty));
        assert_eq!(Reason::new("a\nb"), Err(ReasonError::Control));
        assert_eq!(Reason::from_header(b"%C3%28"), Err(ReasonError::Encoding));
        assert_eq!(Reason::from_header(b"a%0Ab"), Err(ReasonError::Control));
    }

    #[test]
    fn a_reason_never_shows_in_debug() {
        let reason = Reason::new("suspected sepsis in patient X").expect("a reason");
        assert_eq!(format!("{reason:?}"), "Reason(..)");
    }
}

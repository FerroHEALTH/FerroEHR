// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The ITS-REST `ETag`: an entity tag whose opaque tag is an openEHR
//! identifier.
//!
//! ITS-REST 1.1.0 `overview/Requests_and_responses.md` §ETag and Last-Modified:
//! the value "is usually taken from e.g. VERSIONED_OBJECT.uid.value,
//! VERSION.uid.value, EHR.ehr_id.value", enclosed in double quotes, and is
//! "of weak-type and should have a weakness indicator `W/` prefix"; the form
//! without `W/` used before Release 1.1.0 is deprecated, and implementations
//! "MAY still support it". The syntax is RFC 9110 §8.8.3:
//! `entity-tag = [ weak ] opaque-tag`, `weak = %s"W/"`,
//! `opaque-tag = DQUOTE *etagc DQUOTE`, `etagc = %x21 / %x23-7E / obs-text`.
//! [`EntityTag`] reads and writes that form, and types the tag as the
//! `openehr-base` identifier a header names: an `OBJECT_VERSION_ID` for a
//! version (`ETag`, `ETag_VERSION`, `ETag_COMPOSITION`), a `HIER_OBJECT_ID`
//! for an EHR or a contribution (`ETag_EHR`, `ETag_CONTRIBUTION`).

use core::fmt;
use core::str::FromStr;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::lexical::IdError;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;

/// An entity tag: whether it is weak, and its opaque tag without the quotes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntityTag {
    weak: bool,
    tag: String,
}

/// Why a field value is no RFC 9110 entity tag.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntityTagError {
    /// The opaque tag does not open with a double quote.
    #[error("an entity tag opens with a double quote, after an optional `W/`")]
    Unquoted,
    /// The opaque tag is not closed by a double quote at the end of the value.
    #[error("an entity tag closes with a double quote")]
    Unclosed,
    /// The opaque tag holds a character `etagc` excludes (a space, a control
    /// character or a double quote).
    #[error("{character:?} at byte {at} is not allowed in an entity tag")]
    Character {
        /// The byte offset of the character in the opaque tag.
        at: usize,
        /// The character.
        character: char,
    },
}

/// Whether `c` is an RFC 9110 `etagc`: `%x21 / %x23-7E / obs-text`.
fn is_etagc(c: char) -> bool {
    c == '\u{21}' || ('\u{23}'..='\u{7e}').contains(&c) || u32::from(c) >= 0x80
}

/// The first character of `tag` that is no `etagc`, as an error.
fn check(tag: &str) -> Result<(), EntityTagError> {
    match tag.char_indices().find(|&(_, c)| !is_etagc(c)) {
        Some((at, character)) => Err(EntityTagError::Character { at, character }),
        None => Ok(()),
    }
}

impl EntityTag {
    /// A weak entity tag, the form ITS-REST 1.1.0 serves (`W/"…"`).
    ///
    /// # Errors
    ///
    /// [`EntityTagError::Character`] when `tag` holds a character no opaque tag
    /// may carry.
    pub fn weak(tag: impl Into<String>) -> Result<Self, EntityTagError> {
        let tag = tag.into();
        check(&tag)?;
        Ok(Self { weak: true, tag })
    }

    /// A strong entity tag (`"…"`), the form deprecated since ITS-REST 1.1.0.
    ///
    /// # Errors
    ///
    /// [`EntityTagError::Character`] when `tag` holds a character no opaque tag
    /// may carry.
    pub fn strong(tag: impl Into<String>) -> Result<Self, EntityTagError> {
        let tag = tag.into();
        check(&tag)?;
        Ok(Self { weak: false, tag })
    }

    /// Whether the tag carries the weakness indicator `W/`.
    #[must_use]
    pub fn is_weak(&self) -> bool {
        self.weak
    }

    /// The opaque tag, without the quotes or the weakness indicator.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// RFC 9110 §8.8.3.2 weak comparison: the opaque tags are equal, whether
    /// or not either is weak (the comparison `If-None-Match` uses).
    #[must_use]
    pub fn weakly_matches(&self, other: &Self) -> bool {
        self.tag == other.tag
    }

    /// RFC 9110 §8.8.3.2 strong comparison: neither is weak and the opaque
    /// tags are equal (the comparison `If-Match` uses).
    #[must_use]
    pub fn strongly_matches(&self, other: &Self) -> bool {
        !self.weak && !other.weak && self.tag == other.tag
    }

    /// The tag as the `OBJECT_VERSION_ID` of a version (`ETag_VERSION`,
    /// `ETag_COMPOSITION`, the `version_uid` an update answers with).
    ///
    /// # Errors
    ///
    /// The [`IdError`] of [`ObjectVersionId::new`] when the tag is no
    /// `OBJECT_VERSION_ID`.
    pub fn object_version_id(&self) -> Result<ObjectVersionId, IdError> {
        ObjectVersionId::new(self.tag.as_str())
    }

    /// The tag as a `HIER_OBJECT_ID` (`ETag_EHR`, the `ehr_id`;
    /// `ETag_CONTRIBUTION`, the `contribution_uid`).
    ///
    /// # Errors
    ///
    /// The [`IdError`] of [`HierObjectId::new`] when the tag is no
    /// `HIER_OBJECT_ID`.
    pub fn hier_object_id(&self) -> Result<HierObjectId, IdError> {
        HierObjectId::new(self.tag.as_str())
    }
}

impl FromStr for EntityTag {
    type Err = EntityTagError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (weak, opaque) = match s.strip_prefix("W/") {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let inner = opaque.strip_prefix('"').ok_or(EntityTagError::Unquoted)?;
        let tag = inner.strip_suffix('"').ok_or(EntityTagError::Unclosed)?;
        check(tag)?;
        Ok(Self {
            weak,
            tag: tag.to_owned(),
        })
    }
}

impl fmt::Display for EntityTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.weak {
            f.write_str("W/")?;
        }
        write!(f, "\"{}\"", self.tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_weak_version_tag_reads_as_its_version_id() {
        // ITS-REST 1.1.0 overview §ETag and Last-Modified, the example value.
        let tag: EntityTag =
            "W/\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2\""
                .parse()
                .unwrap();
        assert!(tag.is_weak());
        let version = tag.object_version_id().unwrap();
        assert_eq!(
            version.object_id_str(),
            "8849182c-82ad-4088-a07f-48ead4180515"
        );
        assert_eq!(
            tag.to_string(),
            "W/\"8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2\""
        );
    }

    #[test]
    fn the_deprecated_strong_form_still_reads() {
        let tag: EntityTag = "\"7d44b88c-4199-4bad-97dc-d78268e01398\"".parse().unwrap();
        assert!(!tag.is_weak());
        assert_eq!(
            tag.hier_object_id().unwrap().value(),
            "7d44b88c-4199-4bad-97dc-d78268e01398"
        );
    }

    #[test]
    fn a_value_outside_the_grammar_is_refused() {
        assert_eq!("abc".parse::<EntityTag>(), Err(EntityTagError::Unquoted));
        assert_eq!("W/abc".parse::<EntityTag>(), Err(EntityTagError::Unquoted));
        assert_eq!(
            "W/\"abc".parse::<EntityTag>(),
            Err(EntityTagError::Unclosed)
        );
        assert_eq!(
            "\"a b\"".parse::<EntityTag>(),
            Err(EntityTagError::Character {
                at: 1,
                character: ' '
            })
        );
        assert!(EntityTag::weak("a\"b").is_err());
    }

    #[test]
    fn a_generated_headers_struct_reads_its_etag() {
        let headers = crate::rest::generated::ehr::EhrCreateCreatedHeaders {
            etag: Some("W/\"7d44b88c-4199-4bad-97dc-d78268e01398\"".to_owned()),
            ..Default::default()
        };
        let tag = headers.entity_tag().unwrap().unwrap();
        assert_eq!(tag.tag(), "7d44b88c-4199-4bad-97dc-d78268e01398");
        let none = crate::rest::generated::ehr::EhrCreateCreatedHeaders::default();
        assert!(none.entity_tag().is_none());
    }

    #[test]
    fn weak_and_strong_comparison_follow_rfc_9110() {
        let weak = EntityTag::weak("1").unwrap();
        let strong = EntityTag::strong("1").unwrap();
        assert!(weak.weakly_matches(&strong));
        assert!(!weak.strongly_matches(&strong));
        assert!(strong.strongly_matches(&strong.clone()));
        assert!(EntityTag::weak("x").unwrap().object_version_id().is_err());
    }
}

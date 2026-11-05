// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Attribute paths (RFC 7644 §3.10, Figure 1 `attrPath`) and PATCH paths
//! (RFC 7644 §3.5.2, Figure 7).
//!
//! Every facet of a path compares case-insensitively (§3.10), so [`AttrName`]
//! and [`AttrPath`] implement equality that way and keep the spelling they were
//! written with for display.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::scim::protocol::filter::ValueFilter;
use crate::scim::protocol::urn;

/// An attribute name: `ATTRNAME` of RFC 7643 §2.1, or the `$ref`
/// sub-attribute RFC 7643 §2.4 defines.
#[derive(Debug, Clone)]
pub struct AttrName(String);

impl AttrName {
    /// Returns the name `text` spells, or `None` when it is not an `ATTRNAME`.
    ///
    /// RFC 7643 §2.1: `ATTRNAME = ALPHA *(nameChar)` with `nameChar = "$" / "-"
    /// / "_" / DIGIT / ALPHA`; RFC 7643 §2.4 names `$ref`, which starts with
    /// the `$` the grammar admits only later in a name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut chars = text.chars();
        let first = chars.next()?;
        let valid = text.eq_ignore_ascii_case("$ref")
            || (first.is_ascii_alphabetic()
                && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '$' | '-' | '_')));
        valid.then(|| Self(text.to_owned()))
    }

    /// Returns the name as it was written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether this name and `other` are the same attribute name.
    #[must_use]
    pub fn matches(&self, other: &str) -> bool {
        self.0.eq_ignore_ascii_case(other)
    }
}

impl PartialEq for AttrName {
    fn eq(&self, other: &Self) -> bool {
        self.matches(&other.0)
    }
}

impl Eq for AttrName {}

impl Hash for AttrName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for byte in self.0.bytes() {
            state.write_u8(byte.to_ascii_lowercase());
        }
    }
}

impl fmt::Display for AttrName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An attribute path, `[URI ":"] ATTRNAME *1subAttr` (RFC 7644 Figure 1).
#[derive(Debug, Clone)]
pub struct AttrPath {
    /// The schema URI that qualifies the name, if the path carries one.
    pub schema: Option<String>,
    /// The attribute name.
    pub name: AttrName,
    /// The sub-attribute of a complex attribute.
    pub sub_attr: Option<AttrName>,
}

/// A text that is not an attribute path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{0}' is not an attribute path")]
pub struct PathError(pub String);

impl AttrPath {
    /// Parses `text` as an attribute path.
    ///
    /// The schema URI is everything before the last `:`, since an attribute
    /// name holds no colon and a schema URN holds several.
    ///
    /// # Errors
    ///
    /// Returns [`PathError`] when `text` is not `[URI ":"] ATTRNAME *1subAttr`.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let invalid = || PathError(text.to_owned());
        let (schema, rest) = match text.rsplit_once(':') {
            Some((schema, rest)) if is_uri(schema) => (Some(schema.to_owned()), rest),
            Some(_) => return Err(invalid()),
            None => (None, text),
        };
        let (name, sub_attr) = match rest.split_once('.') {
            Some((name, sub)) => (name, Some(AttrName::parse(sub).ok_or_else(invalid)?)),
            None => (rest, None),
        };
        Ok(Self {
            schema,
            name: AttrName::parse(name).ok_or_else(invalid)?,
            sub_attr,
        })
    }

    /// Returns the path of the attribute `name` with no schema and no sub-attribute.
    #[must_use]
    pub const fn attribute(name: AttrName) -> Self {
        Self {
            schema: None,
            name,
            sub_attr: None,
        }
    }
}

/// Returns whether `text` has the shape of an absolute URI, `scheme ":" rest`
/// (RFC 3986 §3), which is all a schema URI in a path needs to be told apart
/// from an attribute name.
fn is_uri(text: &str) -> bool {
    let Some((scheme, rest)) = text.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let scheme_valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    scheme_valid && !rest.is_empty()
}

impl PartialEq for AttrPath {
    fn eq(&self, other: &Self) -> bool {
        let schema = match (&self.schema, &other.schema) {
            (Some(left), Some(right)) => urn::same(left, right),
            (None, None) => true,
            _ => false,
        };
        schema && self.name == other.name && self.sub_attr == other.sub_attr
    }
}

impl Eq for AttrPath {}

impl fmt::Display for AttrPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(schema) = &self.schema {
            write!(f, "{schema}:")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(sub) = &self.sub_attr {
            write!(f, ".{sub}")?;
        }
        Ok(())
    }
}

/// The target of a PATCH operation, `attrPath / valuePath [subAttr]` (RFC 7644
/// §3.5.2, Figure 7); `PatchPath::parse` reads one with the filter grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum PatchPath {
    /// An attribute, or a sub-attribute of a complex attribute.
    Attribute(AttrPath),
    /// The values of a multi-valued attribute a filter selects, or one
    /// sub-attribute of each of them.
    Values {
        /// The multi-valued attribute.
        attr: AttrPath,
        /// The filter over its values' sub-attributes.
        filter: ValueFilter,
        /// The sub-attribute of each selected value.
        sub_attr: Option<AttrName>,
    },
}

impl fmt::Display for PatchPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attribute(path) => write!(f, "{path}"),
            Self::Values {
                attr,
                filter,
                sub_attr,
            } => {
                write!(f, "{attr}[{filter}]")?;
                if let Some(sub) = sub_attr {
                    write!(f, ".{sub}")?;
                }
                Ok(())
            }
        }
    }
}

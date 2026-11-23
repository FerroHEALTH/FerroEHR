// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The simplified-path (FLAT key) model: parsing and printing only
//! (`simplified_formats/master04-basic_concepts.adoc` §Field Identifiers, §RM
//! Attributes prefix), e.g. `vital_signs/body_temperature:0/any_event:0/temperature|magnitude`.

use std::fmt::{Display, Formatter};

use crate::flat::error::FlatError;

/// One `/`-separated path step: a name plus an optional zero-based instance
/// index (`master04 §Instance Indexing`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Segment {
    /// The node identifier (a Web-Template node id, `ctx`, or a `_`-prefixed
    /// RM attribute name).
    pub name: String,
    /// The `:i` instance index, when present.
    pub index: Option<u32>,
}

impl Segment {
    /// Whether this segment addresses an optional RM attribute
    /// (`master04 §RM Attributes prefix`).
    #[must_use]
    pub fn is_rm_attribute(&self) -> bool {
        self.name.starts_with('_')
    }
}

/// One `|`-separated attribute suffix part, with an optional `:i` index
/// (`master06 §Participation`: `ctx/participation_identifiers:1|issuer:0`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Suffix {
    /// The suffix name (without the pipe).
    pub name: String,
    /// The `:i` index, when present.
    pub index: Option<u32>,
}

/// A parsed FLAT key: the segment path plus any suffix chain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlatKey {
    /// The `/`-separated segments, in order.
    pub segments: Vec<Segment>,
    /// The `|`-separated suffix chain on the final segment (usually zero or
    /// one entry; `_link:0|meaning|code` produces two).
    pub suffixes: Vec<Suffix>,
}

impl FlatKey {
    /// Whether this key belongs to the context namespace
    /// (`master04 §Context`: context fields MUST use the `ctx/` prefix).
    #[must_use]
    pub fn is_ctx(&self) -> bool {
        self.segments.first().is_some_and(|s| s.name == "ctx")
    }

    /// Parse a FLAT key.
    ///
    /// # Errors
    /// [`FlatError::MalformedPath`] on empty keys, empty segments or suffix
    /// names, or a non-numeric `:index`.
    pub fn parse(key: &str) -> Result<Self, FlatError> {
        let malformed = |reason: &str| FlatError::MalformedPath {
            path: key.to_owned(),
            reason: reason.to_owned(),
        };
        if key.is_empty() {
            return Err(malformed("empty key"));
        }
        let mut pipe_parts = key.split('|');
        let segment_part = pipe_parts.next().unwrap_or_default();
        let mut segments = Vec::new();
        for raw in segment_part.split('/') {
            if raw.is_empty() {
                return Err(malformed("empty path segment"));
            }
            let (name, index) = split_index(raw, key)?;
            segments.push(Segment {
                name: name.to_owned(),
                index,
            });
        }
        let mut suffixes = Vec::new();
        for raw in pipe_parts {
            if raw.is_empty() {
                return Err(malformed("empty attribute suffix"));
            }
            let (name, index) = split_index(raw, key)?;
            suffixes.push(Suffix {
                name: name.to_owned(),
                index,
            });
        }
        Ok(Self { segments, suffixes })
    }
}

impl Display for FlatKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            f.write_str(&seg.name)?;
            if let Some(idx) = seg.index {
                write!(f, ":{idx}")?;
            }
        }
        for suffix in &self.suffixes {
            write!(f, "|{}", suffix.name)?;
            if let Some(idx) = suffix.index {
                write!(f, ":{idx}")?;
            }
        }
        Ok(())
    }
}

/// The largest accepted `:i` instance index (no openEHR spec governs this: our
/// own resource bound).
pub const MAX_INSTANCE_INDEX: u32 = 65_535;

/// The name of one `name[:i]` part as [`split_index`] reads it, or `None` where
/// [`split_index`] refuses it.
pub(crate) fn name_of(raw: &str) -> Option<&str> {
    if raw.is_empty() {
        return None;
    }
    match raw.split_once(':') {
        None => Some(raw),
        Some((name, idx)) => (!name.is_empty()
            && idx.parse::<u32>().is_ok_and(|i| i <= MAX_INSTANCE_INDEX))
        .then_some(name),
    }
}

/// The name and index of one `name[:i]` part of a key [`well_formed`]
/// accepts; allocates nothing.
pub(crate) fn split_name_index(raw: &str) -> (&str, Option<u32>) {
    match raw.split_once(':') {
        None => (raw, None),
        Some((name, idx)) => (name, idx.parse::<u32>().ok()),
    }
}

/// Whether [`FlatKey::parse`] reads `key`; allocates nothing.
pub(crate) fn well_formed(key: &str) -> bool {
    let mut pipe_parts = key.split('|');
    let segment_part = pipe_parts.next().unwrap_or_default();
    !key.is_empty()
        && segment_part.split('/').all(|raw| name_of(raw).is_some())
        && pipe_parts.all(|raw| name_of(raw).is_some())
}

/// Split a trailing `:<digits>` index off a segment or suffix part; node ids
/// cannot contain `:` (`master04 §Node ID Generation Rules`).
fn split_index<'a>(raw: &'a str, whole_key: &str) -> Result<(&'a str, Option<u32>), FlatError> {
    match raw.split_once(':') {
        None => Ok((raw, None)),
        Some((name, idx)) => {
            if name.is_empty() {
                return Err(FlatError::MalformedPath {
                    path: whole_key.to_owned(),
                    reason: "empty name before ':'".to_owned(),
                });
            }
            // The offending lexeme is already in the error message; a
            // `ParseIntError` adds nothing to it.
            let Ok(parsed) = idx.parse::<u32>() else {
                return Err(FlatError::MalformedPath {
                    path: whole_key.to_owned(),
                    reason: format!("non-numeric instance index {idx:?}"),
                });
            };
            if parsed > MAX_INSTANCE_INDEX {
                return Err(FlatError::MalformedPath {
                    path: whole_key.to_owned(),
                    reason: format!(
                        "instance index {parsed} exceeds the supported maximum {MAX_INSTANCE_INDEX}"
                    ),
                });
            }
            Ok((name, Some(parsed)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_leaf_with_suffix() {
        // master04 §Flat format example key.
        let k = FlatKey::parse("vital_signs/body_temperature:0/any_event:0/temperature|magnitude")
            .unwrap();
        assert_eq!(k.segments.len(), 4);
        assert_eq!(k.segments[1].name, "body_temperature");
        assert_eq!(k.segments[1].index, Some(0));
        assert_eq!(k.segments[3].name, "temperature");
        assert_eq!(k.segments[3].index, None);
        assert_eq!(k.suffixes.len(), 1);
        assert_eq!(k.suffixes[0].name, "magnitude");
        assert!(!k.is_ctx());
    }

    #[test]
    fn parses_rm_attribute_and_suffix_chain() {
        // master04 §RM Attributes prefix example: a chained suffix on an
        // indexed `_link` RM attribute.
        let k = FlatKey::parse("path/observation:0/_link:0|meaning|code").unwrap();
        assert_eq!(k.segments[2].name, "_link");
        assert_eq!(k.segments[2].index, Some(0));
        assert!(k.segments[2].is_rm_attribute());
        assert_eq!(
            k.suffixes,
            vec![
                Suffix {
                    name: "meaning".to_owned(),
                    index: None
                },
                Suffix {
                    name: "code".to_owned(),
                    index: None
                }
            ]
        );
    }

    #[test]
    fn parses_ctx_suffix_index() {
        // master06 §Participation non-compact identifiers.
        let k = FlatKey::parse("ctx/participation_identifiers:1|issuer:0").unwrap();
        assert!(k.is_ctx());
        assert_eq!(k.segments[1].index, Some(1));
        assert_eq!(k.suffixes[0].name, "issuer");
        assert_eq!(k.suffixes[0].index, Some(0));
    }

    #[test]
    fn display_round_trips() {
        for key in [
            "ctx/language",
            "vital_signs/body_temperature:0/any_event:0/temperature|magnitude",
            "path/observation:0/_link:0|meaning|code",
            "ctx/participation_identifiers:1|issuer:0",
            "vital_signs/temperature:0/value/_normal_range/lower|magnitude",
        ] {
            assert_eq!(FlatKey::parse(key).unwrap().to_string(), key);
        }
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["", "a//b", "a|", "a:x", "a/:0", "a|code:x"] {
            assert!(
                matches!(FlatKey::parse(bad), Err(FlatError::MalformedPath { .. })),
                "expected malformed: {bad}"
            );
        }
    }
}

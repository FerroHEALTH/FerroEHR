// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The name and the version a stored query is identified by.
//!
//! ITS-REST 1.1.0 `specifications/docs/query/Qualified_query_name.md`:
//! "Stored queries are identified by their name, used as
//! `qualified_query_name`, and an optional `version` number." The generated
//! contract carries them as these types (the `QueryName` and `QueryVersion`
//! component schemas and the `version` path parameter), so every consumer
//! reads one grammar.

use std::fmt;
use std::str::FromStr;

/// The ITS-REST separator between a query's namespace and its name.
const NAMESPACE_SEPARATOR: &str = "::";

/// The query name ITS-REST reserves.
const RESERVED: &str = "aql";

/// A stored query's qualified name, `[{namespace}::]{query-name}`.
///
/// `Qualified_query_name.md`: the `query-name` "may include any combination
/// of characters, matched by the pattern `[a-zA-Z0-9_.-]`" and "must not be
/// `aql` (case-insensitive), as that is a reserved name"; the optional
/// namespace is "a reverse domain name", which takes the same characters.
///
/// A reserved name reads, because a request that names one addresses a
/// stored query that cannot exist, which `responses/404_Query.yaml` answers;
/// [`QualifiedQueryName::check_registrable`] refuses it where a query is
/// stored.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QualifiedQueryName {
    namespace: Option<String>,
    name: String,
}

/// Why a text is no qualified query name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryNameError {
    /// The query name, or a namespace written before `::`, is empty.
    #[error("a stored query name and its namespace are not empty")]
    Empty,
    /// A character outside `[a-zA-Z0-9_.-]`, or more than one `::`.
    #[error(
        "a stored query name is [{{namespace}}::]{{query-name}} over the characters a-z, A-Z, \
         0-9, _, . and -"
    )]
    Malformed,
    /// The query name is `aql`, which ITS-REST reserves.
    #[error("the query name aql is reserved")]
    Reserved,
    /// The whole name is `.` or `..`, a dot-segment no request path carries.
    #[error("a stored query name without a namespace is not . or ..")]
    DotSegment,
}

impl QualifiedQueryName {
    /// The name `name` in `namespace`.
    ///
    /// # Errors
    ///
    /// [`QueryNameError`] when either part breaks the grammar `FromStr`
    /// reads.
    pub fn new(namespace: Option<&str>, name: &str) -> Result<Self, QueryNameError> {
        for part in namespace.into_iter().chain([name]) {
            if part.is_empty() {
                return Err(QueryNameError::Empty);
            }
            if !part.chars().all(is_name_char) {
                return Err(QueryNameError::Malformed);
            }
        }
        // NOTE: the path parameter expands with `style: simple` (RFC 6570 §3.2.2), which keeps `.`,
        // and RFC 3986 §5.2.4 removes the dot-segments `.` and `..`, so no URL names such a query.
        if namespace.is_none() && matches!(name, "." | "..") {
            return Err(QueryNameError::DotSegment);
        }
        Ok(Self {
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
        })
    }

    /// The namespace, when the name has one.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// The query name, without its namespace.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the query name is the reserved `aql`, in any case.
    #[must_use]
    pub fn is_reserved(&self) -> bool {
        self.name.eq_ignore_ascii_case(RESERVED)
    }

    /// Checks that a query may be stored under this name.
    ///
    /// # Errors
    ///
    /// [`QueryNameError::Reserved`] when the query name is `aql` in any case
    /// (`Qualified_query_name.md`: it "must not be `aql`").
    pub fn check_registrable(&self) -> Result<(), QueryNameError> {
        if self.is_reserved() {
            Err(QueryNameError::Reserved)
        } else {
            Ok(())
        }
    }
}

impl FromStr for QualifiedQueryName {
    type Err = QueryNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.split_once(NAMESPACE_SEPARATOR) {
            Some((namespace, name)) => Self::new(Some(namespace), name),
            None => Self::new(None, value),
        }
    }
}

impl fmt::Display for QualifiedQueryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_qualified(f, self.namespace.as_deref(), &self.name)
    }
}

/// The pattern `definition_query_list` reads its `qualified_query_name` as.
///
/// `specifications/operations/definition_query_list.yaml`: the stored
/// queries are "matched by `qualified_query_name` as pattern", given "in the
/// format of `[{namespace}::]{query-name}`, and when is empty, it will be
/// treated as \"wildcard\" in the search". So `org.openehr::` names every
/// query of that namespace, `::q` every query whose name starts with `q` in
/// any namespace, and a pattern without `::` a prefix of the whole name
/// ("will list all versions of all queries with names starting with
/// `org.openehr`"). A part that is not empty takes the characters of a
/// [`QualifiedQueryName`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryNamePattern {
    namespace: Option<String>,
    name: String,
}

impl QueryNamePattern {
    /// Whether the pattern is written with `::`, so its parts match the
    /// namespace and the query name each.
    #[must_use]
    pub fn is_qualified(&self) -> bool {
        self.namespace.is_some()
    }

    /// The namespace the pattern names; `None` without `::` and for the
    /// empty namespace, the wildcard.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace
            .as_deref()
            .filter(|namespace| !namespace.is_empty())
    }

    /// The prefix of the query name, or of the whole name without `::`; the
    /// empty prefix matches every name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl From<QualifiedQueryName> for QueryNamePattern {
    fn from(name: QualifiedQueryName) -> Self {
        Self {
            namespace: name.namespace,
            name: name.name,
        }
    }
}

impl FromStr for QueryNamePattern {
    type Err = QueryNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (namespace, name) = match value.split_once(NAMESPACE_SEPARATOR) {
            Some((namespace, name)) => (Some(namespace), name),
            None => (None, value),
        };
        if namespace.is_none() && name.is_empty() {
            return Err(QueryNameError::Empty);
        }
        if !namespace
            .into_iter()
            .chain([name])
            .all(|part| part.chars().all(is_name_char))
        {
            return Err(QueryNameError::Malformed);
        }
        Ok(Self {
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
        })
    }
}

impl fmt::Display for QueryNamePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_qualified(f, self.namespace.as_deref(), &self.name)
    }
}

/// Writes `[{namespace}::]{name}`, the form both a name and a pattern take.
fn write_qualified(f: &mut fmt::Formatter<'_>, namespace: Option<&str>, name: &str) -> fmt::Result {
    match namespace {
        Some(namespace) => write!(f, "{namespace}{NAMESPACE_SEPARATOR}{name}"),
        None => f.write_str(name),
    }
}

/// Whether `c` may appear in a query name or a namespace.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// A stored query's version, `major.minor.patch`.
///
/// `Qualified_query_name.md`: "The `version` identifier is in the format
/// specified by SEMVER style (i.e. `major.minor.patch`)." Each part is a
/// decimal number without a leading zero, as Semantic Versioning 2.0.0 §2
/// writes it, so a version has one spelling. Versions order by major, then
/// minor, then patch, so `1.10.0` is above `1.9.0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueryVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

/// Why a text is no version, or no version pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryVersionError {
    /// The text is not `major.minor.patch` over numbers without a leading
    /// zero.
    #[error("a stored query version is major.minor.patch, each a number without a leading zero")]
    Malformed,
    /// The text is not `{major}`, `{major}.{minor}` or `major.minor.patch`.
    #[error(
        "a stored query version pattern is {{major}}, {{major}}.{{minor}} or major.minor.patch"
    )]
    Pattern,
}

impl QueryVersion {
    /// The version `major.minor.patch`.
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// The major version.
    #[must_use]
    pub const fn major(&self) -> u64 {
        self.major
    }

    /// The minor version.
    #[must_use]
    pub const fn minor(&self) -> u64 {
        self.minor
    }

    /// The patch version.
    #[must_use]
    pub const fn patch(&self) -> u64 {
        self.patch
    }
}

impl FromStr for QueryVersion {
    type Err = QueryVersionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match numbers(value)
            .ok_or(QueryVersionError::Malformed)?
            .as_slice()
        {
            [major, minor, patch] => Ok(Self::new(*major, *minor, *patch)),
            _ => Err(QueryVersionError::Malformed),
        }
    }
}

impl fmt::Display for QueryVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The dot-separated decimal numbers of `value`, `None` when a part is empty,
/// holds anything but digits, has a leading zero or exceeds `u64`.
fn numbers(value: &str) -> Option<Vec<u64>> {
    value
        .split('.')
        .map(|part| {
            let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            let leading_zero = part.len() > 1 && part.starts_with('0');
            if digits && !leading_zero {
                part.parse::<u64>().ok()
            } else {
                None
            }
        })
        .collect()
}

/// The version a stored query is addressed at: exact, or a `{major}` or
/// `{major}.{minor}` prefix.
///
/// ITS-REST `parameters/path/version.yaml`: "This can be an exact version
/// (e.g. `1.7.1`), or a pattern as partial prefix, in a form of `{major}` or
/// `{major}.{minor}` (e.g. `1` or `1.0`), in which case the highest (latest)
/// version matching the prefix will be considered." The same parameter
/// addresses an ADL 2 template's version, so it is typed there too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VersionPattern {
    /// `major.minor.patch`: that version only.
    Exact(QueryVersion),
    /// `{major}`: every version with that major number.
    Major(u64),
    /// `{major}.{minor}`: every version with those numbers.
    MajorMinor(u64, u64),
}

impl VersionPattern {
    /// Whether `version` is one this pattern selects.
    #[must_use]
    pub fn matches(&self, version: &QueryVersion) -> bool {
        match *self {
            Self::Exact(exact) => exact == *version,
            Self::Major(major) => version.major == major,
            Self::MajorMinor(major, minor) => version.major == major && version.minor == minor,
        }
    }

    /// The highest of `versions` this pattern selects, the one ITS-REST
    /// addresses.
    #[must_use]
    pub fn select<'a>(
        &self,
        versions: impl IntoIterator<Item = &'a QueryVersion>,
    ) -> Option<&'a QueryVersion> {
        versions.into_iter().filter(|v| self.matches(v)).max()
    }

    /// The version, when the pattern names one exactly.
    #[must_use]
    pub const fn exact(&self) -> Option<QueryVersion> {
        match *self {
            Self::Exact(version) => Some(version),
            Self::Major(_) | Self::MajorMinor(..) => None,
        }
    }
}

impl FromStr for VersionPattern {
    type Err = QueryVersionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match numbers(value).ok_or(QueryVersionError::Pattern)?.as_slice() {
            [major] => Ok(Self::Major(*major)),
            [major, minor] => Ok(Self::MajorMinor(*major, *minor)),
            [major, minor, patch] => Ok(Self::Exact(QueryVersion::new(*major, *minor, *patch))),
            _ => Err(QueryVersionError::Pattern),
        }
    }
}

impl fmt::Display for VersionPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(version) => version.fmt(f),
            Self::Major(major) => write!(f, "{major}"),
            Self::MajorMinor(major, minor) => write!(f, "{major}.{minor}"),
        }
    }
}

/// Serializes and reads each type as its text, the form the ITS-REST schemas
/// give it (`type: string`).
macro_rules! text_serde {
    ($($ty:ty),+) => {$(
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    )+};
}

text_serde!(
    QualifiedQueryName,
    QueryNamePattern,
    QueryVersion,
    VersionPattern
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_examples_are_qualified_query_names() {
        for text in [
            "org.openehr::my_compositions",
            "my_compositions",
            "ehr::all_influenza_vacc_candidates",
        ] {
            let name: QualifiedQueryName = text.parse().expect("a spec example");
            assert_eq!(name.to_string(), text);
        }
        let name: QualifiedQueryName = "org.openehr::my_compositions".parse().expect("valid");
        assert_eq!(
            (name.namespace(), name.name()),
            (Some("org.openehr"), "my_compositions")
        );
    }

    #[test]
    fn a_name_outside_the_grammar_is_refused() {
        for (text, error) in [
            ("", QueryNameError::Empty),
            ("::name", QueryNameError::Empty),
            ("org::", QueryNameError::Empty),
            ("a::b::c", QueryNameError::Malformed),
            ("with space", QueryNameError::Malformed),
            ("..", QueryNameError::DotSegment),
        ] {
            assert_eq!(text.parse::<QualifiedQueryName>(), Err(error), "{text:?}");
        }
        assert!(
            "org::..".parse::<QualifiedQueryName>().is_ok(),
            "a namespace keeps the segment"
        );
        for text in ["AQL", "org.openehr::Aql"] {
            let name: QualifiedQueryName = text.parse().expect("a reserved name reads");
            assert!(name.is_reserved(), "{text}");
            assert_eq!(name.check_registrable(), Err(QueryNameError::Reserved));
        }
        let name: QualifiedQueryName = "org.openehr::aqlx".parse().expect("valid");
        assert_eq!(name.check_registrable(), Ok(()));
    }

    #[test]
    fn an_empty_part_of_a_list_pattern_is_the_wildcard() {
        let pattern = |text: &str| text.parse::<QueryNamePattern>().expect("a pattern");
        let namespace = pattern("org.cadasto.bench::");
        assert!(namespace.is_qualified());
        assert_eq!(
            (namespace.namespace(), namespace.name()),
            (Some("org.cadasto.bench"), "")
        );
        let name = pattern("::probe");
        assert!(name.is_qualified());
        assert_eq!((name.namespace(), name.name()), (None, "probe"));
        let both = pattern("::");
        assert_eq!((both.namespace(), both.name()), (None, ""));
        let bare = pattern("org.cadasto.bench");
        assert!(!bare.is_qualified());
        assert_eq!((bare.namespace(), bare.name()), (None, "org.cadasto.bench"));
        for text in [
            "org.cadasto.bench::",
            "::probe",
            "::",
            "org.openehr::q",
            "org",
        ] {
            assert_eq!(pattern(text).to_string(), text);
        }
        for (text, error) in [
            ("", QueryNameError::Empty),
            ("a::b::c", QueryNameError::Malformed),
            ("with space::", QueryNameError::Malformed),
        ] {
            assert_eq!(text.parse::<QueryNamePattern>(), Err(error), "{text:?}");
        }
    }

    #[test]
    fn versions_order_by_number_and_have_one_spelling() {
        let v = |t: &str| t.parse::<QueryVersion>().expect("a version");
        assert!(v("1.10.0") > v("1.9.0"));
        assert_eq!(v("2.0.1").to_string(), "2.0.1");
        for text in ["1", "1.0", "01.0.0", "1.0.0-rc.1", "1..0", "a.b.c"] {
            assert_eq!(
                text.parse::<QueryVersion>(),
                Err(QueryVersionError::Malformed),
                "{text}"
            );
        }
    }

    #[test]
    fn a_pattern_selects_the_highest_version_its_prefix_matches() {
        let versions: Vec<QueryVersion> = ["1.0.0", "1.2.3", "1.10.0", "2.0.0"]
            .iter()
            .map(|t| t.parse().expect("v"))
            .collect();
        let pick = |t: &str| {
            t.parse::<VersionPattern>()
                .expect("a pattern")
                .select(&versions)
                .map(ToString::to_string)
        };
        assert_eq!(pick("1").as_deref(), Some("1.10.0"));
        assert_eq!(pick("1.2").as_deref(), Some("1.2.3"));
        assert_eq!(pick("1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(pick("3"), None);
        assert_eq!(
            "1.0"
                .parse::<VersionPattern>()
                .map(|p| p.to_string())
                .as_deref(),
            Ok("1.0")
        );
        assert_eq!(
            "1.0.0.0".parse::<VersionPattern>(),
            Err(QueryVersionError::Pattern)
        );
        assert_eq!("1.0".parse::<VersionPattern>().map(|p| p.exact()), Ok(None));
    }

    #[test]
    fn each_type_reads_and_writes_its_text_in_json() {
        let name: QualifiedQueryName = serde_json::from_str("\"ehr::q\"").expect("reads");
        assert_eq!(serde_json::to_string(&name).expect("writes"), "\"ehr::q\"");
        assert!(serde_json::from_str::<QualifiedQueryName>("\"a b\"").is_err());
        let version: QueryVersion = serde_json::from_str("\"1.0.2\"").expect("reads");
        assert_eq!(
            serde_json::to_string(&version).expect("writes"),
            "\"1.0.2\""
        );
    }
}

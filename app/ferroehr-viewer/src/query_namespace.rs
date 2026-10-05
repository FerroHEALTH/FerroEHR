// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Namespace-derived stored-query grouping: the pure rules the `/queries`
//! screen, the dashboard tiles, and both save flows share.
//!
//! Component-free plain Rust with ordinary unit tests, compiled for both
//! the `ssr` and `hydrate` targets.
//!
//! A stored query's group **is** the namespace of its qualified name. ITS-REST
//! defines the stored-query identifier as `[{namespace}::]{query-name}`, with
//! the namespace optional and, when present, "in a form of a reverse domain
//! name, which allows for separation of use of stored queries by teams,
//! companies, etc."
//! (`ITS-REST specifications/docs/query/Qualified_query_name.md` §Qualified
//! query name) — so the grouping the viewer shows is the separation the
//! identifier already carries: durable in the CDR, visible to every API
//! client, and nothing the viewer has to store.
//!
//! NOTE: the *presentation* of that grouping (headings, tiles, the label for
//! the unqualified bucket) is our own design/extension — no openEHR spec
//! governs the viewer. Only the name format these rules read is spec-bound,
//! cited above.

use crate::queries_api::StoredQueryRow;

/// The heading for stored queries whose name carries no namespace.
///
/// The namespace is optional per the spec, so this labels an absence rather
/// than naming a namespace — which is why [`QueryNamespaceGroup::namespace`]
/// stays `None` for that bucket and cannot collide with a real namespace of
/// the same text.
pub const UNQUALIFIED_LABEL: &str = "unqualified";

/// One derived group: the namespace (`None` for the unqualified bucket) and
/// its member stored queries in listing order. A plain record — it is built
/// from a listing, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryNamespaceGroup {
    /// The namespace prefix shared by every member, or `None` when the members
    /// carry no namespace at all.
    pub namespace: Option<String>,
    /// The member stored queries, in the order the CDR listed them.
    pub members: Vec<StoredQueryRow>,
}

/// The namespace prefix of a qualified stored-query name — the text before the
/// FIRST `::` — or `None` when the name carries none.
///
/// Reading `[{namespace}::]{query-name}` left to right makes the first `::`
/// the separator; the spec's `query-name` character class (`[a-zA-Z0-9_.-]`)
/// admits no colon, so a name with a second `::` is not spec-valid and its
/// leading segment is still taken as the namespace. An empty prefix (`::name`)
/// is not a namespace.
#[must_use]
pub fn namespace_of(qualified_name: &str) -> Option<&str> {
    let (namespace, _) = qualified_name.split_once("::")?;
    let namespace = namespace.trim();
    (!namespace.is_empty()).then_some(namespace)
}

/// The bare query name: a qualified name with its namespace prefix removed,
/// or the whole name when it carries none.
#[must_use]
pub fn bare_name_of(qualified_name: &str) -> &str {
    match qualified_name.split_once("::") {
        Some((namespace, rest)) if !namespace.trim().is_empty() => rest,
        _ => qualified_name,
    }
}

/// Split a qualified name into the viewer's two save fields: `(namespace,
/// bare name)`, with an empty namespace for an unqualified name.
///
/// This is what pre-fills the save form when a stored query is opened in the
/// editor.
#[must_use]
pub fn split_qualified(qualified_name: &str) -> (String, String) {
    (
        namespace_of(qualified_name).unwrap_or_default().to_owned(),
        bare_name_of(qualified_name).to_owned(),
    )
}

/// Compose the qualified name a save writes, from the namespace field and the
/// name field.
///
/// Both parts are trimmed; the namespace is optional (per the spec cited in
/// the module docs), so an empty one yields the bare name.
///
/// A `name` that ALREADY carries a `::` prefix is taken as fully qualified and
/// wins: typing `org.example::vitals` into the name field saves exactly that,
/// so the field accepts either form.
#[must_use]
pub fn qualify(namespace: &str, name: &str) -> String {
    let name = name.trim();
    if namespace_of(name).is_some() {
        return name.to_owned();
    }
    let namespace = namespace.trim();
    if namespace.is_empty() {
        name.to_owned()
    } else {
        format!("{namespace}::{name}")
    }
}

/// Split a `name@version` stored-query reference into its qualified name and
/// version, or `None` when it lacks the `@version` suffix.
///
/// Splits on the LAST `@` so a qualified name is never mistaken for the
/// version.
///
/// `name@version` is the viewer's own reference form for one stored-query
/// VERSION (the raw editor's `?load=` hand-off carries it) — the openEHR REST
/// API keeps the two as separate path segments, so this pairing is our
/// design/extension, not a spec form.
///
/// NOTE: the version is OPAQUE text here — never parsed, compared, or
/// normalized, because prefix resolution is the server's job (ITS-REST
/// `specifications/docs/query/Qualified_query_name.md` §Qualified query name).
#[must_use]
pub fn split_query_ref(reference: &str) -> Option<(String, String)> {
    reference
        .rsplit_once('@')
        .filter(|(name, version)| !name.is_empty() && !version.is_empty())
        .map(|(name, version)| (name.to_owned(), version.to_owned()))
}

/// The `?load=` hand-off link into `route` for one stored-query VERSION.
///
/// `name@version` ([`split_query_ref`]'s input) is percent-encoded as ONE
/// query-string value: a qualified name may carry `::`, `/`, `&`, `=` or `#`,
/// any of which would otherwise truncate the value or forge a second parameter.
/// The router percent-DEcodes query parameters before a screen reads them.
///
/// NOTE: no openEHR spec governs the viewer's internal links — our own
/// design/extension.
#[must_use]
pub fn load_href(route: &str, name: &str, version: &str) -> String {
    format!(
        "{route}?load={}",
        urlencoding::encode(&format!("{name}@{version}"))
    )
}

/// How the CDR should resolve the version of a stored query being READ.
///
/// ITS-REST defines three forms, and this is all three: "The `version`
/// identifier is in the format specified by SEMVER style (i.e.
/// `major.minor.patch`). When only a partial `version` pattern is supplied, or
/// when `version` is not supplied at all, the system must use the latest
/// `version` with the supplied prefix - i.e. if only `major` or `major.minor`
/// is used, then the latest query version matching supplied prefix will be
/// used." (ITS-REST `specifications/docs/query/Qualified_query_name.md`
/// §Qualified query name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionResolution {
    /// No `version` at all — the CDR uses the latest version of the query.
    Latest,
    /// A partial `{major}` or `{major}.{minor}` prefix — the CDR uses the
    /// latest version matching it.
    Prefix,
    /// A complete `major.minor.patch` — exactly that version.
    Exact,
}

impl VersionResolution {
    /// The three forms in the order the spec lists them (latest, prefix,
    /// exact) — the order a picker offers them in.
    pub const ALL: [Self; 3] = [Self::Latest, Self::Prefix, Self::Exact];

    /// The stable form value a `<select>`/`?mode=` uses for this form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::Prefix => "prefix",
            Self::Exact => "exact",
        }
    }

    /// Read a form value back, defaulting to [`Self::Exact`] for anything
    /// unrecognized — the least surprising reading of user input, because it
    /// resolves to the one version the operator can see in the field.
    #[must_use]
    pub fn from_str_or_exact(raw: &str) -> Self {
        match raw {
            "latest" => Self::Latest,
            "prefix" => Self::Prefix,
            _ => Self::Exact,
        }
    }

    /// The picker label, worded as the spec describes the form.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Latest => "Latest version (no version sent)",
            Self::Prefix => "Version prefix (latest match)",
            Self::Exact => "Exact version",
        }
    }

    /// Which form fits `version` as written: no version at all is
    /// [`Self::Latest`], a `{major}`/`{major}.{minor}` pattern is
    /// [`Self::Prefix`], anything else is read as [`Self::Exact`] (and then
    /// validated by [`resolve_version`]).
    #[must_use]
    pub fn of(version: &str) -> Self {
        let version = version.trim();
        if version.is_empty() {
            Self::Latest
        } else if is_semver_prefix(version) {
            Self::Prefix
        } else {
            Self::Exact
        }
    }
}

/// Why a version cannot be resolved in the chosen form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionResolutionError {
    /// A prefix form given something that is not a `{major}` /
    /// `{major}.{minor}` pattern.
    #[error(
        "`{0}` is not a version prefix — a prefix is `{{major}}` or `{{major}}.{{minor}}` \
         (for example `1` or `1.2`) and resolves to the latest version matching it"
    )]
    NotAPrefix(String),
    /// An exact form given something that is not a complete triple.
    #[error(
        "`{0}` is not an exact version — an exact version is `major.minor.patch` \
         (for example `1.2.0`)"
    )]
    NotExact(String),
}

/// The `version` path segment a read should send: `None` for
/// [`VersionResolution::Latest`] (the version is omitted from the URL
/// entirely), `Some` for the two supplied forms.
///
/// # Errors
/// [`VersionResolutionError`] when `version` does not fit the chosen form, so
/// a malformed pattern never reaches the CDR as a version.
pub fn resolve_version(
    mode: VersionResolution,
    version: &str,
) -> Result<Option<String>, VersionResolutionError> {
    let version = version.trim();
    match mode {
        VersionResolution::Latest => Ok(None),
        VersionResolution::Prefix => {
            if is_semver_prefix(version) {
                Ok(Some(version.to_owned()))
            } else {
                Err(VersionResolutionError::NotAPrefix(version.to_owned()))
            }
        }
        VersionResolution::Exact => {
            if is_full_semver(version) {
                Ok(Some(version.to_owned()))
            } else {
                Err(VersionResolutionError::NotExact(version.to_owned()))
            }
        }
    }
}

/// Is `version` a partial SEMVER prefix — `{major}` or `{major}.{minor}`, each
/// part a plain non-negative integer?
///
/// The spec names exactly those two partial forms: "if only `major` or
/// `major.minor` is used, then the latest query version matching supplied
/// prefix will be used" (ITS-REST
/// `specifications/docs/query/Qualified_query_name.md` §Qualified query name).
/// A complete triple is NOT a prefix (it is [`is_full_semver`]).
#[must_use]
pub fn is_semver_prefix(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 1 && parts.len() != 2 {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && part.parse::<u64>().is_ok()
    })
}

/// Does `version` carry all three SEMVER parts (`major.minor.patch`, each a
/// plain non-negative integer)?
///
/// A stored-query version "is in the format specified by SEMVER style (i.e.
/// `major.minor.patch`)", and a PARTIAL pattern (`{major}` or
/// `{major}.{minor}`) means "the latest query version matching supplied
/// prefix" (ITS-REST `specifications/docs/query/Qualified_query_name.md`
/// §Qualified query name). Prefix RESOLUTION is a read concept: storing at a
/// prefix would file a query under a version string later prefix lookups would
/// treat as a pattern, so the save screens require all three parts.
///
/// NOTE: rejecting a prefix ON STORE is our own guard — the spec describes the
/// prefix form without saying what storing at one means.
#[must_use]
pub fn is_full_semver(version: &str) -> bool {
    let mut parts = version.split('.');
    let three = [parts.next(), parts.next(), parts.next()];
    parts.next().is_none()
        && three.iter().all(|part| {
            part.is_some_and(|part| {
                !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && part.parse::<u64>().is_ok()
            })
        })
}

/// The next MINOR version after `version` (`1.4.2` → `1.5.0`), or `None` when
/// `version` is not a full SEMVER triple or its minor part cannot be
/// incremented.
///
/// Used to pre-fill the save screens after loading a stored query, so editing a
/// loaded definition proposes a NEW version instead of colliding with the
/// immutable one it came from (ITS-REST
/// `operations/definition_query_version_store.yaml` answers `409` for an
/// existing pair).
///
/// NOTE: which part to bump is our own UX choice — no openEHR spec governs an
/// viewer. The field stays editable.
#[must_use]
pub fn next_minor(version: &str) -> Option<String> {
    if !is_full_semver(version) {
        return None;
    }
    let mut parts = version.split('.');
    let major = parts.next()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    Some(format!("{major}.{}.0", minor.checked_add(1)?))
}

/// Group a stored-query listing by the namespace of each name: namespaced
/// groups first, sorted by namespace, then the unqualified bucket (omitted
/// when empty). Members keep the CDR's listing order.
#[must_use]
pub fn group_by_namespace(rows: &[StoredQueryRow]) -> Vec<QueryNamespaceGroup> {
    let mut named: std::collections::BTreeMap<String, Vec<StoredQueryRow>> =
        std::collections::BTreeMap::new();
    let mut unqualified: Vec<StoredQueryRow> = Vec::new();
    for row in rows {
        match namespace_of(&row.name) {
            Some(namespace) => named
                .entry(namespace.to_owned())
                .or_default()
                .push(row.clone()),
            None => unqualified.push(row.clone()),
        }
    }
    let mut out: Vec<QueryNamespaceGroup> = named
        .into_iter()
        .map(|(namespace, members)| QueryNamespaceGroup {
            namespace: Some(namespace),
            members,
        })
        .collect();
    if !unqualified.is_empty() {
        out.push(QueryNamespaceGroup {
            namespace: None,
            members: unqualified,
        });
    }
    out
}

/// The heading a derived group renders under: the namespace itself, or
/// [`UNQUALIFIED_LABEL`] for the bucket of names that omit one.
#[must_use]
pub fn group_label(namespace: Option<&str>) -> &str {
    namespace.unwrap_or(UNQUALIFIED_LABEL)
}

#[cfg(test)]
mod tests {
    use crate::queries_api::StoredQueryRow;
    use crate::query_namespace::{
        UNQUALIFIED_LABEL, VersionResolution, VersionResolutionError, bare_name_of,
        group_by_namespace, group_label, is_full_semver, is_semver_prefix, load_href, namespace_of,
        next_minor, qualify, resolve_version, split_qualified, split_query_ref,
    };

    /// A listing row with only the fields the grouping reads.
    fn row(name: &str, version: &str) -> StoredQueryRow {
        StoredQueryRow {
            name: name.to_owned(),
            version: version.to_owned(),
            query: String::new(),
            saved: String::new(),
        }
    }

    #[test]
    fn namespace_of_reads_the_spec_examples() {
        // The three valid `qualified_query_name` examples from
        // ITS-REST docs/query/Qualified_query_name.md.
        assert_eq!(
            namespace_of("org.openehr::my_compositions"),
            Some("org.openehr")
        );
        assert_eq!(namespace_of("my_compositions"), None);
        assert_eq!(
            namespace_of("ehr::all_influenza_vacc_candidates"),
            Some("ehr")
        );
    }

    #[test]
    fn namespace_of_handles_the_edge_cases() {
        // Empty name.
        assert_eq!(namespace_of(""), None);
        // No separator at all.
        assert_eq!(namespace_of("plain_name"), None);
        // A single colon is not the separator.
        assert_eq!(namespace_of("a:b"), None);
        // A second `::` is not spec-valid; the FIRST separator still wins.
        assert_eq!(namespace_of("a::b::c"), Some("a"));
        // An empty prefix is not a namespace.
        assert_eq!(namespace_of("::name"), None);
        // A trailing separator names a namespace with an empty query name.
        assert_eq!(namespace_of("ns::"), Some("ns"));
        // Whitespace around the namespace is trimmed away.
        assert_eq!(namespace_of("  ns  ::q"), Some("ns"));
        // Whitespace-only prefix is no namespace.
        assert_eq!(namespace_of("   ::q"), None);
    }

    #[test]
    fn namespace_of_is_char_boundary_safe_for_unicode() {
        // Multi-byte namespace and multi-byte query name both survive; the
        // split is on the `::` pattern, never a byte offset.
        assert_eq!(namespace_of("målinger::blodtryk"), Some("målinger"));
        assert_eq!(bare_name_of("målinger::blodtryk"), "blodtryk");
        assert_eq!(namespace_of("组织::查询"), Some("组织"));
        assert_eq!(bare_name_of("组织::查询"), "查询");
        // Emoji (4-byte scalar) directly adjacent to the separator.
        assert_eq!(namespace_of("🏥::q"), Some("🏥"));
        assert_eq!(bare_name_of("ns::🏥"), "🏥");
    }

    #[test]
    fn bare_name_drops_only_a_real_namespace_prefix() {
        assert_eq!(
            bare_name_of("org.openehr::my_compositions"),
            "my_compositions"
        );
        assert_eq!(bare_name_of("my_compositions"), "my_compositions");
        assert_eq!(bare_name_of("a::b::c"), "b::c");
        assert_eq!(bare_name_of("ns::"), "");
        // No namespace to drop → the name is returned whole.
        assert_eq!(bare_name_of("::name"), "::name");
        assert_eq!(bare_name_of(""), "");
    }

    #[test]
    fn qualify_composes_the_two_save_fields() {
        assert_eq!(qualify("org.example", "vitals"), "org.example::vitals");
        // The namespace is optional.
        assert_eq!(qualify("", "vitals"), "vitals");
        // Both parts are trimmed.
        assert_eq!(
            qualify("  org.example  ", "  vitals  "),
            "org.example::vitals"
        );
    }

    #[test]
    fn qualify_lets_a_typed_prefix_win_over_the_namespace_field() {
        // Typing the whole qualified name into the name field saves exactly it.
        assert_eq!(qualify("", "org.example::vitals"), "org.example::vitals");
        assert_eq!(
            qualify("ignored", "org.example::vitals"),
            "org.example::vitals"
        );
    }

    #[test]
    fn split_and_qualify_round_trip() {
        // Every qualified name the SPEC admits survives split → qualify
        // unchanged: `[{namespace}::]{query-name}` where `query-name` is
        // matched by `[a-zA-Z0-9_.-]` (ITS-REST
        // `specifications/docs/query/Qualified_query_name.md` §Qualified query
        // name), so a spec-valid bare name can never itself contain `::` and
        // can never collide with the typed-prefix rule.
        for name in [
            "org.openehr::my_compositions",
            "my_compositions",
            "ehr::all_influenza_vacc_candidates",
            "ns::",
            "::name",
            "",
            "målinger::blodtryk",
        ] {
            let (namespace, bare) = split_qualified(name);
            assert_eq!(qualify(&namespace, &bare), name, "round trip of `{name}`");
        }
    }

    #[test]
    fn a_second_double_colon_normalizes_to_the_last_segment_pair() {
        // `a::b::c` is NOT a spec-valid qualified name — the bare name would
        // have to contain `::`, which the `query-name` charset excludes — so it
        // cannot round-trip through two fields. Split reads the FIRST `::`
        // (namespace `a`), and re-composing applies the documented
        // typed-prefix rule, which yields `b::c`. Not silent: the save screens
        // render the composed name ("Saves as …") before the click.
        let (namespace, bare) = split_qualified("a::b::c");
        assert_eq!(namespace, "a");
        assert_eq!(bare, "b::c");
        assert_eq!(qualify(&namespace, &bare), "b::c");
    }

    #[test]
    fn split_query_ref_parses_name_at_version() {
        assert_eq!(
            split_query_ref("org.example::vitals@1.2.3"),
            Some(("org.example::vitals".to_owned(), "1.2.3".to_owned()))
        );
        assert_eq!(split_query_ref("no_at_sign"), None);
        assert_eq!(split_query_ref("trailing@"), None);
        assert_eq!(split_query_ref("@leading"), None);
        // Splits on the LAST '@' so a name is never mistaken for the version.
        assert_eq!(
            split_query_ref("weird@name@2.0.0"),
            Some(("weird@name".to_owned(), "2.0.0".to_owned()))
        );
    }

    #[test]
    fn group_by_namespace_sorts_namespaces_and_puts_the_unqualified_bucket_last() {
        let rows = vec![
            row("org.zeta::b", "1.0.0"),
            row("plain", "1.0.0"),
            row("org.alpha::a", "1.0.0"),
            row("org.zeta::a", "2.0.0"),
            row("другой", "1.0.0"),
        ];
        let groups = group_by_namespace(&rows);
        let labels: Vec<&str> = groups
            .iter()
            .map(|g| group_label(g.namespace.as_deref()))
            .collect();
        assert_eq!(labels, vec!["org.alpha", "org.zeta", UNQUALIFIED_LABEL]);
        // Members keep the listing order within their group.
        let zeta = &groups[1].members;
        assert_eq!(zeta.len(), 2);
        assert_eq!(zeta[0].name, "org.zeta::b");
        assert_eq!(zeta[1].name, "org.zeta::a");
        // The unqualified bucket holds both un-namespaced names.
        assert_eq!(groups[2].members.len(), 2);
        assert_eq!(groups[2].namespace, None);
    }

    #[test]
    fn group_by_namespace_omits_an_empty_unqualified_bucket() {
        let groups = group_by_namespace(&[row("ns::a", "1.0.0")]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].namespace.as_deref(), Some("ns"));
    }

    #[test]
    fn group_by_namespace_of_nothing_is_no_groups() {
        assert!(group_by_namespace(&[]).is_empty());
    }

    #[test]
    fn group_label_names_the_namespace_or_the_bucket() {
        assert_eq!(group_label(Some("org.example")), "org.example");
        assert_eq!(group_label(None), UNQUALIFIED_LABEL);
    }

    #[test]
    fn full_semver_needs_three_numeric_parts() {
        assert!(is_full_semver("1.0.0"));
        assert!(is_full_semver("0.0.0"));
        assert!(is_full_semver("10.20.30"));
        // Partial patterns are the READ form (prefix resolution), never a
        // store version.
        assert!(!is_full_semver("1"));
        assert!(!is_full_semver("1.0"));
        assert!(!is_full_semver("1.0.0.0"));
        // Non-numeric, empty, signed, and pre-release/build forms all fail:
        // the viewer stores only a concrete triple.
        assert!(!is_full_semver(""));
        assert!(!is_full_semver("1.0."));
        assert!(!is_full_semver(".0.0"));
        assert!(!is_full_semver("1.0.x"));
        assert!(!is_full_semver("1.-1.0"));
        assert!(!is_full_semver("1.0.0-rc.1"));
        assert!(!is_full_semver("1.0.0+build"));
        assert!(!is_full_semver(" 1.0.0"));
        // Numeric but unrepresentable parts are rejected rather than silently
        // truncated.
        assert!(!is_full_semver("1.99999999999999999999.0"));
    }

    #[test]
    fn next_minor_bumps_the_middle_part_and_resets_the_patch() {
        assert_eq!(next_minor("1.0.0").as_deref(), Some("1.1.0"));
        assert_eq!(next_minor("1.4.2").as_deref(), Some("1.5.0"));
        assert_eq!(next_minor("0.9.9").as_deref(), Some("0.10.0"));
        // The major part is carried verbatim, never re-formatted.
        assert_eq!(next_minor("07.1.0").as_deref(), Some("07.2.0"));
    }

    #[test]
    fn semver_prefix_is_one_or_two_numeric_parts() {
        // The two partial forms the spec names.
        assert!(is_semver_prefix("1"));
        assert!(is_semver_prefix("1.2"));
        assert!(is_semver_prefix("0"));
        assert!(is_semver_prefix("10.20"));
        // A complete triple is the exact form, not a prefix.
        assert!(!is_semver_prefix("1.2.3"));
        // Malformed patterns.
        assert!(!is_semver_prefix(""));
        assert!(!is_semver_prefix("1."));
        assert!(!is_semver_prefix(".1"));
        assert!(!is_semver_prefix("v1"));
        assert!(!is_semver_prefix("1.x"));
        assert!(!is_semver_prefix("1.-2"));
        assert!(!is_semver_prefix("99999999999999999999"));
    }

    #[test]
    fn the_three_read_forms_resolve_to_their_path_segment() {
        // Latest: the version is omitted from the URL entirely.
        assert_eq!(resolve_version(VersionResolution::Latest, ""), Ok(None));
        // A stale version text is ignored in the latest form.
        assert_eq!(
            resolve_version(VersionResolution::Latest, "1.0.0"),
            Ok(None)
        );
        // Prefix: the pattern is sent as-is and the CDR picks the latest match.
        assert_eq!(
            resolve_version(VersionResolution::Prefix, "1"),
            Ok(Some("1".to_owned()))
        );
        assert_eq!(
            resolve_version(VersionResolution::Prefix, " 1.2 "),
            Ok(Some("1.2".to_owned()))
        );
        // Exact: a complete triple only.
        assert_eq!(
            resolve_version(VersionResolution::Exact, "1.2.3"),
            Ok(Some("1.2.3".to_owned()))
        );
    }

    #[test]
    fn a_version_that_does_not_fit_its_form_is_refused() {
        assert_eq!(
            resolve_version(VersionResolution::Prefix, "1.2.3"),
            Err(VersionResolutionError::NotAPrefix("1.2.3".to_owned()))
        );
        assert_eq!(
            resolve_version(VersionResolution::Prefix, ""),
            Err(VersionResolutionError::NotAPrefix(String::new()))
        );
        assert_eq!(
            resolve_version(VersionResolution::Exact, "1.2"),
            Err(VersionResolutionError::NotExact("1.2".to_owned()))
        );
        assert_eq!(
            resolve_version(VersionResolution::Exact, ""),
            Err(VersionResolutionError::NotExact(String::new()))
        );
    }

    #[test]
    fn the_form_of_a_version_text_is_read_from_its_shape() {
        assert_eq!(VersionResolution::of(""), VersionResolution::Latest);
        assert_eq!(VersionResolution::of("   "), VersionResolution::Latest);
        assert_eq!(VersionResolution::of("1"), VersionResolution::Prefix);
        assert_eq!(VersionResolution::of("1.2"), VersionResolution::Prefix);
        assert_eq!(VersionResolution::of("1.2.3"), VersionResolution::Exact);
        // Anything else is read as the exact form and validated there.
        assert_eq!(VersionResolution::of("latest"), VersionResolution::Exact);
    }

    #[test]
    fn resolution_form_values_round_trip() {
        for mode in VersionResolution::ALL {
            assert_eq!(VersionResolution::from_str_or_exact(mode.as_str()), mode);
            assert!(!mode.label().is_empty());
        }
        // Unknown input falls back to the exact form.
        assert_eq!(
            VersionResolution::from_str_or_exact("nonsense"),
            VersionResolution::Exact
        );
    }

    #[test]
    fn load_href_escapes_the_qualified_ref_as_one_value() {
        assert_eq!(
            load_href("/queries/aql", "my_query", "1.0.0"),
            "/queries/aql?load=my_query%401.0.0"
        );
        // A qualified name carries `::` and `/`; a `&`/`=` must not become an
        // extra parameter.
        assert_eq!(
            load_href("/queries/builder", "org.example::c/name/value", "1.2.3"),
            "/queries/builder?load=org.example%3A%3Ac%2Fname%2Fvalue%401.2.3"
        );
        assert_eq!(
            load_href("/queries/stored", "a&b=c", "1"),
            "/queries/stored?load=a%26b%3Dc%401"
        );
    }

    #[test]
    fn load_href_round_trips_back_through_split_query_ref() {
        // The router decodes `?load=` before a screen reads it, so decoding the
        // emitted value must hand `split_query_ref` the original pair.
        for (name, version) in [
            ("my_query", "1.0.0"),
            ("org.example::c/name/value", "1.2.3"),
            ("a&b=c", "1"),
            ("blodtryk_målinger", "2.0.0"),
        ] {
            let href = load_href("/queries/stored", name, version);
            let value = href
                .strip_prefix("/queries/stored?load=")
                .expect("the helper always emits <route>?load=<value>");
            let decoded = urlencoding::decode(value).expect("valid UTF-8 percent-encoding");
            assert_eq!(
                split_query_ref(&decoded),
                Some((name.to_owned(), version.to_owned()))
            );
        }
    }

    #[test]
    fn next_minor_declines_what_it_cannot_bump() {
        assert_eq!(next_minor("1.0"), None);
        assert_eq!(next_minor("latest"), None);
        assert_eq!(next_minor(""), None);
        assert_eq!(next_minor("1.0.0-rc.1"), None);
        // A saturated minor cannot be incremented, so no version is proposed
        // rather than a wrong one.
        assert_eq!(next_minor(&format!("1.{}.0", u64::MAX)), None);
    }
}

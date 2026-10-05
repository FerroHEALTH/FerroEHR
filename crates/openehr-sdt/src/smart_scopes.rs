// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART on openEHR resource-scope grammar.
//!
//! The grammar is
//! `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`,
//! plus the launch-context scopes of master07 §Context Selection and master09
//! §Experimental: Episode Context.
//!
//! Hand-written (like the `flat` module, SMART App Launch is an ITS-REST
//! sub-specification with no machine-readable model): ONE grammar, consumed by
//! the CDR's scope gate (`ferroehr-rest::smart`) and by any REST client that
//! previews what a scope string grants (the viewer) — the two can never
//! drift because they parse with the same code.
//!
//! A **total** parser: [`SmartScope::parse`] maps every scope string the token's
//! `scope` claim carries onto a typed value; anything the grammar does not
//! recognise becomes [`SmartScope::Other`] and is retained but inert
//! (forward-compat — master07 notes that standard SMART scopes may travel
//! alongside the openEHR ones and are non-normative here). The resource-scope
//! form is `<compartment>/<resource>.<permission>` (master08 §Resource Scopes).

use std::fmt;

/// One parsed scope from the token's space-delimited `scope` claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartScope {
    /// `launch` — embedded-iframe launch marker (master07 §Embedded iFrame
    /// Launch). Advisory: the CDR does not perform launch selection.
    Launch,
    /// `launch/patient` | `launch/episode` — a requested launch context
    /// (master07 §Context Selection; episode is master09 experimental).
    LaunchContext(LaunchContext),
    /// An `OpenID` Connect / identity scope (`openid`, `profile`, `fhirUser`,
    /// `offline_access`, …) — master08 §Scopes "Identity Claims".
    Identity(String),
    /// A resource scope `<compartment>/<resource>.<permission>`
    /// (master08 §Resource Scopes).
    Resource(ResourceScope),
    /// Any other scope string, retained verbatim but inert.
    Other(String),
}

/// The launch context a `launch/<ctx>` scope requests (master07/master09).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchContext {
    /// `launch/patient` — patient/EHR context (master07 §Context Selection).
    Patient,
    /// `launch/episode` — episode context (master09, experimental).
    Episode,
    /// Any other `launch/<x>` value, retained inert.
    Other(String),
}

/// A parsed `<compartment>/<resource>.<permission>` resource scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceScope {
    /// The access-delegation compartment (master08 §Resource Scopes).
    pub compartment: Compartment,
    /// The resource family + its id pattern.
    pub resource: ResourceSelector,
    /// The permitted operations (`c`/`r`/`u`/`d`/`s`).
    pub permissions: Permissions,
}

/// The scope-of-access-delegation compartment (master08 §Resource Scopes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compartment {
    /// `patient` — limited to the current EHR/Patient in context.
    Patient,
    /// `user` — the authenticated user's security profile.
    User,
    /// `system` — backend, no user context, all data.
    System,
}

/// A resource family + the id pattern it matches (master08 §Resource Scopes):
/// `template-<templateId>`, `composition-<templateId>`, `aql-<queryName>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceSelector {
    /// `template-<templateId>` — operational templates.
    Template(Pattern),
    /// `composition-<templateId>` — compositions of a given template.
    Composition(Pattern),
    /// `aql-<queryName>` — stored (name) or ad-hoc (`*`) AQL queries.
    Aql(Pattern),
}

impl ResourceSelector {
    /// The resource family this selector targets, independent of its pattern.
    #[must_use]
    pub const fn family(&self) -> ResourceFamily {
        match self {
            ResourceSelector::Template(_) => ResourceFamily::Template,
            ResourceSelector::Composition(_) => ResourceFamily::Composition,
            ResourceSelector::Aql(_) => ResourceFamily::Aql,
        }
    }

    /// The id pattern.
    #[must_use]
    pub const fn pattern(&self) -> &Pattern {
        match self {
            ResourceSelector::Template(p)
            | ResourceSelector::Composition(p)
            | ResourceSelector::Aql(p) => p,
        }
    }
}

/// The resource family axis, used by the enforcement layer to map an operation's
/// resource kind (`ferroehr-rest`'s `access::authz::request::ResourceKind`) onto
/// the SMART grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceFamily {
    /// Operational templates (`template-…`).
    Template,
    /// Compositions (`composition-…`).
    Composition,
    /// AQL queries (`aql-…`).
    Aql,
}

/// The set of CRUDS permissions a resource scope grants (master08 §Resource
/// Scopes permission list). Parsed from the `.<permission>` tail, order-free.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the flags mirror the five SMART CRUDS letters (c/r/u/d/s) 1:1 (ITS-REST SMART API); collapsing them into bitflags would hide the spec's own field names"
)]
pub struct Permissions {
    /// `c` — create.
    pub create: bool,
    /// `r` — read.
    pub read: bool,
    /// `u` — update.
    pub update: bool,
    /// `d` — delete.
    pub delete: bool,
    /// `s` — search / execute (e.g. AQL).
    pub search: bool,
}

/// A single CRUDS permission (the operation axis the enforcement layer resolves
/// from an operation's `AccessMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Create (`c`).
    Create,
    /// Read (`r`).
    Read,
    /// Update (`u`).
    Update,
    /// Delete (`d`).
    Delete,
    /// Search / execute (`s`).
    Search,
}

impl Permissions {
    /// Whether this set contains a given permission.
    // A by-value receiver: `Permissions` is a 5-byte `Copy` set, so the copy
    // is cheaper than the reference (clippy::trivially_copy_pass_by_ref), and
    // method-call auto-copy keeps every `&Permissions` call site unchanged.
    #[must_use]
    pub const fn contains(self, perm: Permission) -> bool {
        match perm {
            Permission::Create => self.create,
            Permission::Read => self.read,
            Permission::Update => self.update,
            Permission::Delete => self.delete,
            Permission::Search => self.search,
        }
    }

    /// Parse the `.<permission>` tail (e.g. `rs`, `crud`, `cruds`). Returns
    /// `None` on an empty tail or any character outside `{c,r,u,d,s}` (the whole
    /// scope then falls back to [`SmartScope::Other`], forward-compat).
    fn parse(tail: &str) -> Option<Self> {
        if tail.is_empty() {
            return None;
        }
        let mut p = Permissions::default();
        for ch in tail.chars() {
            match ch {
                'c' => p.create = true,
                'r' => p.read = true,
                'u' => p.update = true,
                'd' => p.delete = true,
                's' => p.search = true,
                _ => return None,
            }
        }
        Some(p)
    }
}

/// A `<templateId>`/`<queryName>` glob (master08 §Resource Scopes pattern
/// table). Matching semantics:
/// - a bare `*` (or `**`) matches **all** ids (the table's "All available
///   templates or queries" row);
/// - within a `namespace::name` pattern, `*` matches any run of characters
///   **within one `::`-delimited segment** (it does not cross `::`), and `**`
///   matches any run including `::`;
/// - every other character (including `::` and `.`) is literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern(String);

impl Pattern {
    /// Build a pattern from its raw scope text.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The raw pattern text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this pattern matches a concrete template id / query name.
    #[must_use]
    pub fn matches(&self, candidate: &str) -> bool {
        // master08 §Resource Scopes: a bare `*`/`**` matches all ids across all
        // namespaces (the "All available templates or queries" row), overriding
        // the otherwise segment-local `*`.
        if self.0 == "*" || self.0 == "**" {
            return true;
        }
        glob_match(self.0.as_bytes(), candidate.as_bytes())
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Backtracking glob matcher. `**` matches any run of bytes (including `:`);
/// `*` matches any run of bytes that contains no `:` (so it never crosses the
/// `::` namespace delimiter); all other bytes are literal.
fn glob_match(pat: &[u8], text: &[u8]) -> bool {
    match pat.split_first() {
        None => text.is_empty(),
        Some((&b'*', after_star)) => match after_star.strip_prefix(b"*") {
            // `**` — consume 0..=len bytes of anything.
            Some(rest) => {
                (0..=text.len()).any(|i| text.get(i..).is_some_and(|tail| glob_match(rest, tail)))
            }
            None => match_single_star(after_star, text),
        },
        Some((&c, pat_rest)) => match text.split_first() {
            Some((&t, text_rest)) if t == c => glob_match(pat_rest, text_rest),
            _ => false,
        },
    }
}

/// The single-`*` case of [`glob_match`]: consume bytes, stopping before any
/// `:` so the star never crosses the `::` namespace delimiter.
fn match_single_star(after_star: &[u8], text: &[u8]) -> bool {
    let mut i = 0;
    loop {
        let Some(tail) = text.get(i..) else {
            return false;
        };
        if glob_match(after_star, tail) {
            return true;
        }
        if tail.first().is_none_or(|&b| b == b':') {
            return false;
        }
        i += 1;
    }
}

impl SmartScope {
    /// Parse one scope string. Total: unrecognised forms become
    /// [`SmartScope::Other`].
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw {
            "launch" => return SmartScope::Launch,
            "openid" | "profile" | "fhirUser" | "offline_access" | "online_access" => {
                return SmartScope::Identity(raw.to_owned());
            }
            _ => {}
        }

        // launch/<ctx> — a requested launch context (master07/master09).
        if let Some(ctx) = raw.strip_prefix("launch/") {
            let ctx = match ctx {
                "patient" => LaunchContext::Patient,
                "episode" => LaunchContext::Episode,
                other => LaunchContext::Other(other.to_owned()),
            };
            return SmartScope::LaunchContext(ctx);
        }

        // <compartment>/<resource>.<permission>
        if let Some(scope) = parse_resource_scope(raw) {
            return SmartScope::Resource(scope);
        }

        SmartScope::Other(raw.to_owned())
    }

    /// Parse a whole space-delimited `scope` claim string into scopes.
    #[must_use]
    pub fn parse_all(scope_claim: &str) -> Vec<SmartScope> {
        scope_claim
            .split_whitespace()
            .map(SmartScope::parse)
            .collect()
    }
}

impl fmt::Display for SmartScope {
    /// The scope in the grammar's canonical form; `SmartScope::parse` reads it
    /// back to the same value for every scope the parser produced.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmartScope::Launch => f.write_str("launch"),
            SmartScope::LaunchContext(ctx) => write!(f, "launch/{ctx}"),
            SmartScope::Identity(raw) | SmartScope::Other(raw) => f.write_str(raw),
            SmartScope::Resource(scope) => scope.fmt(f),
        }
    }
}

impl SmartScope {
    /// The space-delimited `scope` string for `scopes`, the printer matching
    /// [`SmartScope::parse_all`]: each scope in its canonical form, joined by
    /// one space.
    #[must_use]
    pub fn format_all<'a>(scopes: impl IntoIterator<Item = &'a SmartScope>) -> String {
        let mut out = String::new();
        for scope in scopes {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&scope.to_string());
        }
        out
    }
}

impl fmt::Display for LaunchContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchContext::Patient => f.write_str("patient"),
            LaunchContext::Episode => f.write_str("episode"),
            LaunchContext::Other(raw) => f.write_str(raw),
        }
    }
}

impl fmt::Display for ResourceScope {
    /// `<compartment>/<resource>.<permission>` (master08 §Resource Scopes),
    /// the permissions in the grammar's `c`/`r`/`u`/`d`/`s` order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}.{}",
            self.compartment, self.resource, self.permissions
        )
    }
}

impl fmt::Display for Compartment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Compartment::Patient => "patient",
            Compartment::User => "user",
            Compartment::System => "system",
        })
    }
}

impl fmt::Display for ResourceSelector {
    /// `template-<id>`, `composition-<id>` or `aql-<name>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let family = match self.family() {
            ResourceFamily::Template => "template",
            ResourceFamily::Composition => "composition",
            ResourceFamily::Aql => "aql",
        };
        write!(f, "{family}-{}", self.pattern())
    }
}

impl fmt::Display for Permissions {
    /// The granted letters in the grammar's `c`/`r`/`u`/`d`/`s` order; an
    /// empty set prints nothing, which the grammar does not accept as a tail.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (granted, letter) in [
            (self.create, "c"),
            (self.read, "r"),
            (self.update, "u"),
            (self.delete, "d"),
            (self.search, "s"),
        ] {
            if granted {
                f.write_str(letter)?;
            }
        }
        Ok(())
    }
}

/// Parse the `<compartment>/<resource>.<permission>` form. `None` when it is not
/// a well-formed resource scope (the caller then keeps it as `Other`).
fn parse_resource_scope(raw: &str) -> Option<ResourceScope> {
    let (compartment_str, rest) = raw.split_once('/')?;
    let compartment = match compartment_str {
        "patient" => Compartment::Patient,
        "user" => Compartment::User,
        "system" => Compartment::System,
        _ => return None,
    };

    // The permission tail is the segment after the LAST '.', but template ids
    // and query names contain '.' (e.g. `Template.v0`, `bloodpressure.v1`), so
    // split on the final '.' only.
    let (resource_str, perm_str) = rest.rsplit_once('.')?;
    let permissions = Permissions::parse(perm_str)?;

    let resource = parse_resource_selector(resource_str)?;
    Some(ResourceScope {
        compartment,
        resource,
        permissions,
    })
}

/// Parse `template-<id>` / `composition-<id>` / `aql-<name>` into a selector.
fn parse_resource_selector(raw: &str) -> Option<ResourceSelector> {
    if let Some(id) = raw.strip_prefix("template-") {
        return Some(ResourceSelector::Template(Pattern::new(id)));
    }
    if let Some(id) = raw.strip_prefix("composition-") {
        return Some(ResourceSelector::Composition(Pattern::new(id)));
    }
    if let Some(name) = raw.strip_prefix("aql-") {
        return Some(ResourceSelector::Aql(Pattern::new(name)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(raw: &str) -> ResourceScope {
        match SmartScope::parse(raw) {
            SmartScope::Resource(r) => r,
            other => panic!("expected resource scope, got {other:?}"),
        }
    }

    // ── launch-context + identity scopes (master07/08/09) ─────────────────────

    #[test]
    fn launch_and_context_scopes() {
        assert_eq!(SmartScope::parse("launch"), SmartScope::Launch);
        assert_eq!(
            SmartScope::parse("launch/patient"),
            SmartScope::LaunchContext(LaunchContext::Patient)
        );
        assert_eq!(
            SmartScope::parse("launch/episode"),
            SmartScope::LaunchContext(LaunchContext::Episode)
        );
        assert_eq!(
            SmartScope::parse("launch/encounter"),
            SmartScope::LaunchContext(LaunchContext::Other("encounter".to_owned()))
        );
    }

    #[test]
    fn identity_scopes() {
        for s in ["openid", "profile", "fhirUser", "offline_access"] {
            assert_eq!(SmartScope::parse(s), SmartScope::Identity(s.to_owned()));
        }
    }

    #[test]
    fn unknown_scope_is_inert_other() {
        assert_eq!(
            SmartScope::parse("something/weird"),
            SmartScope::Other("something/weird".to_owned())
        );
        // A bad compartment falls through to Other, not a panic.
        assert_eq!(
            SmartScope::parse("admin/composition-*.r"),
            SmartScope::Other("admin/composition-*.r".to_owned())
        );
    }

    /// master04 §Service Discovery's own example document advertises
    /// `"scopes_supported": [… "patient/*.rs", "user/*.rs" …]`, but master08
    /// §Resource Scopes closes the `<resource>` position to `template-` /
    /// `composition-` / `aql-` — a bare `*` there is not a resource noun, so
    /// neither form is parseable under the grammar the same specification
    /// defines. NOTE: released-vs-released conflict; the total parser demotes
    /// both to inert `Other` (retained verbatim, granting and denying nothing),
    /// which is the only reading that neither rejects the spec's own example
    /// nor invents authority its grammar never wrote.
    #[test]
    fn master04_example_wildcard_resource_is_inert() {
        for raw in ["patient/*.rs", "user/*.rs"] {
            assert_eq!(SmartScope::parse(raw), SmartScope::Other(raw.to_owned()));
        }
        // The whole master04 example scope set still parses end to end: the
        // grammatical members keep their meaning, the two ungrammatical ones
        // ride along inert.
        let parsed = SmartScope::parse_all(
            "openid profile launch launch/patient patient/*.rs user/*.rs offline_access",
        );
        assert_eq!(parsed.len(), 7);
        assert_eq!(
            parsed
                .iter()
                .filter(|s| matches!(s, SmartScope::Other(_)))
                .count(),
            2
        );
        assert!(!parsed.iter().any(|s| matches!(s, SmartScope::Resource(_))));
    }

    // ── compartments (master08 §Resource Scopes) ──────────────────────────────

    #[test]
    fn compartments_parse() {
        assert_eq!(
            resource("patient/composition-*.r").compartment,
            Compartment::Patient
        );
        assert_eq!(
            resource("user/composition-*.r").compartment,
            Compartment::User
        );
        assert_eq!(
            resource("system/composition-*.r").compartment,
            Compartment::System
        );
    }

    // ── resource families (master08 §Resource Scopes) ─────────────────────────

    #[test]
    fn resource_families_parse() {
        assert_eq!(
            resource("user/template-*.cruds").resource.family(),
            ResourceFamily::Template
        );
        assert_eq!(
            resource("patient/composition-*.r").resource.family(),
            ResourceFamily::Composition
        );
        assert_eq!(
            resource("patient/aql-*.rs").resource.family(),
            ResourceFamily::Aql
        );
    }

    // ── permission tails (master08 §Resource Scopes permission list) ──────────

    #[test]
    fn permission_tails_parse_order_free() {
        let p = resource("patient/composition-*.crud").permissions;
        assert!(p.create && p.read && p.update && p.delete && !p.search);

        let p = resource("patient/aql-*.rs").permissions;
        assert!(p.read && p.search && !p.create);

        let p = resource("system/aql-*.cruds").permissions;
        assert!(p.create && p.read && p.update && p.delete && p.search);

        assert!(
            resource("patient/composition-*.r")
                .permissions
                .contains(Permission::Read)
        );
        assert!(
            !resource("patient/composition-*.r")
                .permissions
                .contains(Permission::Create)
        );
    }

    #[test]
    fn bad_permission_char_falls_back_to_other() {
        // `x` is not a CRUDS permission → the whole scope is retained inert.
        assert!(matches!(
            SmartScope::parse("patient/composition-*.rx"),
            SmartScope::Other(_)
        ));
        // An empty permission tail is also not a resource scope.
        assert!(matches!(
            SmartScope::parse("patient/composition-*."),
            SmartScope::Other(_)
        ));
    }

    // ── template id / query name with dots survives the tail split ────────────

    #[test]
    fn dotted_ids_keep_their_dots() {
        let r = resource("patient/composition-MyHospital::Template.v0.r");
        assert_eq!(r.resource.pattern().as_str(), "MyHospital::Template.v0");
        assert!(r.permissions.read);

        let r = resource("patient/aql-org.openehr::bloodpressure.v1.rs");
        assert_eq!(
            r.resource.pattern().as_str(),
            "org.openehr::bloodpressure.v1"
        );
        assert!(r.permissions.read && r.permissions.search);
    }

    // ── the master08 pattern table (§Resource Scopes) ─────────────────────────

    #[test]
    fn pattern_exact_match_only() {
        // `MyHospital::Template.v0` — exact match only.
        let p = Pattern::new("MyHospital::Template.v0");
        assert!(p.matches("MyHospital::Template.v0"));
        assert!(!p.matches("MyHospital::Template.v1"));
        assert!(!p.matches("OtherHospital::Template.v0"));
        assert!(!p.matches("Template.v0"));
    }

    #[test]
    fn pattern_query_exact_match_only() {
        // `org.openehr::bloodpressure.v1` — exact.
        let p = Pattern::new("org.openehr::bloodpressure.v1");
        assert!(p.matches("org.openehr::bloodpressure.v1"));
        assert!(!p.matches("org.openehr::bloodpressure.v2"));
    }

    #[test]
    fn pattern_any_namespace_fixed_name() {
        // `*::Template.v0` — Template.v0 from any namespace.
        let p = Pattern::new("*::Template.v0");
        assert!(p.matches("MyHospital::Template.v0"));
        assert!(p.matches("org.openehr::Template.v0"));
        assert!(!p.matches("MyHospital::Template.v1"));
        // `*` must not cross `::`, so a two-level namespace does not match.
        assert!(!p.matches("A::B::Template.v0"));
        // A candidate with no namespace does not satisfy a `ns::name` pattern.
        assert!(!p.matches("Template.v0"));
    }

    #[test]
    fn pattern_fixed_namespace_any_name() {
        // `MyHospital::*` — any template within MyHospital namespace.
        let p = Pattern::new("MyHospital::*");
        assert!(p.matches("MyHospital::Template.v0"));
        assert!(p.matches("MyHospital::Anything.v9"));
        assert!(!p.matches("OtherHospital::Template.v0"));
        // `*` does not cross `::`.
        assert!(!p.matches("MyHospital::Sub::Template.v0"));
    }

    #[test]
    fn pattern_bare_star_matches_all() {
        // `*` — All available templates or queries (the table's broad row); this
        // deliberately overrides the segment-local `*` rule.
        let p = Pattern::new("*");
        assert!(p.matches("Template.v0"));
        assert!(p.matches("MyHospital::Template.v0"));
        assert!(p.matches("org.openehr::bloodpressure.v1"));
        assert!(p.matches(""));
    }

    #[test]
    fn pattern_double_star_is_recursive() {
        // `**` crosses `::` (recursive), matching everything.
        let p = Pattern::new("**");
        assert!(p.matches("A::B::C::deep.v1"));
        // A `**` in name position spans the remainder including `::`.
        let p = Pattern::new("MyHospital::**");
        assert!(p.matches("MyHospital::Sub::Template.v0"));
        assert!(p.matches("MyHospital::Template.v0"));
    }

    // ── the maximal scope table rows (master08 §Resource Scopes) ──────────────

    #[test]
    fn maximal_table_rows_parse() {
        for row in [
            "patient/composition-MyTemplate.crud",
            "user/composition-MyTemplate.crud",
            "system/composition-MyTemplate.crud",
            "user/template-MyTemplate.crud",
            "system/template-MyTemplate.crud",
            "patient/aql-MyQuery.rs",
            "user/aql-MyQuery.cruds",
            "system/aql-MyQuery.cruds",
        ] {
            assert!(
                matches!(SmartScope::parse(row), SmartScope::Resource(_)),
                "row did not parse as a resource scope: {row}"
            );
        }
    }

    #[test]
    fn parse_all_splits_the_scope_claim() {
        let scopes = SmartScope::parse_all("openid launch/patient patient/composition-*.rs");
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[0], SmartScope::Identity("openid".to_owned()));
        assert_eq!(scopes[1], SmartScope::LaunchContext(LaunchContext::Patient));
        assert!(matches!(scopes[2], SmartScope::Resource(_)));
    }

    // ── printing back in the grammar (Display) ────────────────────────────────

    #[test]
    fn display_prints_the_canonical_form() {
        for (raw, canonical) in [
            ("launch", "launch"),
            ("launch/patient", "launch/patient"),
            ("launch/encounter", "launch/encounter"),
            ("openid", "openid"),
            ("patient/composition-*.rs", "patient/composition-*.rs"),
            (
                "user/template-MyHospital::Template.v0.duc",
                "user/template-MyHospital::Template.v0.cud",
            ),
            (
                "system/aql-org.openehr::bloodpressure.v1.srrs",
                "system/aql-org.openehr::bloodpressure.v1.rs",
            ),
            ("admin/composition-*.r", "admin/composition-*.r"),
        ] {
            assert_eq!(SmartScope::parse(raw).to_string(), canonical, "{raw}");
        }
    }

    #[test]
    fn display_round_trips_through_parse() {
        let samples = [
            "",
            "launch",
            "launch/",
            "launch/patient",
            "launch/episode",
            "launch/x/y",
            "openid",
            "profile",
            "fhirUser",
            "offline_access",
            "online_access",
            "patient/composition-*.r",
            "user/template-**.cruds",
            "system/aql-*.sdcru",
            "patient/composition-a/b.c.r",
            "patient/composition-.r",
            "patient/aql-x.",
            "patient/aql-x.rx",
            "patient/*.rs",
            "user/composition-a b.r",
            "system/x-y.r",
        ];
        for raw in samples {
            let parsed = SmartScope::parse(raw);
            let printed = parsed.to_string();
            assert_eq!(SmartScope::parse(&printed), parsed, "{raw}");
            // The canonical form is a fixed point.
            assert_eq!(SmartScope::parse(&printed).to_string(), printed, "{raw}");
        }
    }

    #[test]
    fn permutations_of_every_permission_set_round_trip() {
        let letters = ['c', 'r', 'u', 'd', 's'];
        for mask in 1u8..32 {
            let mut tail: Vec<char> = letters
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, c)| *c)
                .collect();
            tail.reverse();
            let tail: String = tail.into_iter().collect();
            let raw = format!("patient/composition-*.{tail}");
            let parsed = SmartScope::parse(&raw);
            assert!(matches!(parsed, SmartScope::Resource(_)), "{raw}");
            assert_eq!(SmartScope::parse(&parsed.to_string()), parsed, "{raw}");
        }
    }

    #[test]
    fn format_all_matches_parse_all() {
        let claim = "openid  launch/patient\tpatient/composition-*.sr user/*.rs";
        let scopes = SmartScope::parse_all(claim);
        let printed = SmartScope::format_all(&scopes);
        assert_eq!(
            printed,
            "openid launch/patient patient/composition-*.rs user/*.rs"
        );
        assert_eq!(SmartScope::parse_all(&printed), scopes);
        assert_eq!(SmartScope::format_all(&[]), "");
    }

    #[test]
    fn system_aql_wildcard_grants_all_queries() {
        // master08 NOTE: `system/aql-*.rs` grants access to all queries.
        let r = resource("system/aql-*.rs");
        assert_eq!(r.compartment, Compartment::System);
        assert!(r.resource.pattern().matches("any::query.v1"));
        assert!(r.resource.pattern().matches("adhoc"));
        assert!(r.permissions.search && r.permissions.read);
    }
}

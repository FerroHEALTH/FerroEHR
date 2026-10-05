// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scope previewer's presentation model: one pure function from a SMART
//! scope string to the grant the viewer renders for it.
//!
//! **The parse is never ours.** It is `openehr_sdt::smart_scopes` — the
//! ONE SMART on openEHR grammar
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`
//! §Resource Scopes, plus the master07/master09 launch contexts), the same code
//! the CDR's scope gate enforces with, so what this drawer explains and what the
//! CDR permits cannot drift. This module only turns a parsed scope into labels,
//! chips and copy — and, for a string the grammar REJECTED, the diagnosis that
//! says which part of the form is wrong instead of dropping it silently.
//!
//! Everything here is a pure function of the input string (no clock, no request,
//! no I/O), which is what makes the previewer hydration-safe by construction and
//! lets it run in the browser rather than round-tripping the BFF for a parse.
//!
//! **Capability is not authorization.** A scope NARROWS what a token may ask
//! for; it grants nothing by itself. master08 §Scopes: the Platform validates
//! requested scopes "against the _Application_ registration metadata, applicable
//! access control policies, the authenticated user's permissions" — so the CDR
//! remains the enforcer and a previewed grant is an upper bound, never a promise.

use openehr_sdt::smart_scopes::{
    Compartment, LaunchContext, Permissions, ResourceScope, ResourceSelector, SmartScope,
};

/// The master08 §Resource Scopes form, quoted verbatim in the previewer's
/// diagnostics (the one sentence a mistyped scope needs).
pub const RESOURCE_SCOPE_FORM: &str = "<compartment>/<resource>.<permission>";

/// The standing capability-vs-authorization caveat the drawer states (master08
/// §Scopes.
///
/// The Platform validates every requested scope against the client
/// registration, the access-control policies and the user's own permissions).
pub const CAPABILITY_NOTE: &str = "Scopes narrow access, they never grant it: they say what a token may ask for, and the CDR still decides every request against its own policies and your permissions. A grant shown here is an upper bound.";

/// One scope string and what the grammar made of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The scope string exactly as written (always shown, so an unrecognised
    /// scope is visible rather than swallowed).
    pub raw: String,
    /// The rendered reading of that string.
    pub detail: GrantDetail,
}

/// The four readings the master08 grammar yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantDetail {
    /// A resource scope `<compartment>/<resource>.<permission>`.
    Resource(ResourceGrant),
    /// A launch marker or launch-context request (master07 §Context Selection;
    /// the episode context is master09, experimental).
    Context {
        /// Row label.
        label: String,
        /// What requesting the context means.
        note: &'static str,
    },
    /// An `OpenID` Connect identity scope (master08 §Scopes, "Identity Claims").
    Identity {
        /// Row label.
        label: String,
        /// What the claim scope does — and does not — carry.
        note: &'static str,
    },
    /// The grammar did not recognise the string: it is retained verbatim and
    /// inert. `expected` carries the master08 explanation whenever the string
    /// LOOKS like a resource scope, so a typo is actionable instead of silent.
    Unrecognized {
        /// Which part of the resource-scope form is wrong, when it is one.
        expected: Option<String>,
    },
}

/// A resource scope rendered as its parts: who it delegates to, what it reaches,
/// and which operations it permits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceGrant {
    /// The compartment keyword (`patient` / `user` / `system`).
    pub compartment: &'static str,
    /// What that compartment narrows access to (master08 §Resource Scopes).
    pub compartment_note: &'static str,
    /// The resource family the scope reaches.
    pub family: &'static str,
    /// The template-id / query-name pattern, as written.
    pub pattern: String,
    /// How that pattern matches (master08 §Resource Scopes pattern table).
    pub pattern_note: &'static str,
    /// The permitted operations, in CRUDS order.
    pub permissions: Vec<&'static str>,
    /// A bare `*`/`**` pattern: broad access, which master08's NOTE says to use
    /// cautiously ("`system/aql-*.rs` would grant access to all registered and
    /// ad-hoc AQL queries system-wide").
    pub broad: bool,
}

/// Render one scope string.
#[must_use]
pub fn grant(raw: &str) -> Grant {
    let detail = match SmartScope::parse(raw) {
        SmartScope::Launch => GrantDetail::Context {
            label: "Launch marker".to_owned(),
            note: "Marks an embedded launch. Advisory only — the CDR performs no launch selection.",
        },
        SmartScope::LaunchContext(context) => context_detail(&context),
        SmartScope::Identity(name) => GrantDetail::Identity {
            label: format!("Identity claim · {name}"),
            note: "An OpenID Connect scope: it identifies the user to the application and reaches no clinical data.",
        },
        SmartScope::Resource(scope) => GrantDetail::Resource(resource_grant(&scope)),
        SmartScope::Other(_) => GrantDetail::Unrecognized {
            expected: resource_expectation(raw),
        },
    };
    Grant {
        raw: raw.to_owned(),
        detail,
    }
}

/// Render a whole space-delimited scope claim (what a token carries, and what
/// the previewer field accepts).
#[must_use]
pub fn grants(scope_claim: &str) -> Vec<Grant> {
    scope_claim.split_whitespace().map(grant).collect()
}

/// Render the session's scope list. Each entry is itself split on whitespace, so
/// a single claim string stored whole reads the same as a pre-split list.
#[must_use]
pub fn grants_of(scopes: &[String]) -> Vec<Grant> {
    scopes
        .iter()
        .flat_map(|scope| grants(scope.as_str()))
        .collect()
}

/// Where the authority for this session's requests comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicySource {
    /// How the session authenticates to the CDR.
    pub label: &'static str,
    /// What decides what it may do.
    pub note: &'static str,
}

/// The policy source for a session's authentication method (`"basic"` /
/// `"oidc"`, as [`crate::auth::SessionInfo::method`] reports it).
///
/// The viewer states only what the session actually carries: a Basic session
/// replays a CDR account, an OIDC session carries a token whose scopes it can
/// show. Roles are the CDR's own resolution from that same credential — the
/// viewer never claims to know them.
#[must_use]
pub fn policy_source(method: &str) -> PolicySource {
    match method {
        "basic" => PolicySource {
            label: "Basic authentication",
            note: "The viewer replays this account's CDR credentials on every call, so the CDR applies that account's own privileges. A Basic session carries no SMART scopes.",
        },
        "oidc" => PolicySource {
            label: "OIDC bearer token",
            note: "The CDR resolves this session's roles and permissions from the same access token; the scopes below are the ones the token was issued with.",
        },
        _ => PolicySource {
            label: "Viewer session",
            note: "The CDR decides every request from the credential this session holds.",
        },
    }
}

/// The launch-context reading (master07 §Context Selection; master09 §Episode
/// Context is experimental).
fn context_detail(context: &LaunchContext) -> GrantDetail {
    match context {
        LaunchContext::Patient => GrantDetail::Context {
            label: "Launch context · patient".to_owned(),
            note: "Asks the launching platform to put one patient/EHR in context; a patient-compartment scope is then read against that EHR.",
        },
        LaunchContext::Episode => GrantDetail::Context {
            label: "Launch context · episode".to_owned(),
            note: "Asks for an episode of care in context. Experimental in the specification.",
        },
        LaunchContext::Other(other) => GrantDetail::Context {
            label: format!("Launch context · {other}"),
            note: "Not a context openEHR defines. Carried alongside the openEHR scopes and inert here.",
        },
    }
}

/// Split a parsed resource scope into its rendered parts.
fn resource_grant(scope: &ResourceScope) -> ResourceGrant {
    let (compartment, compartment_note) = match scope.compartment {
        // The three compartment bullets of master08 §Resource Scopes.
        Compartment::Patient => (
            "patient",
            "Restricted to the EHR of the patient in context.",
        ),
        Compartment::User => (
            "user",
            "Subject to the authenticated user's security profile — not limited to the patient in context.",
        ),
        Compartment::System => (
            "system",
            "A backend client acting without a user context: across all data.",
        ),
    };
    let family = match &scope.resource {
        ResourceSelector::Template(_) => "Operational templates",
        ResourceSelector::Composition(_) => "Compositions of the matching template",
        ResourceSelector::Aql(_) => "AQL queries",
    };
    let pattern = scope.resource.pattern().as_str().to_owned();
    let broad = pattern == "*" || pattern == "**";
    ResourceGrant {
        compartment,
        compartment_note,
        family,
        pattern_note: pattern_note(&pattern, broad),
        pattern,
        permissions: permission_labels(scope.permissions),
        broad,
    }
}

/// How a pattern matches, per the master08 §Resource Scopes pattern table.
fn pattern_note(pattern: &str, broad: bool) -> &'static str {
    if broad {
        "All available templates or queries — including ad-hoc AQL."
    } else if pattern.contains('*') {
        "Wildcard pattern: * matches within one namespace segment, ** across segments."
    } else {
        "Exact match only."
    }
}

/// The permitted operations as chip labels, always in CRUDS order (master08
/// §Resource Scopes permission list).
fn permission_labels(permissions: Permissions) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if permissions.create {
        labels.push("create");
    }
    if permissions.read {
        labels.push("read");
    }
    if permissions.update {
        labels.push("update");
    }
    if permissions.delete {
        labels.push("delete");
    }
    if permissions.search {
        labels.push("search");
    }
    labels
}

/// Explain why a resource-SHAPED string is not a resource scope.
///
/// Only ever called for a string the grammar already rejected — the verdict is
/// always [`SmartScope::parse`]'s, never this function's. It walks the same four
/// elements master08 §Resource Scopes names (compartment, resource, permission
/// tail, pattern) to say which one is wrong; a string with no `/` at all is not
/// resource-shaped and gets no explanation (it is simply an opaque scope the
/// grammar retains inert).
fn resource_expectation(raw: &str) -> Option<String> {
    let (compartment, rest) = raw.split_once('/')?;
    if !matches!(compartment, "patient" | "user" | "system") {
        return Some(format!(
            "\"{compartment}\" is not a compartment. A resource scope is {RESOURCE_SCOPE_FORM}, and master08 defines exactly three compartments: patient, user, system."
        ));
    }
    let Some((resource, tail)) = rest.rsplit_once('.') else {
        return Some(format!(
            "\"{rest}\" carries no .<permission> tail. A resource scope is {RESOURCE_SCOPE_FORM} — for example patient/composition-*.rs."
        ));
    };
    if tail.is_empty()
        || tail
            .chars()
            .any(|c| !matches!(c, 'c' | 'r' | 'u' | 'd' | 's'))
    {
        return Some(format!(
            "\"{tail}\" is not a permission tail. master08 allows only c (create), r (read), u (update), d (delete) and s (search), in any order — for example .rs or .crud."
        ));
    }
    if !(resource.starts_with("template-")
        || resource.starts_with("composition-")
        || resource.starts_with("aql-"))
    {
        return Some(format!(
            "\"{resource}\" is not a resource. master08 defines template-<templateId>, composition-<templateId> and aql-<queryName>."
        ));
    }
    Some(format!(
        "Not a resource scope. The master08 form is {RESOURCE_SCOPE_FORM} — for example patient/composition-*.rs."
    ))
}

#[cfg(test)]
mod tests {
    use crate::scopes::{
        Grant, GrantDetail, ResourceGrant, grant, grants, grants_of, policy_source,
    };

    fn resource(raw: &str) -> ResourceGrant {
        match grant(raw).detail {
            GrantDetail::Resource(resource) => resource,
            other => panic!("expected a resource grant for {raw}, got {other:?}"),
        }
    }

    fn expectation(raw: &str) -> String {
        match grant(raw).detail {
            GrantDetail::Unrecognized {
                expected: Some(expected),
            } => expected,
            other => panic!("expected an explained rejection for {raw}, got {other:?}"),
        }
    }

    #[test]
    fn a_patient_composition_scope_renders_its_parts() {
        let grant = resource("patient/composition-*.rs");
        assert_eq!(grant.compartment, "patient");
        assert_eq!(grant.family, "Compositions of the matching template");
        assert_eq!(grant.pattern, "*");
        assert_eq!(grant.permissions, ["read", "search"]);
        assert!(grant.broad, "a bare * is broad access (master08 NOTE)");
    }

    #[test]
    fn permission_chips_keep_cruds_order_regardless_of_the_tail_order() {
        assert_eq!(
            resource("system/aql-MyQuery.sdurc").permissions,
            ["create", "read", "update", "delete", "search"]
        );
        assert_eq!(
            resource("user/template-MyTemplate.crud").permissions,
            ["create", "read", "update", "delete"]
        );
    }

    #[test]
    fn a_dotted_exact_pattern_survives_and_reads_as_exact() {
        let grant = resource("user/template-MyHospital::Template.v0.crud");
        assert_eq!(grant.compartment, "user");
        assert_eq!(grant.family, "Operational templates");
        assert_eq!(grant.pattern, "MyHospital::Template.v0");
        assert_eq!(grant.pattern_note, "Exact match only.");
        assert!(!grant.broad);
    }

    #[test]
    fn identity_and_launch_scopes_are_labelled_as_such() {
        assert!(matches!(
            grant("openid").detail,
            GrantDetail::Identity { .. }
        ));
        assert!(matches!(
            grant("offline_access").detail,
            GrantDetail::Identity { .. }
        ));
        assert!(matches!(
            grant("launch").detail,
            GrantDetail::Context { .. }
        ));
        let GrantDetail::Context { label, .. } = grant("launch/patient").detail else {
            panic!("launch/patient is a launch context");
        };
        assert!(label.contains("patient"), "{label}");
    }

    #[test]
    fn an_opaque_scope_stays_verbatim_and_inert_without_a_diagnosis() {
        let Grant { raw, detail } = grant("some-vendor-scope");
        assert_eq!(raw, "some-vendor-scope");
        assert_eq!(detail, GrantDetail::Unrecognized { expected: None });
    }

    #[test]
    fn every_wrong_element_of_the_resource_form_explains_itself() {
        assert!(
            expectation("admin/composition-*.r").contains("not a compartment"),
            "a bad compartment names the three master08 compartments"
        );
        assert!(
            expectation("patient/composition-*").contains(".<permission>"),
            "a missing tail names the permission tail"
        );
        assert!(
            expectation("patient/composition-*.rx").contains("permission tail"),
            "a bad permission letter names the CRUDS letters"
        );
        assert!(
            expectation("patient/thing-*.r").contains("not a resource"),
            "an unknown resource names the three master08 resources"
        );
        // Every diagnosis quotes the form itself or the offending element.
        assert!(expectation("patient/composition-*.").contains("permission"));
    }

    #[test]
    fn a_claim_string_splits_on_whitespace() {
        let rendered = grants("openid  launch/patient\npatient/composition-*.rs");
        assert_eq!(rendered.len(), 3);
        assert_eq!(rendered[0].raw, "openid");
        assert_eq!(rendered[2].raw, "patient/composition-*.rs");
        assert!(grants("   ").is_empty(), "blank input renders nothing");
    }

    #[test]
    fn the_session_scope_list_renders_in_order() {
        let session = [
            "openid".to_owned(),
            "patient/aql-*.rs".to_owned(),
            "user/template-MyTemplate.crud".to_owned(),
        ];
        let rendered = grants_of(&session);
        assert_eq!(rendered.len(), 3);
        assert!(matches!(rendered[0].detail, GrantDetail::Identity { .. }));
        assert!(matches!(rendered[1].detail, GrantDetail::Resource(_)));
        assert!(matches!(rendered[2].detail, GrantDetail::Resource(_)));
    }

    #[test]
    fn each_authentication_method_names_its_policy_source() {
        let basic = policy_source("basic");
        assert_eq!(basic.label, "Basic authentication");
        assert!(
            basic.note.contains("no SMART scopes"),
            "a Basic session says why it has no scopes: {}",
            basic.note
        );
        let oidc = policy_source("oidc");
        assert_eq!(oidc.label, "OIDC bearer token");
        assert!(oidc.note.contains("access token"), "{}", oidc.note);
        // An unexpected method still names the enforcer rather than guessing.
        assert!(policy_source("something-else").note.contains("CDR"));
    }

    #[test]
    fn the_master08_maximal_table_renders_as_resource_grants() {
        // The maximal scope table of master08 §Resource Scopes, with the
        // placeholders filled in.
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
            let grant = resource(row);
            assert!(
                !grant.permissions.is_empty(),
                "{row} must render at least one permission chip"
            );
            assert!(
                !grant.compartment_note.is_empty(),
                "{row} explains its compartment"
            );
        }
    }
}

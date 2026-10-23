// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Your access: who you are, how you signed in, this session's scopes as
//! parsed grants (E8), and a previewer that reads any scope.
//!
//! Whether a scope is a scope is decided by `openehr_sdt::smart_scopes`, the
//! grammar the CDR's scope gate enforces with
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`
//! §Resource Scopes), so the page never explains a scope the CDR would refuse
//! as a grant ([`crate::scopes`]). The grant FerroUI's scope list
//! (`view::scope`) draws is built from that parse. A scope narrows what a
//! token may ask for and grants nothing by itself: the CDR decides every
//! request. No openEHR spec governs the viewer: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::scope::{
    Compartment, Grant, Operation, Operations, Request, Scope, ScopeGrant, scope_grants,
};
use ferrohealth_ui::view::surface::{Header, definitions, page_heading};
use ferrohealth_ui::view::transform::{Density, Run, RunState, run_button, transform_options};
use leptos::prelude::*;
use leptos_meta::Title;
use openehr_sdt::smart_scopes::{self, Permissions, SmartScope};

use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{card_panel, quiet, section};
use crate::scopes::GrantDetail;

/// The grant FerroUI draws for `raw`, built from the CDR's grammar: its
/// verdict, its compartment, its resource and its operations.
#[must_use]
pub fn grant_of(raw: &str) -> ScopeGrant {
    let reading = crate::scopes::grant(raw);
    let grant = match (SmartScope::parse(raw), reading.detail) {
        (SmartScope::Resource(scope), GrantDetail::Resource(resource)) => Grant::Granted {
            scope: Scope {
                compartment: compartment(scope.compartment),
                resource: scope.resource.to_string(),
                operations: operations(scope.permissions),
                constraint: None,
            },
            resource: tr_with(
                Msg::ScopeResource,
                &[
                    ("family", resource.family.into()),
                    ("pattern", resource.pattern.into()),
                ],
            ),
            broad: resource.broad,
        },
        (SmartScope::Launch, _) => Grant::Request(Request::Launch),
        (SmartScope::LaunchContext(context), _) => Grant::Request(Request::LaunchContext {
            resource: context.to_string(),
            role: None,
        }),
        (SmartScope::Identity(name), _) => match identity(&name) {
            Some(request) => Grant::Request(request),
            None => Grant::Refused(tr(Msg::ScopeNotRecognised).into()),
        },
        (_, GrantDetail::Unrecognized { expected }) => Grant::Refused(
            expected
                .unwrap_or_else(|| tr(Msg::ScopeNotRecognised))
                .into(),
        ),
        (_, _) => Grant::Refused(tr(Msg::ScopeNotRecognised).into()),
    };
    ScopeGrant {
        raw: raw.to_owned(),
        grant,
    }
}

/// FerroUI's compartment for the CDR's.
const fn compartment(compartment: smart_scopes::Compartment) -> Compartment {
    match compartment {
        smart_scopes::Compartment::Patient => Compartment::Patient,
        smart_scopes::Compartment::User => Compartment::User,
        smart_scopes::Compartment::System => Compartment::System,
    }
}

/// FerroUI's operations for the CDR's permissions.
fn operations(permissions: Permissions) -> Operations {
    [
        (permissions.create, Operation::Create),
        (permissions.read, Operation::Read),
        (permissions.update, Operation::Update),
        (permissions.delete, Operation::Delete),
        (permissions.search, Operation::Search),
    ]
    .into_iter()
    .filter_map(|(permitted, operation)| permitted.then_some(operation))
    .collect()
}

/// FerroUI's request for an identity scope the CDR reads, `None` for a
/// name the CDR's grammar does not read as one.
fn identity(name: &str) -> Option<Request> {
    match name {
        "openid" => Some(Request::OpenId),
        "profile" => Some(Request::Profile),
        "fhirUser" => Some(Request::FhirUser),
        "online_access" => Some(Request::OnlineAccess),
        "offline_access" => Some(Request::OfflineAccess),
        _ => None,
    }
}

/// The grants of every scope in `claim`, space-separated.
#[must_use]
pub fn grants_of(claim: &str) -> Vec<ScopeGrant> {
    claim.split_whitespace().map(grant_of).collect()
}

/// Your access.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn AccessPage() -> impl IntoView {
    let session = crate::request::session();
    let idle = crate::request::state().map_or(0, |state| state.config.session.idle_minutes);
    let (who, how, method, scopes) = match &session {
        Some(session) => {
            let (how, method) = match session.credential {
                crate::session::Credential::Basic { .. } => (tr(Msg::AccessHowBasic), "basic"),
                crate::session::Credential::Bearer { .. } => (tr(Msg::AccessHowOidc), "oidc"),
            };
            (
                session.identity.clone(),
                how,
                method,
                session.scopes.clone(),
            )
        }
        None => (String::new(), String::new(), "", Vec::new()),
    };
    let policy = crate::scopes::policy_source(method);
    let facts = definitions(vec![
        (
            tr(Msg::AccessSignedInAs),
            view! { <strong>{who}</strong> }.into_any(),
        ),
        (tr(Msg::AccessHow), how.into_any()),
        (tr(Msg::AccessDecides), policy.note.into_any()),
        (
            tr(Msg::AccessSession),
            tr_with(Msg::AccessSessionEnds, &[("minutes", idle.into())]).into_any(),
        ),
    ]);
    let claim = scopes.join(" ");
    let held = if method == "basic" || claim.trim().is_empty() {
        section(
            Some(tr(Msg::AccessScopes)),
            None,
            None,
            quiet(tr(Msg::AccessNoScopes)),
        )
    } else {
        let grants = grants_of(&claim);
        let count = u64::try_from(grants.len()).unwrap_or_default();
        section(
            Some(tr(Msg::AccessScopes)),
            Some(count),
            None,
            scope_grants(tr(Msg::AccessScopes), grants),
        )
    };
    let typed = crate::request::query("scope").unwrap_or_default();
    let previewed = if typed.is_empty() {
        quiet(tr(Msg::AccessPreviewEmpty))
    } else {
        scope_grants(tr(Msg::AccessPreviewed), grants_of(&typed))
    };
    let field = Field::new("scope", tr(Msg::AccessScopeField))
        .holds_id()
        .view(view! {
            <input
                id="scope"
                name="scope"
                type="text"
                class="fui-input fui-mono"
                spellcheck="false"
                autocomplete="off"
                value=typed
                placeholder=tr(Msg::AccessScopePlaceholder)
            />
        });
    let run = run_button(Run {
        id: "scope-read".to_owned(),
        label: tr(Msg::AccessReadIt),
        busy: tr(Msg::AccessReadIt),
        glyph: Glyph::SEARCH,
        shortcut: None,
        operation: None,
        state: RunState::Ready,
    });
    let preview = view! {
        <form method="get" action="/access">
            {transform_options(Density::Plain, field, run)}
        </form>
        {previewed}
    };
    let body = card_panel(view! {
        {section(Some(tr(Msg::AccessWho)), None, None, facts)}
        {held}
        {section(
            Some(tr(Msg::AccessPreview)),
            None,
            None,
            view! {
                {ferrohealth_ui::view::surface::section_intro(tr(Msg::AccessPreviewIntro))}
                {preview}
            },
        )}
    });
    let heading = page_heading(
        Header {
            title: tr(Msg::YourAccess).into(),
            subtitle: Some(tr(Msg::AccessSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ferrohealth_ui::view::action::ActionGroup::none(),
    );
    view! {
        <Title text=tr(Msg::YourAccess) />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_ui::view::scope::{Compartment, Grant, Operation, Request};

    use super::grant_of;

    #[test]
    fn a_resource_scope_is_drawn_from_the_cdr_parse() {
        let Grant::Granted {
            scope,
            resource,
            broad,
        } = grant_of("patient/composition-*.rs").grant
        else {
            panic!("the CDR reads a resource scope");
        };
        assert_eq!(scope.compartment, Compartment::Patient);
        assert_eq!(scope.resource, "composition-*");
        assert!(scope.operations.permits(Operation::Read));
        assert!(scope.operations.permits(Operation::Search));
        assert!(!scope.operations.permits(Operation::Create));
        assert!(!resource.is_empty(), "the resource is said in words");
        assert!(broad, "a bare `*` is broad");
    }

    #[test]
    fn an_aql_scope_the_cdr_reads_is_granted_with_its_own_resource() {
        let Grant::Granted { scope, broad, .. } = grant_of("system/aql-MyQuery.sdurc").grant else {
            panic!("the CDR reads an AQL scope");
        };
        assert_eq!(scope.compartment, Compartment::System);
        assert_eq!(scope.resource, "aql-MyQuery");
        for operation in Operation::ALL {
            assert!(scope.operations.permits(operation), "{operation:?}");
        }
        assert!(!broad, "an exact name is narrow");
    }

    #[test]
    fn identity_and_launch_scopes_are_requests_and_a_typo_is_refused() {
        assert_eq!(grant_of("openid").grant, Grant::Request(Request::OpenId));
        assert_eq!(grant_of("profile").grant, Grant::Request(Request::Profile));
        assert_eq!(
            grant_of("fhirUser").grant,
            Grant::Request(Request::FhirUser)
        );
        assert_eq!(
            grant_of("offline_access").grant,
            Grant::Request(Request::OfflineAccess)
        );
        assert_eq!(
            grant_of("online_access").grant,
            Grant::Request(Request::OnlineAccess)
        );
        assert_eq!(grant_of("launch").grant, Grant::Request(Request::Launch));
        assert_eq!(
            grant_of("launch/patient").grant,
            Grant::Request(Request::LaunchContext {
                resource: "patient".to_owned(),
                role: None,
            })
        );
        assert!(matches!(
            grant_of("patient/composition-*.rx").grant,
            Grant::Refused(_)
        ));
    }
}

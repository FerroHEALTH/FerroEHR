// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Your access: who you are, how you signed in, this session's scopes as
//! parsed grants (E8), and a previewer that reads any scope.
//!
//! Whether a scope is a scope is decided by `ferrohealth_smart::openehr::smart_scopes`, the
//! grammar the CDR's scope gate enforces with
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`
//! §Resource Scopes), so the page never explains a scope the CDR would refuse
//! as a grant ([`crate::scopes`]). The grant FerroUI's scope list
//! (`view::scope`) draws is built from that parse. A scope narrows what a
//! token may ask for and grants nothing by itself: the CDR decides every
//! request. No openEHR spec governs the viewer: our own design.

use ferrohealth_smart::openehr::smart_scopes::{self, Permissions, SmartScope};
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::scope::{
    Compartment, Grant, Operation, Operations, Request, Scope, ScopeGrant, scope_grants,
};
use ferrohealth_ui::view::surface::{Header, definitions, page_heading};
use ferrohealth_ui::view::transform::{Density, Run, RunState, run_button, transform_options};

use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::{card_panel, quiet, section};
use crate::routes::Drawn;
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
                ConsoleMsg::ScopeResource,
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
            None => Grant::Refused(tr(ConsoleMsg::ScopeNotRecognised).into()),
        },
        (_, GrantDetail::Unrecognized { expected }) => Grant::Refused(
            expected
                .unwrap_or_else(|| tr(ConsoleMsg::ScopeNotRecognised))
                .into(),
        ),
        (_, _) => Grant::Refused(tr(ConsoleMsg::ScopeNotRecognised).into()),
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
/// Your access.
#[expect(
    clippy::too_many_lines,
    reason = "one page: who is signed in, the scopes held and the previewer"
)]
#[must_use]
pub fn page() -> Drawn {
    let session = crate::request::signed();
    let idle = crate::request::settings().idle_minutes;
    let (who, how, method, scopes) = match &session {
        Some(session) => {
            let (how, method) = match session.method() {
                crate::request::Method::Basic => (tr(ConsoleMsg::AccessHowBasic), "basic"),
                crate::request::Method::Oidc => (tr(ConsoleMsg::AccessHowOidc), "oidc"),
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
            tr(ConsoleMsg::AccessSignedInAs),
            h(format!("<strong>{}</strong>", t(who))),
        ),
        (tr(ConsoleMsg::AccessHow), t(how)),
        (tr(ConsoleMsg::AccessDecides), t(policy.note)),
        (
            tr(ConsoleMsg::AccessSession),
            t(tr_with(
                ConsoleMsg::AccessSessionEnds,
                &[("minutes", idle.into())],
            )),
        ),
    ]);
    let claim = scopes.join(" ");
    let held = if method == "basic" || claim.trim().is_empty() {
        section(
            Some(tr(ConsoleMsg::AccessScopes)),
            None,
            None,
            quiet(tr(ConsoleMsg::AccessNoScopes)),
        )
    } else {
        let grants = grants_of(&claim);
        let count = u64::try_from(grants.len()).unwrap_or_default();
        section(
            Some(tr(ConsoleMsg::AccessScopes)),
            Some(count),
            None,
            scope_grants(&current(), &tr(ConsoleMsg::AccessScopes), grants),
        )
    };
    let typed = crate::request::query("scope").unwrap_or_default();
    let previewed = if typed.is_empty() {
        quiet(tr(ConsoleMsg::AccessPreviewEmpty))
    } else {
        scope_grants(&current(), &tr(ConsoleMsg::AccessPreviewed), grants_of(&typed))
    };
    let field = Field::new("scope", tr(ConsoleMsg::AccessScopeField))
        .holds_id()
        .view(h(format!(
            r#"<input id="scope" name="scope" type="text" class="fui-input fui-mono" spellcheck="false" autocomplete="off" value="{}" placeholder="{}">"#,
            t(typed),
            t(tr(ConsoleMsg::AccessScopePlaceholder))
        )));
    let run = run_button(Run {
        id: "scope-read".to_owned(),
        label: tr(ConsoleMsg::AccessReadIt),
        busy: tr(ConsoleMsg::AccessReadIt),
        glyph: Glyph::SEARCH,
        shortcut: None,
        operation: None,
        state: RunState::Ready,
    });
    let preview = h(format!(
        r#"<form method="get" action="/access">{}</form>{previewed}"#,
        transform_options(Density::Plain, field, run)
    ));
    let body = card_panel(Html::join([
        section(Some(tr(ConsoleMsg::AccessWho)), None, None, facts),
        held,
        section(
            Some(tr(ConsoleMsg::AccessPreview)),
            None,
            None,
            ferrohealth_ui::view::surface::section_intro(tr(ConsoleMsg::AccessPreviewIntro))
                .then(preview),
        ),
    ]));
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::YourAccess).into(),
            subtitle: Some(tr(ConsoleMsg::AccessSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ferrohealth_ui::view::action::ActionGroup::none(),
    );
    Drawn::page(
        tr(Msg::YourAccess),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
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

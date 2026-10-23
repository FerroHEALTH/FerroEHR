// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Your access: who you are, how you signed in, this session's scopes as
//! parsed grants (E8), and a previewer that reads any scope.
//!
//! Whether a scope is a scope is decided by `openehr_sdt::smart_scopes`, the
//! grammar the CDR's scope gate enforces with
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`
//! §Resource Scopes), so the page never explains a scope the CDR would refuse
//! as a grant ([`crate::scopes`]). FerroUI's scope grant (`view::scope`) draws
//! the breakdown. A scope narrows what a token may ask for and grants nothing
//! by itself: the CDR decides every request. No openEHR spec governs the
//! viewer: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::scope::{Grant, ScopeGrant, scope_grants};
use ferrohealth_ui::view::surface::{Header, definitions, page_heading};
use ferrohealth_ui::view::transform::{Density, Run, RunState, run_button, transform_options};
use leptos::prelude::*;
use leptos_meta::Title;

use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{card_panel, quiet, section};
use crate::scopes::GrantDetail;

/// The grant FerroUI draws for `raw`, its verdict the CDR's grammar's.
#[must_use]
pub fn grant_of(raw: &str) -> ScopeGrant {
    let reading = crate::scopes::grant(raw);
    match reading.detail {
        GrantDetail::Unrecognized { expected } => ScopeGrant {
            raw: raw.to_owned(),
            grant: Grant::Refused(
                expected
                    .unwrap_or_else(|| tr(Msg::ScopeNotRecognised))
                    .into(),
            ),
        },
        GrantDetail::Resource(resource) => ScopeGrant::read(raw, |_| {
            (
                tr_with(
                    Msg::ScopeResource,
                    &[
                        ("family", resource.family.clone().into()),
                        ("pattern", resource.pattern.clone().into()),
                    ],
                ),
                resource.broad,
            )
        }),
        GrantDetail::Context { .. } | GrantDetail::Identity { .. } => {
            ScopeGrant::read(raw, |_| (String::new(), false))
        }
    }
}

// TODO(#3942): the CDR grammar builds the grant with FerroHEALTH/FerroUI#350.
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

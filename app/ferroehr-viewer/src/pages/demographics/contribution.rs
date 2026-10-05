// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `/demographics/contribution/{uid}` — the demographic CONTRIBUTION viewer.
//!
//! A read-only screen over `GET /demographic/contribution/{contribution_uid}`
//! ("Retrieves a CONTRIBUTION identified by `contribution_uid`",
//! `operations/demographic_contribution_get.yaml`), reached from the
//! contribution a version's envelope names on any demographic History tab.
//!
//! Committing a contribution is deliberately NOT here: every party and
//! relationship write on these screens already commits its own CONTRIBUTION,
//! and a raw change-set authoring form is a different feature.
//!
//! The RM attributes shown are the CONTRIBUTION class's own
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.contribution.adoc`):
//! `uid`, the `audit` `AUDIT_DETAILS`, and `versions` — "the set of references
//! to Versions causing changes to this EHR" — each of which the demographic
//! surface serves as an `OBJECT_REF` whose `type` names the RM class of the
//! object that changed, which is what lets each row link to that object.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use leptos_router::components::A;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssr")]
use serde_json::Value;

use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::format_view::DocumentPane;
use crate::components::notice::inline_error;
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::error::ViewerError;
use crate::pages::demographics::party::fact_row;
use crate::pages::demographics::{PartyKind, party_href, relationship_href};
use crate::uid::container_uid_of;

/// One version a CONTRIBUTION changed, as its `OBJECT_REF` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContributionVersion {
    /// `OBJECT_REF.type` — the RM class of the changed object.
    pub rm_type: String,
    /// `OBJECT_REF.id.value` — the changed version's `OBJECT_VERSION_ID`.
    pub version_uid: String,
}

/// The viewer's view of one demographic CONTRIBUTION.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContributionState {
    /// The canonical CONTRIBUTION JSON, pretty-printed for the pane.
    pub body: String,
    /// `CONTRIBUTION.uid.value`.
    pub uid: String,
    /// `audit.committer.name`.
    pub committer: String,
    /// `audit.time_committed.value`.
    pub time_committed: String,
    /// `audit.change_type.value`.
    pub change_type: String,
    /// `audit.description.value`, empty when the commit carried none.
    pub description: String,
    /// The versions this contribution changed.
    pub versions: Vec<ContributionVersion>,
}

/// Read one demographic CONTRIBUTION.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty uid; CDR transport errors pass
/// through; a non-2xx CDR answer (the `404` for an unknown contribution, or for
/// an EHR-scoped one, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_demographic_contribution(
    /// The CONTRIBUTION uid to read.
    contribution_uid: String,
) -> Result<ContributionState, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let contribution_uid = contribution_uid.trim();
    if contribution_uid.is_empty() {
        return Err(ViewerError::Invalid(
            "a contribution uid is required".to_owned(),
        ));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/contribution/{}",
        urlencoding::encode(contribution_uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    parse_contribution(&body)
}

#[cfg(feature = "ssr")]
/// Flatten a CONTRIBUTION body into a [`ContributionState`]. Defensive
/// throughout — an absent attribute reads as empty rather than failing the
/// screen.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_contribution(body: &str) -> Result<ContributionState, ViewerError> {
    let doc: Value = serde_json::from_str(body)
        .map_err(|e| ViewerError::Internal(format!("CONTRIBUTION JSON: {e}")))?;
    let versions = doc
        .get("versions")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .map(|reference| ContributionVersion {
                    rm_type: reference
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    version_uid: super::json_str(reference, &["id", "value"]),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ContributionState {
        body: crate::components::format_view::pretty_body(
            body,
            crate::format::ReprFormat::CanonicalJson,
        ),
        uid: crate::uid::uid_value_of_document(&doc),
        committer: super::json_str(&doc, &["audit", "committer", "name"]),
        time_committed: super::json_str(&doc, &["audit", "time_committed", "value"]),
        change_type: super::json_str(&doc, &["audit", "change_type", "value"]),
        description: super::json_str(&doc, &["audit", "description", "value"]),
        versions,
    })
}

/// The `/demographics/contribution/{uid}` screen: the commit's audit facts, the
/// versions it changed, and the whole CONTRIBUTION document.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn DemographicContributionPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let uid = Signal::derive(move || params.with(|p| p.get("uid").unwrap_or_default()));
    let resource: Resource<Result<ContributionState, ViewerError>> = Resource::new(
        move || uid.get(),
        |id| async move { fetch_demographic_contribution(id).await },
    );
    let heading = Signal::derive(move || {
        let id = uid.get();
        let short: String = id.chars().take(8).collect();
        format!("Contribution {short}…")
    });

    let body = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(state) => contribution_view(&state),
                    Err(e) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => {
                        view! {
                            <div
                                role="alert"
                                id="contribution-not-found"
                                class="rounded-card border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger"
                            >
                                "The CDR holds no demographic contribution with this id. A contribution committed against an EHR is read on that EHR's own screen instead."
                            </div>
                        }
                            .into_any()
                    }
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();

    view! {
        <Title text="Contribution" />
        <div class="p-6">
            <PageHeader
                title=heading
                crumbs=vec![Crumb::new("Demographics", super::browse_href(PartyKind::Person))]
                mono=true
            />
            {body}
        </div>
    }
}

/// The loaded contribution: its audit facts, its changed versions, its document.
fn contribution_view(state: &ContributionState) -> AnyView {
    let facts = view! {
        <section class=CARD_PAD id="contribution-facts">
            <h2 class=CARD_TITLE>"Commit"</h2>
            <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
                {fact_row("contribution", "contribution-uid", state.uid.clone())}
                {fact_row("committed", "committed", state.time_committed.clone())}
                {fact_row("committer", "committer", state.committer.clone())}
                {fact_row("change type", "change-type", state.change_type.clone())}
                {fact_row("description", "description", state.description.clone())}
            </div>
        </section>
    }
    .into_any();
    let versions = versions_section(state.versions.clone());
    let pretty = RwSignal::new(state.body.clone());
    view! {
        <div class="flex flex-col gap-4">
            {facts} {versions} <section class=CARD_PAD>
                <h2 class=CARD_TITLE>"Document"</h2>
                <div id="contribution-document">
                    <DocumentPane body=pretty />
                </div>
            </section>
        </div>
    }
    .into_any()
}

/// The versions this contribution changed, each linked to the object it belongs
/// to.
///
/// `<For>` keyed on the version's own `OBJECT_VERSION_ID` — stable, unique,
/// data-derived.
fn versions_section(versions: Vec<ContributionVersion>) -> AnyView {
    if versions.is_empty() {
        return view! {
            <section class=CARD_PAD>
                <h2 class=CARD_TITLE>"Versions"</h2>
                <EmptyState
                    icon=icondata_lu::LuFileClock
                    message="No versions listed"
                    hint="A CONTRIBUTION always references at least one version; if none is listed, the CDR reported an empty change set."
                />
            </section>
        }
        .into_any();
    }
    let rows = view! {
        <For each=move || versions.clone() key=|version| version.version_uid.clone() let:version>
            {version_row(&version)}
        </For>
    }
    .into_any();
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Versions in this contribution"</h2>
            {table_shell(&["Type", "Version"], rows)}
        </section>
    }
    .into_any()
}

/// One changed version: its RM type and its id, linked to the object's own
/// screen when the type names one this section can route to.
fn version_row(version: &ContributionVersion) -> AnyView {
    let rm_type = version.rm_type.clone();
    let version_uid = version.version_uid.clone();
    let shown = version_uid.clone();
    let href = object_href(&version.rm_type, &version_uid);
    let cell = match href {
        Some(href) => view! {
            <td class=CELL_MONO>
                <A href=href attr:class="text-accent hover:underline">
                    {shown}
                </A>
            </td>
        }
        .into_any(),
        None => view! { <td class=CELL_MONO>{shown}</td> }.into_any(),
    };
    view! {
        <tr class=ROW>
            <td class=CELL data-contribution-version=version_uid>
                {rm_type}
            </td>
            {cell}
        </tr>
    }
    .into_any()
}

/// The viewer route for a changed object, from the RM type its `OBJECT_REF`
/// declares: a party kind, the relationship extension, or `None` for anything
/// else (an EHR-scoped type can never appear on a demographic contribution, but
/// a link is only offered where one is real).
fn object_href(rm_type: &str, version_uid: &str) -> Option<String> {
    if rm_type == "PARTY_RELATIONSHIP" {
        return Some(relationship_href(&container_uid_of(version_uid)));
    }
    PartyKind::from_rm_type(rm_type).map(|kind| party_href(kind, &container_uid_of(version_uid)))
}

#[cfg(test)]
mod tests {
    use super::object_href;

    #[test]
    fn a_changed_version_links_to_the_object_it_belongs_to() {
        assert_eq!(
            object_href("PERSON", "8849182c::example.org::2").as_deref(),
            Some("/demographics/person/8849182c")
        );
        assert_eq!(
            object_href("PARTY_RELATIONSHIP", "7d44aa01::example.org::1").as_deref(),
            Some("/demographics/relationship/7d44aa01")
        );
        // An abstract or unknown type gets no link rather than a 404 one.
        assert_eq!(object_href("PARTY", "x::y::1"), None);
        assert_eq!(object_href("COMPOSITION", "x::y::1"), None);
        assert_eq!(object_href("", "x::y::1"), None);
    }
}

#[cfg(all(test, feature = "ssr"))]
mod wire_tests {
    use super::parse_contribution;

    #[test]
    fn parses_the_audit_facts_and_the_changed_versions() {
        let body = r#"{
            "_type": "CONTRIBUTION",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "c9"},
            "audit": {
                "_type": "AUDIT_DETAILS",
                "system_id": "example.org",
                "committer": {"_type": "PARTY_IDENTIFIED", "name": "Dr Jane Williams"},
                "time_committed": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"},
                "change_type": {"_type": "DV_CODED_TEXT", "value": "creation"},
                "description": {"_type": "DV_TEXT", "value": "PERSON creation"}
            },
            "versions": [
                {"_type": "OBJECT_REF", "namespace": "demographic", "type": "PERSON",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::1"}},
                {"_type": "OBJECT_REF", "namespace": "demographic", "type": "PARTY_RELATIONSHIP",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::example.org::1"}}
            ]
        }"#;
        let state = parse_contribution(body).expect("a valid CONTRIBUTION");
        assert_eq!(state.uid, "c9");
        assert_eq!(state.committer, "Dr Jane Williams");
        assert_eq!(state.time_committed, "2026-07-12T10:00:00Z");
        assert_eq!(state.change_type, "creation");
        assert_eq!(state.description, "PERSON creation");
        assert_eq!(state.versions.len(), 2);
        assert_eq!(state.versions[0].rm_type, "PERSON");
        assert_eq!(state.versions[0].version_uid, "8849182c::example.org::1");
        assert_eq!(state.versions[1].rm_type, "PARTY_RELATIONSHIP");
        // The pane gets the document pretty-printed.
        assert!(state.body.contains("\n  \"_type\""));
    }

    #[test]
    fn a_sparse_contribution_reads_as_empty_facts_and_a_bad_body_errors() {
        let state = parse_contribution("{}").expect("an object parses");
        assert_eq!(state.uid, "");
        assert!(state.versions.is_empty());
        assert!(parse_contribution("not json").is_err());
    }
}

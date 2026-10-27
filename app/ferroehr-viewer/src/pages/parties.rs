// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Parties, their relationships and their contributions.
//!
//! Open a party by its id and the demographic tag index, New party, one party
//! with its tabs, its editor and delete, relationships, and the demographic
//! contribution (FerroUI `design/consoles/ferroehr/`, pages 15.1 to 15.18).
//!
//! The Demographic API publishes no listing of parties, so the page opens a
//! party by kind and id, and lists the one collection the release does
//! publish, the demographic tags (ITS-REST `demographic_tags_get`). The API
//! is `DEVELOPMENT` state in ITS-REST 1.1.0; the relationship routes are
//! FerroEHR's own extension ([`crate::bff::demographics`]). The tab of a
//! party is the address's `tab`, the version its `version`. No openEHR spec
//! governs the viewer's pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::diagnostics::DiagnosticsWords;
use ferrohealth_ui::view::dialog::{Confirmation, danger_dialog};
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::reading::{Level, Reading, reading};
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, page_heading,
};
use ferrohealth_ui::view::tabs::{Tab, binder};
use ferrohealth_ui::view::template::list::{ListCard, list_card, list_empty};
use ferrohealth_ui::view::versions::{
    Standing, Version, VersionHistory, older_version, version_history,
};
use ferrohealth_ui::widget::action_menu::{Act, Action, Item, Menu};
use ferrohealth_ui::widget::editor::EditorWords;
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

use crate::bff::demographics::contribution::ContributionState;
#[cfg(not(target_arch = "wasm32"))]
use crate::bff::demographics::contribution::fetch_demographic_contribution;
use crate::bff::demographics::party::PartyState;
#[cfg(not(target_arch = "wasm32"))]
use crate::bff::demographics::party::{create_party, delete_party, fetch_party, update_party};
use crate::bff::demographics::relationship::{PartyRefView, RelationshipState};
#[cfg(not(target_arch = "wasm32"))]
use crate::bff::demographics::relationship::{
    create_relationship, delete_relationship, fetch_relationship, update_relationship,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::bff::demographics::tags::{
    delete_party_tag, fetch_party_tags, list_demographic_tags, set_party_tag,
};
use crate::bff::demographics::{PartyKind, contribution_href, party_href, relationship_href};
#[cfg(not(target_arch = "wasm32"))]
use crate::bff::demographics::{
    VersionedFamily, fetch_demographic_revision_history, fetch_demographic_version_document,
    fetch_versioned_object, resolve_demographic_version_at_time, resolve_party_kind,
};
use crate::bff::wire::{VersionEntry, VersionedObjectFacts};
use crate::clinical::RenderedSection;
use crate::components::{CodeEditor, EditorField, EditorLanguage, EditorSetup};
use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::flash::{Done, redirect_with};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, message, tr, tr_with};
use crate::pages::common::{
    EditorDock, Refused, ago, at_time_row, card_panel, count, dock_editor, docked,
    document_section, drawn, findings_of, info_note, marks_of, page_model, panel, plain_section,
    section, short, shortened, state_mark, version_number, when,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::pages::common::{discarded, is_status, version_uid_at};
#[cfg(not(target_arch = "wasm32"))]
use crate::pages::item_tags::{TagAct, tag_act, tag_done};
use crate::pages::item_tags::{TagPanel, tag_panel};

/// The id of Delete party's confirmation dialog.
const DELETE_DIALOG: &str = "delete-party";

/// The id of Delete relationship's confirmation dialog.
const DELETE_RELATIONSHIP: &str = "delete-relationship";

/// The id of the editors' form.
const EDIT_FORM: &str = "party-edit";

/// The reader's word for a kind.
fn kind_words(kind: PartyKind) -> String {
    tr(match kind {
        PartyKind::Agent => RecordsMsg::KindAgent,
        PartyKind::Group => RecordsMsg::KindGroup,
        PartyKind::Organisation => RecordsMsg::KindOrganisation,
        PartyKind::Person => RecordsMsg::KindPerson,
        PartyKind::Role => RecordsMsg::KindRole,
    })
}

/// The kind choices of a select, `kind` chosen.
fn kind_options(kind: PartyKind) -> AnyView {
    PartyKind::ALL
        .into_iter()
        .map(|option| {
            view! {
                <option value=option.segment() selected=option == kind>
                    {kind_words(option)}
                </option>
            }
        })
        .collect_view()
        .into_any()
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's demographic JSON wire for a party's identities and name"
)]
/// The name a party goes by: the text values of its first identity's
/// elements, joined, or its short id when it carries none.
fn display_name(state: &PartyState) -> String {
    use serde_json::Value;
    let words: Vec<String> = serde_json::from_str::<Value>(&state.identities)
        .ok()
        .and_then(|identities| identities.as_array().and_then(|list| list.first().cloned()))
        .map(|identity| {
            let mut words = Vec::new();
            collect_text(&identity, &mut words);
            words
        })
        .unwrap_or_default();
    if words.is_empty() {
        shortened(&state.versioned_object_uid)
    } else {
        words.join(" ")
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's demographic JSON wire for a party's identities and name"
)]
/// The `DV_TEXT` values of the `ELEMENT`s under `node`, in order.
fn collect_text(node: &serde_json::Value, words: &mut Vec<String>) {
    use serde_json::Value;
    match node {
        Value::Object(object) => {
            let element = object.get("_type").and_then(Value::as_str) == Some("ELEMENT");
            if element {
                if let Some(text) = node.pointer("/value/value").and_then(Value::as_str) {
                    words.push(text.to_owned());
                }
                return;
            }
            for (key, child) in object {
                if key != "name" {
                    collect_text(child, words);
                }
            }
        }
        Value::Array(list) => list.iter().for_each(|child| collect_text(child, words)),
        _ => {}
    }
}

/// The kind's segment and the id the route names.
fn party_route() -> (String, String) {
    crate::routes::params().with(|params| {
        (
            params.get("kind").unwrap_or_default(),
            params.get("uid").unwrap_or_default(),
        )
    })
}

/// The id the route names.
fn uid_param() -> String {
    crate::routes::params()
        .with(|params| params.get("uid"))
        .unwrap_or_default()
}

/// The trail of the pages under a party.
fn party_crumbs() -> Vec<Crumb> {
    vec![Crumb::new(tr(Msg::NavParties), "/parties")]
}

/// One row of the demographic tag index: the tag and the party it is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Indexed {
    /// The tag's key.
    key: String,
    /// Its value.
    value: String,
    /// The party's name.
    on: String,
    /// The party's page.
    href: Option<String>,
    /// The party's kind.
    kind: Option<PartyKind>,
}

#[cfg(not(target_arch = "wasm32"))]
/// The demographic tag index, each target named by asking the CDR which
/// party holds it.
async fn tag_index() -> Result<Vec<Indexed>, ViewerError> {
    let tags = list_demographic_tags(String::new(), String::new(), String::new()).await?;
    let mut named: Vec<(String, Option<PartyKind>, String)> = Vec::new();
    let mut rows = Vec::new();
    for tag in tags {
        let container = crate::uid::container_uid_of(&tag.target);
        let known = named.iter().find(|(uid, _, _)| *uid == container).cloned();
        let (kind, name) = if let Some((_, kind, name)) = known {
            (kind, name)
        } else {
            let kind = resolve_party_kind(container.clone())
                .await?
                .as_deref()
                .and_then(PartyKind::from_segment);
            let name = match kind {
                Some(kind) => fetch_party(kind.segment().to_owned(), container.clone())
                    .await?
                    .map_or_else(|| shortened(&container), |state| display_name(&state)),
                None => shortened(&container),
            };
            named.push((container.clone(), kind, name.clone()));
            (kind, name)
        };
        rows.push(Indexed {
            key: tag.key,
            value: tag.value,
            on: name,
            href: kind.map(|kind| party_href(kind, &container)),
            kind,
        });
    }
    Ok(rows)
}

/// The tag index as a table.
fn index_table(rows: &[Indexed]) -> AnyView {
    let columns = [
        Column::text("key", tr(RecordsMsg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("on", tr(RecordsMsg::PartiesTagOn)).role(Role::Meta),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Meta),
    ];
    let body = rows
        .iter()
        .map(|row| {
            let on = match &row.href {
                Some(href) => view! {
                    <a class="fui-row-link" href=href.clone()>
                        {row.on.clone()}
                    </a>
                }
                .into_any(),
                None => row.on.clone().into_any(),
            };
            view! {
                <tr>
                    <th scope="row" data-role="title">
                        <code>{row.key.clone()}</code>
                    </th>
                    {cell(&columns[1], row.value.clone())}
                    {cell(&columns[2], on)}
                    {cell(&columns[3], row.kind.map(kind_words).unwrap_or_default())}
                </tr>
            }
            .into_any()
        })
        .collect_view()
        .into_any();
    let caption = tr(RecordsMsg::PartiesTagIndex);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(body),
    })
    .into_any()
}

#[cfg(not(target_arch = "wasm32"))]
/// Opens the party the form names: its page when its kind holds it, the page
/// of the kind that does when another holds it. `Err` is what the field says.
async fn open(kind: PartyKind, uid: &str) -> Result<String, Result<String, ViewerError>> {
    match fetch_party(kind.segment().to_owned(), uid.to_owned()).await {
        Ok(_) => return Ok(party_href(kind, uid)),
        Err(error) if !is_status(&error, http::StatusCode::NOT_FOUND) => return Err(Err(error)),
        Err(_) => {}
    }
    match resolve_party_kind(uid.to_owned()).await {
        Ok(Some(segment)) => match PartyKind::from_segment(&segment) {
            Some(other) => Ok(party_href(other, uid)),
            None => Err(Ok(tr(RecordsMsg::PartiesNoneHolds))),
        },
        Ok(None) => Err(Ok(tr_with(
            RecordsMsg::PartiesNoKindHolds,
            &[("kind", kind_words(kind).into())],
        ))),
        Err(error) => Err(Err(error)),
    }
}

/// Parties' model: the party asked for and the tag index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum PartiesModel {
    /// The open form and the tag index.
    Shown {
        /// The kind asked for.
        kind: PartyKind,
        /// The id asked for.
        uid: Option<String>,
        /// What the id field says is wrong with it.
        invalid: Option<String>,
        /// The CDR's refusal of the open.
        refused: Option<Refused>,
        /// The tag index, or the CDR's refusal of it.
        index: Result<Vec<Indexed>, Refused>,
    },
    /// The party asked for opened, so the page redirected to it.
    Left,
}

/// Loads Parties: opens the party the address asks for, or reads the tag
/// index.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_parties(
    /// The path and query of the page.
    href: String,
) -> Result<PartiesModel, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_parties().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_parties`] reads, on the server.
async fn read_parties() -> PartiesModel {
    let kind = crate::request::query("kind")
        .as_deref()
        .and_then(PartyKind::from_segment)
        .unwrap_or(PartyKind::Person);
    let uid = crate::request::query("uid")
        .map(|uid| uid.trim().to_owned())
        .filter(|uid| !uid.is_empty());
    let (invalid, refused) = match &uid {
        Some(uid) => match open(kind, uid).await {
            Ok(href) => {
                leptos_axum::redirect(&href);
                return PartiesModel::Left;
            }
            Err(Ok(message)) => (Some(message), None),
            Err(Err(error)) => (
                None,
                Some(Refused::new(tr(RecordsMsg::PartiesOpenRefused), error)),
            ),
        },
        None => (None, None),
    };
    let index = tag_index()
        .await
        .map_err(|error| Refused::new(tr(RecordsMsg::PartiesTagsRefused), error));
    PartiesModel::Shown {
        kind,
        uid,
        invalid,
        refused,
        index,
    }
}

/// Parties' body: the open form and the tag index.
fn parties_view(model: PartiesModel) -> AnyView {
    let PartiesModel::Shown {
        kind,
        uid,
        invalid,
        refused,
        index,
    } = model
    else {
        return ().into_any();
    };
    let error = invalid.clone().map(|message| {
        view! {
            <p id="p-id-error" class="fui-field-error">
                {message}
            </p>
        }
    });
    let form = view! {
        <form class="fui-card fui-panel" method="get" action="/parties">
            <div class="fui-transform-options">
                {Field::new("p-kind", tr(Msg::ColumnKind))
                    .view(
                        view! {
                            <select id="p-kind" name="kind" class="fui-select">
                                {kind_options(kind)}
                            </select>
                        },
                    )}
                {Field::new("p-id", tr(RecordsMsg::PartiesUid))
                    .holds_id()
                    .after(error)
                    .view(
                        view! {
                            <input
                                id="p-id"
                                name="uid"
                                type="text"
                                class="fui-input fui-mono"
                                spellcheck="false"
                                autocomplete="off"
                                value=uid.clone().unwrap_or_default()
                                aria-invalid=invalid.is_some().then_some("true")
                                aria-describedby=invalid.is_some().then_some("p-id-error")
                            />
                        },
                    )} <div class="fui-transform-run">
                    <button type="submit" class="fui-button fui-button-secondary">
                        {ferrohealth_ui::icon::icon(Glyph::ARROW_RIGHT, "16")}
                        {tr(Msg::PartiesOpen)}
                    </button>
                </div>
            </div>
        </form>
    };
    let (index, total) = match index {
        Ok(rows) if rows.is_empty() => (list_empty(tr(RecordsMsg::PartiesNoTags), None), Some(0)),
        Ok(rows) => (index_table(&rows), u64::try_from(rows.len()).ok()),
        Err(refusal) => (refusal.view(), None),
    };
    let card = list_card(ListCard {
        title: tr(RecordsMsg::PartiesTagsTitle),
        count: total.map(ferrohealth_ui::view::template::list::HeadingCount::All),
        order: None,
        tabs: None,
        actions: None,
        body: index,
        pager: None,
    });
    let top = refused.map(|refusal| refusal.view());
    view! {
        <div class="fui-list-page">
            {info_note(tr(RecordsMsg::PartiesNote))} {form} {top} {card}
        </div>
    }
    .into_any()
}

/// Parties: open a party by its id, and the tags on parties.
#[component]
pub fn PartiesPage() -> impl IntoView {
    let heading = page_heading(
        Header {
            title: tr(Msg::NavParties).into(),
            subtitle: Some(tr(RecordsMsg::PartiesSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLUS),
            tr(RecordsMsg::PartiesNew),
            Act::Link("/parties/new".to_owned()),
        )),
    );
    let model = page_model!(load_parties());
    let body = drawn(model, parties_view);
    view! {
        <Title text=tr(Msg::NavParties) />
        {heading}
        {body}
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::disallowed_types,
    reason = "the starting document is the CDR's demographic JSON wire"
)]
/// A starting document for a party of `kind`: the type, an archetype and one
/// identity with a name.
fn starter(kind: PartyKind) -> String {
    let rm = kind.rm_type();
    let segment = kind.segment();
    let document = serde_json::json!({
        "_type": rm,
        "archetype_node_id": format!("openEHR-DEMOGRAPHIC-{rm}.{segment}.v1"),
        "name": { "_type": "DV_TEXT", "value": rm },
        "archetype_details": {
            "archetype_id": { "value": format!("openEHR-DEMOGRAPHIC-{rm}.{segment}.v1") },
            "rm_version": "1.1.0"
        },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": format!("openEHR-DEMOGRAPHIC-PARTY_IDENTITY.{segment}_name.v1"),
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "Details" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0002",
                    "name": { "_type": "DV_TEXT", "value": "Name" },
                    "value": { "_type": "DV_TEXT", "value": "" }
                }]
            }
        }]
    });
    format!("{document:#}")
}

/// The editor of a party document.
fn party_editor(
    text: String,
    opened: Option<String>,
    findings: Option<(String, ferrohealth_ui::view::diagnostics::FileFindings)>,
    label: String,
) -> AnyView {
    view! {
        <CodeEditor
            setup=EditorSetup {
                field: EditorField::Document,
                language: EditorLanguage::Json,
                text,
                opened,
                marks: marks_of(findings.as_ref()),
                label,
                lead: tr(RecordsMsg::EditorJson),
                facts: vec![tr(Msg::EditorUtf8)],
                findings,
            }
            words=EditorWords::of_page()
            diagnostics=DiagnosticsWords::of_page()
        />
    }
    .into_any()
}

/// New party's form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NewParty {
    /// The kind of party.
    kind: PartyKind,
    /// The document.
    text: String,
    /// The create the CDR refused, when the post was one.
    refused: Option<Refused>,
}

/// A creating page's model: its form, or the redirect its post answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Creating<T> {
    /// The form.
    Form(Box<T>),
    /// The post redirected.
    Left,
}

/// Loads New party: answers its post (Discard, Start over, Create).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_new_party(
    /// The path and query of the page.
    href: String,
) -> Result<Creating<NewParty>, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_new_party().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_new_party`] reads, on the server.
async fn read_new_party() -> Creating<NewParty> {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect("/parties");
        return Creating::Left;
    }
    let kind = posted
        .as_ref()
        .map(|posted| posted.text("kind"))
        .or_else(|| crate::request::query("kind"))
        .as_deref()
        .and_then(PartyKind::from_segment)
        .unwrap_or(PartyKind::Person);
    let text = posted
        .as_ref()
        .filter(|posted| posted.intent() != "kind")
        .and_then(|posted| posted.get(&EditorField::Document.name()))
        .unwrap_or_else(|| starter(kind));
    let mut refused = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        match create_party(kind.segment().to_owned(), text.clone()).await {
            Ok(uid) => {
                redirect_with(
                    &party_href(kind, &uid),
                    &Done::ok(tr(RecordsMsg::PartyCreated)).line(shortened(&uid)),
                );
                return Creating::Left;
            }
            Err(error) => refused = Some(Refused::new(tr(RecordsMsg::PartyCreateRefused), error)),
        }
    }
    Creating::Form(Box::new(NewParty {
        kind,
        text,
        refused,
    }))
}

/// New party's page, its decision bar docked.
fn new_party_view(model: Creating<NewParty>) -> AnyView {
    let Creating::Form(form) = model else {
        return ().into_any();
    };
    let NewParty {
        kind,
        text,
        refused,
    } = *form;
    let field = EditorField::Document.name();
    let findings = refused
        .as_ref()
        .and_then(|refusal| findings_of(&refusal.error, &text, &field));
    let line = if refused.is_some() {
        (Glyph::CIRCLE_X, tr(RecordsMsg::PartyNotCreated))
    } else {
        (Glyph::FILE_PLUS, tr(RecordsMsg::PartyNotYet))
    };
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: tr(RecordsMsg::PartyCreate),
        blocked: None,
    });
    let heading = editor_heading(
        message(RecordsMsg::PartyNewTitle),
        &Mode::Creating,
        Header {
            title: String::new().into(),
            subtitle: Some(
                tr_with(
                    RecordsMsg::PartyNewSubtitle,
                    &[("kind", kind_words(kind).into())],
                )
                .into(),
            ),
            crumbs: party_crumbs(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr(RecordsMsg::PartiesNew) />
        {heading}
        <div class="fui-stack">
            {refused.map(|refusal| refusal.view())}
            <form id=EDIT_FORM method="post" action="/parties/new">
                {card_panel(
                    section(
                        None,
                        None,
                        None,
                        view! {
                            <div class="fui-form-grid">
                                <div class="fui-span-2">
                                    {Field::new("pk", tr(Msg::ColumnKind))
                                        .help(tr(RecordsMsg::PartyKindHelp))
                                        .view(
                                            view! {
                                                <div class="fui-cluster">
                                                    <select id="pk" name="kind" class="fui-select">
                                                        {kind_options(kind)}
                                                    </select>
                                                    <button
                                                        type="submit"
                                                        name="intent"
                                                        value="kind"
                                                        class="fui-button fui-button-secondary fui-button-small"
                                                    >
                                                        {tr(RecordsMsg::PartyStartOver)}
                                                    </button>
                                                </div>
                                            },
                                        )}
                                </div>
                            </div>
                        },
                    ),
                )}
                {party_editor(text, None, findings, tr(RecordsMsg::PartyNewEditorLabel))}
            </form>
        </div>
    }
    .into_any()
}

/// New party.
#[component]
pub fn NewPartyPage() -> impl IntoView {
    let dock_promise = crate::shell::promise_dock();
    let model = page_model!(load_new_party());
    let page = docked(model, dock_promise, new_party_view);
    view! { {page} }
}

/// The tabs of a party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PartyTab {
    /// Its facts and identities.
    Overview,
    /// Its document.
    Document,
    /// Its versions.
    Versions,
    /// The relationships it is the source of.
    Relationships,
    /// Its tags.
    Tags,
}

impl PartyTab {
    /// Every tab, in order.
    const ALL: [Self; 5] = [
        Self::Overview,
        Self::Document,
        Self::Versions,
        Self::Relationships,
        Self::Tags,
    ];

    #[cfg(not(target_arch = "wasm32"))]
    /// The tab the address names.
    fn asked() -> Self {
        let asked = crate::request::query("tab").unwrap_or_default();
        Self::ALL
            .into_iter()
            .find(|tab| tab.key() == asked)
            .unwrap_or(Self::Overview)
    }

    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Document => "document",
            Self::Versions => "versions",
            Self::Relationships => "relationships",
            Self::Tags => "tags",
        }
    }

    /// The tab's label.
    fn label(self) -> String {
        match self {
            Self::Overview => tr(Msg::PartyOverview),
            Self::Document => tr(RecordsMsg::CompositionDocument),
            Self::Versions => tr(Msg::CompositionVersions),
            Self::Relationships => tr(RecordsMsg::PartyRelationships),
            Self::Tags => tr(RecordsMsg::EhrTabTags),
        }
    }
}

/// The address of the tab `tab` of the party, at `version` when older.
fn party_tab_href(kind: PartyKind, uid: &str, tab: PartyTab, version: Option<&str>) -> String {
    let base = party_href(kind, uid);
    let mut pairs = Vec::new();
    if tab != PartyTab::Overview {
        pairs.push(format!("tab={}", tab.key()));
    }
    if let Some(version) = version {
        pairs.push(format!("version={}", urlencoding::encode(version)));
    }
    if pairs.is_empty() {
        base
    } else {
        format!("{base}?{}", pairs.join("&"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's demographic JSON wire for a party's identities and name"
)]
/// The identities of a party, rendered for a clinical reading.
fn rendered_identities(state: &PartyState) -> Vec<RenderedSection> {
    use serde_json::Value;
    let list: Vec<Value> = serde_json::from_str::<Value>(&state.identities)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    list.iter()
        .filter_map(|identity| crate::clinical::render(&identity.to_string()))
        .collect()
}

/// The identities of a party, as a clinical reading.
fn identities(rendered: &[RenderedSection]) -> AnyView {
    let sections: Vec<_> = rendered
        .iter()
        .map(crate::pages::composition::reading_section)
        .collect();
    if sections.is_empty() {
        return info_note(tr(RecordsMsg::PartyNoIdentities));
    }
    reading(Reading {
        sections,
        level: Level::Three,
        folded: None,
    })
}

/// The Overview tab.
fn overview(
    kind: PartyKind,
    uid: &str,
    state: &PartyState,
    rendered: &[RenderedSection],
) -> AnyView {
    let relationships = state.relationships.len();
    let facts = definitions(vec![
        (tr(Msg::ColumnKind), kind_words(kind).into_any()),
        (tr(RecordsMsg::PartyArchetype), view! { <code>{state.archetype_node_id.clone()}</code> }.into_any()),
        (tr(RecordsMsg::PartyCurrentVersion), version_number(&state.version_uid).into_any()),
        (tr(RecordsMsg::PartyIdentities), state.identity_count.to_string().into_any()),
        (
            tr(RecordsMsg::PartyRelationshipsFrom),
            view! {
                <a href=party_tab_href(kind, uid, PartyTab::Relationships, None)>{relationships.to_string()}</a>
            }
            .into_any(),
        ),
    ]);
    panel(view! {
        {section(Some(tr(Msg::PartyFacts)), None, None, facts)}
        {section(Some(tr(RecordsMsg::PartyIdentities)), None, None, identities(rendered))}
    })
}

/// The page of a party reference, when it names a party of a known kind.
fn ref_href(party: &PartyRefView) -> Option<String> {
    PartyKind::from_rm_type(&party.rm_type).map(|kind| party_href(kind, &party.id))
}

/// The Relationships tab: the relationships the party is the source of.
fn relationships(kind: PartyKind, uid: &str, state: &PartyState) -> AnyView {
    let relate = ferrohealth_ui::view::button::link_button(
        format!(
            "/parties/relationships/new?source_kind={}&source={}",
            kind.segment(),
            urlencoding::encode(uid)
        ),
        ferrohealth_ui::view::button::Variant::Secondary,
        Some(Glyph::LINK),
        tr(RecordsMsg::PartyRelate),
    );
    let columns = [
        Column::text("type", tr(RecordsMsg::RelationshipType)).role(Role::Title),
        Column::text("target", tr(RecordsMsg::RelationshipTarget)).role(Role::Sub),
        Column::text("kind", tr(RecordsMsg::RelationshipTargetKind)).role(Role::Meta),
    ];
    let body = if state.relationships.is_empty() {
        list_empty(tr(RecordsMsg::PartyNoRelationships), None)
    } else {
        let rows = state
            .relationships
            .iter()
            .map(|relationship| {
                // NOTE: R9: a row is one link; a relationship held inline in the party has no uid of
                // its own, so its row opens the target instead of a relationship page.
                let href = if relationship.uid.is_empty() {
                    ref_href(&relationship.target)
                } else {
                    Some(relationship_href(&relationship.uid))
                };
                let sub = view! {
                    <span class="fui-row-sub">
                        <code>{relationship.archetype_node_id.clone()}</code>
                    </span>
                }.into_any();
                let title = title_cell(href.as_deref(), relationship.name.clone(), Some(sub));
                let target = shortened(&relationship.target.id).into_any();
                let target_kind = PartyKind::from_rm_type(&relationship.target.rm_type)
                    .map_or_else(|| relationship.target.rm_type.clone(), kind_words);
                view! { <tr>{title} {cell(&columns[1], target)} {cell(&columns[2], target_kind)}</tr> }.into_any()
            })
            .collect_view()
            .into_any();
        let caption = tr(RecordsMsg::PartyRelationships);
        data_table(DataTable {
            caption: &caption,
            columns: &columns,
            sorting: Sorting::default(),
            body: Body::Rows(rows),
        })
        .into_any()
    };
    panel(section(
        Some(tr(RecordsMsg::PartyRelationships)),
        u64::try_from(state.relationships.len()).ok(),
        Some(relate),
        view! {
            {ferrohealth_ui::view::surface::section_intro(tr(RecordsMsg::PartyRelationshipsIntro))}
            {body}
        },
    ))
}

/// A version history of a demographic object, the version in view marked.
fn demographic_history(
    versions: &[VersionEntry],
    reading: Option<&str>,
    deleted: bool,
    href: &dyn Fn(Option<&str>) -> String,
    at_action: String,
) -> AnyView {
    let latest = versions.first().map(|version| version.version_id.clone());
    let rows = versions
        .iter()
        .map(|version| {
            let number = version_number(&version.version_id);
            let current = Some(&version.version_id) == latest.as_ref();
            Version {
                href: (!(current && deleted)).then(|| href((!current).then_some(number.as_str()))),
                standing: match (current, deleted) {
                    (true, true) => Standing::Deleted,
                    (true, false) => Standing::Current,
                    _ => Standing::Older,
                },
                read: reading == Some(number.as_str()),
                number,
                change: version.change_type.clone(),
                committer: version.committer.clone(),
                when: ago(&version.committed),
                contribution: ().into_any(),
            }
        })
        .collect();
    version_history(VersionHistory {
        caption: tr(Msg::CompositionVersions),
        versions: rows,
        at_time: Some(at_time_row(
            at_action,
            &crate::request::query("at").unwrap_or_default(),
            tr(RecordsMsg::PartyAtTimeOpens),
        )),
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The revision history of the versioned object `uid` of `family`, newest
/// first.
async fn history_of(family: VersionedFamily, uid: &str) -> Result<Vec<VersionEntry>, ViewerError> {
    let mut versions =
        fetch_demographic_revision_history(family.segment().to_owned(), uid.to_owned()).await?;
    versions.sort_by_key(|version| {
        std::cmp::Reverse(
            version_number(&version.version_id)
                .parse::<u64>()
                .unwrap_or(0),
        )
    });
    Ok(versions)
}

/// The envelope facts of one version of a demographic object.
fn envelope(facts: Result<VersionedObjectFacts, Refused>) -> AnyView {
    match facts {
        Ok(facts) => section(
            Some(tr_with(RecordsMsg::CompositionVersionN, &[("version", version_number(&facts.version_id).into())])),
            None,
            None,
            definitions(vec![
                (tr(RecordsMsg::FactLifecycle), facts.lifecycle_state.clone().into_any()),
                (
                    tr(RecordsMsg::CompositionPreceding),
                    if facts.preceding_version_uid.is_empty() {
                        tr(RecordsMsg::CompositionFirstVersion).into_any()
                    } else {
                        short(&facts.preceding_version_uid)
                    },
                ),
                (
                    tr(RecordsMsg::FactContribution),
                    view! { <a href=contribution_href(&facts.contribution_uid)>{short(&facts.contribution_uid)}</a> }.into_any(),
                ),
                (tr(RecordsMsg::FactSigned), tr(if facts.signed { Msg::Yes } else { Msg::No }).into_any()),
            ]),
        ),
        Err(refusal) => plain_section(refusal.view()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// What a post to a party's page asks, done: delete, set or remove a tag.
async fn party_act(
    kind: PartyKind,
    uid: &str,
    name: &str,
) -> (Option<Refused>, Option<(ViewerError, TagAct)>) {
    let Some(posted) = crate::posted::posted() else {
        return (None, None);
    };
    if posted.intent() == "delete-party" {
        return match delete_party(kind.segment().to_owned(), posted.text("version")).await {
            Ok(()) => {
                redirect_with(
                    &party_href(kind, uid),
                    &Done::ok(tr(RecordsMsg::PartyDeleted)).line(name.to_owned()),
                );
                (None, None)
            }
            Err(error) => (
                Some(Refused::new(tr(RecordsMsg::PartyDeleteRefused), error)),
                None,
            ),
        };
    }
    let Some(act) = tag_act() else {
        return (None, None);
    };
    let segment = kind.segment().to_owned();
    let done = match &act {
        TagAct::Set { key, value, path } => {
            set_party_tag(
                segment,
                uid.to_owned(),
                key.clone(),
                value.clone(),
                path.clone(),
            )
            .await
        }
        TagAct::Remove(key) => delete_party_tag(segment, uid.to_owned(), key.clone()).await,
    };
    match done {
        Ok(()) => {
            redirect_with(
                &party_tab_href(kind, uid, PartyTab::Tags, None),
                &tag_done(&act, name),
            );
            (None, None)
        }
        Err(error) => (None, Some((error, act))),
    }
}

/// What a party's tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum PartyBody {
    /// Its facts and identities, rendered.
    Overview(Vec<RenderedSection>),
    /// Its document, pretty-printed.
    Document(String),
    /// The relationships it is the source of.
    Relationships,
    /// Its tags.
    Tags(TagPanel),
    /// Its versions: the version in view, its envelope and its document
    /// when an older one is read.
    Versions {
        /// The id of the version in view.
        in_view: String,
        /// Its envelope, or the CDR's refusal of it.
        envelope: Result<VersionedObjectFacts, Refused>,
        /// Its document pretty-printed, when an older one is read.
        document: Option<Result<String, Refused>>,
    },
}

/// A party the CDR holds, as its page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Party {
    /// Its kind.
    kind: PartyKind,
    /// The name it goes by.
    name: String,
    /// Its current version, `None` when it is deleted.
    current: Option<PartyState>,
    /// Its versions, newest first, or the CDR's refusal of them.
    versions: Result<Vec<VersionEntry>, Refused>,
    /// The id of its latest version.
    latest: String,
    /// The version that deleted it, when it is deleted.
    deleted: Option<VersionEntry>,
    /// The number of the last version before its deletion.
    before_deletion: Option<String>,
    /// The number of the older version read.
    reading_number: Option<String>,
    /// The delete the CDR refused.
    acted: Option<Refused>,
    /// The tab shown.
    tab: PartyTab,
    /// What it shows.
    body: PartyBody,
}

/// One party's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum PartyModel {
    /// The party.
    Shown(Box<Party>),
    /// The CDR's refusal of it.
    Missing(Refused),
    /// The address names no kind of party.
    NoKind,
}

/// Loads the party `uid` of `kind`: answers its post, reads its versions,
/// the version asked for and what the tab shows; a party the CDR does not
/// hold, or a kind it has not, answers `404`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_party(
    /// The path and query of the page.
    href: String,
    /// The kind of party the route names, as its segment.
    kind: String,
    /// The party the route names.
    uid: String,
) -> Result<PartyModel, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_party(&kind, uid).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_party`] reads, on the server.
#[expect(
    clippy::too_many_lines,
    reason = "one loader: the versions, the version in view, the post and the tab's read"
)]
async fn read_party(kind: &str, uid: String) -> PartyModel {
    let kind = PartyKind::from_segment(kind);
    let Some(kind) = kind else {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
        return PartyModel::NoKind;
    };
    let asked = crate::request::query("version");
    let at = crate::request::query("at");
    let current = match fetch_party(kind.segment().to_owned(), uid.clone()).await {
        Ok(current) => current,
        Err(error) => {
            if is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            return PartyModel::Missing(Refused::new(tr(RecordsMsg::PartyRefused), error));
        }
    };
    let versions = history_of(VersionedFamily::Party, &uid).await;
    let latest_entry = versions
        .as_ref()
        .ok()
        .and_then(|versions| versions.first().cloned());
    let latest = current
        .as_ref()
        .map(|state| state.version_uid.clone())
        .or_else(|| latest_entry.as_ref().map(|entry| entry.version_id.clone()))
        .unwrap_or_default();
    let deleted = current.is_none().then(|| latest_entry.clone()).flatten();
    let latest_number = version_number(&latest);
    let before_deletion = deleted.as_ref().and_then(|entry| {
        version_number(&entry.version_id)
            .parse::<u32>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .map(|number| number.to_string())
    });
    let reading_number = asked
        .as_ref()
        .filter(|number| at.is_none() && **number != latest_number)
        .cloned();
    let shown_uid: Result<Option<String>, ViewerError> = match (&at, &reading_number) {
        (Some(at), _) => resolve_demographic_version_at_time(
            VersionedFamily::Party.segment().to_owned(),
            uid.clone(),
            at.clone(),
        )
        .await
        .map(Some),
        (None, Some(number)) => Ok(Some(version_uid_at(&latest, number))),
        _ => Ok(None),
    };
    let shown_body = match &shown_uid {
        Ok(Some(version)) => Some(
            fetch_demographic_version_document(kind.segment().to_owned(), version.clone()).await,
        ),
        _ => None,
    };
    let name = current
        .as_ref()
        .map_or_else(|| shortened(&uid), display_name);
    let (acted, tag_refusal) = party_act(kind, &uid, &name).await;
    let tab = if deleted.is_some() || shown_body.is_some() {
        PartyTab::Versions
    } else {
        PartyTab::asked()
    };
    let body = match (tab, &current) {
        (PartyTab::Overview, Some(state)) => PartyBody::Overview(rendered_identities(state)),
        (PartyTab::Document, Some(state)) => PartyBody::Document(crate::bff::pretty_body(
            &state.body,
            crate::format::ReprFormat::CanonicalJson,
        )),
        (PartyTab::Relationships, Some(_)) => PartyBody::Relationships,
        (PartyTab::Tags, Some(_)) => PartyBody::Tags(TagPanel {
            collection: tr(RecordsMsg::TagCollectionParty),
            tags: fetch_party_tags(kind.segment().to_owned(), uid.clone()).await,
            action: party_tab_href(kind, &uid, PartyTab::Tags, None),
            refused: tag_refusal,
        }),
        _ => {
            let in_view = match &shown_uid {
                Ok(Some(version)) => version.clone(),
                _ => before_deletion
                    .as_deref()
                    .map_or_else(|| latest.clone(), |number| version_uid_at(&latest, number)),
            };
            let envelope = fetch_versioned_object(
                VersionedFamily::Party.segment().to_owned(),
                uid.clone(),
                in_view.clone(),
            )
            .await
            .map_err(|error| Refused::new(tr(RecordsMsg::CompositionVersionRefused), error));
            let document = shown_body.map(|body| {
                body.map(|body| {
                    crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson)
                })
                .map_err(|error| Refused::new(tr(RecordsMsg::CompositionVersionRefused), error))
            });
            PartyBody::Versions {
                in_view,
                envelope,
                document,
            }
        }
    };
    PartyModel::Shown(Box::new(Party {
        kind,
        name,
        current,
        versions: versions
            .map_err(|error| Refused::new(tr(RecordsMsg::CompositionHistoryRefused), error)),
        latest,
        deleted,
        before_deletion,
        reading_number,
        acted,
        tab,
        body,
    }))
}

/// The page of a party the CDR cannot read.
fn party_missing(uid: &str, refusal: &Refused) -> AnyView {
    let heading = page_heading(
        Header {
            title: tr_with(RecordsMsg::PartyTitle, &[("id", shortened(uid).into())]).into(),
            subtitle: None,
            crumbs: party_crumbs(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr(RecordsMsg::PartyRefused) />
        {heading}
        {refusal.view()}
    }
    .into_any()
}

/// The page of an address that names no kind of party.
fn no_kind() -> AnyView {
    crate::pages::errors::not_found_page()
}

/// The tabs of a party: its versions alone when it is deleted.
fn party_tabs(kind: PartyKind, uid: &str, party: &Party, total: usize) -> Vec<Tab> {
    if party.deleted.is_some() {
        return vec![Tab::new(
            PartyTab::Versions.key(),
            party_tab_href(kind, uid, PartyTab::Versions, None),
            PartyTab::Versions.label(),
        )];
    }
    PartyTab::ALL
        .into_iter()
        .map(|each| {
            let tab = Tab::new(
                each.key(),
                party_tab_href(kind, uid, each, None),
                each.label(),
            );
            match each {
                PartyTab::Versions => {
                    tab.count(u64::try_from(total).unwrap_or(u64::MAX), each.label())
                }
                PartyTab::Relationships => tab.count(
                    party.current.as_ref().map_or(0, |state| {
                        u64::try_from(state.relationships.len()).unwrap_or(u64::MAX)
                    }),
                    each.label(),
                ),
                _ => tab,
            }
        })
        .collect()
}

/// A party's page: its header, the tab shown and Delete party's
/// confirmation.
#[expect(
    clippy::too_many_lines,
    reason = "one page: its header, its tabs, the tab shown and its dialog"
)]
fn party_view(uid: &str, party: Party) -> AnyView {
    let kind = party.kind;
    let total = party.versions.as_ref().map_or(0, Vec::len);
    let tabs = party_tabs(kind, uid, &party, total);
    let Party {
        name,
        current,
        versions,
        latest,
        deleted,
        before_deletion,
        reading_number,
        acted,
        tab,
        body,
        ..
    } = party;
    let reading = reading_number.as_deref();
    let older = reading.map(|number| {
        plain_section(older_version(
            number,
            u32::try_from(total).unwrap_or(u32::MAX),
            party_tab_href(kind, uid, PartyTab::Versions, None),
        ))
    });
    let body = match (body, &current) {
        (PartyBody::Overview(rendered), Some(state)) => overview(kind, uid, state, &rendered),
        (PartyBody::Document(pretty), Some(state)) => panel(document_section(
            tr_with(
                RecordsMsg::PartyDocumentTitle,
                &[
                    ("type", state.rm_type.clone().into()),
                    ("version", version_number(&state.version_uid).into()),
                ],
            ),
            Language::Json,
            pretty,
            "party-document",
        )),
        (PartyBody::Relationships, Some(state)) => relationships(kind, uid, state),
        (PartyBody::Tags(tagged), Some(_)) => panel(tag_panel(tagged)),
        (
            PartyBody::Versions {
                in_view,
                envelope: facts,
                document,
            },
            _,
        ) => {
            let history = match &versions {
                Ok(versions) => demographic_history(
                    versions,
                    reading,
                    deleted.is_some(),
                    &|version| party_tab_href(kind, uid, PartyTab::Versions, version),
                    party_href(kind, uid),
                ),
                Err(refusal) => refusal.view(),
            };
            let document = match document {
                Some(Ok(body)) => document_section(
                    tr_with(
                        RecordsMsg::PartyVersionDocument,
                        &[("version", version_number(&in_view).into())],
                    ),
                    Language::Json,
                    body,
                    "party-version",
                ),
                Some(Err(refusal)) => plain_section(refusal.view()),
                None => ().into_any(),
            };
            let intro = deleted.is_some().then(|| {
                ferrohealth_ui::view::surface::section_intro(tr(RecordsMsg::PartyDeletedIntro))
            });
            panel(view! {
                {older}
                {section(
                    Some(tr(Msg::CompositionVersions)),
                    u64::try_from(total).ok(),
                    None,
                    view! {
                        {intro}
                        {history}
                    },
                )}
                {envelope(facts)}
                {document}
            })
        }
        _ => ().into_any(),
    };
    let header = party_header(
        kind,
        uid,
        &name,
        &latest,
        deleted.as_ref(),
        before_deletion.as_deref(),
    );
    let dialog = current.as_ref().map(|state| {
        danger_dialog(
            Confirmation {
                id: DELETE_DIALOG.into(),
                title: tr_with(
                    RecordsMsg::PartyDeleteTitle,
                    &[("name", name.clone().into())],
                )
                .into(),
                message: tr_with(
                    RecordsMsg::PartyDeleteMessage,
                    &[
                        ("count", count(total)),
                        ("relationships", count(state.relationships.len())),
                    ],
                )
                .into(),
            },
            view! {
                <form method="post" action=party_href(kind, uid)>
                    <input type="hidden" name="intent" value="delete-party" />
                    <input type="hidden" name="version" value=state.version_uid.clone() />
                    <button type="submit" class="fui-button fui-button-danger">
                        {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                        {tr(RecordsMsg::PartyDeleteConfirm)}
                    </button>
                </form>
            },
        )
        .into_any()
    });
    view! {
        <Title text=name.clone() />
        {header}
        <div class="fui-stack">
            {acted.map(|refusal| refusal.view())}
            {binder(tr(RecordsMsg::PartyViews), tabs, tab.key(), body)}
        </div>
        {dialog}
    }
    .into_any()
}

/// One party.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn PartyPage() -> impl IntoView {
    let uid = uid_param();
    let (kind_segment, uid_segment) = party_route();
    let model = page_model!(load_party(kind_segment, uid_segment));
    let page = drawn(model, move |model| match model {
        PartyModel::Shown(party) => party_view(&uid, *party),
        PartyModel::Missing(refusal) => party_missing(&uid, &refusal),
        PartyModel::NoKind => no_kind(),
    });
    view! { {page} }
}

/// The More menu's Copy the id of the party `uid`.
fn copy_party_id(uid: &str) -> Item {
    Item::new(Action::new(
        Some(Glyph::COPY),
        tr(RecordsMsg::PartyCopyId),
        Act::Copy(uid.to_owned()),
    ))
}

/// The header of a party: its name, kind and version line, and Edit with the
/// More menu; a deleted party says when it was deleted.
fn party_header(
    kind: PartyKind,
    uid: &str,
    name: &str,
    latest: &str,
    deleted: Option<&VersionEntry>,
    last: Option<&str>,
) -> AnyView {
    let (actions, subtitle, badge) = if let Some(deletion) = deleted {
        (
            ActionGroup::primary(Action::new(
                Some(Glyph::ROTATE_CCW_CLOCK),
                tr_with(
                    RecordsMsg::CompositionReadVersion,
                    &[("version", last.unwrap_or_default().to_owned().into())],
                ),
                Act::Link(party_tab_href(kind, uid, PartyTab::Versions, last)),
            ))
            .more(Menu::new(
                "party-more",
                tr(RecordsMsg::PartyMore),
                vec![copy_party_id(uid)],
            )),
            tr_with(
                RecordsMsg::PartyDeletedLine,
                &[
                    ("kind", kind_words(kind).into()),
                    (
                        "when",
                        ferrohealth_ui::view::time::reading(&deletion.committed).into(),
                    ),
                    ("who", deletion.committer.clone().into()),
                    ("version", version_number(&deletion.version_id).into()),
                ],
            ),
            Some(state_mark(
                FlowState::Off,
                &tr(RecordsMsg::CompositionDeletedMark),
            )),
        )
    } else {
        let more = Menu::new(
            "party-more",
            tr(RecordsMsg::PartyMore),
            vec![
                Item::new(Action::new(
                    Some(Glyph::LINK),
                    tr(RecordsMsg::PartyRelate),
                    Act::Link(format!(
                        "/parties/relationships/new?source_kind={}&source={}",
                        kind.segment(),
                        urlencoding::encode(uid)
                    )),
                )),
                copy_party_id(uid),
            ],
        )
        .destructive(vec![Item::new(Action::new(
            Some(Glyph::TRASH),
            tr(RecordsMsg::PartyDelete),
            Act::Dialog(DELETE_DIALOG.to_owned()),
        ))]);
        (
            ActionGroup::primary(Action::new(
                Some(Glyph::PENCIL),
                tr(Msg::PartyEdit),
                Act::Link(format!("{}/edit", party_href(kind, uid))),
            ))
            .more(more),
            tr_with(
                RecordsMsg::PartyLine,
                &[
                    ("kind", kind_words(kind).into()),
                    ("version", version_number(latest).into()),
                    ("id", shortened(uid).into()),
                ],
            ),
            None,
        )
    };
    page_heading(
        Header {
            title: name.to_owned().into(),
            subtitle: Some(subtitle.into()),
            crumbs: party_crumbs(),
            mono: false,
            badge,
            subtitle_view: None,
        },
        actions,
    )
    .into_any()
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's demographic JSON wire for a party's identities and name"
)]
/// The `identities` and the `details` of an edited party document, as the
/// update takes them; a document that is not a JSON object is passed on
/// whole for the update to refuse.
fn parts_of(text: &str) -> (String, String) {
    use serde_json::Value;
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(object)) => (
            object
                .get("identities")
                .map_or_else(String::new, Value::to_string),
            object
                .get("details")
                .map_or_else(String::new, Value::to_string),
        ),
        _ => (text.to_owned(), String::new()),
    }
}

/// A save the CDR refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Unsaved {
    /// The refusal.
    refusal: Refused,
    /// Whether another client changed the object meanwhile (`412`).
    conflict: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Unsaved {
    /// The refusal `error` of what `summary` says.
    fn new(summary: String, error: ViewerError) -> Self {
        Self {
            conflict: is_status(&error, http::StatusCode::PRECONDITION_FAILED),
            refusal: Refused::new(summary, error),
        }
    }
}

/// What Edit a party's form holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PartyEditor {
    /// The party's kind.
    kind: PartyKind,
    /// The current version's document, pretty-printed.
    current: String,
    /// The id of the current version.
    current_uid: String,
    /// The id of the version the edit is based on.
    opened: String,
    /// The document as edited.
    text: String,
    /// The name the party goes by.
    name: String,
    /// The save the CDR refused, when the post was one.
    unsaved: Option<Unsaved>,
}

/// An editing page's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Editing<T> {
    /// The form.
    Editor(Box<T>),
    /// The object is deleted, so this note says it cannot be edited.
    Deleted(String),
    /// The CDR's refusal.
    Refused(Refused),
    /// The address names no kind of party.
    NoKind,
    /// The post redirected.
    Left,
}

/// Loads Edit a party: answers its post (Discard, Save) over the current
/// version.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_edit_party(
    /// The path and query of the page.
    href: String,
    /// The kind of party the route names, as its segment.
    kind: String,
    /// The party the route names.
    uid: String,
) -> Result<Editing<PartyEditor>, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_edit_party(&kind, uid).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_edit_party`] reads, on the server.
async fn read_edit_party(kind: &str, uid: String) -> Editing<PartyEditor> {
    let kind = PartyKind::from_segment(kind);
    let posted = crate::posted::posted();
    let Some(kind) = kind else {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
        return Editing::NoKind;
    };
    if discarded(posted.as_ref()) {
        leptos_axum::redirect(&party_href(kind, &uid));
        return Editing::Left;
    }
    let state = match fetch_party(kind.segment().to_owned(), uid.clone()).await {
        Ok(Some(state)) => state,
        Ok(None) => return Editing::Deleted(tr(RecordsMsg::PartyDeletedNoEdit)),
        Err(error) => return Editing::Refused(Refused::new(tr(RecordsMsg::PartyRefused), error)),
    };
    let current = crate::bff::pretty_body(&state.body, crate::format::ReprFormat::CanonicalJson);
    let opened = posted
        .as_ref()
        .map(|posted| posted.text("version"))
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| state.version_uid.clone());
    let text = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Document.name()))
        .unwrap_or_else(|| current.clone());
    let name = display_name(&state);
    let mut unsaved = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        let (identities, details) = parts_of(&text);
        match update_party(
            kind.segment().to_owned(),
            uid.clone(),
            opened.clone(),
            text.clone(),
            identities,
            details,
        )
        .await
        {
            Ok(saved) => {
                redirect_with(
                    &party_href(kind, &uid),
                    &Done::ok(tr(RecordsMsg::PartySaved)).line(tr_with(
                        RecordsMsg::CompositionSavedVersion,
                        &[("version", version_number(&saved).into())],
                    )),
                );
                return Editing::Left;
            }
            Err(error) => unsaved = Some(Unsaved::new(tr(RecordsMsg::PartySaveRefused), error)),
        }
    }
    Editing::Editor(Box::new(PartyEditor {
        kind,
        current,
        current_uid: state.version_uid,
        opened,
        text,
        name,
        unsaved,
    }))
}

/// Edit a party's page, its decision bar docked.
fn edit_party_view(uid: &str, editor: PartyEditor) -> AnyView {
    let PartyEditor {
        kind,
        current,
        current_uid,
        opened,
        text,
        name,
        unsaved,
    } = editor;
    let field = EditorField::Document.name();
    let findings = unsaved
        .as_ref()
        .and_then(|unsaved| findings_of(&unsaved.refusal.error, &text, &field));
    let conflict = unsaved.as_ref().is_some_and(|unsaved| unsaved.conflict);
    let line = if unsaved.is_some() {
        (Glyph::CIRCLE_X, tr(RecordsMsg::EditorNotSaved))
    } else {
        (Glyph::PENCIL, tr(RecordsMsg::EditorOpened))
    };
    let top = unsaved.map(|unsaved| {
        let reload = unsaved.conflict.then(|| {
            view! {
                <div class="fui-cluster">
                    {ferrohealth_ui::view::button::link_button(
                        format!("{}/edit", party_href(kind, uid)),
                        ferrohealth_ui::view::button::Variant::Secondary,
                        Some(Glyph::REFRESH_CW),
                        tr_with(
                            RecordsMsg::EditorReloadVersion,
                            &[("version", version_number(&current_uid).into())],
                        ),
                    )}
                    <span class="fui-small fui-muted">{tr(RecordsMsg::EditorReloadReplaces)}</span>
                </div>
            }
        });
        view! {
            {unsaved.refusal.view()}
            {reload}
        }
        .into_any()
    });
    let save = version_number(&opened).parse::<u32>().map_or_else(
        |_| tr(Msg::EditorSave),
        |number| {
            tr_with(
                RecordsMsg::EditorSaveAsVersion,
                &[("version", number.saturating_add(1).into())],
            )
        },
    );
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save,
        blocked: conflict.then(|| tr(RecordsMsg::EditorReloadFirst)),
    });
    let mut trail = party_crumbs();
    trail.push(Crumb::new(name.clone(), party_href(kind, uid)));
    let heading = editor_heading(
        message(RecordsMsg::EditorTitle),
        &Mode::Editing(name.clone()),
        Header {
            title: String::new().into(),
            subtitle: Some(
                tr_with(
                    RecordsMsg::CompositionEditSubtitle,
                    &[("version", version_number(&opened).into())],
                )
                .into(),
            ),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr_with(Msg::CompositionEditTitle, &[("name", name.into())]) />
        {heading}
        <div class="fui-stack">
            {top} <form id=EDIT_FORM method="post" action=format!("{}/edit", party_href(kind, uid))>
                <input type="hidden" name="version" value=opened />
                {card_panel(
                    section(
                        Some(tr(RecordsMsg::CompositionDocument)),
                        None,
                        None,
                        view! {
                            {ferrohealth_ui::view::surface::section_intro(
                                tr(RecordsMsg::PartyEditIntro),
                            )}
                            {party_editor(
                                text,
                                Some(current),
                                findings,
                                tr(RecordsMsg::PartyEditorLabel),
                            )}
                        },
                    ),
                )}
            </form>
        </div>
    }
    .into_any()
}

/// The page an editing model draws when it holds no form.
fn not_editing<T>(model: Editing<T>, draw: impl FnOnce(T) -> AnyView) -> AnyView {
    match model {
        Editing::Editor(editor) => draw(*editor),
        Editing::Deleted(note) => info_note(note),
        Editing::Refused(refusal) => refusal.view(),
        Editing::NoKind => no_kind(),
        Editing::Left => ().into_any(),
    }
}

/// Edit a party.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn EditPartyPage() -> impl IntoView {
    let uid = uid_param();
    let dock_promise = crate::shell::promise_dock();
    let (kind_segment, uid_segment) = party_route();
    let model = page_model!(load_edit_party(kind_segment, uid_segment));
    let page = docked(model, dock_promise, move |model| {
        not_editing(model, |editor| edit_party_view(&uid, editor))
    });
    view! { {page} }
}

/// The facts of a relationship's two ends.
fn ends(relationship: &RelationshipState) -> Vec<(String, AnyView)> {
    let end = |party: &PartyRefView| {
        let kind = PartyKind::from_rm_type(&party.rm_type)
            .map_or_else(|| party.rm_type.clone(), kind_words);
        let name = shortened(&party.id);
        match ref_href(party) {
            Some(href) => view! {
                <a href=href>{name}</a>
                {format!(" · {kind}")}
            }
            .into_any(),
            None => view! {
                {name}
                {format!(" · {kind}")}
            }
            .into_any(),
        }
    };
    vec![
        (
            tr(RecordsMsg::RelationshipSource),
            end(&relationship.source),
        ),
        (
            tr(RecordsMsg::RelationshipTarget),
            end(&relationship.target),
        ),
    ]
}

/// One relationship's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum RelationshipModel {
    /// The relationship and its versions.
    Shown(Box<RelationshipState>, Result<Vec<VersionEntry>, Refused>),
    /// The relationship is deleted.
    Deleted,
    /// The CDR's refusal of it or of its delete.
    Refused(Refused),
    /// The delete redirected.
    Left,
}

/// Loads the relationship `uid`: answers its delete and reads its versions;
/// one the CDR does not hold answers `404`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_relationship(
    /// The path and query of the page.
    href: String,
    /// The id the route names.
    uid: String,
) -> Result<RelationshipModel, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_relationship(uid).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_relationship`] reads, on the server.
async fn read_relationship(uid: String) -> RelationshipModel {
    let state = match fetch_relationship(uid.clone()).await {
        Ok(Some(state)) => state,
        Ok(None) => return RelationshipModel::Deleted,
        Err(error) => {
            if is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            return RelationshipModel::Refused(Refused::new(
                tr(RecordsMsg::RelationshipRefused),
                error,
            ));
        }
    };
    if let Some(posted) = crate::posted::posted_for("delete-relationship") {
        return match delete_relationship(posted.text("version")).await {
            Ok(()) => {
                redirect_with(
                    ref_href(&state.source).as_deref().unwrap_or("/parties"),
                    &Done::ok(tr(RecordsMsg::RelationshipDeletedToast)).line(state.name.clone()),
                );
                RelationshipModel::Left
            }
            Err(error) => RelationshipModel::Refused(Refused::new(
                tr(RecordsMsg::RelationshipDeleteRefused),
                error,
            )),
        };
    }
    let versions = history_of(VersionedFamily::PartyRelationship, &uid)
        .await
        .map_err(|error| Refused::new(tr(RecordsMsg::CompositionHistoryRefused), error));
    RelationshipModel::Shown(Box::new(state), versions)
}

/// One relationship's page.
#[expect(
    clippy::too_many_lines,
    reason = "one page: its facts, its versions, its header and its dialog"
)]
fn relationship_view(
    uid: &str,
    state: &RelationshipState,
    versions: &Result<Vec<VersionEntry>, Refused>,
) -> AnyView {
    let source_href = ref_href(&state.source);
    let total = versions.as_ref().map_or(0, Vec::len);
    let history = match versions {
        Ok(versions) => demographic_history(
            versions,
            None,
            false,
            &|_| relationship_href(uid),
            relationship_href(uid),
        ),
        Err(refusal) => refusal.view(),
    };
    let mut facts = ends(state);
    facts.push((
        tr(RecordsMsg::RelationshipType),
        view! {
            {state.name.clone()}
            " "
            <code class="fui-small">{state.archetype_node_id.clone()}</code>
        }
        .into_any(),
    ));
    facts.push((
        tr(RecordsMsg::RelationshipDetails),
        if state.details.is_empty() {
            tr(Msg::RelationshipNone).into_any()
        } else {
            view! { <code>{state.details.clone()}</code> }.into_any()
        },
    ));
    facts.push((
        tr(RecordsMsg::RelationshipValidity),
        if state.time_validity.is_empty() {
            tr(Msg::RelationshipNone).into_any()
        } else {
            view! { <code>{state.time_validity.clone()}</code> }.into_any()
        },
    ));
    let mut trail = party_crumbs();
    if let Some(href) = source_href {
        trail.push(Crumb::new(shortened(&state.source.id), href));
    }
    let more = Menu::new(
        "relationship-more",
        tr(RecordsMsg::RelationshipMore),
        vec![Item::new(Action::new(
            Some(Glyph::COPY),
            tr(RecordsMsg::RelationshipCopyId),
            Act::Copy(uid.to_owned()),
        ))],
    )
    .destructive(vec![Item::new(Action::new(
        Some(Glyph::TRASH),
        tr(RecordsMsg::RelationshipDelete),
        Act::Dialog(DELETE_RELATIONSHIP.to_owned()),
    ))]);
    let heading = page_heading(
        Header {
            title: state.name.clone().into(),
            subtitle: Some(
                tr_with(
                    RecordsMsg::RelationshipLine,
                    &[
                        ("version", version_number(&state.version_uid).into()),
                        ("id", shortened(uid).into()),
                    ],
                )
                .into(),
            ),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PENCIL),
            tr(Msg::PartyEdit),
            Act::Link(format!("{}/edit", relationship_href(uid))),
        ))
        .more(more),
    );
    let dialog = danger_dialog(
        Confirmation {
            id: DELETE_RELATIONSHIP.into(),
            title: tr_with(
                RecordsMsg::RelationshipDeleteTitle,
                &[("name", state.name.clone().into())],
            )
            .into(),
            message: tr(RecordsMsg::RelationshipDeleteMessage).into(),
        },
        view! {
            <form method="post" action=relationship_href(uid)>
                <input type="hidden" name="intent" value="delete-relationship" />
                <input type="hidden" name="version" value=state.version_uid.clone() />
                <button type="submit" class="fui-button fui-button-danger">
                    {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                    {tr(RecordsMsg::RelationshipDeleteConfirm)}
                </button>
            </form>
        },
    );
    view! {
        <Title text=state.name.clone() />
        {heading}
        <div class="fui-stack">
            {card_panel(
                view! {
                    {section(Some(tr(Msg::PartyFacts)), None, None, definitions(facts))}
                    {section(
                        Some(tr(Msg::CompositionVersions)),
                        u64::try_from(total).ok(),
                        None,
                        history,
                    )}
                },
            )}
        </div>
        {dialog}
    }
    .into_any()
}

/// One relationship.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn RelationshipPage() -> impl IntoView {
    let uid = uid_param();
    let model = page_model!(load_relationship(uid.clone()));
    let page = drawn(model, move |model| match model {
        RelationshipModel::Shown(state, versions) => relationship_view(&uid, &state, &versions),
        RelationshipModel::Deleted => info_note(tr(RecordsMsg::RelationshipDeleted)),
        RelationshipModel::Refused(refusal) => refusal.view(),
        RelationshipModel::Left => ().into_any(),
    });
    view! { {page} }
}

/// The details editor of a relationship.
fn details_editor(text: String, opened: Option<String>) -> AnyView {
    view! {
        <CodeEditor
            setup=EditorSetup {
                field: EditorField::Details,
                language: EditorLanguage::Json,
                text,
                opened,
                marks: Vec::new(),
                label: tr(RecordsMsg::RelationshipDetails),
                lead: tr(RecordsMsg::EditorJson),
                facts: vec![tr(Msg::EditorUtf8)],
                findings: None,
            }
            words=EditorWords::of_page()
            diagnostics=DiagnosticsWords::of_page()
        />
    }
    .into_any()
}

/// What a save of a relationship did not do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Unwritten {
    /// A field the save needs is empty: the line says which.
    Missing(String),
    /// The CDR refused it, another client's change meanwhile (`412`) among
    /// the reasons.
    Refused(Unsaved),
}

impl Unwritten {
    /// What it says, drawn.
    fn view(&self) -> AnyView {
        match self {
            Self::Missing(line) => crate::pages::common::quiet(line.clone()),
            Self::Refused(unsaved) => unsaved.refusal.view(),
        }
    }

    /// Whether another client changed the relationship meanwhile.
    const fn conflict(&self) -> bool {
        matches!(self, Self::Refused(Unsaved { conflict: true, .. }))
    }
}

/// What New relationship's form holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NewRelationship {
    /// The source party's id.
    source: String,
    /// Its kind.
    source_kind: PartyKind,
    /// The name the source goes by.
    source_name: String,
    /// The relationship's type.
    kind_type: String,
    /// The target party's id.
    target: String,
    /// Its kind.
    target_kind: PartyKind,
    /// The details document.
    details: String,
    /// What the save did not do, when the post was one.
    unwritten: Option<Unwritten>,
}

/// Loads New relationship: answers its post (Discard, Create) and names the
/// source party.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_new_relationship(
    /// The path and query of the page.
    href: String,
) -> Result<Creating<NewRelationship>, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_new_relationship().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_new_relationship`] reads, on the server.
async fn read_new_relationship() -> Creating<NewRelationship> {
    let posted = crate::posted::posted();
    let field = |name: &str| {
        posted
            .as_ref()
            .map(|posted| posted.text(name))
            .or_else(|| crate::request::query(name))
            .unwrap_or_default()
    };
    let source = field("source");
    let source_kind = PartyKind::from_segment(&field("source_kind")).unwrap_or(PartyKind::Person);
    let kind_type = field("type");
    let target = field("target");
    let target_kind =
        PartyKind::from_segment(&field("target_kind")).unwrap_or(PartyKind::Organisation);
    let details = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Details.name()))
        .unwrap_or_default();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect(&party_tab_href(
            source_kind,
            &source,
            PartyTab::Relationships,
            None,
        ));
        return Creating::Left;
    }
    let saving = posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save");
    let mut unwritten = None;
    // NOTE: the fields are not `required` in the markup: Discard posts the same form, and the
    // browser would refuse it too; an empty type or target is answered here instead.
    let missing = kind_type.trim().is_empty() || target.trim().is_empty();
    if saving && missing {
        unwritten = Some(Unwritten::Missing(tr(
            RecordsMsg::RelationshipNeedsTypeAndTarget,
        )));
    } else if saving {
        let node = format!(
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.{}.v1",
            kind_type.trim()
        );
        match create_relationship(
            node,
            kind_type.clone(),
            source.clone(),
            source_kind.segment().to_owned(),
            target.clone(),
            target_kind.segment().to_owned(),
            details.clone(),
        )
        .await
        {
            Ok(uid) => {
                redirect_with(
                    &relationship_href(&uid),
                    &Done::ok(tr(RecordsMsg::RelationshipCreated)).line(kind_type.clone()),
                );
                return Creating::Left;
            }
            Err(error) => {
                unwritten = Some(Unwritten::Refused(Unsaved {
                    refusal: Refused::new(tr(RecordsMsg::RelationshipCreateRefused), error),
                    conflict: false,
                }));
            }
        }
    }
    let source_name = match fetch_party(source_kind.segment().to_owned(), source.clone()).await {
        Ok(Some(state)) => display_name(&state),
        _ => shortened(&source),
    };
    Creating::Form(Box::new(NewRelationship {
        source,
        source_kind,
        source_name,
        kind_type,
        target,
        target_kind,
        details,
        unwritten,
    }))
}

/// New relationship's page, its decision bar docked.
#[expect(
    clippy::too_many_lines,
    reason = "one form: its heading, its refusal, the source, the type and target, the details and the dock"
)]
fn new_relationship_view(model: Creating<NewRelationship>) -> AnyView {
    let Creating::Form(form) = model else {
        return ().into_any();
    };
    let NewRelationship {
        source,
        source_kind,
        source_name,
        kind_type,
        target,
        target_kind,
        details,
        unwritten,
    } = *form;
    let back = party_tab_href(source_kind, &source, PartyTab::Relationships, None);
    let line = if unwritten.is_some() {
        (Glyph::CIRCLE_X, tr(RecordsMsg::PartyNotCreated))
    } else {
        (Glyph::FILE_PLUS, tr(RecordsMsg::PartyNotYet))
    };
    let top = unwritten.map(|unwritten| unwritten.view());
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: tr(RecordsMsg::RelationshipCreate),
        blocked: None,
    });
    let mut trail = party_crumbs();
    trail.push(Crumb::new(source_name.clone(), back));
    let heading = editor_heading(
        message(RecordsMsg::RelationshipNewTitle),
        &Mode::Creating,
        Header {
            title: String::new().into(),
            subtitle: Some(tr(RecordsMsg::RelationshipNewSubtitle).into()),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let from = definitions(vec![(
        tr(RecordsMsg::RelationshipSource),
        view! {
            <a href=party_href(source_kind, &source)>{source_name}</a>
            {format!(" · {}", kind_words(source_kind))}
        }
        .into_any(),
    )]);
    let grid = view! {
        <input type="hidden" name="source" value=source.clone() />
        <input type="hidden" name="source_kind" value=source_kind.segment() />
        <div class="fui-form-grid">
            <div class="fui-span-2">
                {Field::new("r-type", tr(RecordsMsg::RelationshipType))
                    .help(tr(RecordsMsg::RelationshipTypeHelp))
                    .view(
                        view! {
                            <input
                                id="r-type"
                                name="type"
                                type="text"
                                class="fui-input fui-mono"
                                required
                                value=kind_type.clone()
                            />
                        },
                    )}
            </div>
            <div class="fui-span-2">
                {Field::new("r-tk", tr(RecordsMsg::RelationshipTargetKind))
                    .view(
                        view! {
                            <select id="r-tk" name="target_kind" class="fui-select">
                                {kind_options(target_kind)}
                            </select>
                        },
                    )}
            </div>
            <div class="fui-span-2">
                {Field::new("r-tid", tr(RecordsMsg::RelationshipTargetUid))
                    .holds_id()
                    .view(
                        view! {
                            <input
                                id="r-tid"
                                name="target"
                                type="text"
                                class="fui-input fui-mono"
                                spellcheck="false"
                                required
                                value=target.clone()
                            />
                        },
                    )}
            </div>
        </div>
    };
    view! {
        <Title text=tr(RecordsMsg::RelationshipNewPageTitle) />
        {heading}
        <div class="fui-stack">
            {top} <form id=EDIT_FORM method="post" action="/parties/relationships/new">
                {card_panel(
                    view! {
                        {section(Some(tr(RecordsMsg::RelationshipFrom)), None, None, from)}
                        {section(Some(tr(RecordsMsg::RelationshipTitle)), None, None, grid)}
                        {section(
                            Some(tr(RecordsMsg::RelationshipDetails)),
                            None,
                            None,
                            view! {
                                {ferrohealth_ui::view::surface::section_intro(
                                    tr(RecordsMsg::RelationshipDetailsIntro),
                                )}
                                {details_editor(details, None)}
                            },
                        )}
                    },
                )}
            </form>
        </div>
    }
    .into_any()
}

/// New relationship, from the party the address names.
#[component]
pub fn NewRelationshipPage() -> impl IntoView {
    let dock_promise = crate::shell::promise_dock();
    let model = page_model!(load_new_relationship());
    let page = docked(model, dock_promise, new_relationship_view);
    view! { {page} }
}

/// What Edit a relationship's form holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RelationshipEditor {
    /// The relationship as the CDR holds it.
    state: RelationshipState,
    /// The id of the version the edit is based on.
    opened: String,
    /// The type as edited.
    name: String,
    /// The details as edited.
    details: String,
    /// What the save did not do, when the post was one.
    unwritten: Option<Unwritten>,
}

/// Loads Edit a relationship: answers its post (Discard, Save) over the
/// current version.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_edit_relationship(
    /// The path and query of the page.
    href: String,
    /// The id the route names.
    uid: String,
) -> Result<Editing<RelationshipEditor>, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_edit_relationship(uid).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_edit_relationship`] reads, on the server.
async fn read_edit_relationship(uid: String) -> Editing<RelationshipEditor> {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect(&relationship_href(&uid));
        return Editing::Left;
    }
    let state = match fetch_relationship(uid.clone()).await {
        Ok(Some(state)) => state,
        Ok(None) => return Editing::Deleted(tr(RecordsMsg::RelationshipDeleted)),
        Err(error) => {
            return Editing::Refused(Refused::new(tr(RecordsMsg::RelationshipRefused), error));
        }
    };
    let opened = posted
        .as_ref()
        .map(|posted| posted.text("version"))
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| state.version_uid.clone());
    let name = posted
        .as_ref()
        .map_or_else(|| state.name.clone(), |posted| posted.text("type"));
    let details = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Details.name()))
        .unwrap_or_else(|| state.details.clone());
    let saving = posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save");
    let mut unwritten = None;
    if saving && name.trim().is_empty() {
        unwritten = Some(Unwritten::Missing(tr(RecordsMsg::RelationshipNeedsType)));
    } else if saving {
        match update_relationship(
            uid.clone(),
            opened.clone(),
            state.body.clone(),
            name.clone(),
            details.clone(),
        )
        .await
        {
            Ok(saved) => {
                redirect_with(
                    &relationship_href(&uid),
                    &Done::ok(tr(RecordsMsg::RelationshipSaved)).line(tr_with(
                        RecordsMsg::CompositionSavedVersion,
                        &[("version", version_number(&saved).into())],
                    )),
                );
                return Editing::Left;
            }
            Err(error) => {
                unwritten = Some(Unwritten::Refused(Unsaved::new(
                    tr(RecordsMsg::RelationshipSaveRefused),
                    error,
                )));
            }
        }
    }
    Editing::Editor(Box::new(RelationshipEditor {
        state,
        opened,
        name,
        details,
        unwritten,
    }))
}

/// Edit a relationship's page, its decision bar docked.
#[expect(
    clippy::too_many_lines,
    reason = "one editor: its heading, its refusal, its ends, its type, its details and the dock"
)]
fn edit_relationship_view(uid: &str, editor: RelationshipEditor) -> AnyView {
    let RelationshipEditor {
        state,
        opened,
        name,
        details,
        unwritten,
    } = editor;
    let changed = name != state.name || details.trim() != state.details.trim();
    let line = if unwritten.is_some() {
        (Glyph::CIRCLE_X, tr(RecordsMsg::EditorNotSaved))
    } else if changed {
        (Glyph::PENCIL, tr(RecordsMsg::DirectoryChanged))
    } else {
        (Glyph::PENCIL, tr(RecordsMsg::EditorOpened))
    };
    let blocked = unwritten
        .as_ref()
        .is_some_and(Unwritten::conflict)
        .then(|| tr(RecordsMsg::EditorReloadFirst));
    let top = unwritten.map(|unwritten| unwritten.view());
    let save = version_number(&opened).parse::<u32>().map_or_else(
        |_| tr(Msg::EditorSave),
        |number| {
            tr_with(
                RecordsMsg::EditorSaveAsVersion,
                &[("version", number.saturating_add(1).into())],
            )
        },
    );
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save,
        blocked,
    });
    let mut trail = party_crumbs();
    trail.push(Crumb::new(state.name.clone(), relationship_href(uid)));
    let heading = editor_heading(
        message(RecordsMsg::EditorTitle),
        &Mode::Editing(state.name.clone()),
        Header {
            title: String::new().into(),
            subtitle: Some(
                tr_with(
                    RecordsMsg::CompositionEditSubtitle,
                    &[("version", version_number(&opened).into())],
                )
                .into(),
            ),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr_with(Msg::CompositionEditTitle, &[("name", state.name.clone().into())]) />
        {heading}
        <div class="fui-stack">
            {top}
            <form id=EDIT_FORM method="post" action=format!("{}/edit", relationship_href(uid))>
                <input type="hidden" name="version" value=opened />
                {card_panel(
                    view! {
                        {section(
                            Some(tr(RecordsMsg::RelationshipEnds)),
                            None,
                            None,
                            view! {
                                {ferrohealth_ui::view::surface::section_intro(
                                    tr(RecordsMsg::RelationshipEndsIntro),
                                )}
                                {definitions(ends(&state))}
                            },
                        )}
                        {section(
                            Some(tr(RecordsMsg::RelationshipTitle)),
                            None,
                            None,
                            view! {
                                <div class="fui-form-grid">
                                    <div class="fui-span-2">
                                        {Field::new("re-ty", tr(RecordsMsg::RelationshipType))
                                            .view(
                                                view! {
                                                    <input
                                                        id="re-ty"
                                                        name="type"
                                                        type="text"
                                                        class="fui-input fui-mono"
                                                        required
                                                        value=name
                                                    />
                                                },
                                            )}
                                    </div>
                                </div>
                            },
                        )}
                        {section(
                            Some(tr(RecordsMsg::RelationshipDetails)),
                            None,
                            None,
                            view! {
                                {ferrohealth_ui::view::surface::section_intro(
                                    tr(RecordsMsg::RelationshipDetailsIntro),
                                )}
                                {details_editor(details, Some(state.details.clone()))}
                            },
                        )}
                    },
                )}
            </form>
        </div>
    }
    .into_any()
}

/// Edit a relationship: its type and its details; its ends are fixed.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn EditRelationshipPage() -> impl IntoView {
    let uid = uid_param();
    let dock_promise = crate::shell::promise_dock();
    let model = page_model!(load_edit_relationship(uid.clone()));
    let page = docked(model, dock_promise, move |model| {
        not_editing(model, |editor| edit_relationship_view(&uid, editor))
    });
    view! { {page} }
}

/// Loads the demographic contribution `uid`; one the CDR does not hold
/// answers `404`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load_party_contribution(
    /// The path and query of the page.
    href: String,
    /// The id the route names.
    uid: String,
) -> Result<Result<ContributionState, Refused>, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_party_contribution(uid).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load_party_contribution`] reads, on the server.
async fn read_party_contribution(uid: String) -> Result<ContributionState, Refused> {
    match fetch_demographic_contribution(uid).await {
        Ok(mut state) => {
            state.body =
                crate::bff::pretty_body(&state.body, crate::format::ReprFormat::CanonicalJson);
            Ok(state)
        }
        Err(error) => {
            if is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            Err(Refused::new(tr(RecordsMsg::ContributionRefused), error))
        }
    }
}

/// A demographic contribution's audit, the versions it made and its record.
fn party_contribution_view(state: &ContributionState) -> AnyView {
    let mut rows = vec![
        (tr(RecordsMsg::FactCommitted), when(&state.time_committed)),
        (
            tr(RecordsMsg::FactCommitter),
            state.committer.clone().into_any(),
        ),
        (
            tr(RecordsMsg::FactChange),
            state.change_type.clone().into_any(),
        ),
    ];
    if !state.description.is_empty() {
        rows.push((
            tr(RecordsMsg::FactDescription),
            state.description.clone().into_any(),
        ));
    }
    let columns = [
        Column::text("object", tr(RecordsMsg::ColumnObject)).role(Role::Title),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Sub),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
    ];
    let made = state
        .versions
        .iter()
        .map(|version| {
            let object = crate::uid::container_uid_of(&version.version_uid);
            let href = PartyKind::from_rm_type(&version.rm_type)
                .map(|kind| party_href(kind, &object))
                .or_else(|| {
                    (version.rm_type == "PARTY_RELATIONSHIP").then(|| relationship_href(&object))
                });
            let title = title_cell(href.as_deref(), shortened(&object), None);
            view! {
                <tr>
                    {title} {cell(&columns[1], version.rm_type.clone())}
                    {cell(&columns[2], version_number(&version.version_uid))}
                </tr>
            }
            .into_any()
        })
        .collect_view()
        .into_any();
    let caption = tr(RecordsMsg::ContributionVersions);
    let table = data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(made),
    });
    card_panel(view! {
        {section(Some(tr(RecordsMsg::ContributionAudit)), None, None, definitions(rows))}
        {section(
            Some(tr(RecordsMsg::ContributionVersions)),
            u64::try_from(state.versions.len()).ok(),
            None,
            table,
        )}
        {document_section(
            tr(RecordsMsg::ContributionRecord),
            Language::Json,
            state.body.clone(),
            "contribution-record",
        )}
    })
}

/// A demographic contribution.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn PartyContributionPage() -> impl IntoView {
    let uid = uid_param();
    let title = tr_with(
        RecordsMsg::ContributionTitle,
        &[("id", shortened(&uid).into())],
    );
    let heading = page_heading(
        Header {
            title: title.clone().into(),
            subtitle: Some(tr(RecordsMsg::ContributionSubtitle).into()),
            crumbs: party_crumbs(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let model = page_model!(load_party_contribution(uid));
    let body = drawn(
        model,
        |model: Result<ContributionState, Refused>| match model {
            Ok(state) => party_contribution_view(&state),
            Err(refusal) => refusal.view(),
        },
    );
    view! {
        <Title text=title />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}

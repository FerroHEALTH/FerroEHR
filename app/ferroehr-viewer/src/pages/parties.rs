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

#![expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's demographic JSON wire for a party's identities and name"
)]

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
use leptos_router::hooks::use_params_map;
use serde_json::Value;

use crate::bff::demographics::contribution::fetch_demographic_contribution;
use crate::bff::demographics::party::{
    PartyState, create_party, delete_party, fetch_party, update_party,
};
use crate::bff::demographics::relationship::{
    PartyRefView, RelationshipState, create_relationship, delete_relationship, fetch_relationship,
    update_relationship,
};
use crate::bff::demographics::tags::{
    delete_party_tag, fetch_party_tags, list_demographic_tags, set_party_tag,
};
use crate::bff::demographics::{
    PartyKind, VersionedFamily, contribution_href, fetch_demographic_revision_history,
    fetch_demographic_version_document, fetch_versioned_object, party_href, relationship_href,
    resolve_demographic_version_at_time, resolve_party_kind,
};
use crate::bff::wire::VersionEntry;
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::islands::{CodeEditor, EditorField, EditorLanguage, EditorSetup};
use crate::locale::{Msg, message, tr, tr_with};
use crate::pages::common::{
    EditorDock, ago, at_time_row, awaited, card_panel, count, discarded, dock_editor,
    document_section, findings_of, info_note, is_status, marks_of, panel, plain_section, refused,
    section, short, shortened, state_mark, version_number, version_uid_at, when,
};
use crate::pages::item_tags::{TagAct, TagPanel, tag_act, tag_done, tag_panel};

/// The id of Delete party's confirmation dialog.
const DELETE_DIALOG: &str = "delete-party";

/// The id of Delete relationship's confirmation dialog.
const DELETE_RELATIONSHIP: &str = "delete-relationship";

/// The id of the editors' form.
const EDIT_FORM: &str = "party-edit";

/// The reader's word for a kind.
fn kind_words(kind: PartyKind) -> String {
    tr(match kind {
        PartyKind::Agent => Msg::KindAgent,
        PartyKind::Group => Msg::KindGroup,
        PartyKind::Organisation => Msg::KindOrganisation,
        PartyKind::Person => Msg::KindPerson,
        PartyKind::Role => Msg::KindRole,
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

/// The name a party goes by: the text values of its first identity's
/// elements, joined, or its short id when it carries none.
fn display_name(state: &PartyState) -> String {
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

/// The `DV_TEXT` values of the `ELEMENT`s under `node`, in order.
fn collect_text(node: &Value, words: &mut Vec<String>) {
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

/// The kind and the id the route names.
fn party_params() -> (Option<PartyKind>, String) {
    use_params_map().with_untracked(|params| {
        (
            params
                .get("kind")
                .as_deref()
                .and_then(PartyKind::from_segment),
            params.get("uid").unwrap_or_default(),
        )
    })
}

/// The id the route names.
fn uid_param() -> String {
    use_params_map()
        .with_untracked(|params| params.get("uid"))
        .unwrap_or_default()
}

/// The trail of the pages under a party.
fn party_crumbs() -> Vec<Crumb> {
    vec![Crumb::new(tr(Msg::NavParties), "/parties")]
}

/// One row of the demographic tag index: the tag and the party it is on.
#[derive(Debug, Clone)]
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

/// The demographic tag index, each target named by asking the CDR which
/// party holds it.
async fn tag_index() -> Result<Vec<Indexed>, ViewerError> {
    let tags = list_demographic_tags(String::new(), String::new(), String::new()).await?;
    let mut named: Vec<(String, Option<PartyKind>, String)> = Vec::new();
    let mut rows = Vec::new();
    for tag in tags {
        let container = crate::uid::container_uid_of(&tag.target);
        let known = named.iter().find(|(uid, _, _)| *uid == container).cloned();
        let (kind, name) = if let Some((_, kind, name)) = known { (kind, name) } else {
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
        Column::text("key", tr(Msg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("on", tr(Msg::PartiesTagOn)).role(Role::Meta),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Meta),
    ];
    let body = rows
        .iter()
        .map(|row| {
            let on = match &row.href {
                Some(href) => {
                    view! { <a class="fui-row-link" href=href.clone()>{row.on.clone()}</a> }
                        .into_any()
                }
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
    let caption = tr(Msg::PartiesTagIndex);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(body),
    })
    .into_any()
}

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
            None => Err(Ok(tr(Msg::PartiesNoneHolds))),
        },
        Ok(None) => Err(Ok(tr_with(
            Msg::PartiesNoKindHolds,
            &[("kind", kind_words(kind).into())],
        ))),
        Err(error) => Err(Err(error)),
    }
}

/// Parties: open a party by its id, and the tags on parties.
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn PartiesPage() -> impl IntoView {
    let kind = crate::request::query("kind")
        .as_deref()
        .and_then(PartyKind::from_segment)
        .unwrap_or(PartyKind::Person);
    let uid = crate::request::query("uid")
        .map(|uid| uid.trim().to_owned())
        .filter(|uid| !uid.is_empty());
    let heading = page_heading(
        Header {
            title: tr(Msg::NavParties).into(),
            subtitle: Some(tr(Msg::PartiesSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLUS),
            tr(Msg::PartiesNew),
            Act::Link("/parties/new".to_owned()),
        )),
    );
    let body = awaited(async move {
        let (invalid, top) = match &uid {
            Some(uid) => match open(kind, uid).await {
                Ok(href) => {
                    leptos_axum::redirect(&href);
                    return ().into_any();
                }
                Err(Ok(message)) => (Some(message), None),
                Err(Err(error)) => (
                    None,
                    Some(refused(tr(Msg::PartiesOpenRefused), &error, None)),
                ),
            },
            None => (None, None),
        };
        let error = invalid
            .clone()
            .map(|message| view! { <p id="p-id-error" class="fui-field-error">{message}</p> });
        let form = view! {
            <form class="fui-card fui-panel" method="get" action="/parties">
                <div class="fui-transform-options">
                    {Field::new("p-kind", tr(Msg::ColumnKind))
                        .view(view! {
                            <select id="p-kind" name="kind" class="fui-select">
                                {kind_options(kind)}
                            </select>
                        })}
                    {Field::new("p-id", tr(Msg::PartiesUid))
                        .holds_id()
                        .after(error)
                        .view(view! {
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
                        })}
                    <div class="fui-transform-run">
                        <button type="submit" class="fui-button fui-button-secondary">
                            {ferrohealth_ui::icon::icon(Glyph::ARROW_RIGHT, "16")}
                            {tr(Msg::PartiesOpen)}
                        </button>
                    </div>
                </div>
            </form>
        };
        let (index, total) = match tag_index().await {
            Ok(rows) if rows.is_empty() => (list_empty(tr(Msg::PartiesNoTags), None), Some(0)),
            Ok(rows) => (index_table(&rows), u64::try_from(rows.len()).ok()),
            Err(error) => (refused(tr(Msg::PartiesTagsRefused), &error, None), None),
        };
        let card = list_card(ListCard {
            title: tr(Msg::PartiesTagsTitle),
            count: total,
            order: None,
            tabs: None,
            actions: None,
            body: index,
            pager: None,
        });
        view! {
            <div class="fui-list-page">
                {info_note(tr(Msg::PartiesNote))}
                {form}
                {top}
                {card}
            </div>
        }
        .into_any()
    });
    view! {
        <Title text=tr(Msg::NavParties) />
        {heading}
        {body}
    }
}

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
                lead: tr(Msg::EditorJson),
                facts: vec![tr(Msg::EditorUtf8)],
                findings,
            }
            words=EditorWords::of_page()
            diagnostics=DiagnosticsWords::of_page()
        />
    }
    .into_any()
}

/// New party.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn NewPartyPage() -> impl IntoView {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect("/parties");
    }
    let page = awaited(async move {
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
            .and_then(|posted| posted.get(EditorField::Document.name()))
            .unwrap_or_else(|| starter(kind));
        let mut top = None;
        let mut findings = None;
        let mut line = (Glyph::FILE_PLUS, tr(Msg::PartyNotYet));
        if posted
            .as_ref()
            .is_some_and(|posted| posted.intent() == "save")
        {
            match create_party(kind.segment().to_owned(), text.clone()).await {
                Ok(uid) => {
                    redirect_with(
                        &party_href(kind, &uid),
                        &Done::ok(tr(Msg::PartyCreated)).line(shortened(&uid)),
                    );
                    return ().into_any();
                }
                Err(error) => {
                    line = (Glyph::CIRCLE_X, tr(Msg::PartyNotCreated));
                    findings = findings_of(&error, &text, EditorField::Document.name());
                    top = Some(refused(tr(Msg::PartyCreateRefused), &error, None));
                }
            }
        }
        dock_editor(EditorDock {
            form: EDIT_FORM.to_owned(),
            line: line.1,
            glyph: line.0,
            save: tr(Msg::PartyCreate),
            blocked: None,
        });
        let heading = editor_heading(
            message(Msg::PartyNewTitle),
            &Mode::Creating,
            Header {
                title: String::new().into(),
                subtitle: Some(
                    tr_with(Msg::PartyNewSubtitle, &[("kind", kind_words(kind).into())]).into(),
                ),
                crumbs: party_crumbs(),
                mono: false,
                badge: None,
            },
            ActionGroup::none(),
        );
        view! {
            <Title text=tr(Msg::PartiesNew) />
            {heading}
            <div class="fui-stack">
                {top}
                <form id=EDIT_FORM method="post" action="/parties/new">
                    {card_panel(section(None, None, None, view! {
                        <div class="fui-form-grid">
                            <div class="fui-span-2">
                                {Field::new("pk", tr(Msg::ColumnKind))
                                    .help(tr(Msg::PartyKindHelp))
                                    .view(view! {
                                        <div class="fui-cluster">
                                            <select id="pk" name="kind" class="fui-select">
                                                {kind_options(kind)}
                                            </select>
                                            <button type="submit" name="intent" value="kind" class="fui-button fui-button-secondary fui-button-small">
                                                {tr(Msg::PartyStartOver)}
                                            </button>
                                        </div>
                                    })}
                            </div>
                        </div>
                    }))}
                    {party_editor(text, None, findings, tr(Msg::PartyNewEditorLabel))}
                </form>
            </div>
        }
        .into_any()
    });
    view! { {page} }
}

/// The tabs of a party.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        tr(match self {
            Self::Overview => Msg::PartyOverview,
            Self::Document => Msg::CompositionDocument,
            Self::Versions => Msg::CompositionVersions,
            Self::Relationships => Msg::PartyRelationships,
            Self::Tags => Msg::EhrTabTags,
        })
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

/// The identities of a party, as a clinical reading.
fn identities(state: &PartyState) -> AnyView {
    let list: Vec<Value> = serde_json::from_str::<Value>(&state.identities)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let sections: Vec<_> = list
        .iter()
        .filter_map(|identity| crate::clinical::render(&identity.to_string()))
        .map(|rendered| crate::pages::composition::reading_section(&rendered))
        .collect();
    if sections.is_empty() {
        return info_note(tr(Msg::PartyNoIdentities));
    }
    reading(Reading {
        sections,
        level: Level::Three,
        folded: None,
    })
}

/// The Overview tab.
fn overview(kind: PartyKind, uid: &str, state: &PartyState) -> AnyView {
    let relationships = state.relationships.len();
    let facts = definitions(vec![
        (tr(Msg::ColumnKind), kind_words(kind).into_any()),
        (tr(Msg::PartyArchetype), view! { <code>{state.archetype_node_id.clone()}</code> }.into_any()),
        (tr(Msg::PartyCurrentVersion), version_number(&state.version_uid).into_any()),
        (tr(Msg::PartyIdentities), state.identity_count.to_string().into_any()),
        (
            tr(Msg::PartyRelationshipsFrom),
            view! {
                <a href=party_tab_href(kind, uid, PartyTab::Relationships, None)>{relationships.to_string()}</a>
            }
            .into_any(),
        ),
    ]);
    panel(view! {
        {section(Some(tr(Msg::PartyFacts)), None, None, facts)}
        {section(Some(tr(Msg::PartyIdentities)), None, None, identities(state))}
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
        tr(Msg::PartyRelate),
    );
    let columns = [
        Column::text("type", tr(Msg::RelationshipType)).role(Role::Title),
        Column::text("target", tr(Msg::RelationshipTarget)).role(Role::Sub),
        Column::text("kind", tr(Msg::RelationshipTargetKind)).role(Role::Meta),
    ];
    let body = if state.relationships.is_empty() {
        list_empty(tr(Msg::PartyNoRelationships), None)
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
                let sub = view! { <span class="fui-row-sub"><code>{relationship.archetype_node_id.clone()}</code></span> }.into_any();
                let title = title_cell(href.as_deref(), relationship.name.clone(), Some(sub));
                let target = shortened(&relationship.target.id).into_any();
                let target_kind = PartyKind::from_rm_type(&relationship.target.rm_type)
                    .map_or_else(|| relationship.target.rm_type.clone(), kind_words);
                view! { <tr>{title} {cell(&columns[1], target)} {cell(&columns[2], target_kind)}</tr> }.into_any()
            })
            .collect_view()
            .into_any();
        let caption = tr(Msg::PartyRelationships);
        data_table(DataTable {
            caption: &caption,
            columns: &columns,
            sorting: Sorting::default(),
            body: Body::Rows(rows),
        })
        .into_any()
    };
    panel(section(
        Some(tr(Msg::PartyRelationships)),
        u64::try_from(state.relationships.len()).ok(),
        Some(relate),
        view! {
            {ferrohealth_ui::view::surface::section_intro(tr(Msg::PartyRelationshipsIntro))}
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
            tr(Msg::PartyAtTimeOpens),
        )),
    })
}

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
async fn envelope(family: VersionedFamily, uid: &str, version_uid: &str) -> AnyView {
    match fetch_versioned_object(family.segment().to_owned(), uid.to_owned(), version_uid.to_owned()).await {
        Ok(facts) => section(
            Some(tr_with(Msg::CompositionVersionN, &[("version", version_number(&facts.version_id).into())])),
            None,
            None,
            definitions(vec![
                (tr(Msg::FactLifecycle), facts.lifecycle_state.clone().into_any()),
                (
                    tr(Msg::CompositionPreceding),
                    if facts.preceding_version_uid.is_empty() {
                        tr(Msg::CompositionFirstVersion).into_any()
                    } else {
                        short(&facts.preceding_version_uid)
                    },
                ),
                (
                    tr(Msg::FactContribution),
                    view! { <a href=contribution_href(&facts.contribution_uid)>{short(&facts.contribution_uid)}</a> }.into_any(),
                ),
                (tr(Msg::FactSigned), tr(if facts.signed { Msg::Yes } else { Msg::No }).into_any()),
            ]),
        ),
        Err(error) => plain_section(refused(tr(Msg::CompositionVersionRefused), &error, None)),
    }
}

/// What a post to a party's page asks, done: delete, set or remove a tag.
async fn party_act(
    kind: PartyKind,
    uid: &str,
    name: &str,
) -> (Option<AnyView>, Option<(ViewerError, TagAct)>) {
    let Some(posted) = crate::posted::posted() else {
        return (None, None);
    };
    if posted.intent() == "delete-party" {
        return match delete_party(kind.segment().to_owned(), posted.text("version")).await {
            Ok(()) => {
                redirect_with(
                    &party_href(kind, uid),
                    &Done::ok(tr(Msg::PartyDeleted)).line(name.to_owned()),
                );
                (None, None)
            }
            Err(error) => (
                Some(refused(tr(Msg::PartyDeleteRefused), &error, None)),
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

/// One party.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn PartyPage() -> impl IntoView {
    let (kind, uid) = party_params();
    let tab = PartyTab::asked();
    let asked = crate::request::query("version");
    let at = crate::request::query("at");
    let page = awaited(async move {
        let Some(kind) = kind else {
            crate::request::set_status(http::StatusCode::NOT_FOUND);
            return crate::pages::errors::not_found_page().into_any();
        };
        let current = match fetch_party(kind.segment().to_owned(), uid.clone()).await {
            Ok(current) => current,
            Err(error) => {
                if is_status(&error, http::StatusCode::NOT_FOUND) {
                    crate::request::set_status(http::StatusCode::NOT_FOUND);
                }
                let heading = page_heading(
                    Header {
                        title: tr_with(Msg::PartyTitle, &[("id", shortened(&uid).into())]).into(),
                        subtitle: None,
                        crumbs: party_crumbs(),
                        mono: false,
                        badge: None,
                    },
                    ActionGroup::none(),
                );
                return view! {
                    <Title text=tr(Msg::PartyRefused) />
                    {heading}
                    {refused(tr(Msg::PartyRefused), &error, None)}
                }
                .into_any();
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
                fetch_demographic_version_document(kind.segment().to_owned(), version.clone())
                    .await,
            ),
            _ => None,
        };
        let name = current
            .as_ref()
            .map_or_else(|| shortened(&uid), display_name);
        let (acted, tag_refusal) = party_act(kind, &uid, &name).await;
        let total = versions.as_ref().map_or(0, Vec::len);
        let reading = reading_number.as_deref();
        let tabs: Vec<Tab> = if deleted.is_some() {
            vec![Tab::new(
                PartyTab::Versions.key(),
                party_tab_href(kind, &uid, PartyTab::Versions, None),
                PartyTab::Versions.label(),
            )]
        } else {
            PartyTab::ALL
                .into_iter()
                .map(|each| {
                    let tab = Tab::new(
                        each.key(),
                        party_tab_href(kind, &uid, each, None),
                        each.label(),
                    );
                    match each {
                        PartyTab::Versions => {
                            tab.count(u64::try_from(total).unwrap_or(u64::MAX), each.label())
                        }
                        PartyTab::Relationships => tab.count(
                            current.as_ref().map_or(0, |state| {
                                u64::try_from(state.relationships.len()).unwrap_or(u64::MAX)
                            }),
                            each.label(),
                        ),
                        _ => tab,
                    }
                })
                .collect()
        };
        let tab = if deleted.is_some() || shown_body.is_some() {
            PartyTab::Versions
        } else {
            tab
        };
        let older = reading.map(|number| {
            plain_section(older_version(
                number,
                u32::try_from(total).unwrap_or(u32::MAX),
                party_tab_href(kind, &uid, PartyTab::Versions, None),
            ))
        });
        let body = match (tab, &current) {
            (PartyTab::Overview, Some(state)) => overview(kind, &uid, state),
            (PartyTab::Document, Some(state)) => panel(document_section(
                tr_with(
                    Msg::PartyDocumentTitle,
                    &[
                        ("type", state.rm_type.clone().into()),
                        ("version", version_number(&state.version_uid).into()),
                    ],
                ),
                Language::Json,
                crate::bff::pretty_body(&state.body, crate::format::ReprFormat::CanonicalJson),
                "party-document",
            )),
            (PartyTab::Relationships, Some(state)) => relationships(kind, &uid, state),
            (PartyTab::Tags, Some(_)) => {
                let stored_tags = fetch_party_tags(kind.segment().to_owned(), uid.clone()).await;
                panel(tag_panel(TagPanel {
                    collection: tr(Msg::TagCollectionParty),
                    tags: stored_tags,
                    action: party_tab_href(kind, &uid, PartyTab::Tags, None),
                    refused: tag_refusal,
                }))
            }
            _ => {
                let history = match &versions {
                    Ok(versions) => demographic_history(
                        versions,
                        reading,
                        deleted.is_some(),
                        &|version| party_tab_href(kind, &uid, PartyTab::Versions, version),
                        party_href(kind, &uid),
                    ),
                    Err(error) => refused(tr(Msg::CompositionHistoryRefused), error, None),
                };
                let in_view = match &shown_uid {
                    Ok(Some(version)) => version.clone(),
                    _ => before_deletion
                        .as_deref()
                        .map_or_else(|| latest.clone(), |number| version_uid_at(&latest, number)),
                };
                let document = match shown_body {
                    Some(Ok(body)) => document_section(
                        tr_with(
                            Msg::PartyVersionDocument,
                            &[("version", version_number(&in_view).into())],
                        ),
                        Language::Json,
                        crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson),
                        "party-version",
                    ),
                    Some(Err(error)) => {
                        plain_section(refused(tr(Msg::CompositionVersionRefused), &error, None))
                    }
                    None => ().into_any(),
                };
                let intro = deleted.is_some().then(|| {
                    ferrohealth_ui::view::surface::section_intro(tr(Msg::PartyDeletedIntro))
                });
                panel(view! {
                    {older}
                    {section(Some(tr(Msg::CompositionVersions)), u64::try_from(total).ok(), None, view! { {intro} {history} })}
                    {envelope(VersionedFamily::Party, &uid, &in_view).await}
                    {document}
                })
            }
        };
        let header = party_header(
            kind,
            &uid,
            &name,
            &latest,
            deleted.as_ref(),
            before_deletion.as_deref(),
        );
        let dialog = current.as_ref().map(|state| {
            danger_dialog(
                Confirmation {
                    id: DELETE_DIALOG,
                    title: tr_with(Msg::PartyDeleteTitle, &[("name", name.clone().into())]).into(),
                    message: tr_with(
                        Msg::PartyDeleteMessage,
                        &[
                            ("count", count(total)),
                            ("relationships", count(state.relationships.len())),
                        ],
                    )
                    .into(),
                },
                view! {
                    <form method="post" action=party_href(kind, &uid)>
                        <input type="hidden" name="intent" value="delete-party" />
                        <input type="hidden" name="version" value=state.version_uid.clone() />
                        <button type="submit" class="fui-button fui-button-danger">
                            {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                            {tr(Msg::PartyDeleteConfirm)}
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
                {acted}
                {binder(tr(Msg::PartyViews), tabs, tab.key(), body)}
            </div>
            {dialog}
        }
        .into_any()
    });
    view! { {page} }
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
    let (actions, subtitle, badge) = if let Some(deletion) = deleted { (
        ActionGroup::primary(Action::new(
            Some(Glyph::ROTATE_CCW_CLOCK),
            tr_with(
                Msg::CompositionReadVersion,
                &[("version", last.unwrap_or_default().to_owned().into())],
            ),
            Act::Link(party_tab_href(kind, uid, PartyTab::Versions, last)),
        )),
        tr_with(
            Msg::PartyDeletedLine,
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
        Some(state_mark(FlowState::Off, &tr(Msg::CompositionDeletedMark))),
    ) } else {
        let more = Menu::new(
            "party-more",
            tr(Msg::PartyMore),
            vec![Item::new(Action::new(
                Some(Glyph::LINK),
                tr(Msg::PartyRelate),
                Act::Link(format!(
                    "/parties/relationships/new?source_kind={}&source={}",
                    kind.segment(),
                    urlencoding::encode(uid)
                )),
            ))],
        )
        .destructive(vec![Item::new(Action::new(
            Some(Glyph::TRASH),
            tr(Msg::PartyDelete),
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
                Msg::PartyLine,
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
        },
        actions,
    )
    .into_any()
}

/// The `identities` and the `details` of an edited party document, as the
/// update takes them; a document that is not a JSON object is passed on
/// whole for the update to refuse.
fn parts_of(text: &str) -> (String, String) {
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

/// Edit a party.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn EditPartyPage() -> impl IntoView {
    let (kind, uid) = party_params();
    let posted = crate::posted::posted();
    let dock_promise = crate::shell::promise_dock();
    let page = awaited(async move {
        let _dock_promise = dock_promise;
        let Some(kind) = kind else {
            crate::request::set_status(http::StatusCode::NOT_FOUND);
            return crate::pages::errors::not_found_page().into_any();
        };
        if discarded(posted.as_ref()) {
            leptos_axum::redirect(&party_href(kind, &uid));
            return ().into_any();
        }
        let state = match fetch_party(kind.segment().to_owned(), uid.clone()).await {
            Ok(Some(state)) => state,
            Ok(None) => return info_note(tr(Msg::PartyDeletedNoEdit)),
            Err(error) => return refused(tr(Msg::PartyRefused), &error, None),
        };
        let current =
            crate::bff::pretty_body(&state.body, crate::format::ReprFormat::CanonicalJson);
        let opened = posted
            .as_ref()
            .map(|posted| posted.text("version"))
            .filter(|version| !version.is_empty())
            .unwrap_or_else(|| state.version_uid.clone());
        let text = posted
            .as_ref()
            .and_then(|posted| posted.get(EditorField::Document.name()))
            .unwrap_or_else(|| current.clone());
        let name = display_name(&state);
        let mut top = None;
        let mut blocked = None;
        let mut findings = None;
        let mut line = (Glyph::PENCIL, tr(Msg::EditorOpened));
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
                        &Done::ok(tr(Msg::PartySaved)).line(tr_with(
                            Msg::CompositionSavedVersion,
                            &[("version", version_number(&saved).into())],
                        )),
                    );
                    return ().into_any();
                }
                Err(error) => {
                    line = (Glyph::CIRCLE_X, tr(Msg::EditorNotSaved));
                    findings = findings_of(&error, &text, EditorField::Document.name());
                    let conflict = is_status(&error, http::StatusCode::PRECONDITION_FAILED);
                    if conflict {
                        blocked = Some(tr(Msg::EditorReloadFirst));
                    }
                    let reload = conflict.then(|| {
                        view! {
                            <div class="fui-cluster">
                                {ferrohealth_ui::view::button::link_button(
                                    format!("{}/edit", party_href(kind, &uid)),
                                    ferrohealth_ui::view::button::Variant::Secondary,
                                    Some(Glyph::REFRESH_CW),
                                    tr_with(Msg::EditorReloadVersion, &[("version", version_number(&state.version_uid).into())]),
                                )}
                                <span class="fui-small fui-muted">{tr(Msg::EditorReloadReplaces)}</span>
                            </div>
                        }
                    });
                    top = Some(
                        view! { {refused(tr(Msg::PartySaveRefused), &error, None)} {reload} }
                            .into_any(),
                    );
                }
            }
        }
        let save = version_number(&opened).parse::<u32>().map_or_else(
            |_| tr(Msg::EditorSave),
            |number| {
                tr_with(
                    Msg::EditorSaveAsVersion,
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
        trail.push(Crumb::new(name.clone(), party_href(kind, &uid)));
        let heading = editor_heading(
            message(Msg::EditorTitle),
            &Mode::Editing(name.clone()),
            Header {
                title: String::new().into(),
                subtitle: Some(
                    tr_with(
                        Msg::CompositionEditSubtitle,
                        &[("version", version_number(&opened).into())],
                    )
                    .into(),
                ),
                crumbs: trail,
                mono: false,
                badge: None,
            },
            ActionGroup::none(),
        );
        view! {
            <Title text=tr_with(Msg::CompositionEditTitle, &[("name", name.clone().into())]) />
            {heading}
            <div class="fui-stack">
                {top}
                <form id=EDIT_FORM method="post" action=format!("{}/edit", party_href(kind, &uid))>
                    <input type="hidden" name="version" value=opened.clone() />
                    {card_panel(section(Some(tr(Msg::CompositionDocument)), None, None, view! {
                        {ferrohealth_ui::view::surface::section_intro(tr(Msg::PartyEditIntro))}
                        {party_editor(text, Some(current), findings, tr(Msg::PartyEditorLabel))}
                    }))}
                </form>
            </div>
        }
        .into_any()
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
            Some(href) => view! { <a href=href>{name}</a> {format!(" · {kind}")} }.into_any(),
            None => view! { {name} {format!(" · {kind}")} }.into_any(),
        }
    };
    vec![
        (tr(Msg::RelationshipSource), end(&relationship.source)),
        (tr(Msg::RelationshipTarget), end(&relationship.target)),
    ]
}

/// One relationship.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn RelationshipPage() -> impl IntoView {
    let uid = uid_param();
    let page = awaited(async move {
        let state = match fetch_relationship(uid.clone()).await {
            Ok(Some(state)) => state,
            Ok(None) => return info_note(tr(Msg::RelationshipDeleted)),
            Err(error) => {
                if is_status(&error, http::StatusCode::NOT_FOUND) {
                    crate::request::set_status(http::StatusCode::NOT_FOUND);
                }
                return refused(tr(Msg::RelationshipRefused), &error, None);
            }
        };
        let source_href = ref_href(&state.source);
        if let Some(posted) = crate::posted::posted_for("delete-relationship") {
            match delete_relationship(posted.text("version")).await {
                Ok(()) => {
                    redirect_with(
                        source_href.as_deref().unwrap_or("/parties"),
                        &Done::ok(tr(Msg::RelationshipDeletedToast)).line(state.name.clone()),
                    );
                    return ().into_any();
                }
                Err(error) => {
                    return refused(tr(Msg::RelationshipDeleteRefused), &error, None);
                }
            }
        }
        let versions = history_of(VersionedFamily::PartyRelationship, &uid).await;
        let total = versions.as_ref().map_or(0, Vec::len);
        let history = match &versions {
            Ok(versions) => demographic_history(
                versions,
                None,
                false,
                &|_| relationship_href(&uid),
                relationship_href(&uid),
            ),
            Err(error) => refused(tr(Msg::CompositionHistoryRefused), error, None),
        };
        let mut facts = ends(&state);
        facts.push((
            tr(Msg::RelationshipType),
            view! { {state.name.clone()} " " <code class="fui-small">{state.archetype_node_id.clone()}</code> }.into_any(),
        ));
        facts.push((
            tr(Msg::RelationshipDetails),
            if state.details.is_empty() {
                tr(Msg::RelationshipNone).into_any()
            } else {
                view! { <code>{state.details.clone()}</code> }.into_any()
            },
        ));
        facts.push((
            tr(Msg::RelationshipValidity),
            if state.time_validity.is_empty() {
                tr(Msg::RelationshipNone).into_any()
            } else {
                view! { <code>{state.time_validity.clone()}</code> }.into_any()
            },
        ));
        let mut trail = party_crumbs();
        if let Some(href) = source_href.clone() {
            trail.push(Crumb::new(shortened(&state.source.id), href));
        }
        let more = Menu::new("relationship-more", tr(Msg::RelationshipMore), Vec::new())
            .destructive(vec![Item::new(Action::new(
                Some(Glyph::TRASH),
                tr(Msg::RelationshipDelete),
                Act::Dialog(DELETE_RELATIONSHIP.to_owned()),
            ))]);
        let heading = page_heading(
            Header {
                title: state.name.clone().into(),
                subtitle: Some(
                    tr_with(
                        Msg::RelationshipLine,
                        &[
                            ("version", version_number(&state.version_uid).into()),
                            ("id", shortened(&uid).into()),
                        ],
                    )
                    .into(),
                ),
                crumbs: trail,
                mono: false,
                badge: None,
            },
            ActionGroup::primary(Action::new(
                Some(Glyph::PENCIL),
                tr(Msg::PartyEdit),
                Act::Link(format!("{}/edit", relationship_href(&uid))),
            ))
            .more(more),
        );
        let dialog = danger_dialog(
            Confirmation {
                id: DELETE_RELATIONSHIP,
                title: tr_with(
                    Msg::RelationshipDeleteTitle,
                    &[("name", state.name.clone().into())],
                )
                .into(),
                message: tr(Msg::RelationshipDeleteMessage).into(),
            },
            view! {
                <form method="post" action=relationship_href(&uid)>
                    <input type="hidden" name="intent" value="delete-relationship" />
                    <input type="hidden" name="version" value=state.version_uid.clone() />
                    <button type="submit" class="fui-button fui-button-danger">
                        {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                        {tr(Msg::RelationshipDeleteConfirm)}
                    </button>
                </form>
            },
        );
        view! {
            <Title text=state.name.clone() />
            {heading}
            <div class="fui-stack">
                {card_panel(view! {
                    {section(Some(tr(Msg::PartyFacts)), None, None, definitions(facts))}
                    {section(Some(tr(Msg::CompositionVersions)), u64::try_from(total).ok(), None, history)}
                })}
            </div>
            {dialog}
        }
        .into_any()
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
                label: tr(Msg::RelationshipDetails),
                lead: tr(Msg::EditorJson),
                facts: vec![tr(Msg::EditorUtf8)],
                findings: None,
            }
            words=EditorWords::of_page()
            diagnostics=DiagnosticsWords::of_page()
        />
    }
    .into_any()
}

/// New relationship, from the party the address names.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn NewRelationshipPage() -> impl IntoView {
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
        .and_then(|posted| posted.get(EditorField::Details.name()))
        .unwrap_or_default();
    let back = party_tab_href(source_kind, &source, PartyTab::Relationships, None);
    if discarded(posted.as_ref()) {
        leptos_axum::redirect(&back);
    }
    let saving = posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save");
    let dock_promise = crate::shell::promise_dock();
    let page = awaited(async move {
        let _dock_promise = dock_promise;
        let mut top = None;
        let mut line = (Glyph::FILE_PLUS, tr(Msg::PartyNotYet));
        // NOTE: the fields are not `required` in the markup: Discard posts the same form, and the
        // browser would refuse it too; an empty type or target is answered here instead.
        let missing = kind_type.trim().is_empty() || target.trim().is_empty();
        if saving && missing {
            line = (Glyph::CIRCLE_X, tr(Msg::PartyNotCreated));
            top = Some(crate::pages::common::quiet(tr(Msg::RelationshipNeedsTypeAndTarget)));
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
                        &Done::ok(tr(Msg::RelationshipCreated)).line(kind_type.clone()),
                    );
                    return ().into_any();
                }
                Err(error) => {
                    line = (Glyph::CIRCLE_X, tr(Msg::PartyNotCreated));
                    top = Some(refused(tr(Msg::RelationshipCreateRefused), &error, None));
                }
            }
        }
        let source_name = match fetch_party(source_kind.segment().to_owned(), source.clone()).await
        {
            Ok(Some(state)) => display_name(&state),
            _ => shortened(&source),
        };
        dock_editor(EditorDock {
            form: EDIT_FORM.to_owned(),
            line: line.1,
            glyph: line.0,
            save: tr(Msg::RelationshipCreate),
            blocked: None,
        });
        let mut trail = party_crumbs();
        trail.push(Crumb::new(source_name.clone(), back.clone()));
        let heading = editor_heading(
            message(Msg::RelationshipNewTitle),
            &Mode::Creating,
            Header {
                title: String::new().into(),
                subtitle: Some(tr(Msg::RelationshipNewSubtitle).into()),
                crumbs: trail,
                mono: false,
                badge: None,
            },
            ActionGroup::none(),
        );
        let from = definitions(vec![(
            tr(Msg::RelationshipSource),
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
                    {Field::new("r-type", tr(Msg::RelationshipType))
                        .help(tr(Msg::RelationshipTypeHelp))
                        .view(view! { <input id="r-type" name="type" type="text" class="fui-input fui-mono" value=kind_type.clone() /> })}
                </div>
                <div class="fui-span-2">
                    {Field::new("r-tk", tr(Msg::RelationshipTargetKind))
                        .view(view! { <select id="r-tk" name="target_kind" class="fui-select">{kind_options(target_kind)}</select> })}
                </div>
                <div class="fui-span-2">
                    {Field::new("r-tid", tr(Msg::RelationshipTargetUid))
                        .holds_id()
                        .view(view! { <input id="r-tid" name="target" type="text" class="fui-input fui-mono" spellcheck="false" value=target.clone() /> })}
                </div>
            </div>
        };
        view! {
            <Title text=tr(Msg::RelationshipNewPageTitle) />
            {heading}
            <div class="fui-stack">
                {top}
                <form id=EDIT_FORM method="post" action="/parties/relationships/new">
                    {card_panel(view! {
                        {section(Some(tr(Msg::RelationshipFrom)), None, None, from)}
                        {section(Some(tr(Msg::RelationshipTitle)), None, None, grid)}
                        {section(Some(tr(Msg::RelationshipDetails)), None, None, view! {
                            {ferrohealth_ui::view::surface::section_intro(tr(Msg::RelationshipDetailsIntro))}
                            {details_editor(details.clone(), None)}
                        })}
                    })}
                </form>
            </div>
        }
        .into_any()
    });
    view! { {page} }
}

/// Edit a relationship: its type and its details; its ends are fixed.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn EditRelationshipPage() -> impl IntoView {
    let uid = uid_param();
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect(&relationship_href(&uid));
    }
    let dock_promise = crate::shell::promise_dock();
    let page = awaited(async move {
        let _dock_promise = dock_promise;
        let state = match fetch_relationship(uid.clone()).await {
            Ok(Some(state)) => state,
            Ok(None) => return info_note(tr(Msg::RelationshipDeleted)),
            Err(error) => return refused(tr(Msg::RelationshipRefused), &error, None),
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
            .and_then(|posted| posted.get(EditorField::Details.name()))
            .unwrap_or_else(|| state.details.clone());
        let mut top = None;
        let mut blocked = None;
        let changed = name != state.name || details.trim() != state.details.trim();
        let mut line = if changed {
            (Glyph::PENCIL, tr(Msg::DirectoryChanged))
        } else {
            (Glyph::PENCIL, tr(Msg::EditorOpened))
        };
        let saving = posted
            .as_ref()
            .is_some_and(|posted| posted.intent() == "save");
        if saving && name.trim().is_empty() {
            line = (Glyph::CIRCLE_X, tr(Msg::EditorNotSaved));
            top = Some(crate::pages::common::quiet(tr(Msg::RelationshipNeedsType)));
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
                        &Done::ok(tr(Msg::RelationshipSaved)).line(tr_with(
                            Msg::CompositionSavedVersion,
                            &[("version", version_number(&saved).into())],
                        )),
                    );
                    return ().into_any();
                }
                Err(error) => {
                    line = (Glyph::CIRCLE_X, tr(Msg::EditorNotSaved));
                    if is_status(&error, http::StatusCode::PRECONDITION_FAILED) {
                        blocked = Some(tr(Msg::EditorReloadFirst));
                    }
                    top = Some(refused(tr(Msg::RelationshipSaveRefused), &error, None));
                }
            }
        }
        let save = version_number(&opened).parse::<u32>().map_or_else(
            |_| tr(Msg::EditorSave),
            |number| {
                tr_with(
                    Msg::EditorSaveAsVersion,
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
        trail.push(Crumb::new(state.name.clone(), relationship_href(&uid)));
        let heading = editor_heading(
            message(Msg::EditorTitle),
            &Mode::Editing(state.name.clone()),
            Header {
                title: String::new().into(),
                subtitle: Some(
                    tr_with(
                        Msg::CompositionEditSubtitle,
                        &[("version", version_number(&opened).into())],
                    )
                    .into(),
                ),
                crumbs: trail,
                mono: false,
                badge: None,
            },
            ActionGroup::none(),
        );
        view! {
            <Title text=tr_with(Msg::CompositionEditTitle, &[("name", state.name.clone().into())]) />
            {heading}
            <div class="fui-stack">
                {top}
                <form id=EDIT_FORM method="post" action=format!("{}/edit", relationship_href(&uid))>
                    <input type="hidden" name="version" value=opened.clone() />
                    {card_panel(view! {
                        {section(Some(tr(Msg::RelationshipEnds)), None, None, view! {
                            {ferrohealth_ui::view::surface::section_intro(tr(Msg::RelationshipEndsIntro))}
                            {definitions(ends(&state))}
                        })}
                        {section(Some(tr(Msg::RelationshipTitle)), None, None, view! {
                            <div class="fui-form-grid">
                                <div class="fui-span-2">
                                    {Field::new("re-ty", tr(Msg::RelationshipType))
                                        .view(view! { <input id="re-ty" name="type" type="text" class="fui-input fui-mono" value=name.clone() /> })}
                                </div>
                            </div>
                        })}
                        {section(Some(tr(Msg::RelationshipDetails)), None, None, view! {
                            {ferrohealth_ui::view::surface::section_intro(tr(Msg::RelationshipDetailsIntro))}
                            {details_editor(details.clone(), Some(state.details.clone()))}
                        })}
                    })}
                </form>
            </div>
        }
        .into_any()
    });
    view! { {page} }
}

/// A demographic contribution.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn PartyContributionPage() -> impl IntoView {
    let uid = uid_param();
    let title = tr_with(Msg::ContributionTitle, &[("id", shortened(&uid).into())]);
    let heading = page_heading(
        Header {
            title: title.clone().into(),
            subtitle: Some(tr(Msg::ContributionSubtitle).into()),
            crumbs: party_crumbs(),
            mono: false,
            badge: None,
        },
        ActionGroup::none(),
    );
    let body = awaited(async move {
        let state = match fetch_demographic_contribution(uid.clone()).await {
            Ok(state) => state,
            Err(error) => {
                if is_status(&error, http::StatusCode::NOT_FOUND) {
                    crate::request::set_status(http::StatusCode::NOT_FOUND);
                }
                return refused(tr(Msg::ContributionRefused), &error, None);
            }
        };
        let mut rows = vec![
            (tr(Msg::FactCommitted), when(&state.time_committed)),
            (tr(Msg::FactCommitter), state.committer.clone().into_any()),
            (tr(Msg::FactChange), state.change_type.clone().into_any()),
        ];
        if !state.description.is_empty() {
            rows.push((
                tr(Msg::FactDescription),
                state.description.clone().into_any(),
            ));
        }
        let columns = [
            Column::text("object", tr(Msg::ColumnObject)).role(Role::Title),
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
                        (version.rm_type == "PARTY_RELATIONSHIP")
                            .then(|| relationship_href(&object))
                    });
                let title = title_cell(href.as_deref(), shortened(&object), None);
                view! {
                    <tr>
                        {title}
                        {cell(&columns[1], version.rm_type.clone())}
                        {cell(&columns[2], version_number(&version.version_uid))}
                    </tr>
                }
                .into_any()
            })
            .collect_view()
            .into_any();
        let caption = tr(Msg::ContributionVersions);
        let table = data_table(DataTable {
            caption: &caption,
            columns: &columns,
            sorting: Sorting::default(),
            body: Body::Rows(made),
        });
        card_panel(view! {
            {section(Some(tr(Msg::ContributionAudit)), None, None, definitions(rows))}
            {section(Some(tr(Msg::ContributionVersions)), u64::try_from(state.versions.len()).ok(), None, table)}
            {document_section(
                tr(Msg::ContributionRecord),
                Language::Json,
                crate::bff::pretty_body(&state.body, crate::format::ReprFormat::CanonicalJson),
                "contribution-record",
            )}
        })
    });
    view! {
        <Title text=title />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}

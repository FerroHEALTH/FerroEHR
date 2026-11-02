// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One composition with its tabs, Reading, Document, Versions and Tags, its
//! delete, and its editors, Edit as a new version and New composition
//! (FerroUI `design/consoles/ferroehr/`, pages 14.1 to 14.14).
//!
//! The Reading tab draws the canonical document as FerroUI's clinical
//! reading (E3), template-free ([`crate::clinical`]). The Document tab shows
//! the document in the representation the reader picks, as the CDR converts
//! it. The Versions tab is the revision history with At a time, and an older
//! version opens read-only. A deleted composition says so in its header and
//! keeps its versions readable (ITS-REST `composition_get` answers `204` for
//! a deleted version). The tab is the address's `tab`, the version its
//! `version`, so every view keeps its own address. No openEHR spec governs
//! the viewer's pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action, Item, Menu};
use ferrohealth_ui::view::coding::{Coding, Layout, coding};
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, danger_form_dialog};
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::reading::{Element, Level, Reading, Section, Value as Shown, reading};
use ferrohealth_ui::view::segmented::{choice_group, segmented_links};
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, page_heading,
};
use ferrohealth_ui::view::tabs::{Tab, binder};
use ferrohealth_ui::view::versions::older_version;

use crate::bff::composition::{
    delete_composition, fetch_composition, fetch_version_at_time, fetch_versioned_composition,
    fetch_versions, update_composition,
};
use crate::bff::ehr_tags::composition_tags_path;
use crate::bff::item_tags::{delete_tag, read_tags, set_tag};
use crate::bff::wire::VersionEntry;
use crate::bff::wire::VersionedObjectFacts;
use crate::clinical::{RenderedNode, RenderedSection};
use crate::components::{EditorField, EditorLanguage, EditorSetup, code_editor};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::format::ReprFormat;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    EditorDock, Refused, card_panel, dock_editor, document_section, drawn, findings_of, info_note,
    marks_of, panel, plain_section, section, short, shortened, state_mark, version_number, when,
};
use crate::pages::common::{discarded, is_status, version_uid_at};
use crate::pages::ehr::{crumbs, ehr_href, ehr_id_param};
use crate::pages::item_tags::{TagAct, tag_act, tag_done};
use crate::pages::item_tags::{TagPanel, tag_panel};
use crate::pages::versions::History;
use crate::routes::Drawn;

/// The id of Delete composition's confirmation dialog.
const DELETE_DIALOG: &str = "delete-composition";

/// The id of the editors' form.
const EDIT_FORM: &str = "composition-edit";

/// The page of the composition `uid` of the EHR `ehr_id`.
#[must_use]
pub fn composition_href(ehr_id: &str, uid: &str) -> String {
    format!(
        "{}/compositions/{}",
        ehr_href(ehr_id),
        urlencoding::encode(uid)
    )
}

/// The tabs of a composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompTab {
    /// The clinical reading.
    Reading,
    /// The document as the CDR sends it.
    Document,
    /// The revision history.
    Versions,
    /// The tags.
    Tags,
}

impl CompTab {
    /// The tab the address names, the Reading for none.
    fn asked() -> Self {
        match crate::request::query("tab").as_deref() {
            Some("document") => Self::Document,
            Some("versions") => Self::Versions,
            Some("tags") => Self::Tags,
            _ => Self::Reading,
        }
    }

    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Document => "document",
            Self::Versions => "versions",
            Self::Tags => "tags",
        }
    }
}

/// The address of the tab `tab` of the composition, at `version` when it is
/// an older one.
fn tab_href(ehr_id: &str, uid: &str, tab: CompTab, version: Option<&str>) -> String {
    let base = composition_href(ehr_id, uid);
    let mut pairs = Vec::new();
    if tab != CompTab::Reading {
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

/// The header facts of a composition document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Head {
    /// `name.value`.
    name: String,
    /// `archetype_details.template_id.value`.
    template: String,
    /// `context.start_time.value`.
    started: String,
    /// `composer.name`.
    composer: String,
}

impl Head {
    #[expect(
        clippy::disallowed_types,
        reason = "the viewer reads the CDR's COMPOSITION JSON wire for the header's name, template and context"
    )]
    /// The facts of the canonical JSON `body`; a body that is not JSON names
    /// none of them.
    fn of(body: &str) -> Self {
        use serde_json::Value;
        // NOTE: the header facts are a projection of a document the CDR accepted; one it sends unparsable is shown in the Document tab as sent.
        let Ok(doc) = serde_json::from_str::<Value>(body) else {
            return Self::default();
        };
        let text = |pointer: &str| {
            doc.pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            name: text("/name/value"),
            template: text("/archetype_details/template_id/value"),
            started: text("/context/start_time/value"),
            composer: text("/composer/name"),
        }
    }
}

/// The FerroUI reading section of a rendered RM section.
pub(crate) fn reading_section(rendered: &RenderedSection) -> Section {
    let mut elements = Vec::new();
    let mut sections = Vec::new();
    for child in &rendered.children {
        match child {
            RenderedNode::Row(row) => {
                let value = match &row.code {
                    Some(code) => {
                        let (system, code) = code.split_once("::").unwrap_or(("", code));
                        let mut coded = Coding::new(code, row.value.clone());
                        if !system.is_empty() {
                            coded = coded.system(system);
                        }
                        Shown::Coded(coding(&current(), coded, Layout::Inline))
                    }
                    None => Shown::Text(row.value.clone()),
                };
                elements.push(Element {
                    label: row.label.clone(),
                    value,
                });
            }
            RenderedNode::Section(section) => sections.push(reading_section(section)),
        }
    }
    Section {
        title: rendered.title.clone(),
        node: rendered.archetype_node_id.clone(),
        elements,
        sections,
    }
}

/// The Reading tab: the document as a clinical reading, or a note when it
/// is not a canonical document the reading understands.
fn reading_panel(
    ehr_id: &str,
    uid: &str,
    rendered: Option<RenderedSection>,
    older: Option<Html>,
) -> Html {
    let drawn = match rendered {
        Some(document) => {
            let section = reading_section(&document);
            let mut sections = section.sections;
            if !section.elements.is_empty() {
                sections.insert(
                    0,
                    Section {
                        title: tr(RecordsMsg::CompositionContext),
                        node: None,
                        elements: section.elements,
                        sections: Vec::new(),
                    },
                );
            }
            reading(
                &current(),
                Reading {
                    sections,
                    level: Level::Two,
                    folded: None,
                },
            )
        }
        None => plain_section(info_note(t(tr(RecordsMsg::CompositionNoReading)))),
    };
    let full = older.is_none().then(|| {
        plain_section(ferrohealth_ui::view::button::link_button(
            tab_href(ehr_id, uid, CompTab::Document, None),
            ferrohealth_ui::view::button::Variant::Ghost,
            Some(Glyph::FILE_CODE),
            tr(RecordsMsg::CompositionShowDocument),
        ))
    });
    panel(Html::join([
        opt(older.map(plain_section)),
        drawn,
        opt(full),
    ]))
}

/// The representations the Document tab offers.
const FORMATS: [(&str, ReprFormat); 4] = [
    ("json", ReprFormat::CanonicalJson),
    ("xml", ReprFormat::CanonicalXml),
    ("flat", ReprFormat::Flat),
    ("structured", ReprFormat::Structured),
];

/// The representation the address names, canonical JSON for none.
fn asked_format() -> (usize, ReprFormat) {
    let asked = crate::request::query("format").unwrap_or_default();
    FORMATS
        .iter()
        .position(|(key, _)| *key == asked)
        .and_then(|index| FORMATS.get(index).map(|(_, format)| (index, *format)))
        .unwrap_or((0, ReprFormat::CanonicalJson))
}

/// The Document tab's document: the representation it is in and the
/// document as the CDR converted it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Converted {
    /// The place of the representation in [`FORMATS`].
    current: usize,
    /// The representation.
    format: ReprFormat,
    /// The document pretty-printed, `None` for a deleted version, or the
    /// CDR's refusal.
    shown: Result<Option<String>, Refused>,
}

/// Reads the Document tab's document: the version `version_uid` in the
/// representation the address names.
async fn converted(ehr_id: &str, version_uid: &str) -> Converted {
    let (current, format) = asked_format();
    let shown = fetch_composition(ehr_id.to_owned(), version_uid.to_owned(), format)
        .await
        .map(|body| body.map(|body| crate::bff::pretty_body(&body, format)))
        .map_err(|error| Refused::new(tr(RecordsMsg::CompositionDocumentRefused), error));
    Converted {
        current,
        format,
        shown,
    }
}

/// The Document tab: the representation switch and the document as the CDR
/// converted it.
fn document_panel(ehr_id: &str, uid: &str, version: Option<&str>, converted: Converted) -> Html {
    let Converted {
        current,
        format,
        shown,
    } = converted;
    let base = tab_href(ehr_id, uid, CompTab::Document, version);
    let joiner = if base.contains('?') { '&' } else { '?' };
    let choices = FORMATS
        .iter()
        .map(|(key, format)| (format!("{base}{joiner}format={key}"), format.label()))
        .collect();
    let switch = h(format!(
        r#"<div class="fui-cluster">{}<span class="fui-small fui-muted">{}</span></div>"#,
        segmented_links(tr(Msg::CompositionFormat), choices, current),
        t(tr(RecordsMsg::CompositionConverts))
    ));
    let shown = match shown {
        Ok(Some(body)) => document_section(
            tr_with(
                RecordsMsg::CompositionDocumentTitle,
                &[("format", format.label().into())],
            ),
            if format == ReprFormat::CanonicalXml {
                Language::Xml
            } else {
                Language::Json
            },
            body,
            "composition-document",
        ),
        Ok(None) => plain_section(info_note(t(tr(RecordsMsg::CompositionDeletedVersion)))),
        Err(refusal) => plain_section(refusal.view()),
    };
    panel(plain_section(switch).then(shown))
}

/// The revision history of the composition, the version in view marked.
fn history(
    ehr_id: &str,
    uid: &str,
    versions: &[VersionEntry],
    reading: Option<&str>,
    deleted: bool,
) -> Html {
    crate::pages::versions::history(
        History {
            versions,
            reading,
            deleted,
            contribution: Html::empty(),
            caption: tr(Msg::CompositionVersions),
            at_action: composition_href(ehr_id, uid),
            at_opens: tr(RecordsMsg::CompositionAtTimeOpens),
        },
        |number| match number {
            None => tab_href(ehr_id, uid, CompTab::Versions, None),
            Some(number) => tab_href(ehr_id, uid, CompTab::Reading, Some(number)),
        },
    )
}

/// The Versions tab: the history, the version in view's envelope and the
/// versioned composition.
fn versions_panel(
    ehr_id: &str,
    uid: &str,
    versions: &Result<Vec<VersionEntry>, Refused>,
    details: Result<VersionedObjectFacts, Refused>,
    reading: Option<&str>,
    deleted: bool,
) -> Html {
    let count = versions
        .as_ref()
        .map_or(None, |versions| u64::try_from(versions.len()).ok());
    let history = match versions {
        Ok(versions) => history(ehr_id, uid, versions, reading, deleted),
        Err(refusal) => refusal.view(),
    };
    let facts = match details {
        Ok(details) => {
            let version = crate::pages::versions::envelope(
                &details,
                short(&details.contribution_uid),
                vec![(
                    tr(RecordsMsg::CompositionHoldsContent),
                    t(tr(if details.has_data { Msg::Yes } else { Msg::No })),
                )],
            );
            let object = section(
                Some(tr(RecordsMsg::CompositionVersioned)),
                None,
                None,
                definitions(vec![
                    (
                        tr(RecordsMsg::CompositionVersionedObject),
                        short(&details.object_uid),
                    ),
                    (
                        tr(RecordsMsg::ColumnEhr),
                        h(format!(
                            r#"<a href="{}">{}</a>"#,
                            t(ehr_href(ehr_id)),
                            short(ehr_id)
                        )),
                    ),
                    (
                        tr(RecordsMsg::CompositionFirstCommitted),
                        when(&details.time_created),
                    ),
                ]),
            );
            version.then(object)
        }
        Err(refusal) => plain_section(refusal.view()),
    };
    panel(section(Some(tr(Msg::CompositionVersions)), count, None, history).then(facts))
}

/// What a post to the composition's page asks, done: delete, set or remove a
/// tag. `Some` holds the refusal to draw, and for a tag the draft it keeps.
async fn act(
    ehr_id: &str,
    uid: &str,
    name: &str,
) -> (Option<Refused>, Option<(ViewerError, TagAct)>) {
    let Some(posted) = crate::posted::posted() else {
        return (None, None);
    };
    if posted.intent() == "delete-composition" {
        return match delete_composition(ehr_id.to_owned(), posted.text("version")).await {
            Ok(()) => {
                redirect_with(
                    &ehr_href(ehr_id),
                    &Done::ok(tr(RecordsMsg::CompositionDeleted)).line(name.to_owned()),
                );
                (None, None)
            }
            Err(error) => (
                Some(Refused::new(
                    tr(RecordsMsg::CompositionDeleteRefused),
                    error,
                )),
                None,
            ),
        };
    }
    let Some(act) = tag_act() else {
        return (None, None);
    };
    let tags = composition_tags_path(ehr_id, uid);
    let done = match &act {
        TagAct::Set { key, value, path } => set_tag(&tags, key, value, path).await,
        TagAct::Remove(key) => delete_tag(&tags, key).await,
    };
    match done {
        Ok(()) => {
            redirect_with(
                &tab_href(ehr_id, uid, CompTab::Tags, None),
                &tag_done(&act, name),
            );
            (None, None)
        }
        Err(error) => (None, Some((error, act))),
    }
}

/// The More menu's Copy the id of the composition `uid`.
fn copy_id(uid: &str) -> Item {
    Item::new(Action::new(
        Some(Glyph::COPY),
        tr(RecordsMsg::CompositionCopyId),
        Act::Copy(uid.to_owned()),
    ))
}

/// The context line of the composition's header: its template, the version
/// read, when it started and who composed it.
fn context_line(head: &Head, latest: &str, reading: Option<&str>) -> String {
    match reading {
        Some(_) => tr_with(
            RecordsMsg::CompositionLineOlder,
            &[
                ("template", head.template.clone().into()),
                (
                    "started",
                    ferrohealth_ui::view::time::reading(&current(), &head.started).into(),
                ),
                ("composer", head.composer.clone().into()),
            ],
        ),
        None => tr_with(
            RecordsMsg::CompositionLine,
            &[
                ("template", head.template.clone().into()),
                ("version", version_number(latest).into()),
                (
                    "started",
                    ferrohealth_ui::view::time::reading(&current(), &head.started).into(),
                ),
                ("composer", head.composer.clone().into()),
            ],
        ),
    }
}

/// The composition's header: its name, its template and context line, and
/// Edit with the More menu; a deleted composition says when it was deleted.
fn header(
    ehr_id: &str,
    uid: &str,
    head: &Head,
    latest: &str,
    reading: Option<&str>,
    deleted: Option<&VersionEntry>,
) -> Html {
    let more = Menu::new(
        "composition-more",
        tr(RecordsMsg::CompositionMore),
        vec![copy_id(uid)],
    );
    let (actions, subtitle, badge) = if let Some(deletion) = deleted {
        let last = version_number(&deletion.version_id)
            .parse::<u32>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .map_or_else(String::new, |number| number.to_string());
        (
            ActionGroup::primary(Action::new(
                Some(Glyph::ROTATE_CCW_CLOCK),
                tr_with(
                    RecordsMsg::CompositionReadVersion,
                    &[("version", last.clone().into())],
                ),
                Act::Link(tab_href(ehr_id, uid, CompTab::Reading, Some(&last))),
            ))
            .more(more),
            tr_with(
                RecordsMsg::CompositionDeletedLine,
                &[
                    ("template", head.template.clone().into()),
                    (
                        "when",
                        ferrohealth_ui::view::time::reading(&current(), &deletion.committed).into(),
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
        let primary = if reading.is_some() {
            tr(RecordsMsg::CompositionEditLatest)
        } else {
            tr(RecordsMsg::CompositionEdit)
        };
        let menu = more.destructive(vec![Item::new(Action::new(
            Some(Glyph::TRASH),
            tr(RecordsMsg::CompositionDelete),
            Act::Dialog(DELETE_DIALOG.to_owned()),
        ))]);
        let line = context_line(head, latest, reading);
        (
            ActionGroup::primary(Action::new(
                Some(Glyph::PENCIL),
                primary,
                Act::Link(format!("{}/edit", composition_href(ehr_id, uid))),
            ))
            .more(menu),
            line,
            None,
        )
    };
    let name = if head.name.is_empty() {
        shortened(uid)
    } else {
        head.name.clone()
    };
    page_heading(
        &current(),
        Header {
            title: name.into(),
            subtitle: Some(subtitle.into()),
            crumbs: crumbs(ehr_id),
            mono: false,
            badge,
            subtitle_view: None,
        },
        actions,
    )
}

/// The confirmation of Delete composition.
fn delete_dialog(ehr_id: &str, uid: &str, name: &str, latest: &str, count: usize) -> Html {
    let next = version_number(latest).parse::<u32>().map_or_else(
        |_| String::new(),
        |number| number.saturating_add(1).to_string(),
    );
    danger_form_dialog(
        &current(),
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr_with(
                RecordsMsg::CompositionDeleteTitle,
                &[("name", name.to_owned().into())],
            )
            .into(),
            message: tr_with(
                RecordsMsg::CompositionDeleteMessage,
                &[
                    ("next", next.into()),
                    ("count", crate::pages::common::count(count)),
                ],
            )
            .into(),
        },
        ConfirmForm {
            action: composition_href(ehr_id, uid),
            fields: h(format!(
                r#"<input type="hidden" name="intent" value="delete-composition"><input type="hidden" name="version" value="{}">"#,
                t(latest)
            )),
        },
        h(format!(
            r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
            t(tr(RecordsMsg::CompositionDeleteConfirm))
        )),
    )
}

/// The composition id the route names.
fn uid_param() -> String {
    crate::routes::params().get("uid").unwrap_or_default()
}

/// The version the Reading tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Read {
    /// Its clinical reading, `None` when it is not a canonical document the
    /// reading understands.
    Clinical(Option<RenderedSection>),
    /// The version is a deletion.
    Deleted,
}

/// What a composition's tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TabBody {
    /// The version shown, or the CDR's refusal of it.
    Reading(Result<Read, Refused>),
    /// The document in the representation asked for.
    Document(Converted),
    /// The envelope of the version in view.
    Versions(Result<VersionedObjectFacts, Refused>),
    /// The tags.
    Tags(TagPanel),
}

/// A composition the CDR holds, as its page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Composition {
    /// Its versions, newest first, or the CDR's refusal of them.
    versions: Result<Vec<VersionEntry>, Refused>,
    /// The id of its latest version.
    latest: String,
    /// The version that deleted it, when it is deleted.
    deleted: Option<VersionEntry>,
    /// The number of the version in view, when it is not the latest.
    reading_number: Option<String>,
    /// The header facts.
    head: Head,
    /// The delete the CDR refused.
    acted: Option<Refused>,
    /// The tab shown.
    tab: CompTab,
    /// What it shows.
    body: TabBody,
}

/// One composition's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CompositionModel {
    /// The composition.
    Shown(Box<Composition>),
    /// The CDR's refusal of it.
    Missing(Refused),
}

/// Loads the composition `uid` of the EHR `ehr_id`: answers its post, reads
/// its versions, the version in view and what the tab asked for shows; a
/// composition the CDR does not hold answers `404`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load(
    // The EHR the route names.
    ehr_id: String,
    // The id the route names.
    uid: String,
) -> Result<CompositionModel, ViewerError> {
    crate::session::require_session()?;
    Ok(read_page(ehr_id, uid).await)
}

/// What [`load`] reads.
#[expect(
    clippy::too_many_lines,
    reason = "one loader: the versions, the version in view, the post and the tab's read"
)]
async fn read_page(ehr_id: String, uid: String) -> CompositionModel {
    let asked = crate::request::query("version");
    let at = crate::request::query("at");
    let mut versions = fetch_versions(ehr_id.clone(), uid.clone()).await;
    if let Ok(versions) = &mut versions {
        versions.sort_by_key(|version| {
            std::cmp::Reverse(
                version_number(&version.version_id)
                    .parse::<u64>()
                    .unwrap_or(0),
            )
        });
    }
    let latest_entry = versions
        .as_ref()
        .ok()
        .and_then(|versions| versions.first().cloned());
    let latest = latest_entry
        .as_ref()
        .map_or_else(String::new, |entry| entry.version_id.clone());
    let current = fetch_composition(ehr_id.clone(), uid.clone(), ReprFormat::CanonicalJson).await;
    let current = match current {
        Ok(current) => current,
        Err(error) => {
            if is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            return CompositionModel::Missing(Refused::new(
                tr(RecordsMsg::CompositionRefused),
                error,
            ));
        }
    };
    let deleted = current.is_none().then(|| latest_entry.clone()).flatten();
    let latest_number = version_number(&latest);
    let default_reading = deleted.as_ref().and_then(|entry| {
        version_number(&entry.version_id)
            .parse::<u32>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .map(|number| number.to_string())
    });
    let reading_number = match (&at, &asked) {
        (Some(_), _) => None,
        (None, Some(number)) if *number != latest_number => Some(number.clone()),
        _ => default_reading.clone(),
    };
    let shown: Result<Option<String>, ViewerError> = match (&at, &reading_number) {
        (Some(at), _) => fetch_version_at_time(ehr_id.clone(), uid.clone(), at.clone())
            .await
            .map(Some),
        (None, Some(number)) => {
            fetch_composition(
                ehr_id.clone(),
                version_uid_at(&latest, number),
                ReprFormat::CanonicalJson,
            )
            .await
        }
        _ => Ok(current.clone()),
    };
    let head_body = match (&shown, &current) {
        (Ok(Some(body)), _) | (_, Some(body)) => body.clone(),
        _ => String::new(),
    };
    let head = Head::of(&head_body);
    let name = if head.name.is_empty() {
        shortened(&uid)
    } else {
        head.name.clone()
    };
    let (acted, tag_refusal) = act(&ehr_id, &uid, &name).await;
    let reading = reading_number.as_deref().filter(|_| deleted.is_none());
    let tab = CompTab::asked();
    let tab = if tags_allowed(deleted.is_some(), reading) || tab != CompTab::Tags {
        tab
    } else {
        CompTab::Reading
    };
    let version_uid = reading_number
        .as_deref()
        .map_or_else(|| latest.clone(), |number| version_uid_at(&latest, number));
    let body = match tab {
        CompTab::Reading => TabBody::Reading(
            shown
                .map(|body| {
                    body.map_or(Read::Deleted, |body| {
                        Read::Clinical(crate::clinical::render(&body))
                    })
                })
                .map_err(|error| Refused::new(tr(RecordsMsg::CompositionVersionRefused), error)),
        ),
        CompTab::Document => TabBody::Document(converted(&ehr_id, &version_uid).await),
        CompTab::Versions => TabBody::Versions(
            fetch_versioned_composition(ehr_id.clone(), uid.clone(), version_uid.clone())
                .await
                .map_err(|error| Refused::new(tr(RecordsMsg::CompositionVersionRefused), error)),
        ),
        CompTab::Tags => {
            let stored_tags = read_tags(&composition_tags_path(&ehr_id, &uid)).await;
            TabBody::Tags(TagPanel {
                collection: tr(RecordsMsg::TagCollectionComposition),
                tags: stored_tags,
                action: tab_href(&ehr_id, &uid, CompTab::Tags, None),
                refused: tag_refusal,
            })
        }
    };
    CompositionModel::Shown(Box::new(Composition {
        versions: versions
            .map_err(|error| Refused::new(tr(RecordsMsg::CompositionHistoryRefused), error)),
        latest,
        deleted,
        reading_number,
        head,
        acted,
        tab,
        body,
    }))
}

/// The page of a composition the CDR cannot read, with its title.
fn missing(ehr_id: &str, uid: &str, refusal: &Refused) -> (String, Html) {
    let heading = page_heading(
        &current(),
        Header {
            title: tr_with(
                RecordsMsg::CompositionTitle,
                &[("id", shortened(uid).into())],
            )
            .into(),
            subtitle: None,
            crumbs: crumbs(ehr_id),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    (
        tr(RecordsMsg::CompositionRefused),
        heading.then(refusal.view()),
    )
}

/// The tabs of a composition: the version in view and the versions of a
/// deleted one, the reading and the versions of an older version, and every
/// tab of the latest.
fn tabs(
    ehr_id: &str,
    uid: &str,
    deleted: bool,
    reading_number: Option<&str>,
    count: usize,
) -> Vec<Tab> {
    let versions = Tab::new(
        CompTab::Versions.key(),
        tab_href(ehr_id, uid, CompTab::Versions, None),
        tr(Msg::CompositionVersions),
    )
    .count(
        u64::try_from(count).unwrap_or(u64::MAX),
        tr(Msg::CompositionVersions),
    );
    if deleted {
        vec![
            Tab::new(
                CompTab::Reading.key(),
                tab_href(ehr_id, uid, CompTab::Reading, reading_number),
                tr_with(
                    RecordsMsg::CompositionVersionN,
                    &[(
                        "version",
                        reading_number.unwrap_or_default().to_owned().into(),
                    )],
                ),
            ),
            versions,
        ]
    } else if reading_number.is_some() {
        vec![
            Tab::new(
                CompTab::Reading.key(),
                tab_href(ehr_id, uid, CompTab::Reading, reading_number),
                tr(RecordsMsg::CompositionReading),
            ),
            versions,
        ]
    } else {
        vec![
            Tab::new(
                CompTab::Reading.key(),
                tab_href(ehr_id, uid, CompTab::Reading, None),
                tr(RecordsMsg::CompositionReading),
            ),
            Tab::new(
                CompTab::Document.key(),
                tab_href(ehr_id, uid, CompTab::Document, None),
                tr(RecordsMsg::CompositionDocument),
            ),
            versions,
            Tab::new(
                CompTab::Tags.key(),
                tab_href(ehr_id, uid, CompTab::Tags, None),
                tr(RecordsMsg::EhrTabTags),
            ),
        ]
    }
}

/// A composition's page with its title: its header, the tab shown and Delete
/// composition's confirmation.
fn composition_view(ehr_id: &str, uid: &str, composition: Composition) -> (String, Html) {
    let Composition {
        versions,
        latest,
        deleted,
        reading_number,
        head,
        acted,
        tab,
        body,
    } = composition;
    let name = if head.name.is_empty() {
        shortened(uid)
    } else {
        head.name.clone()
    };
    let count = versions.as_ref().map_or(0, Vec::len);
    let reading = reading_number.as_deref().filter(|_| deleted.is_none());
    let older = reading_number.as_deref().map(|number| {
        if deleted.is_some() {
            info_note(t(tr_with(
                RecordsMsg::CompositionLastBeforeDeletion,
                &[("version", number.to_owned().into())],
            )))
        } else {
            older_version(
                &current(),
                number,
                u32::try_from(count).unwrap_or(u32::MAX),
                composition_href(ehr_id, uid),
            )
        }
    });
    let tabs = tabs(
        ehr_id,
        uid,
        deleted.is_some(),
        reading_number.as_deref(),
        count,
    );
    let body = match body {
        TabBody::Reading(Ok(Read::Clinical(rendered))) => {
            reading_panel(ehr_id, uid, rendered, older)
        }
        TabBody::Reading(Ok(Read::Deleted)) => panel(plain_section(info_note(t(tr(
            RecordsMsg::CompositionDeletedVersion,
        ))))),
        TabBody::Reading(Err(refusal)) => panel(plain_section(refusal.view())),
        TabBody::Document(converted) => document_panel(ehr_id, uid, reading, converted),
        TabBody::Versions(details) => {
            versions_panel(ehr_id, uid, &versions, details, reading, deleted.is_some())
        }
        TabBody::Tags(tagged) => panel(tag_panel(tagged)),
    };
    let dialog = deleted
        .is_none()
        .then(|| delete_dialog(ehr_id, uid, &name, &latest, count));
    let binder = binder(
        &current(),
        tr(RecordsMsg::CompositionViews),
        tabs,
        tab.key(),
        body,
    );
    let page = Html::join([
        header(ehr_id, uid, &head, &latest, reading, deleted.as_ref()),
        h(format!(
            r#"<div class="fui-stack">{}{binder}</div>"#,
            opt(acted.map(|refusal| refusal.view()))
        )),
        opt(dialog),
    ]);
    (name, page)
}

/// One composition.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let uid = uid_param();
    match load(ehr_id.clone(), uid.clone()).await {
        Ok(CompositionModel::Shown(composition)) => {
            let (title, body) = composition_view(&ehr_id, &uid, *composition);
            Drawn::page(title, body)
        }
        Ok(CompositionModel::Missing(refusal)) => {
            let (title, body) = missing(&ehr_id, &uid, &refusal);
            Drawn::page(title, body)
        }
        Err(error) => Drawn::page(
            tr(RecordsMsg::CompositionRefused),
            drawn(Err::<(), _>(error), |()| Html::empty()),
        ),
    }
}

/// Whether the Tags tab is offered: on the latest version of a composition
/// that is not deleted.
const fn tags_allowed(deleted: bool, reading: Option<&str>) -> bool {
    !deleted && reading.is_none()
}

/// The editor of a composition document: its toolbar, its gutter marks and
/// the CDR's findings beside it.
fn document_editor(
    text: String,
    opened: Option<String>,
    language: EditorLanguage,
    findings: Option<(String, ferrohealth_ui::view::diagnostics::FileFindings)>,
    label: String,
) -> Html {
    let lead = match language {
        EditorLanguage::Xml => tr(RecordsMsg::EditorXml),
        _ => tr(RecordsMsg::EditorJson),
    };
    code_editor(EditorSetup {
        field: EditorField::Document,
        language,
        text,
        opened,
        marks: marks_of(findings.as_ref()),
        label,
        lead,
        facts: vec![tr(Msg::EditorUtf8)],
        findings,
    })
}

/// A save the CDR refused.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Unsaved {
    /// The refusal.
    refusal: Refused,
    /// Whether another client changed the composition meanwhile (`412`).
    conflict: bool,
}

/// What Edit as a new version's form holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    /// The latest version's document, pretty-printed.
    current: String,
    /// The id of the latest version.
    latest: String,
    /// The id of the version the edit is based on.
    opened: String,
    /// The document as edited.
    text: String,
    /// The composition's name.
    name: String,
    /// The save the CDR refused, when the post was one.
    unsaved: Option<Unsaved>,
}

/// Edit as a new version's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EditModel {
    /// The form.
    Editor(Box<Editor>),
    /// The composition is deleted.
    Deleted,
    /// The CDR's refusal.
    Refused(Refused),
    /// The post redirected.
    Left,
}

/// Loads Edit as a new version: answers its post (Discard, Save) over the
/// latest version.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load_edit(
    // The EHR the route names.
    ehr_id: String,
    // The id the route names.
    uid: String,
) -> Result<EditModel, ViewerError> {
    crate::session::require_session()?;
    Ok(read_edit(ehr_id, uid).await)
}

/// What [`load_edit`] reads.
async fn read_edit(ehr_id: String, uid: String) -> EditModel {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect(&composition_href(&ehr_id, &uid));
        return EditModel::Left;
    }
    let current =
        match fetch_composition(ehr_id.clone(), uid.clone(), ReprFormat::CanonicalJson).await {
            Ok(Some(body)) => crate::bff::pretty_body(&body, ReprFormat::CanonicalJson),
            Ok(None) => return EditModel::Deleted,
            Err(error) => {
                return EditModel::Refused(Refused::new(tr(RecordsMsg::CompositionRefused), error));
            }
        };
    let latest = match fetch_versions(ehr_id.clone(), uid.clone()).await {
        Ok(versions) => versions
            .into_iter()
            .max_by_key(|version| {
                version_number(&version.version_id)
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .map(|version| version.version_id)
            .unwrap_or_default(),
        Err(error) => {
            return EditModel::Refused(Refused::new(
                tr(RecordsMsg::CompositionHistoryRefused),
                error,
            ));
        }
    };
    let opened = posted
        .as_ref()
        .map(|posted| posted.text("version"))
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| latest.clone());
    let text = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Document.name()))
        .unwrap_or_else(|| current.clone());
    let head = Head::of(&current);
    let mut unsaved = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        match update_composition(ehr_id.clone(), uid.clone(), opened.clone(), text.clone()).await {
            Ok(saved) => {
                redirect_with(
                    &composition_href(&ehr_id, &uid),
                    &Done::ok(tr(RecordsMsg::CompositionSaved)).line(tr_with(
                        RecordsMsg::CompositionSavedVersion,
                        &[("version", version_number(&saved).into())],
                    )),
                );
                return EditModel::Left;
            }
            Err(error) => {
                unsaved = Some(Unsaved {
                    conflict: is_status(&error, http::StatusCode::PRECONDITION_FAILED),
                    refusal: Refused::new(tr(RecordsMsg::CompositionSaveRefused), error),
                });
            }
        }
    }
    EditModel::Editor(Box::new(Editor {
        current,
        latest,
        opened,
        text,
        name: head.name,
        unsaved,
    }))
}

/// Edit as a new version's page, its decision bar docked.
fn edit_page(ehr_id: &str, uid: &str, editor: Editor) -> (String, Html) {
    let Editor {
        current,
        latest,
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
            h(format!(
                r#"<div class="fui-cluster">{}<span class="fui-small fui-muted">{}</span></div>"#,
                ferrohealth_ui::view::button::link_button(
                    format!("{}/edit", composition_href(ehr_id, uid)),
                    ferrohealth_ui::view::button::Variant::Secondary,
                    Some(Glyph::REFRESH_CW),
                    tr_with(
                        RecordsMsg::EditorReloadVersion,
                        &[("version", version_number(&latest).into())],
                    ),
                ),
                t(tr(RecordsMsg::EditorReloadReplaces))
            ))
        });
        unsaved.refusal.view().then(opt(reload))
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
    let mut trail = crumbs(ehr_id);
    trail.push(Crumb::new(
        if name.is_empty() {
            shortened(uid)
        } else {
            name.clone()
        },
        composition_href(ehr_id, uid),
    ));
    let heading = editor_heading(
        &crate::locale::current(),
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
    let form = h(format!(
        r#"<form id="{EDIT_FORM}" method="post" action="{action}"><input type="hidden" name="version" value="{version}">{editor}</form>"#,
        action = t(format!("{}/edit", composition_href(ehr_id, uid))),
        version = t(&opened),
        editor = document_editor(
            text,
            Some(current),
            EditorLanguage::Json,
            findings,
            tr(RecordsMsg::CompositionEditorLabel),
        ),
    ));
    (
        tr_with(Msg::CompositionEditTitle, &[("name", name.into())]),
        heading.then(h(format!(
            r#"<div class="fui-stack">{}{form}</div>"#,
            opt(top)
        ))),
    )
}

/// Edit as a new version.
pub async fn edit_composition_page() -> Drawn {
    let ehr_id = ehr_id_param();
    let uid = uid_param();
    let (title, body) = match load_edit(ehr_id.clone(), uid.clone()).await {
        Ok(EditModel::Editor(editor)) => edit_page(&ehr_id, &uid, *editor),
        Ok(EditModel::Deleted) => (
            tr(RecordsMsg::CompositionDeletedNoEdit),
            info_note(t(tr(RecordsMsg::CompositionDeletedNoEdit))),
        ),
        Ok(EditModel::Refused(refusal)) => (tr(RecordsMsg::CompositionRefused), refusal.view()),
        Ok(EditModel::Left) => (String::new(), Html::empty()),
        Err(error) => (
            tr(RecordsMsg::CompositionRefused),
            drawn(Err::<(), _>(error), |()| Html::empty()),
        ),
    };
    Drawn::page(title, body)
}

/// The formats New composition commits in.
const NEW_FORMATS: [(&str, ReprFormat); 3] = [
    ("json", ReprFormat::CanonicalJson),
    ("xml", ReprFormat::CanonicalXml),
    ("flat", ReprFormat::Flat),
];

/// What New composition's form holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NewForm {
    /// The key of the format it commits in.
    format_key: String,
    /// The template it follows.
    template: String,
    /// The document.
    text: String,
    /// The commit the CDR refused, when the post was one.
    refused: Option<Refused>,
    /// The composition templates' ids, or the CDR's refusal of them.
    templates: Result<Vec<String>, Refused>,
}

/// New composition's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NewModel {
    /// The form.
    Form(Box<NewForm>),
    /// The post redirected.
    Left,
}

/// The format the key `format_key` names, canonical JSON for none.
fn new_format(format_key: &str) -> ReprFormat {
    NEW_FORMATS
        .iter()
        .find(|(key, _)| *key == format_key)
        .map_or(ReprFormat::CanonicalJson, |(_, format)| *format)
}

/// Loads New composition: answers its post (Discard, Commit) and reads the
/// templates.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load_new(
    // The EHR the route names.
    ehr_id: String,
) -> Result<NewModel, ViewerError> {
    crate::session::require_session()?;
    Ok(read_new(ehr_id).await)
}

/// What [`load_new`] reads.
async fn read_new(ehr_id: String) -> NewModel {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect(&ehr_href(&ehr_id));
        return NewModel::Left;
    }
    let format_key = posted
        .as_ref()
        .map(|posted| posted.text("format"))
        .filter(|key| NEW_FORMATS.iter().any(|(known, _)| known == key))
        .unwrap_or_else(|| "json".to_owned());
    let template = posted
        .as_ref()
        .map(|posted| posted.text("template"))
        .or_else(|| crate::request::query("template"))
        .unwrap_or_default();
    let text = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Document.name()))
        .unwrap_or_default();
    let mut refused = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        match crate::bff::ehr_detail::compositions::commit_composition(
            ehr_id.clone(),
            new_format(&format_key),
            template.clone(),
            text.clone(),
        )
        .await
        {
            Ok(uid) => {
                let object = crate::uid::container_uid_of(&uid);
                redirect_with(
                    &composition_href(&ehr_id, &object),
                    &Done::ok(tr(RecordsMsg::CompositionCommitted)).line(shortened(&object)),
                );
                return NewModel::Left;
            }
            Err(error) => {
                refused = Some(Refused::new(
                    tr(RecordsMsg::CompositionCommitRefused),
                    error,
                ));
            }
        }
    }
    let templates = crate::bff::templates::list_templates()
        .await
        .map(|rows| {
            let mut ids: Vec<String> = rows
                .into_iter()
                .filter(crate::bff::templates::TemplateRow::is_composition)
                .map(|row| row.template_id)
                .collect();
            ids.sort();
            ids.dedup();
            ids
        })
        .map_err(|error| Refused::new(tr(RecordsMsg::CompositionTemplatesRefused), error));
    NewModel::Form(Box::new(NewForm {
        format_key,
        template,
        text,
        refused,
        templates,
    }))
}

/// New composition's page, its decision bar docked.
fn new_form(ehr_id: &str, form: NewForm) -> Html {
    let NewForm {
        format_key,
        template,
        text,
        refused,
        templates,
    } = form;
    let format = new_format(&format_key);
    let field = EditorField::Document.name();
    let findings = refused
        .as_ref()
        .and_then(|refusal| findings_of(&refusal.error, &text, &field));
    let line = if refused.is_some() {
        (Glyph::CIRCLE_X, tr(RecordsMsg::CompositionNotCommitted))
    } else {
        (Glyph::FILE_PLUS, tr(RecordsMsg::CompositionNotYet))
    };
    let template_field = match templates {
        Ok(ids) => {
            let options: Html = ids
                .into_iter()
                .map(|id| {
                    h(format!(
                        r#"<option value="{value}"{selected}>{value}</option>"#,
                        value = t(&id),
                        selected = crate::markup::flag("selected", id == template),
                    ))
                })
                .collect();
            Field::new("nt", tr(Msg::FilterTemplate))
                .help(tr(RecordsMsg::CompositionTemplateHelp))
                .view(h(format!(
                    r#"<select id="nt" name="template" class="fui-select"><option value="">{}</option>{options}</select>"#,
                    t(tr(RecordsMsg::StagedFromDocument))
                )))
        }
        Err(refusal) => refusal.view(),
    };
    let formats: Vec<(&'static str, String)> = NEW_FORMATS
        .iter()
        .map(|(key, format)| (*key, format.label().to_owned()))
        .collect();
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: tr(RecordsMsg::CompositionCommit),
        blocked: None,
    });
    let heading = editor_heading(
        &current(),
        message(RecordsMsg::CompositionNewTitle),
        &Mode::Creating,
        Header {
            title: String::new().into(),
            subtitle: Some(tr(RecordsMsg::CompositionNewSubtitle).into()),
            crumbs: crumbs(ehr_id),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let language = if format == ReprFormat::CanonicalXml {
        EditorLanguage::Xml
    } else {
        EditorLanguage::Json
    };
    let choices = card_panel(section(
        None,
        None,
        None,
        h(format!(
            r#"<div class="fui-form-grid"><div class="fui-span-2">{template_field}</div><div class="fui-span-2">{}</div></div>"#,
            choice_group("format", tr(Msg::CompositionFormat), &formats, &format_key,)
        )),
    ));
    let form = h(format!(
        r#"<form id="{EDIT_FORM}" method="post" action="{action}">{choices}{editor}</form>"#,
        action = t(format!("{}/compositions/new", ehr_href(ehr_id))),
        editor = document_editor(
            text,
            None,
            language,
            findings,
            tr(RecordsMsg::CompositionNewEditorLabel),
        ),
    ));
    heading.then(h(format!(
        r#"<div class="fui-stack">{}{form}</div>"#,
        opt(refused.map(|refusal| refusal.view()))
    )))
}

/// New composition.
pub async fn new_page() -> Drawn {
    let ehr_id = ehr_id_param();
    let body = drawn(load_new(ehr_id.clone()).await, |model| match model {
        NewModel::Form(form) => new_form(&ehr_id, *form),
        NewModel::Left => Html::empty(),
    });
    Drawn::page(tr(RecordsMsg::CompositionNewPageTitle), body)
}

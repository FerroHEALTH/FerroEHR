// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Status tab and Edit status (FerroUI `design/consoles/ferroehr/`,
//! pages 12.12 to 12.14 and 13).
//!
//! The tab shows the `EHR_STATUS` version in view: its facts, its document as
//! the CDR sent it with the subject id masked as in the header, and its
//! versions (E2) with At a time (ITS-REST `ehr_status_get_at_time` and
//! `versioned_ehr_status`). An older version opens read-only with one notice
//! and the way to the latest. Edit status changes `is_queryable`,
//! `is_modifiable` and `other_details` and sends every other attribute back
//! verbatim ([`crate::bff::ehr_detail::status::edit`]); the save carries the
//! version it was based on as `If-Match`, so a status another client changed
//! meanwhile is refused (`412`), never overwritten. No openEHR spec governs
//! the viewer's pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::diagnostics::DiagnosticsWords;
use ferrohealth_ui::view::form::keyed_check_field;
use ferrohealth_ui::view::surface::{Crumb, Header, Mode, definitions, editor_heading};
use ferrohealth_ui::view::versions::{
    Standing, Version, VersionHistory, older_version, version_history,
};
use ferrohealth_ui::widget::editor::EditorWords;
#[cfg(not(target_arch = "wasm32"))]
use ferrohealth_ui::widget::identifier::{Domain, KEEP, Masked};
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::bff::ehr_detail::status::{
    EhrStatusState, fetch_ehr_status, fetch_ehr_status_version, fetch_status_revision_history,
    fetch_status_version_at_time, fetch_versioned_status, parse_status_state, update_ehr_status,
};
use crate::bff::wire::{VersionEntry, VersionedObjectFacts};
#[cfg(not(target_arch = "wasm32"))]
use crate::flash::{Done, redirect_with};
use crate::islands::{CodeEditor, EditorField, EditorLanguage, EditorSetup};
use crate::locale::{Msg, message, tr, tr_with};
use crate::pages::common::{
    EditorDock, Refused, ago, at_time_row, card_panel, dock_editor, docked, document_section,
    drawn, page_model, panel, plain_section, section, short, shortened, version_number,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::pages::common::{is_status, version_uid_at};
#[cfg(not(target_arch = "wasm32"))]
use crate::pages::ehr::EhrModel;
use crate::pages::ehr::{EhrTab, crumbs, ehr_id_param, ehr_view, tab_href};

#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's JSON wire to mask the subject in the document it shows"
)]
/// The document with its subject id masked as the header masks it.
fn masked_document(body: &str) -> String {
    use serde_json::Value;
    let Ok(mut doc) = serde_json::from_str::<Value>(body) else {
        return body.to_owned();
    };
    let Some(id) = doc
        .pointer_mut("/subject/external_ref/id/value")
        .filter(|value| value.is_string())
    else {
        return crate::bff::pretty_body(body, crate::format::ReprFormat::CanonicalJson);
    };
    let value = id.as_str().unwrap_or_default().to_owned();
    *id = Value::String(Masked::new(Domain::new(String::new()), &value, KEEP).drawn());
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| body.to_owned())
}

/// The version history (E2) of the status, the version in view marked.
fn history(ehr_id: &str, versions: &[VersionEntry], reading: Option<&str>) -> AnyView {
    let latest = versions.first().map(|version| version.version_id.clone());
    let rows = versions
        .iter()
        .map(|version| {
            let number = version_number(&version.version_id);
            let current = Some(&version.version_id) == latest.as_ref();
            Version {
                href: Some(if current {
                    tab_href(ehr_id, EhrTab::Status)
                } else {
                    format!("{}?version={number}", tab_href(ehr_id, EhrTab::Status))
                }),
                standing: if current {
                    Standing::Current
                } else {
                    Standing::Older
                },
                read: reading == Some(number.as_str()),
                number,
                change: version.change_type.clone(),
                committer: version.committer.clone(),
                when: ago(&version.committed),
                contribution: tr(Msg::NoneShown).into_any(),
            }
        })
        .collect();
    version_history(VersionHistory {
        caption: tr(Msg::StatusVersions),
        versions: rows,
        at_time: Some(at_time_row(
            tab_href(ehr_id, EhrTab::Status),
            &crate::request::query("at").unwrap_or_default(),
            tr(Msg::StatusAtTimeOpens),
        )),
    })
}

/// The status version the tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Shown {
    /// The version history, or the CDR's refusal of it.
    versions: Result<Vec<VersionEntry>, Refused>,
    /// The number of the latest version.
    latest_number: String,
    /// The number of the version asked for, when one was.
    older: Option<String>,
    /// The id of the version shown.
    version_uid: String,
    /// Its envelope facts, or the CDR's refusal of them.
    facts: Result<VersionedObjectFacts, Refused>,
    /// Its document, the subject id masked.
    document: String,
    /// Whether it lets the EHR be queried.
    is_queryable: bool,
}

/// The Status tab's model: the version shown, or why none is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum StatusPanel {
    /// The version shown.
    Shown(Box<Shown>),
    /// The CDR's refusal of the status or of the version asked for.
    Refused(Refused),
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads the Status tab: the version the request asks for, its facts and the
/// versions.
async fn load_status(ehr_id: String) -> EhrModel<StatusPanel> {
    crate::pages::ehr::load_ehr(&ehr_id, async |facts| {
        let asked = crate::request::query("version");
        let at = crate::request::query("at");
        let current = match facts.status {
            Ok(current) => current,
            Err(error) => {
                return StatusPanel::Refused(Refused::new(tr(Msg::StatusRefused), error));
            }
        };
        let versions = fetch_status_revision_history(ehr_id.clone()).await;
        let latest_number = version_number(&current.version_uid);
        let shown: Result<(EhrStatusState, Option<String>), crate::error::ViewerError> =
            match (&asked, &at) {
                (_, Some(at)) => fetch_status_version_at_time(ehr_id.clone(), at.clone())
                    .await
                    .and_then(|body| parse_status_state(&body, None))
                    .map(|state| {
                        let number = version_number(&state.version_uid);
                        (state, Some(number))
                    }),
                (Some(number), None) if *number != latest_number => {
                    let uid = version_uid_at(&current.version_uid, number);
                    fetch_ehr_status_version(ehr_id.clone(), uid)
                        .await
                        .and_then(|body| parse_status_state(&body, None))
                        .map(|state| (state, Some(number.clone())))
                }
                _ => Ok((current.clone(), None)),
            };
        let (shown, older) = match shown {
            Ok(shown) => shown,
            Err(error) => {
                return StatusPanel::Refused(Refused::new(tr(Msg::StatusVersionRefused), error));
            }
        };
        let facts = fetch_versioned_status(ehr_id.clone(), shown.version_uid.clone())
            .await
            .map_err(|error| Refused::new(tr(Msg::StatusFactsRefused), error));
        StatusPanel::Shown(Box::new(Shown {
            versions: versions.map_err(|error| Refused::new(tr(Msg::StatusHistoryRefused), error)),
            latest_number,
            older,
            document: masked_document(&shown.body),
            version_uid: shown.version_uid,
            facts,
            is_queryable: shown.is_queryable,
        }))
    })
    .await
}

/// The Status tab's panel.
fn status_panel(ehr_id: &str, shown: StatusPanel) -> AnyView {
    let shown = match shown {
        StatusPanel::Shown(shown) => *shown,
        StatusPanel::Refused(refusal) => return panel(plain_section(refusal.view())),
    };
    let Shown {
        versions,
        latest_number,
        older,
        version_uid,
        facts,
        document,
        is_queryable,
    } = shown;
    let count = versions.as_ref().map_or(0, |versions| {
        u32::try_from(versions.len()).unwrap_or(u32::MAX)
    });
    let notice = older
        .as_ref()
        .filter(|number| **number != latest_number)
        .map(|number| {
            plain_section(older_version(
                number,
                count,
                tab_href(ehr_id, EhrTab::Status),
            ))
        });
    let mut rows = vec![(
        tr(Msg::FactVersion),
        version_number(&version_uid).into_any(),
    )];
    if let Ok(facts) = &facts {
        rows.push((tr(Msg::FactContribution), short(&facts.contribution_uid)));
        rows.push((
            tr(Msg::FactSigned),
            tr(if facts.signed { Msg::Yes } else { Msg::No }).into_any(),
        ));
        rows.push((
            tr(Msg::FactLifecycle),
            facts.lifecycle_state.clone().into_any(),
        ));
    }
    let edit = (older.is_none() || older.as_deref() == Some(latest_number.as_str())).then(|| {
        ferrohealth_ui::view::button::link_button(
            format!("{}/edit", tab_href(ehr_id, EhrTab::Status)),
            ferrohealth_ui::view::button::Variant::Secondary,
            Some(Glyph::PENCIL),
            tr(Msg::StatusEdit),
        )
    });
    let refused_facts = facts.err().map(|refusal| refusal.view());
    let this = section(
        Some(tr(Msg::StatusThisVersion)),
        None,
        edit,
        view! {
            {definitions(rows)}
            {refused_facts}
        },
    );
    let document = document_section(
        tr_with(
            Msg::StatusDocumentTitle,
            &[("version", version_number(&version_uid).into())],
        ),
        Language::Json,
        document,
        "status-document",
    );
    let reading = older.as_deref();
    let history = match &versions {
        Ok(versions) => section(
            Some(tr(Msg::StatusVersions)),
            Some(u64::from(count)),
            None,
            history(ehr_id, versions, reading),
        ),
        Err(refusal) => section(Some(tr(Msg::StatusVersions)), None, None, refusal.view()),
    };
    let queryable = (!is_queryable)
        .then(|| plain_section(crate::pages::common::info_note(tr(Msg::StatusNotQueryable))));
    panel(view! {
        {notice}
        {queryable}
        {this}
        {section(
            Some(tr(Msg::StatusDocument)),
            None,
            None,
            view! { {ferrohealth_ui::view::surface::section_intro(tr(Msg::StatusDocumentIntro))} },
        )}
        {document}
        {history}
    })
}

/// An EHR's Status tab.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StatusTab() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let title = tr_with(Msg::StatusTitle, &[("id", shortened(&ehr_id).into())]);
    let model = page_model!(load_status(ehr_id.clone()));
    let body = drawn(model, move |model| {
        ehr_view(&ehr_id, EhrTab::Status, model, |shown| {
            status_panel(&ehr_id, shown)
        })
    });
    view! {
        <Title text=title />
        {body}
    }
}

/// The id of Edit status's form.
const EDIT_FORM: &str = "status-edit";

/// A save the CDR refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Unsaved {
    /// The refusal.
    refusal: Refused,
    /// Whether another client changed the status meanwhile (`412`).
    conflict: bool,
}

/// What Edit status's form holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Editor {
    /// The id of the version the edit is based on.
    opened_uid: String,
    /// The id of the current version.
    current_uid: String,
    /// `is_queryable`.
    queryable: bool,
    /// `is_modifiable`.
    modifiable: bool,
    /// `other_details` as edited.
    details: String,
    /// `other_details` as the current version holds it.
    opened_details: String,
    /// The save the CDR refused, when the post was one.
    unsaved: Option<Unsaved>,
}

/// Edit status's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum EditModel {
    /// The form.
    Editor(Box<Editor>),
    /// The CDR's refusal of the current status.
    Refused(Refused),
    /// The post redirected.
    Left,
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads Edit status: answers its post (Discard, Save) and reads the current
/// status.
async fn load_edit(ehr_id: String) -> EditModel {
    let posted = crate::posted::posted();
    if crate::pages::common::discarded(posted.as_ref()) {
        leptos_axum::redirect(&tab_href(&ehr_id, EhrTab::Status));
        return EditModel::Left;
    }
    let current = match fetch_ehr_status(ehr_id.clone()).await {
        Ok(current) => current,
        Err(error) => return EditModel::Refused(Refused::new(tr(Msg::StatusRefused), error)),
    };
    let opened_uid = posted
        .as_ref()
        .map(|posted| posted.text("version"))
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| current.version_uid.clone());
    let (queryable, modifiable, details) = match &posted {
        Some(posted) => (
            posted.checked("is_queryable"),
            posted.checked("is_modifiable"),
            posted.get("other_details").unwrap_or_default(),
        ),
        None => (
            current.is_queryable,
            current.is_modifiable,
            current.other_details.clone(),
        ),
    };
    let mut unsaved = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        match update_ehr_status(
            ehr_id.clone(),
            opened_uid.clone(),
            current.body.clone(),
            queryable,
            modifiable,
            details.clone(),
        )
        .await
        {
            Ok(uid) => {
                let line = if uid.is_empty() {
                    tr(Msg::StatusSavedLine)
                } else {
                    tr_with(
                        Msg::StatusSavedVersion,
                        &[("version", version_number(&uid).into())],
                    )
                };
                redirect_with(
                    &tab_href(&ehr_id, EhrTab::Status),
                    &Done::ok(tr(Msg::StatusSaved)).line(line),
                );
                return EditModel::Left;
            }
            Err(error) => {
                unsaved = Some(Unsaved {
                    conflict: is_status(&error, http::StatusCode::PRECONDITION_FAILED),
                    refusal: Refused::new(tr(Msg::StatusSaveRefused), error),
                });
            }
        }
    }
    EditModel::Editor(Box::new(Editor {
        opened_uid,
        current_uid: current.version_uid,
        queryable,
        modifiable,
        details,
        opened_details: current.other_details,
        unsaved,
    }))
}

/// Edit status's form, its decision bar docked.
#[expect(
    clippy::too_many_lines,
    reason = "one editor: its refusal, its docked decisions, its flags and its document"
)]
fn edit_form(ehr_id: &str, editor: Editor) -> AnyView {
    let Editor {
        opened_uid,
        current_uid,
        queryable,
        modifiable,
        details,
        opened_details,
        unsaved,
    } = editor;
    let conflict = unsaved.as_ref().is_some_and(|unsaved| unsaved.conflict);
    let line = if unsaved.is_some() {
        (Glyph::CIRCLE_X, tr(Msg::EditorNotSaved))
    } else {
        (Glyph::PENCIL, tr(Msg::EditorOpened))
    };
    let top = unsaved.map(|unsaved| {
        let reload = unsaved.conflict.then(|| {
            view! {
                <div class="fui-cluster">
                    {ferrohealth_ui::view::button::link_button(
                        format!("{}/edit", tab_href(ehr_id, EhrTab::Status)),
                        ferrohealth_ui::view::button::Variant::Secondary,
                        Some(Glyph::REFRESH_CW),
                        tr_with(
                            Msg::EditorReloadVersion,
                            &[("version", version_number(&current_uid).into())],
                        ),
                    )} <span class="fui-small fui-muted">{tr(Msg::EditorReloadReplaces)}</span>
                </div>
            }
        });
        view! {
            {unsaved.refusal.view()}
            {reload}
        }
        .into_any()
    });
    let next = version_number(&opened_uid).parse::<u32>().map_or_else(
        |_| tr(Msg::EditorSave),
        |number| {
            tr_with(
                Msg::EditorSaveAsVersion,
                &[("version", (number + 1).into())],
            )
        },
    );
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: next,
        blocked: conflict.then(|| tr(Msg::EditorReloadFirst)),
    });
    let flags = view! {
        {keyed_check_field(
            "is_queryable".to_owned(),
            tr(Msg::FlagQueryable),
            tr(Msg::StatusQueryableHelp),
            view! {
                <input type="hidden" name="is_queryable" value="off" />
                <input
                    type="checkbox"
                    id="is_queryable"
                    name="is_queryable"
                    class="fui-checkbox"
                    checked=queryable
                />
            },
        )}
        {keyed_check_field(
            "is_modifiable".to_owned(),
            tr(Msg::FlagModifiable),
            tr(Msg::StatusModifiableHelp),
            view! {
                <input type="hidden" name="is_modifiable" value="off" />
                <input
                    type="checkbox"
                    id="is_modifiable"
                    name="is_modifiable"
                    class="fui-checkbox"
                    checked=modifiable
                />
            },
        )}
    };
    let editor = view! {
        <CodeEditor
            setup=EditorSetup {
                field: EditorField::OtherDetails,
                language: EditorLanguage::Json,
                text: details,
                opened: Some(opened_details),
                marks: Vec::new(),
                label: tr(Msg::StatusOtherDetails),
                lead: tr(Msg::EditorJson),
                facts: vec![tr(Msg::EditorUtf8)],
                findings: None,
            }
            words=EditorWords::of_page()
            diagnostics=DiagnosticsWords::of_page()
        />
    };
    view! {
        {top}
        <form
            id=EDIT_FORM
            method="post"
            action=format!("{}/edit", tab_href(ehr_id, EhrTab::Status))
        >
            <input type="hidden" name="version" value=opened_uid />
            {card_panel(
                view! {
                    {section(Some(tr(Msg::StatusCapabilities)), None, None, flags)}
                    {section(
                        Some(tr(Msg::StatusOtherDetails)),
                        None,
                        None,
                        view! {
                            {ferrohealth_ui::view::surface::section_intro(
                                tr(Msg::StatusOtherDetailsIntro),
                            )}
                            {editor}
                        },
                    )}
                },
            )}
        </form>
    }
    .into_any()
}

/// Edit status.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn StatusEditPage() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let dock_promise = crate::shell::promise_dock();
    let model = page_model!(load_edit(ehr_id.clone()));
    let body = docked(model, dock_promise, {
        let ehr_id = ehr_id.clone();
        move |model| match model {
            EditModel::Editor(editor) => edit_form(&ehr_id, *editor),
            EditModel::Refused(refusal) => refusal.view(),
            EditModel::Left => ().into_any(),
        }
    });
    let mut trail = crumbs(&ehr_id);
    trail.push(Crumb::new(
        tr(Msg::EhrTabStatus),
        tab_href(&ehr_id, EhrTab::Status),
    ));
    let heading = editor_heading(
        message(Msg::EditorTitle),
        &Mode::Editing(tr(Msg::EhrTabStatus)),
        Header {
            title: String::new().into(),
            subtitle: Some(tr(Msg::StatusEditSubtitle).into()),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr(Msg::StatusEditTitle) />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::{Shown, StatusPanel, status_panel};
    use crate::error::ViewerError;
    use crate::locale::{Msg, tr};
    use crate::pages::common::Refused;

    #[test]
    fn a_refused_envelope_draws_its_refusal_where_the_facts_would_be() {
        let html = leptos::prelude::Owner::new().with(|| {
            let shown = Shown {
                versions: Err(Refused::new(
                    tr(Msg::StatusHistoryRefused),
                    ViewerError::Internal("history".to_owned()),
                )),
                latest_number: "1".to_owned(),
                older: None,
                version_uid: "8849182c-82ad-4088-a07f-48ead4180515::ferroehr::1".to_owned(),
                facts: Err(Refused::new(
                    tr(Msg::StatusFactsRefused),
                    ViewerError::Cdr {
                        status: 503,
                        message: "the versioned status is unavailable".to_owned(),
                    },
                )),
                document: "{}".to_owned(),
                is_queryable: true,
            };
            leptos::prelude::RenderHtml::to_html(status_panel(
                "8849182c-82ad-4088-a07f-48ead4180515",
                StatusPanel::Shown(Box::new(shown)),
            ))
        });
        assert!(
            html.contains(&tr(Msg::StatusFactsRefused)),
            "the refusal's summary is drawn: {html}"
        );
        assert!(
            html.contains("the versioned status is unavailable"),
            "the CDR's own words are drawn: {html}"
        );
        assert!(
            !html.contains(&tr(Msg::FactLifecycle)),
            "no envelope fact is drawn as if read: {html}"
        );
    }
}

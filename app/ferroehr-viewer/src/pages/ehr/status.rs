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

#![expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's JSON wire to mask the subject in the document it shows"
)]

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
use ferrohealth_ui::widget::identifier::{Domain, KEEP, Masked};
use leptos::prelude::*;
use leptos_meta::Title;
use serde_json::Value;

use crate::bff::ehr_detail::status::{
    EhrStatusState, fetch_ehr_status, fetch_ehr_status_version, fetch_status_revision_history,
    fetch_status_version_at_time, fetch_versioned_status, parse_status_state, update_ehr_status,
};
use crate::bff::wire::VersionEntry;
use crate::flash::{Done, redirect_with};
use crate::islands::{CodeEditor, EditorField, EditorLanguage, EditorSetup};
use crate::locale::{Msg, message, tr, tr_with};
use crate::pages::common::{
    EditorDock, ago, at_time_row, awaited, card_panel, dock_editor, document_section, is_status,
    panel, plain_section, refused, section, short, shortened, version_number, version_uid_at,
};
use crate::pages::ehr::{EhrTab, crumbs, ehr_id_param, ehr_page, tab_href};

/// The document with its subject id masked as the header masks it.
fn masked_document(body: &str) -> String {
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

/// An EHR's Status tab.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn StatusTab() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let asked = crate::request::query("version");
    let at = crate::request::query("at");
    let title = tr_with(Msg::StatusTitle, &[("id", shortened(&ehr_id).into())]);
    let body = awaited({
        let ehr_id = ehr_id.clone();
        async move {
            ehr_page(ehr_id.clone(), EhrTab::Status, move |facts| async move {
                let current = match facts.status {
                    Ok(current) => current,
                    Err(error) => {
                        return panel(plain_section(refused(tr(Msg::StatusRefused), &error, None)));
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
                        return panel(plain_section(refused(
                            tr(Msg::StatusVersionRefused),
                            &error,
                            None,
                        )));
                    }
                };
                let facts_of =
                    fetch_versioned_status(ehr_id.clone(), shown.version_uid.clone()).await;
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
                            tab_href(&ehr_id, EhrTab::Status),
                        ))
                    });
                let mut rows = vec![(
                    tr(Msg::FactVersion),
                    version_number(&shown.version_uid).into_any(),
                )];
                if let Ok(facts) = &facts_of {
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
                let edit = (older.is_none() || older.as_deref() == Some(latest_number.as_str()))
                    .then(|| {
                        ferrohealth_ui::view::button::link_button(
                            format!("{}/edit", tab_href(&ehr_id, EhrTab::Status)),
                            ferrohealth_ui::view::button::Variant::Secondary,
                            Some(Glyph::PENCIL),
                            tr(Msg::StatusEdit),
                        )
                    });
                let this = section(
                    Some(tr(Msg::StatusThisVersion)),
                    None,
                    edit,
                    definitions(rows),
                );
                let document = document_section(
                    tr_with(
                        Msg::StatusDocumentTitle,
                        &[("version", version_number(&shown.version_uid).into())],
                    ),
                    Language::Json,
                    masked_document(&shown.body),
                    "status-document",
                );
                let reading = older.as_deref();
                let history = match &versions {
                    Ok(versions) => section(
                        Some(tr(Msg::StatusVersions)),
                        Some(u64::from(count)),
                        None,
                        history(&ehr_id, versions, reading),
                    ),
                    Err(error) => section(
                        Some(tr(Msg::StatusVersions)),
                        None,
                        None,
                        refused(tr(Msg::StatusHistoryRefused), error, None),
                    ),
                };
                let queryable = (!shown.is_queryable).then(|| {
                    plain_section(crate::pages::common::info_note(tr(Msg::StatusNotQueryable)))
                });
                panel(view! {
                    {notice}
                    {queryable}
                    {this}
                    {section(
                        Some(tr(Msg::StatusDocument)),
                        None,
                        None,
                        view! {
                            {ferrohealth_ui::view::surface::section_intro(
                                tr(Msg::StatusDocumentIntro),
                            )}
                        },
                    )}
                    {document}
                    {history}
                })
            })
            .await
        }
    });
    view! {
        <Title text=title />
        {body}
    }
}

/// The id of Edit status's form.
const EDIT_FORM: &str = "status-edit";

/// Edit status.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn StatusEditPage() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let posted = crate::posted::posted();
    if crate::pages::common::discarded(posted.as_ref()) {
        leptos_axum::redirect(&tab_href(&ehr_id, EhrTab::Status));
    }
    let dock_promise = crate::shell::promise_dock();
    let body = awaited({
        let ehr_id = ehr_id.clone();
        async move {
            let _dock_promise = dock_promise;
            let current = match fetch_ehr_status(ehr_id.clone()).await {
                Ok(current) => current,
                Err(error) => return refused(tr(Msg::StatusRefused), &error, None),
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
            let mut top = None;
            let mut blocked = None;
            let mut line = (Glyph::PENCIL, tr(Msg::EditorOpened));
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
                        return ().into_any();
                    }
                    Err(error) => {
                        line = (Glyph::CIRCLE_X, tr(Msg::EditorNotSaved));
                        let conflict = is_status(&error, http::StatusCode::PRECONDITION_FAILED);
                        if conflict {
                            blocked = Some(tr(Msg::EditorReloadFirst));
                        }
                        let reload = conflict.then(|| {
                            view! {
                                <div class="fui-cluster">
                                    {ferrohealth_ui::view::button::link_button(
                                        format!("{}/edit", tab_href(&ehr_id, EhrTab::Status)),
                                        ferrohealth_ui::view::button::Variant::Secondary,
                                        Some(Glyph::REFRESH_CW),
                                        tr_with(
                                            Msg::EditorReloadVersion,
                                            &[("version", version_number(&current.version_uid).into())],
                                        ),
                                    )}
                                    <span class="fui-small fui-muted">
                                        {tr(Msg::EditorReloadReplaces)}
                                    </span>
                                </div>
                            }
                        });
                        top = Some(
                            view! {
                                {refused(tr(Msg::StatusSaveRefused), &error, None)}
                                {reload}
                            }
                            .into_any(),
                        );
                    }
                }
            }
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
                blocked,
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
                        text: details.clone(),
                        opened: Some(current.other_details.clone()),
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
                    action=format!("{}/edit", tab_href(&ehr_id, EhrTab::Status))
                >
                    <input type="hidden" name="version" value=opened_uid.clone() />
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

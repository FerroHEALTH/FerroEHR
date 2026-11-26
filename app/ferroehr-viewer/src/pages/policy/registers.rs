// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The admin registers in the Access policy area: the audit category map and
//! the access delay, each its version drawn by its members, its history back
//! to version 0, and its editor.
//!
//! A register is read and written whole over its version
//! ([`crate::bff::registers`]); the save names the version read in
//! `If-Match`, so a register another administrator changed meanwhile is
//! refused (`412`). No openEHR specification governs the registers: our own
//! design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action};
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, section_intro,
};

use crate::bff::registers::{
    Register, RegisterVersion, fetch_register, fetch_register_history, put_register,
};
use crate::components::{EditorField, EditorLanguage, EditorSetup, code_editor};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::policy::PolicyMsg;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    EditorDock, Refused, card_panel, discarded, dock_editor, document_section, findings_of,
    marks_of, panel, plain_section, section, when,
};
use crate::pages::editor::{Unsaved, editor_line};
use crate::pages::form::Form;
use crate::pages::policy::{PolicyTab, frame, shown};
use crate::routes::Drawn;

/// The page of `register`.
#[must_use]
pub fn register_href(register: Register) -> String {
    format!("/access-policy/registers/{}", register.key())
}

/// The words a register goes by.
#[must_use]
pub fn register_words(register: Register) -> String {
    tr(match register {
        Register::AuditCategories => PolicyMsg::RegisterAuditCategories,
        Register::AccessDelay => PolicyMsg::RegisterAccessDelay,
    })
}

/// The refusal of a read of a register, worded for a `403` and a `405`.
fn refusal_of(error: ViewerError) -> Refused {
    let summary = if matches!(error, ViewerError::Forbidden(_)) {
        PolicyMsg::RegisterForbidden
    } else if crate::pages::common::is_status(&error, http::StatusCode::METHOD_NOT_ALLOWED) {
        PolicyMsg::AdminOff
    } else {
        PolicyMsg::PolicyRefused
    };
    Refused::new(tr(summary), error)
}

/// The register the route names.
fn register_param() -> Option<Register> {
    crate::routes::params()
        .get("register")
        .and_then(|key| Register::parse(&key))
}

/// The register page's model.
#[derive(Debug, Clone, PartialEq)]
struct Read {
    /// The version shown.
    shown: RegisterVersion,
    /// Whether it is the newest.
    newest: bool,
    /// Every version, newest first, or the CDR's refusal of them.
    history: Result<Vec<RegisterVersion>, Refused>,
}

/// What a register page reads: the newest version, or the version asked
/// for from the history.
async fn read(register: Register) -> Result<Read, Refused> {
    let newest = fetch_register(register).await.map_err(refusal_of)?;
    let history = fetch_register_history(register).await.map_err(refusal_of);
    let asked = crate::request::query("version").and_then(|number| number.parse::<i64>().ok());
    let older = asked
        .filter(|number| *number != newest.version)
        .and_then(|number| {
            history
                .as_ref()
                .ok()
                .and_then(|versions| versions.iter().find(|version| version.version == number))
                .cloned()
        });
    Ok(match older {
        Some(shown) => Read {
            shown,
            newest: false,
            history,
        },
        None => Read {
            shown: newest,
            newest: true,
            history,
        },
    })
}

/// A version's facts: its number, committer and time; the default says it
/// is the default.
fn version_facts(version: &RegisterVersion) -> Html {
    let committer = version
        .committer
        .clone()
        .unwrap_or_else(|| tr(PolicyMsg::RegisterDefault));
    definitions(vec![
        (tr(PolicyMsg::FactVersion), t(version.version.to_string())),
        (tr(PolicyMsg::FactCommitter), t(committer)),
        (
            tr(PolicyMsg::FactCommitted),
            version
                .committed_at
                .as_deref()
                .map_or_else(|| t(tr(PolicyMsg::RegisterDefault)), when),
        ),
    ])
}

/// The history, newest first, each version opening read-only.
fn history_table(register: Register, versions: &[RegisterVersion], shown: i64) -> Html {
    let columns = [
        Column::text("version", tr(PolicyMsg::FactVersion)).role(Role::Title),
        Column::text("committer", tr(PolicyMsg::FactCommitter)).role(Role::Sub),
        Column::text("committed", tr(PolicyMsg::FactCommitted)).role(Role::Meta),
    ];
    let rows = versions
        .iter()
        .map(|version| {
            let href = format!("{}?version={}", register_href(register), version.version);
            let words = if version.version == shown {
                tr_with(
                    PolicyMsg::RegisterVersionShown,
                    &[("version", version.version.to_string().into())],
                )
            } else {
                version.version.to_string()
            };
            h(format!(
                "<tr>{}{}{}</tr>",
                title_cell(None, Some(&href), t(words), None),
                cell(
                    &columns[1],
                    t(version
                        .committer
                        .clone()
                        .unwrap_or_else(|| tr(PolicyMsg::RegisterDefault)))
                ),
                cell(
                    &columns[2],
                    version
                        .committed_at
                        .as_deref()
                        .map_or_else(Html::empty, when)
                ),
            ))
        })
        .collect();
    let caption = tr(PolicyMsg::VersionsCaption);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// A register page's panel.
fn register_panel(register: Register, read: Result<Read, Refused>) -> (ActionGroup, Html) {
    let Read {
        shown: version,
        newest,
        history,
    } = match read {
        Ok(read) => read,
        Err(refusal) => return (ActionGroup::default(), panel(plain_section(refusal.view()))),
    };
    let older = (!newest).then(|| {
        plain_section(crate::pages::common::info_note(t(tr(
            PolicyMsg::RegisterOlder,
        ))))
    });
    let members = definitions(
        version
            .members
            .iter()
            .map(|(key, value)| (crate::pages::configuration::label_of(key), shown(value)))
            .collect(),
    );
    let history = match &history {
        Ok(versions) => history_table(register, versions, version.version),
        Err(refusal) => refusal.view(),
    };
    let intro = match register {
        Register::AuditCategories => PolicyMsg::RegisterAuditCategoriesIntro,
        Register::AccessDelay => PolicyMsg::RegisterAccessDelayIntro,
    };
    let actions = if newest {
        ActionGroup::primary(Action::new(
            Some(Glyph::PENCIL),
            tr(PolicyMsg::RegisterEdit),
            Act::Link(format!("{}/edit", register_href(register))),
        ))
    } else {
        ActionGroup::default()
    };
    let body = panel(Html::join([
        opt(older),
        section(
            Some(register_words(register)),
            None,
            None,
            section_intro(tr(intro)).then(version_facts(&version)),
        ),
        section(Some(tr(PolicyMsg::RegisterDocument)), None, None, members),
        section(
            Some(tr(PolicyMsg::VersionsCaption)),
            None,
            None,
            section_intro(tr(PolicyMsg::RegisterHistoryIntro)).then(history),
        ),
        document_section(
            tr_with(
                PolicyMsg::RegisterDocumentTitle,
                &[("version", version.version.to_string().into())],
            ),
            Language::Json,
            version.document.clone(),
            "register-document",
        ),
    ]));
    (actions, body)
}

/// Access policy › a register.
pub async fn page() -> Drawn {
    let Some(register) = register_param() else {
        return crate::pages::errors::not_found_page();
    };
    let (actions, body) = register_panel(register, read(register).await);
    Drawn::page(
        register_words(register),
        frame(PolicyTab::Register(register), actions, body),
    )
}

/// The id of a register editor's form.
const EDIT_FORM: &str = "register-edit";

/// What a register editor holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    /// The version the edit replaces.
    opened: i64,
    /// The newest version now.
    current: i64,
    /// The document as typed.
    text: String,
    /// The document as opened.
    opened_text: String,
    /// The save the CDR refused.
    unsaved: Option<Unsaved>,
}

/// What a register editor reads, and the save its post asks.
async fn read_edit(register: Register) -> Form<Editor> {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect(&register_href(register));
        return Form::Left;
    }
    let newest = match fetch_register(register).await {
        Ok(newest) => newest,
        Err(error) => return Form::Refused(refusal_of(error)),
    };
    let opened = posted
        .as_ref()
        .and_then(|posted| posted.text("version").parse::<i64>().ok())
        .unwrap_or(newest.version);
    let text = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Document.name()))
        .unwrap_or_else(|| newest.document.clone());
    let mut unsaved = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        match put_register(register, opened, text.clone()).await {
            Ok(saved) => {
                redirect_with(
                    &register_href(register),
                    &Done::ok(tr(PolicyMsg::RegisterSaved)).line(tr_with(
                        PolicyMsg::RegisterSavedVersion,
                        &[("version", saved.version.to_string().into())],
                    )),
                );
                return Form::Left;
            }
            Err(error) => unsaved = Some(Unsaved::new(tr(PolicyMsg::RegisterSaveRefused), error)),
        }
    }
    Form::Shown(Box::new(Editor {
        opened,
        current: newest.version,
        text,
        opened_text: newest.document,
        unsaved,
    }))
}

/// A register editor's form, its decision bar docked.
fn edit_form(register: Register, editor: Editor) -> Html {
    let Editor {
        opened,
        current,
        text,
        opened_text,
        unsaved,
    } = editor;
    let field = EditorField::Document.name();
    let findings = unsaved
        .as_ref()
        .and_then(|unsaved| findings_of(&unsaved.refusal.error, &text, &field));
    let conflict = unsaved.as_ref().is_some_and(|unsaved| unsaved.conflict);
    let (glyph, line) = editor_line(unsaved.is_some());
    let action = format!("{}/edit", register_href(register));
    let top = unsaved.map(|unsaved| {
        unsaved.view(
            &action,
            tr_with(
                RecordsMsg::EditorReloadVersion,
                &[("version", current.to_string().into())],
            ),
        )
    });
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line,
        glyph,
        save: tr_with(
            RecordsMsg::EditorSaveAsVersion,
            &[("version", opened.saturating_add(1).to_string().into())],
        ),
        blocked: conflict.then(|| tr(RecordsMsg::EditorReloadFirst)),
    });
    let editor = code_editor(EditorSetup {
        field: EditorField::Document,
        language: EditorLanguage::Json,
        text,
        opened: Some(opened_text),
        marks: marks_of(findings.as_ref()),
        label: register_words(register),
        lead: tr(RecordsMsg::EditorJson),
        facts: vec![tr(Msg::EditorUtf8)],
        findings,
    });
    opt(top).then(h(format!(
        r#"<form id="{EDIT_FORM}" method="post" action="{action}"><input type="hidden" name="version" value="{opened}">{panel}</form>"#,
        action = t(&action),
        panel = card_panel(section(
            Some(tr(PolicyMsg::RegisterDocument)),
            None,
            None,
            section_intro(tr(PolicyMsg::RegisterEditIntro)).then(editor),
        )),
    )))
}

/// Edit a register.
pub async fn edit_page() -> Drawn {
    let Some(register) = register_param() else {
        return crate::pages::errors::not_found_page();
    };
    let body = match read_edit(register).await {
        Form::Shown(editor) => edit_form(register, *editor),
        Form::Refused(refusal) => refusal.view(),
        Form::Not(never) => match never {},
        Form::Left => Html::empty(),
    };
    let heading = editor_heading(
        &current(),
        message(RecordsMsg::EditorTitle),
        &Mode::Editing(register_words(register)),
        Header {
            title: String::new().into(),
            subtitle: Some(tr(PolicyMsg::RegisterEditSubtitle).into()),
            crumbs: vec![
                Crumb::new(tr(Msg::NavAccessPolicy), crate::pages::policy::POLICY_ROUTE),
                Crumb::new(register_words(register), register_href(register)),
            ],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    Drawn::page(
        register_words(register),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Access settings tab and its editor.
//!
//! The tab shows the `EHR_ACCESS` settings in force, drawn by directive, and
//! every version, any of which opens read-only; with no settings the
//! policy's defaults apply. An administrator commits new settings on the
//! person's instruction, naming the version they replace, or repairs
//! settings that do not parse. No openEHR specification governs the tab: our
//! own design over FerroEHR's draft access control package.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, section_intro,
};

use crate::bff::access_policy::Shown;
use crate::bff::ehr_access::{
    AccessSettings, commit_settings, fetch_settings, fetch_settings_history, repair_settings,
};
use crate::bff::wire::VersionEntry;
use crate::components::{EditorField, EditorLanguage, EditorSetup, code_editor};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::policy::PolicyMsg;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{flag, h, opt, t};
use crate::pages::common::{
    EditorDock, Refused, card_panel, discarded, dock_editor, document_section, findings_of,
    info_note, is_status, marks_of, panel, plain_section, section, shortened,
};
use crate::pages::editor::{Unsaved, editor_line, reload_words};
use crate::pages::ehr::{EhrModel, EhrTab, crumbs, ehr_id_param, ehr_view, tab_href};
use crate::pages::form::Form;
use crate::pages::policy::shown;
use crate::pages::versions::History;
use crate::routes::Drawn;
use crate::uid::version_number;

/// The refusal of a read of the settings, worded for a `403`.
fn refusal_of(error: ViewerError) -> Refused {
    let summary = if matches!(error, ViewerError::Forbidden(_)) {
        PolicyMsg::AccessForbidden
    } else {
        PolicyMsg::AccessRefused
    };
    Refused::new(tr(summary), error)
}

/// The tab's model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tab {
    /// The settings shown, `None` while the EHR holds none.
    shown: Option<AccessSettings>,
    /// Whether they are the settings in force.
    current: bool,
    /// Every version, or the CDR's refusal of them.
    versions: Result<Vec<VersionEntry>, Refused>,
}

/// What the tab reads.
async fn read(ehr_id: String) -> EhrModel<Result<Tab, Refused>> {
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let asked = crate::request::query("version").unwrap_or_default();
        let shown = fetch_settings(ehr_id.clone(), asked.clone())
            .await
            .map_err(refusal_of)?;
        let versions = fetch_settings_history(ehr_id.clone())
            .await
            .map_err(refusal_of);
        let newest = versions
            .as_ref()
            .ok()
            .and_then(|versions| versions.first())
            .map(|entry| entry.version_id.clone());
        let current = asked.is_empty() || newest.as_deref() == Some(asked.as_str());
        Ok(Tab {
            shown,
            current,
            versions,
        })
    })
    .await
}

/// The member `key` of a shown object.
fn member<'a>(fields: &'a [(String, Shown)], key: &str) -> Option<&'a Shown> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// The directives as a table: kind, effect, what each covers, whom it
/// names, its period and whether an emergency overrides it.
#[must_use]
pub fn directives_table(directives: &[Shown]) -> Html {
    let columns = [
        Column::text("name", tr(PolicyMsg::DirectiveName)).role(Role::Title),
        Column::text("kind", tr(PolicyMsg::DirectiveKind)).role(Role::Sub),
        Column::text("effect", tr(PolicyMsg::DirectiveEffect)).role(Role::Meta),
        Column::text("data", tr(PolicyMsg::DirectiveCovers)).role(Role::Meta),
        Column::text("applies", tr(PolicyMsg::DirectiveNames)).role(Role::Meta),
        Column::text("period", tr(PolicyMsg::CarePeriod)).role(Role::Wide),
        Column::text("emergency", tr(PolicyMsg::DirectiveEmergency)).role(Role::Wide),
    ];
    let none = || {
        h(format!(
            r#"<span class="fui-muted">{}</span>"#,
            t(tr(PolicyMsg::LeftOutShort))
        ))
    };
    let rows = directives
        .iter()
        .map(|directive| {
            let (name, fields): (String, &[(String, Shown)]) = match directive {
                Shown::Fields { name, fields, .. } => (name.clone().unwrap_or_default(), fields),
                _ => (String::new(), &[]),
            };
            let value = |key: &str| member(fields, key).map_or_else(none, shown);
            h(format!(
                "<tr>{}{}{}{}{}{}{}</tr>",
                title_cell(None, None, t(name), None),
                cell(&columns[1], value("kind")),
                cell(&columns[2], value("effect")),
                cell(&columns[3], value("data")),
                cell(&columns[4], value("applies_to")),
                cell(&columns[5], value("time_validity")),
                cell(&columns[6], value("emergency_override")),
            ))
        })
        .collect();
    let caption = tr(PolicyMsg::AccessDirectives);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The settings drawn: their directives, then every other attribute.
fn settings_view(settings: &AccessSettings) -> Html {
    let directives = match member(&settings.attributes, "directives") {
        Some(Shown::List(items)) if !items.is_empty() => directives_table(items),
        _ => h(format!(
            r#"<p class="fui-small fui-muted">{}</p>"#,
            t(tr(PolicyMsg::AccessNoDirectives))
        )),
    };
    let others: Vec<(String, Html)> = settings
        .attributes
        .iter()
        .filter(|(key, _)| key != "directives")
        .map(|(key, value)| (crate::pages::configuration::label_of(key), shown(value)))
        .collect();
    section(
        Some(tr(PolicyMsg::AccessDirectives)),
        None,
        None,
        section_intro(tr(PolicyMsg::AccessDirectivesIntro)).then(directives),
    )
    .then(if others.is_empty() {
        Html::empty()
    } else {
        section(
            Some(tr(PolicyMsg::SectionOther)),
            None,
            None,
            definitions(others),
        )
    })
}

/// The tab's panel.
fn access_panel(ehr_id: &str, tab: Result<Tab, Refused>) -> Html {
    let Tab {
        shown: settings,
        current,
        versions,
    } = match tab {
        Ok(tab) => tab,
        Err(refusal) => return panel(plain_section(refusal.view())),
    };
    let edit = current.then(|| {
        h(format!(
            r#"<div class="fui-cluster">{}</div>"#,
            ferrohealth_ui::view::button::link_button(
                format!("{}/edit", tab_href(ehr_id, EhrTab::Access)),
                ferrohealth_ui::view::button::Variant::Secondary,
                Some(Glyph::PENCIL),
                tr(PolicyMsg::AccessCommit),
            )
        ))
    });
    let older = (!current).then(|| plain_section(info_note(t(tr(PolicyMsg::AccessOlder)))));
    let body = match &settings {
        None => plain_section(info_note(t(tr(PolicyMsg::AccessNone)))),
        Some(settings) => section(
            Some(tr(PolicyMsg::AccessThis)),
            None,
            edit.clone(),
            definitions(vec![(
                tr(PolicyMsg::FactVersion),
                t(version_number(&settings.version_uid)),
            )]),
        )
        .then(settings_view(settings))
        .then(if settings.settings.is_empty() {
            Html::empty()
        } else {
            document_section(
                tr_with(
                    PolicyMsg::AccessDocument,
                    &[("version", version_number(&settings.version_uid).into())],
                ),
                Language::Json,
                settings.settings.clone(),
                "access-settings",
            )
        }),
    };
    let edit_none = settings.is_none().then(|| plain_section(opt(edit)));
    let tab = tab_href(ehr_id, EhrTab::Access);
    let history = match &versions {
        Ok(versions) if versions.is_empty() => Html::empty(),
        Ok(versions) => section(
            Some(tr(PolicyMsg::VersionsCaption)),
            u64::try_from(versions.len()).ok(),
            None,
            crate::pages::versions::history(
                History {
                    versions,
                    reading: settings
                        .as_ref()
                        .filter(|_| !current)
                        .map(|settings| version_number(&settings.version_uid))
                        .as_deref(),
                    deleted: false,
                    contribution: t(tr(PolicyMsg::OwnContribution)),
                    caption: tr(PolicyMsg::VersionsCaption),
                    at_action: tab.clone(),
                    at_opens: tr(PolicyMsg::AtTimeOpens),
                },
                |number| match number {
                    None => tab.clone(),
                    Some(number) => versions
                        .iter()
                        .find(|entry| version_number(&entry.version_id) == number)
                        .map_or_else(
                            || tab.clone(),
                            |entry| {
                                format!("{tab}?version={}", urlencoding::encode(&entry.version_id))
                            },
                        ),
                },
            ),
        ),
        Err(refusal) => section(
            Some(tr(PolicyMsg::VersionsCaption)),
            None,
            None,
            refusal.view(),
        ),
    };
    panel(Html::join([opt(older), body, opt(edit_none), history]))
}

/// An EHR's Access settings tab.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(PolicyMsg::AccessTitle, &[("id", shortened(&ehr_id).into())]);
    let model = read(ehr_id.clone()).await;
    let body = ehr_view(&ehr_id, EhrTab::Access, model, |tab| {
        access_panel(&ehr_id, tab)
    });
    Drawn::page(title, body)
}

/// The id of the editor's form.
const EDIT_FORM: &str = "access-edit";

/// What the editor holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    /// The version the commit replaces, empty while there is none.
    opened: String,
    /// The version in force now.
    current: String,
    /// The settings as typed.
    text: String,
    /// The settings as opened.
    opened_text: String,
    /// Whether the post repairs settings that do not parse.
    repair: bool,
    /// The write the CDR refused.
    unsaved: Option<Unsaved>,
}

/// The settings an editor opens with: those in force, or `null`.
const NONE: &str = "null";

/// What the editor reads, and the commit or repair its post asks.
async fn read_edit(ehr_id: String) -> Form<Editor> {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect(&tab_href(&ehr_id, EhrTab::Access));
        return Form::Left;
    }
    let (current, opened_text) = match fetch_settings(ehr_id.clone(), String::new()).await {
        Ok(Some(settings)) => {
            let text = if settings.settings.is_empty() {
                NONE.to_owned()
            } else {
                settings.settings
            };
            (settings.version_uid, text)
        }
        Ok(None) => (String::new(), NONE.to_owned()),
        Err(error) if is_status(&error, http::StatusCode::INTERNAL_SERVER_ERROR) => {
            (String::new(), NONE.to_owned())
        }
        Err(error) => return Form::Refused(refusal_of(error)),
    };
    let opened = posted
        .as_ref()
        .map_or_else(|| current.clone(), |posted| posted.text("version"));
    let text = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Document.name()))
        .unwrap_or_else(|| opened_text.clone());
    let repair = posted
        .as_ref()
        .is_some_and(|posted| posted.checked("repair"));
    let mut unsaved = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        let written = if repair {
            repair_settings(ehr_id.clone(), text.clone()).await
        } else {
            commit_settings(ehr_id.clone(), opened.clone(), text.clone()).await
        };
        match written {
            Ok(uid) => {
                redirect_with(
                    &tab_href(&ehr_id, EhrTab::Access),
                    &Done::ok(tr(PolicyMsg::AccessSaved)).line(tr_with(
                        PolicyMsg::SavedVersion,
                        &[("version", version_number(&uid).into())],
                    )),
                );
                return Form::Left;
            }
            Err(error) => unsaved = Some(Unsaved::new(tr(PolicyMsg::AccessSaveRefused), error)),
        }
    }
    Form::Shown(Box::new(Editor {
        opened,
        current,
        text,
        opened_text,
        repair,
        unsaved,
    }))
}

/// The editor's form, its decision bar docked.
fn edit_form(ehr_id: &str, editor: Editor) -> Html {
    let Editor {
        opened,
        current,
        text,
        opened_text,
        repair,
        unsaved,
    } = editor;
    let field = EditorField::Document.name();
    let findings = unsaved
        .as_ref()
        .and_then(|unsaved| findings_of(&unsaved.refusal.error, &text, &field));
    let conflict = unsaved.as_ref().is_some_and(|unsaved| unsaved.conflict);
    let (glyph, line) = editor_line(unsaved.is_some());
    let action = format!("{}/edit", tab_href(ehr_id, EhrTab::Access));
    let top = unsaved.map(|unsaved| unsaved.view(&action, reload_words(&current)));
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line,
        glyph,
        save: tr(PolicyMsg::AccessCommit),
        blocked: conflict.then(|| tr(RecordsMsg::EditorReloadFirst)),
    });
    let editor = code_editor(EditorSetup {
        field: EditorField::Document,
        language: EditorLanguage::Json,
        text,
        opened: Some(opened_text),
        marks: marks_of(findings.as_ref()),
        label: tr(PolicyMsg::AccessEditorLabel),
        lead: tr(RecordsMsg::EditorJson),
        facts: vec![tr(Msg::EditorUtf8)],
        findings,
    });
    let repair_box = h(format!(
        r#"<label class="fui-check"><input type="checkbox" name="repair" value="on" class="fui-checkbox"{}><span class="fui-check-text">{}</span></label>"#,
        flag("checked", repair),
        t(tr(PolicyMsg::AccessRepair))
    ));
    opt(top).then(h(format!(
        r#"<form id="{EDIT_FORM}" method="post" action="{action}"><input type="hidden" name="version" value="{version}">{panel}</form>"#,
        action = t(&action),
        version = t(&opened),
        panel = card_panel(section(
            Some(tr(PolicyMsg::EditDocument)),
            None,
            None,
            section_intro(tr(PolicyMsg::AccessEditIntro))
                .then(editor)
                .then(repair_box),
        )),
    )))
}

/// Commit an EHR's access settings.
pub async fn edit_page() -> Drawn {
    let ehr_id = ehr_id_param();
    let body = match read_edit(ehr_id.clone()).await {
        Form::Shown(editor) => edit_form(&ehr_id, *editor),
        Form::Refused(refusal) => refusal.view(),
        Form::Not(never) => match never {},
        Form::Left => Html::empty(),
    };
    let mut trail = crumbs(&ehr_id);
    trail.push(Crumb::new(
        tr(PolicyMsg::AccessTab),
        tab_href(&ehr_id, EhrTab::Access),
    ));
    let heading = editor_heading(
        &current(),
        message(RecordsMsg::EditorTitle),
        &Mode::Editing(tr(PolicyMsg::AccessTab)),
        Header {
            title: String::new().into(),
            subtitle: Some(tr(PolicyMsg::AccessEditSubtitle).into()),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    Drawn::page(
        tr(PolicyMsg::AccessEditTitle),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

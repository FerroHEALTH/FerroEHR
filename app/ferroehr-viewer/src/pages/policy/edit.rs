// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Edit the access policy: the whole `ACCESS_POLICY` in the code editor,
//! saved as the next version over the one it was opened at.
//!
//! The save names that version in `If-Match`, or creates the first with
//! `If-None-Match: *`, so a policy another administrator changed meanwhile
//! is refused (`412`), never overwritten; a `400` or `422` is drawn beside
//! the editor in the CDR's words, the document kept as typed. No openEHR
//! specification governs the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::surface::{Crumb, Header, Mode, editor_heading, section_intro};

use crate::bff::access_policy::{fetch_policy, put_policy};
use crate::components::{EditorField, EditorLanguage, EditorSetup, code_editor};
use crate::flash::{Done, redirect_with};
use crate::locale::policy::PolicyMsg;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    EditorDock, card_panel, discarded, dock_editor, findings_of, marks_of, section,
};
use crate::pages::editor::{Unsaved, editor_line, reload_words, save_as_words};
use crate::pages::form::Form;
use crate::pages::policy::{EDIT_ROUTE, POLICY_ROUTE, refusal_of};
use crate::routes::Drawn;
use crate::uid::version_number;

/// The id of the editor's form.
const EDIT_FORM: &str = "policy-edit";

/// The first version a new policy starts from: the default archetype, the
/// controller to fill in, and the two attributes the model requires.
const STARTER: &str = r#"{
  "_type": "ACCESS_POLICY",
  "name": { "_type": "DV_TEXT", "value": {name} },
  "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
  "archetype_details": { "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-ACCESS_POLICY.controller.v1" },
    "rm_version": "1.2.0" },
  "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
    "id": { "_type": "HIER_OBJECT_ID", "value": "" } },
  "care_relationship_required": false,
  "purpose_required": false
}"#;

/// The starter document, named in the reader's language.
fn starter() -> String {
    let name = serde_json::to_string(&tr(PolicyMsg::StarterName)).unwrap_or_default();
    STARTER.replace("{name}", &name)
}

/// What the editor holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    /// The version the edit replaces, or `None` for the first.
    opened: Option<String>,
    /// The version in force now, or `None` while none is committed.
    current_uid: Option<String>,
    /// The document as typed.
    text: String,
    /// The document as it was opened.
    opened_text: String,
    /// The save the CDR refused, when the post was one.
    unsaved: Option<Unsaved>,
}

/// What the page reads: the policy in force, and the save the post asks for.
async fn read_edit() -> Form<Editor> {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect(POLICY_ROUTE);
        return Form::Left;
    }
    let current = match fetch_policy().await {
        Ok(current) => current,
        Err(error) => return Form::Refused(refusal_of(error)),
    };
    let current_uid = current.as_ref().map(|state| state.version_uid.clone());
    let opened_text = current
        .as_ref()
        .map_or_else(starter, |state| state.body.clone());
    let opened = match &posted {
        Some(posted) => Some(posted.text("version")).filter(|uid| !uid.is_empty()),
        None => current_uid.clone(),
    };
    let text = posted
        .as_ref()
        .and_then(|posted| posted.get(&EditorField::Document.name()))
        .unwrap_or_else(|| opened_text.clone());
    let mut unsaved = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        match put_policy(opened.clone(), text.clone()).await {
            Ok(uid) => {
                redirect_with(
                    POLICY_ROUTE,
                    &Done::ok(tr(PolicyMsg::Saved)).line(tr_with(
                        PolicyMsg::SavedVersion,
                        &[("version", version_number(&uid).into())],
                    )),
                );
                return Form::Left;
            }
            Err(error) => unsaved = Some(Unsaved::new(tr(PolicyMsg::SaveRefused), error)),
        }
    }
    Form::Shown(Box::new(Editor {
        opened,
        current_uid,
        text,
        opened_text,
        unsaved,
    }))
}

/// The words of Save: the version the save makes.
fn save_words(opened: Option<&str>) -> String {
    opened.map_or_else(|| tr(PolicyMsg::SaveFirst), save_as_words)
}

/// The editor's form, its decision bar docked.
fn edit_form(editor: Editor) -> Html {
    let Editor {
        opened,
        current_uid,
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
    let words = current_uid
        .as_deref()
        .map_or_else(|| tr(PolicyMsg::ReloadNone), reload_words);
    let top = unsaved.map(|unsaved| unsaved.view(EDIT_ROUTE, words));
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line,
        glyph,
        save: save_words(opened.as_deref()),
        blocked: conflict.then(|| tr(RecordsMsg::EditorReloadFirst)),
    });
    let editor = code_editor(EditorSetup {
        field: EditorField::Document,
        language: EditorLanguage::Json,
        text,
        opened: Some(opened_text),
        marks: marks_of(findings.as_ref()),
        label: tr(PolicyMsg::EditorLabel),
        lead: tr(RecordsMsg::EditorJson),
        facts: vec![tr(Msg::EditorUtf8)],
        findings,
    });
    let intro = if opened.is_some() {
        PolicyMsg::EditIntro
    } else {
        PolicyMsg::EditFirstIntro
    };
    let body = section(
        Some(tr(PolicyMsg::EditDocument)),
        None,
        None,
        section_intro(tr(intro)).then(editor),
    );
    opt(top).then(h(format!(
        r#"<form id="{EDIT_FORM}" method="post" action="{EDIT_ROUTE}"><input type="hidden" name="version" value="{version}">{panel}</form>"#,
        version = t(opened.unwrap_or_default()),
        panel = card_panel(body),
    )))
}

/// Edit the access policy.
pub async fn page() -> Drawn {
    let read = read_edit().await;
    let subtitle = match &read {
        Form::Shown(editor) => editor.opened.as_deref().map_or_else(
            || tr(PolicyMsg::EditFirstSubtitle),
            |uid| {
                tr_with(
                    PolicyMsg::EditSubtitle,
                    &[("version", version_number(uid).into())],
                )
            },
        ),
        _ => tr(PolicyMsg::PolicySubtitle),
    };
    let body = match read {
        Form::Shown(editor) => edit_form(*editor),
        Form::Refused(refusal) => refusal.view(),
        Form::Not(never) => match never {},
        Form::Left => Html::empty(),
    };
    let heading = editor_heading(
        &current(),
        message(RecordsMsg::EditorTitle),
        &Mode::Editing(tr(Msg::NavAccessPolicy)),
        Header {
            title: String::new().into(),
            subtitle: Some(subtitle.into()),
            crumbs: vec![Crumb::new(tr(Msg::NavAccessPolicy), POLICY_ROUTE)],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    Drawn::page(
        tr(PolicyMsg::EditTitle),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

#[cfg(test)]
mod tests {
    use super::{save_words, starter};

    #[test]
    fn the_starter_is_an_access_policy() {
        let view = crate::bff::access_policy::read_policy(&starter()).expect("a policy");
        assert_eq!("Access policy", view.name);
    }

    #[test]
    fn save_names_the_version_it_makes() {
        assert!(save_words(Some("a::ferroehr.local::2")).contains('3'));
        assert_eq!("Save as version 1", save_words(None));
    }
}

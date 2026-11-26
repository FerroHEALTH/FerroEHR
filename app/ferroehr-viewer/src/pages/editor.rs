// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What every editor that saves over a version draws the same way.
//!
//! The CDR's refusal of a save with its reload after a conflict, the line its
//! docked bar opens with, and the words of Save and Reload. No openEHR spec
//! governs the viewer's editors: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::button::Variant;

use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::{Refused, is_status};

/// A save of an editor the CDR refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsaved {
    /// The refusal.
    pub refusal: Refused,
    /// Whether another client changed the object meanwhile (`412`).
    pub conflict: bool,
}

impl Unsaved {
    /// The refusal `error` of the save `summary` names.
    #[must_use]
    pub fn new(summary: String, error: ViewerError) -> Self {
        Self {
            conflict: is_status(&error, http::StatusCode::PRECONDITION_FAILED),
            refusal: Refused::new(summary, error),
        }
    }

    /// The refusal drawn above the editor, with the way to reload the
    /// current version at `reload` in the words `words` after a conflict.
    #[must_use]
    pub fn view(self, reload: &str, words: String) -> Html {
        let again = self.conflict.then(|| {
            h(format!(
                r#"<div class="fui-cluster">{}<span class="fui-small fui-muted">{}</span></div>"#,
                ferrohealth_ui::view::button::link_button(
                    reload.to_owned(),
                    Variant::Secondary,
                    Some(Glyph::REFRESH_CW),
                    words,
                ),
                t(tr(crate::locale::records::RecordsMsg::EditorReloadReplaces))
            ))
        });
        self.refusal.view().then(crate::markup::opt(again))
    }
}

/// The line an editor's docked bar opens with: not saved after a refusal,
/// opened otherwise.
#[must_use]
pub fn editor_line(refused: bool) -> (Glyph, String) {
    use crate::locale::records::RecordsMsg;
    if refused {
        (Glyph::CIRCLE_X, tr(RecordsMsg::EditorNotSaved))
    } else {
        (Glyph::PENCIL, tr(RecordsMsg::EditorOpened))
    }
}

/// The words of Save for an editor opened at the version `opened`: the
/// version the save makes.
#[must_use]
pub fn save_as_words(opened: &str) -> String {
    crate::uid::version_number(opened)
        .parse::<u32>()
        .map_or_else(
            |_| tr(Msg::EditorSave),
            |number| {
                tr_with(
                    crate::locale::records::RecordsMsg::EditorSaveAsVersion,
                    &[("version", number.saturating_add(1).into())],
                )
            },
        )
}

/// The words of Reload for the current version `current`.
#[must_use]
pub fn reload_words(current: &str) -> String {
    tr_with(
        crate::locale::records::RecordsMsg::EditorReloadVersion,
        &[("version", crate::uid::version_number(current).into())],
    )
}

/// A JSON document an editor saves whole over a version.
#[derive(Debug)]
pub struct DocumentForm {
    /// The id of the page's form.
    pub form: &'static str,
    /// Where the form posts.
    pub action: String,
    /// The version the save replaces, posted as `version`.
    pub version: String,
    /// The document as typed.
    pub text: String,
    /// The document as opened.
    pub opened: String,
    /// The save the CDR refused, when the post was one.
    pub unsaved: Option<Unsaved>,
    /// The words of Reload after a conflict.
    pub reload: String,
    /// The words of Save.
    pub save: String,
    /// The editor's accessible name.
    pub label: String,
    /// The section's title and its intro.
    pub section: (String, String),
    /// What follows the editor in the form, such as a choice.
    pub after: Html,
}

/// The form of `document`, its decision bar docked: the CDR's refusal over
/// it, the code editor with the findings the refusal names, and the docked
/// Discard and Save.
#[must_use]
pub fn document_form(document: DocumentForm) -> Html {
    use crate::components::{EditorField, EditorLanguage, EditorSetup, code_editor};
    use crate::locale::records::RecordsMsg;
    use crate::pages::common::{EditorDock, card_panel, dock_editor, findings_of, marks_of};
    let DocumentForm {
        form,
        action,
        version,
        text,
        opened,
        unsaved,
        reload,
        save,
        label,
        section,
        after,
    } = document;
    let field = EditorField::Document.name();
    let findings = unsaved
        .as_ref()
        .and_then(|unsaved| findings_of(&unsaved.refusal.error, &text, &field));
    let conflict = unsaved.as_ref().is_some_and(|unsaved| unsaved.conflict);
    let (glyph, line) = editor_line(unsaved.is_some());
    let top = unsaved.map(|unsaved| unsaved.view(&action, reload));
    dock_editor(EditorDock {
        form: form.to_owned(),
        line,
        glyph,
        save,
        blocked: conflict.then(|| tr(RecordsMsg::EditorReloadFirst)),
    });
    let editor = code_editor(EditorSetup {
        field: EditorField::Document,
        language: EditorLanguage::Json,
        text,
        opened: Some(opened),
        marks: marks_of(findings.as_ref()),
        label,
        lead: tr(RecordsMsg::EditorJson),
        facts: vec![tr(Msg::EditorUtf8)],
        findings,
    });
    let (title, intro) = section;
    let body = crate::pages::common::section(
        Some(title),
        None,
        None,
        ferrohealth_ui::view::surface::section_intro(intro)
            .then(editor)
            .then(after),
    );
    crate::markup::opt(top).then(h(format!(
        r#"<form id="{form}" method="post" action="{action}"><input type="hidden" name="version" value="{version}">{panel}</form>"#,
        action = t(&action),
        version = t(&version),
        panel = card_panel(body),
    )))
}

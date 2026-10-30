// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parts of a page the viewer composes from FerroUI's components with
//! its own data (FerroUI `docs/consoles.md`, "What the console owns").
//!
//! - [`subject_reveal`]: an EHR's masked subject id (R13), whose Reveal
//!   leads to the EHR's page drawn with the id revealed for the reader's
//!   seconds ([`reveal_subject`]).
//! - [`code_editor`]: the code editor (B2) of a document a form posts, with
//!   the CDR's diagnostics beside it in step with the cursor (B3).
//! - [`source_upload`]: the upload field of a template source, whose file is
//!   read into the text area the form posts.
//!
//! Each works with no script: Reveal is a link, and the text areas post with
//! their forms. No openEHR spec governs the viewer: our own design.

use std::borrow::Cow;

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::diagnostics::{DiagnosticsWords, FileFindings, Position};
use ferrohealth_ui::view::editor::{
    Editor, EditorWords, LineMark, Opened, Panel, editor_diagnostics, editor_panel,
    editor_split,
};
use ferrohealth_ui::view::identifier::{Identifier, IdentifierWords, Masked, Reveal, identifier};
use ferrohealth_ui::view::upload::{Sending, Upload, UploadWords, upload_field};

use crate::error::ViewerError;
use crate::markup::{h, t};

/// Reads the subject id of the EHR `ehr_id` from its current `EHR_STATUS`,
/// for the masked identifier's Reveal.
///
/// The CDR records the read in its own audit log, as it records every read
/// of the status; the viewer's log names the reader and the EHR, never the
/// value.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; the CDR's
/// refusal of the status read; [`ViewerError::Invalid`] when the EHR names no
/// external subject.
pub async fn reveal_subject(ehr_id: String) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let status = crate::bff::ehr_detail::status::fetch_ehr_status(ehr_id.clone()).await?;
    if status.subject.is_empty() {
        return Err(ViewerError::Invalid(crate::locale::tr_served(
            crate::locale::Msg::RevealNoSubject,
        )));
    }
    tracing::info!(reader = %session.identity, ehr = %ehr_id, "revealed an EHR's subject id");
    Ok(status.subject)
}

/// The masked subject id of an EHR and its Reveal (R13).
///
/// In the state `state`: masked, its Reveal a link to `fallback`, the EHR's
/// page drawn with the id shown; or shown, counting down its seconds; or
/// refused.
#[must_use]
pub fn subject_reveal(id: String, masked: Masked, state: Reveal, fallback: String) -> Html {
    identifier(Identifier {
        id,
        masked,
        show_domain: true,
        state,
        fallback,
        words: IdentifierWords::of(&crate::locale::current()),
    })
}

/// The languages a code editor highlights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorLanguage {
    /// Canonical JSON, a FLAT or a STRUCTURED document.
    Json,
    /// Canonical XML or an operational template.
    Xml,
    /// AQL.
    Aql,
    /// Plain text, such as an ADL 2 source.
    Plain,
}

impl EditorLanguage {
    /// The highlighter's language.
    const fn language(self) -> Language {
        match self {
            Self::Json => Language::Json,
            Self::Xml => Language::Xml,
            Self::Aql => Language::Aql,
            Self::Plain => Language::Plain,
        }
    }
}

/// The text areas the viewer's forms post a document in, each its id and its
/// field name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    /// A composition or a party document.
    Document,
    /// An `EHR_STATUS`'s `other_details`.
    OtherDetails,
    /// A relationship's `details`.
    Details,
    /// An AQL query.
    Aql,
    /// The document of one change a new contribution stages, by its number.
    Staged(u16),
}

impl EditorField {
    /// The text area's id and the name the form posts it as, `staged-3` for
    /// the third staged change.
    #[must_use]
    pub fn name(self) -> Cow<'static, str> {
        match self {
            Self::Document => "document".into(),
            Self::OtherDetails => "other_details".into(),
            Self::Details => "details".into(),
            Self::Aql => "aql".into(),
            Self::Staged(number) => format!("staged-{number}").into(),
        }
    }
}

/// What a code editor draws: its text, its marks and its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSetup {
    /// The text area it fills.
    pub field: EditorField,
    /// The language it highlights.
    pub language: EditorLanguage,
    /// The text, as the page opened or the form posted it.
    pub text: String,
    /// What the text was when the page opened it, or `None` for a new one.
    pub opened: Option<String>,
    /// The lines a diagnostic names.
    pub marks: Vec<LineMark>,
    /// The text area's accessible name.
    pub label: String,
    /// What the text is, at the toolbar's start.
    pub lead: String,
    /// The facts the status line says.
    pub facts: Vec<String>,
    /// The CDR's findings beside the editor, with their heading.
    pub findings: Option<(String, FileFindings)>,
}

/// The code editor (B2) of a document a form posts.
///
/// F8's toolbar, the editor with its gutter marks and the status line, and
/// the CDR's diagnostics beside it in step with the cursor when it refused
/// the document (B3).
#[must_use]
pub fn code_editor(setup: EditorSetup) -> Html {
    let EditorSetup {
        field,
        language,
        text,
        opened,
        marks,
        label,
        lead,
        facts,
        findings,
    } = setup;
    let locale = crate::locale::current();
    let caret = Position { line: 1, column: 1 };
    let editor = Editor {
        id: field.name(),
        language: language.language(),
        text,
        marks,
        editable: true,
    };
    let side = findings.map(|(heading, file)| {
        let words = DiagnosticsWords::of(&locale);
        let list = editor_diagnostics(&heading, &file, &words, &editor, caret);
        h(format!(
            r#"<section class="fui-card fui-panel" aria-label="{label}"><div class="fui-panel-head"><h2 class="fui-card-title"><span>{title}</span></h2></div>{list}</section>"#,
            label = t(&heading),
            title = t(&heading),
        ))
    });
    let panel = editor_panel(Panel {
        editor,
        label,
        lead,
        facts,
        opened: opened.map_or(Opened::New, Opened::Text),
        caret,
        tools: None,
        end: None,
        words: EditorWords::of(&locale),
    });
    match side {
        Some(side) => editor_split(panel, side),
        None => panel,
    }
}

/// The largest template source the upload field reads, in bytes.
pub const SOURCE_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The upload field of a template source: the file chosen or dropped is
/// read into the text area the form posts as `source-text`, and a source can
/// be pasted there with no script.
#[must_use]
pub fn source_upload(
    source: &str,
    label: String,
    choose: String,
    placeholder: String,
    adl2: bool,
) -> Html {
    let accept: &'static [&'static str] = if adl2 {
        &[".adl", ".adls", ".adlt", ".adl2", ".txt"]
    } else {
        &[".opt", ".xml"]
    };
    upload_field(
        &Upload {
            id: "source",
            label: label.into(),
            choose: choose.into(),
            accept,
            max_bytes: SOURCE_MAX_BYTES,
            placeholder: placeholder.into(),
            words: UploadWords::of(&crate::locale::current(), accept, SOURCE_MAX_BYTES),
        },
        Sending {
            text: source,
            busy: false,
            refusal: None,
        },
    )
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's own components: the parts of a page whose library widget
//! shares state with the viewer's code (FerroUI `docs/consoles.md`, "What the
//! console owns").
//!
//! - [`SubjectReveal`]: an EHR's masked subject id (R13) and its Reveal,
//!   which reads the id from the CDR through [`reveal_subject`].
//! - [`CodeEditor`]: the code editor (B2) of a document a form posts, with
//!   the CDR's diagnostics beside it in step with the cursor (B3).
//! - [`SourceUpload`]: the upload field of a template source, whose file is
//!   read into the text area the form posts.
//!
//! Each draws on both halves and hydrates with the page, and each works with
//! the WebAssembly withheld: the masked id's Reveal is a link to the page
//! that shows it, and the text areas post with their forms. No openEHR spec
//! governs the viewer: our own design.

use std::borrow::Cow;

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::view::diagnostics::{DiagnosticsWords, FileFindings, Position};
use ferrohealth_ui::widget::editor::{
    Editor, EditorWords, LineMark, Opened, Panel, editor_diagnostics, editor_panel_by_edit,
    editor_split,
};
use ferrohealth_ui::widget::identifier::{
    Identifier, IdentifierWords, Masked, Reveal, Revealed, identifier,
};
use ferrohealth_ui::widget::upload::{Upload, UploadWords, upload_field};
use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

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
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn reveal_subject(
    /// The EHR whose subject id to read.
    ehr_id: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let status = crate::bff::ehr_detail::status::fetch_ehr_status(ehr_id.clone()).await?;
    if status.subject.is_empty() {
        return Err(ViewerError::Invalid(crate::locale::tr_served(
            crate::locale::Msg::RevealNoSubject,
        )));
    }
    tracing::info!(reader = %session.identity, ehr = %ehr_id, "revealed an EHR's subject id");
    Ok(status.subject)
}

/// The masked subject id of an EHR and its Reveal (R13): the server draws
/// the mask, and Reveal reads the id for `seconds`, the reader's preference
/// capped by the server. Before the page hydrates Reveal is a link to
/// `fallback`, the EHR's page showing the id.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the page always consumes the value"
)]
#[component]
pub fn SubjectReveal(
    /// The id of the identifier's element.
    id: String,
    /// The EHR whose subject it is.
    ehr_id: String,
    /// The subject id as masked on the server: its namespace and tail.
    masked: Masked,
    /// The seconds a revealed id stays shown.
    seconds: u32,
    /// The page that shows the id with no WebAssembly.
    fallback: String,
    /// The identifier's own words, in the page's locale.
    words: IdentifierWords,
) -> impl IntoView {
    let state = RwSignal::new(Reveal::Masked);
    let on_reveal = Callback::new(move |()| {
        state.set(Reveal::Busy);
        let ehr_id = ehr_id.clone();
        leptos::task::spawn_local(async move {
            match reveal_subject(ehr_id).await {
                Ok(value) => state.set(Reveal::Shown(Revealed::new(value, seconds))),
                Err(error) => state.set(Reveal::Refused(error.to_string())),
            }
        });
    });
    identifier(
        Identifier {
            id,
            masked,
            show_domain: true,
            state,
            fallback,
            words,
        },
        on_reveal,
    )
}

/// The languages a code editor highlights, as they cross to the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// The code editor (B2) of a document a form posts: F8's toolbar, the
/// editor with its gutter marks and the status line, and the CDR's
/// diagnostics beside it in step with the cursor when it refused the
/// document (B3).
#[expect(
    clippy::needless_pass_by_value,
    reason = "a component takes its props by value, as the page hands them over"
)]
#[component]
pub fn CodeEditor(
    /// What it draws.
    setup: EditorSetup,
    /// The editor's own words, in the page's locale.
    words: EditorWords,
    /// The diagnostics list's own words, in the page's locale.
    diagnostics: DiagnosticsWords,
) -> impl IntoView {
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
    // The form posts the text area, so the viewer keeps no copy of the text.
    let text = Signal::stored(text);
    let caret = RwSignal::new(Position { line: 1, column: 1 });
    let marks = Signal::stored(marks);
    let editor = || Editor {
        id: field.name(),
        language: language.language(),
        text,
        marks,
        editable: Signal::stored(true),
    };
    let panel = editor_panel_by_edit(
        Panel {
            editor: editor(),
            label: label.clone(),
            lead,
            facts,
            opened: opened.map_or(Opened::New, Opened::Text),
            caret,
            tools: None,
            end: None,
            words,
        },
        |_| (),
    );
    match findings {
        Some((heading, file)) => {
            let side = editor_diagnostics(&heading, &file, &diagnostics, editor(), caret);
            let label = heading.clone();
            let side = view! {
                <section class="fui-card fui-panel" aria-label=label>
                    <div class="fui-panel-head">
                        <h2 class="fui-card-title">
                            <span>{heading}</span>
                        </h2>
                    </div>
                    {side}
                </section>
            }
            .into_any();
            editor_split(panel, side)
        }
        None => panel,
    }
}

/// The largest template source the upload field reads, in bytes.
pub const SOURCE_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The upload field of a template source: the file chosen or dropped is
/// read into the text area the form posts as `source-text`, and a source can
/// be pasted there with no WebAssembly.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the page always consumes the value"
)]
#[component]
pub fn SourceUpload(
    /// The source the text area opens with, as the form last posted it.
    source: String,
    /// The field's label, in the page's locale.
    label: String,
    /// The chooser's words, in the page's locale.
    choose: String,
    /// The text area's placeholder: the format's first line.
    placeholder: String,
    /// Whether it takes an ADL 2 source in place of an operational template.
    adl2: bool,
    /// The field's own words, in the page's locale.
    words: UploadWords,
) -> impl IntoView {
    let source = RwSignal::new(source);
    let accept: &'static [&'static str] = if adl2 {
        &[".adl", ".adls", ".adlt", ".adl2", ".txt"]
    } else {
        &[".opt", ".xml"]
    };
    upload_field(
        Upload {
            id: "source",
            label: label.into(),
            choose: choose.into(),
            accept,
            max_bytes: SOURCE_MAX_BYTES,
            placeholder: placeholder.into(),
            words,
        },
        source,
        Signal::stored(false),
        Signal::stored(None),
    )
}

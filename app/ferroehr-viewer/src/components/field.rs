// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The form-control and button classes: FerroUI's, named once for the viewer.
//!
//! These are class constants rather than wrapper components because half the
//! viewer's inputs are deliberately uncontrolled (the login form) or carry
//! bespoke wiring (the builder's per-datatype editors) — the library
//! standardizes the LOOK, each screen keeps its own behaviour. [`text_field`]
//! is the one assembled control: FerroUI's labelled field around a plain text
//! input bound to a signal, which the administration forms are made of.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::view::button::Variant;
use ferrohealth_ui::view::form;
use ferrohealth_ui::widget::editor::{Editor, LineMark, code_editor};
use leptos::prelude::*;

/// A single-line text input.
pub const INPUT: &str = form::INPUT;

/// A `<select>` control.
pub const SELECT: &str = form::SELECT;

/// A multi-line `<textarea>` in the code face (AQL editor, parameter
/// bindings).
pub const TEXTAREA: &str = form::TEXTAREA;

/// A form label.
pub const LABEL: &str = "fui-label";

/// The primary (solid accent) button.
pub const BTN_PRIMARY: &str = Variant::Primary.class();

/// The secondary (outlined) button.
pub const BTN_SECONDARY: &str = Variant::Secondary.class();

/// The danger button: an outlined affordance that opens a removal.
pub const BTN_DANGER: &str = Variant::Danger.class();

/// The solid destructive button: the confirmation inside a danger dialog.
pub const BTN_DESTRUCTIVE: &str = Variant::Destructive.class();

/// The small secondary button: a row action inside a builder or a list.
pub const BTN_SECONDARY_SMALL: &str = Variant::Secondary.small();

/// The small primary button: the row action in force.
pub const BTN_PRIMARY_SMALL: &str = Variant::Primary.small();

/// The small danger button: a row's remove.
pub const BTN_DANGER_SMALL: &str = Variant::Danger.small();

/// One labelled text input bound to `value`, stacked label-over-control.
///
/// `prop:value` carries the live state and `on:input` writes it back (the
/// `value` attribute would only set the initial value, and an `oninput="…"`
/// JS attribute is forbidden outright). `placeholder` is `None` where the
/// field carries no example, which leaves the attribute off the element
/// entirely.
#[must_use]
pub fn text_field(
    id: String,
    label: &'static str,
    placeholder: Option<&'static str>,
    value: RwSignal<String>,
) -> AnyView {
    let input = view! {
        <input
            id=id.clone()
            type="text"
            class=INPUT
            placeholder=placeholder
            prop:value=move || value.get()
            on:input:target=move |ev| value.set(ev.target().value())
        />
    };
    form::keyed_field(id, label, None, input).into_any()
}

/// How tall a [`code_field`] stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Height {
    /// A document: at least 16rem.
    Document,
    /// A short value of a few lines, such as a parameter object: 6rem.
    Short,
}

/// What every code editor on the page reads, provided once by the root
/// component: whether the page has hydrated, and the empty set of marked
/// lines a plain field shows.
///
/// All of it lives in the root owner, so an editor that stays mounted while
/// the `Suspend` that drew it reloads reads nothing disposed.
#[derive(Debug, Clone)]
pub struct Hydrated {
    /// True once the page has hydrated.
    pub signal: RwSignal<bool>,
    /// No marked line.
    pub no_marks: Signal<Vec<LineMark>>,
    /// The root component's owner, where a field's signals are made.
    pub owner: Option<Owner>,
}

/// FerroUI's highlighted, numbered editor over `value`, its text area `id`
/// (a `<label for=id>` names it), highlighted as `language`.
///
/// The text area is read-only until the page hydrates (FerroUI's editor
/// takes no keystroke it would lose) and, given `disabled`, while it reads
/// true. `value` and `disabled` come from state that outlives a reload of
/// the screen.
#[must_use]
pub fn code_field(
    id: &'static str,
    language: Language,
    value: RwSignal<String>,
    disabled: Option<Signal<bool>>,
    height: Height,
) -> AnyView {
    let Hydrated {
        signal,
        no_marks,
        owner,
    } = use_context::<Hydrated>().unwrap_or_else(|| Hydrated {
        signal: RwSignal::new(false),
        no_marks: Signal::stored(Vec::new()),
        owner: None,
    });
    // NOTE: no specification governs this: our own design. The signals the
    // editor reads are made in the root owner, which a screen's reload never disposes.
    let signals = move || {
        let editable = match disabled {
            Some(disabled) => Signal::derive(move || signal.get() && !disabled.get()),
            None => signal.into(),
        };
        (editable, Signal::from(value))
    };
    let (editable, text) = match owner {
        Some(owner) => owner.with(signals),
        None => signals(),
    };
    let editor = code_editor(
        Editor {
            id,
            language,
            text,
            marks: no_marks,
            editable,
        },
        move |text| value.set(text),
    );
    match height {
        Height::Document => editor,
        Height::Short => view! { <div class="fui-editor-short">{editor}</div> }.into_any(),
    }
}

/// `text` pretty-printed when it is a JSON document, else as it stands, so
/// an editor opens on an indented document whatever the server sent.
#[must_use]
#[expect(
    clippy::disallowed_types,
    reason = "the editor shows the CDR JSON wire as text; the document is parsed only to be \
              re-indented, never read (#1694)"
)]
pub fn pretty_json(text: String) -> String {
    // NOTE: no specification governs this: our own design. A text that is not
    // JSON (XML, FLAT, a draft) is legitimately not of this form and stays.
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::pretty_json;

    #[test]
    fn a_json_document_is_indented_and_anything_else_kept() {
        assert_eq!(
            "{\n  \"_type\": \"ITEM_TREE\",\n  \"a\": 1\n}",
            pretty_json(String::from(r#"{"_type":"ITEM_TREE","a":1}"#))
        );
        assert_eq!("<x/>", pretty_json(String::from("<x/>")));
        assert_eq!("", pretty_json(String::new()));
    }
}

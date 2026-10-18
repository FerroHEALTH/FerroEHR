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

use ferrohealth_ui::view::button::Variant;
use ferrohealth_ui::view::form;
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

/// The destructive button (delete affordances, two-step confirms).
pub const BTN_DANGER: &str = Variant::Danger.class();

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
    let field_id = id.clone();
    view! {
        <div class="fui-field">
            <label class=LABEL r#for=id>
                {label}
            </label>
            <input
                id=field_id
                type="text"
                class=INPUT
                placeholder=placeholder
                prop:value=move || value.get()
                on:input:target=move |ev| value.set(ev.target().value())
            />
        </div>
    }
    .into_any()
}

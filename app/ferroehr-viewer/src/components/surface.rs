// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The card surface classes: FerroUI's card, named once for the viewer.
//!
//! [`titled_card`] is the assembled panel card every operational screen builds
//! its sections from: FerroUI's titled card.

use leptos::prelude::*;

/// The card surface with no padding of its own (content decides): FerroUI's
/// card, its padding taken back.
pub const CARD: &str = "fui-card p-0";

/// The card surface with the standard padding: FerroUI's card.
pub const CARD_PAD: &str = "fui-card";

/// A sunken well (code panes, read-only documents).
pub const WELL: &str = "rounded-card border border-edge bg-sunken p-3";

/// The standard section heading inside a card: FerroUI's card title, one
/// spacing step above its body wherever that body sits in the card.
pub const CARD_TITLE: &str = "fui-card-title mb-3";

/// A titled panel card wrapping an already-erased body: the uniform section of
/// the two-column operational panels (`/system`, `/operations`). `full_width`
/// spans both grid columns.
#[must_use]
pub fn titled_card(title: String, full_width: bool, body: AnyView) -> AnyView {
    let class = if full_width {
        format!("{CARD_PAD} lg:col-span-2")
    } else {
        CARD_PAD.to_owned()
    };
    view! {
        <section class=class>
            <h2 class=CARD_TITLE>{title}</h2>
            {body}
        </section>
    }
    .into_any()
}

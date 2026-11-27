// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a page that edits or creates something reads.
//!
//! Its form, why it has none, the CDR's refusal of what the form starts from,
//! or the redirect its post answered with. No openEHR spec governs the
//! viewer's pages: our own design.

use std::convert::Infallible;

use ferrohealth_ui::html::Html;

use crate::pages::common::Refused;

/// The model of a form page holding `T`, with `Why` the page's own reasons
/// for drawing no form (none by default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form<T, Why = Infallible> {
    /// The form, with what it holds.
    Shown(Box<T>),
    /// Why there is no form to draw.
    Not(Why),
    /// The CDR's refusal of what the form starts from.
    Refused(Refused),
    /// The post redirected.
    Left,
}

impl<T> Form<T> {
    /// The page body: the form `shown` draws, the CDR's refusal, or nothing
    /// once the post redirected.
    pub fn view(self, shown: impl FnOnce(T) -> Html) -> Html {
        match self {
            Self::Shown(form) => shown(*form),
            Self::Refused(refusal) => refusal.view(),
            Self::Left => Html::empty(),
        }
    }
}

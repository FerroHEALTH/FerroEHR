// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version history of one versioned object, as every page draws it.
//!
//! A composition, the EHR status and a demographic object show it newest
//! first, the version in view marked, with the at-time row under it. No
//! openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::versions::{Standing, Version, VersionHistory, version_history};

use ferrohealth_ui::view::surface::definitions;

use crate::bff::wire::{VersionEntry, VersionedObjectFacts};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, tr, tr_with};
use crate::markup::t;
use crate::pages::common::{ago, at_time_row, section, short, version_number};

/// One object's history, and where each of its versions opens.
#[derive(Debug)]
pub struct History<'a> {
    /// The revision history, newest first.
    pub versions: &'a [VersionEntry],
    /// The number of the version in view, when one was asked for.
    pub reading: Option<&'a str>,
    /// Whether the newest version deletes the object.
    pub deleted: bool,
    /// What the contribution cell of every row holds.
    pub contribution: Html,
    /// The table's caption.
    pub caption: String,
    /// The page the at-time row posts to.
    pub at_action: String,
    /// What the at-time row says it opens.
    pub at_opens: String,
}

/// Draws `history`, each version opening at `href` of its number (`None` for
/// the current version); a deleted current version opens nowhere.
pub fn history(history: History<'_>, href: impl Fn(Option<&str>) -> String) -> Html {
    let latest = history.versions.first().map(|version| &version.version_id);
    let rows = history
        .versions
        .iter()
        .map(|version| {
            let number = version_number(&version.version_id);
            let current = Some(&version.version_id) == latest;
            Version {
                href: (!(current && history.deleted))
                    .then(|| href((!current).then_some(number.as_str()))),
                standing: match (current, history.deleted) {
                    (true, true) => Standing::Deleted,
                    (true, false) => Standing::Current,
                    _ => Standing::Older,
                },
                read: history.reading == Some(number.as_str()),
                number,
                change: version.change_type.clone(),
                committer: version.committer.clone(),
                when: ago(&version.committed),
                contribution: history.contribution.clone(),
            }
        })
        .collect();
    version_history(
        &crate::locale::current(),
        VersionHistory {
            caption: history.caption,
            versions: rows,
            at_time: Some(at_time_row(
                history.at_action,
                &crate::request::query("at").unwrap_or_default(),
                history.at_opens,
            )),
        },
    )
}

/// The envelope facts of the version `facts` reads, with `contribution` for
/// its contribution and `more` rows after the four every version carries.
#[must_use]
pub fn envelope(
    facts: &VersionedObjectFacts,
    contribution: Html,
    more: Vec<(String, Html)>,
) -> Html {
    let mut rows = vec![
        (tr(RecordsMsg::FactLifecycle), t(&facts.lifecycle_state)),
        (
            tr(RecordsMsg::CompositionPreceding),
            if facts.preceding_version_uid.is_empty() {
                t(tr(RecordsMsg::CompositionFirstVersion))
            } else {
                short(&facts.preceding_version_uid)
            },
        ),
        (tr(RecordsMsg::FactContribution), contribution),
        (
            tr(RecordsMsg::FactSigned),
            t(tr(if facts.signed { Msg::Yes } else { Msg::No })),
        ),
    ];
    rows.extend(more);
    section(
        Some(tr_with(
            RecordsMsg::CompositionVersionN,
            &[("version", version_number(&facts.version_id).into())],
        )),
        None,
        None,
        definitions(rows),
    )
}

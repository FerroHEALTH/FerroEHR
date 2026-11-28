// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parts every access-log page draws: the filters, the entries as a
//! table and one entry whole.
//!
//! The entries are FerroEHR's access log ([`crate::bff::access_log`]), drawn
//! as the CDR served them: a member the subject view leaves out is absent
//! here too. A justified or an emergency access is marked in the list and
//! its stated reason shown on the entry. No openEHR spec governs the viewer:
//! our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::form::{Field, check};
use ferrohealth_ui::view::surface::definitions;
use ferrohealth_ui::view::template::list::{ActiveFilters, FilterChip, active_filters};

use crate::bff::access_log::{Coded, Entry, LogFilters};
use crate::bff::demographics::{PartyKind, party_href};
use crate::error::ViewerError;
use crate::locale::access::AccessMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{flag, h, t};
use crate::pages::common::{
    Refused, ago, date_range, document_section, info_note, is_status, section, state_mark, when,
};

/// The actions the filter offers, by their token.
const ACTIONS: [(&str, AccessMsg); 5] = [
    ("HL7_AuditEventAction|C", AccessMsg::ActionCreate),
    ("HL7_AuditEventAction|R", AccessMsg::ActionRead),
    ("HL7_AuditEventAction|U", AccessMsg::ActionUpdate),
    ("HL7_AuditEventAction|D", AccessMsg::ActionDelete),
    ("openehr_access|query", AccessMsg::ActionQuery),
];

/// The outcomes the filter offers, by their token.
const OUTCOMES: [(&str, AccessMsg); 3] = [
    ("openehr_access|granted", AccessMsg::OutcomeGranted),
    ("openehr_access|refused", AccessMsg::OutcomeRefused),
    ("openehr_access|failed", AccessMsg::OutcomeFailed),
];

/// The accessor kinds the filter offers, by their token.
const KINDS: [(&str, AccessMsg); 7] = [
    ("openehr_access|person", AccessMsg::KindPerson),
    ("openehr_access|external person", AccessMsg::KindExternal),
    ("openehr_access|delegated person", AccessMsg::KindDelegated),
    (
        "openehr_access|representative",
        AccessMsg::KindRepresentative,
    ),
    ("HL7_SecurityRoleType|datasubject", AccessMsg::KindSubject),
    ("openehr_access|organisation", AccessMsg::KindOrganisation),
    ("openehr_access|system", AccessMsg::KindSystem),
];

/// The grounds the filter offers, as the CDR names them.
const GROUNDS: [(&str, AccessMsg); 2] = [
    ("justification", AccessMsg::GroundJustification),
    ("emergency", AccessMsg::GroundEmergency),
];

/// The filters the address names.
#[must_use]
pub fn read_filters() -> LogFilters {
    let get = |key: &str| crate::request::query(key).unwrap_or_default();
    LogFilters {
        from: get("from"),
        to: get("to"),
        action: get("action"),
        outcome: get("outcome"),
        accessor_kind: get("accessor_kind"),
        justification: get("justification"),
        include_self: get("include_self") == "true",
    }
}

/// The filters as address pairs, `without` left out.
fn pairs(filters: &LogFilters, without: &str) -> Vec<String> {
    let include_self = if filters.include_self { "true" } else { "" };
    [
        ("from", filters.from.as_str()),
        ("to", filters.to.as_str()),
        ("action", filters.action.as_str()),
        ("outcome", filters.outcome.as_str()),
        ("accessor_kind", filters.accessor_kind.as_str()),
        ("justification", filters.justification.as_str()),
        ("include_self", include_self),
    ]
    .into_iter()
    .filter(|(key, value)| *key != without && !value.is_empty())
    .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
    .collect()
}

/// `base` with the filters but `without`.
#[must_use]
pub fn href_with(base: &str, filters: &LogFilters, without: &str) -> String {
    let pairs = pairs(filters, without);
    if pairs.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", pairs.join("&"))
    }
}

/// Whether any filter is set.
#[must_use]
pub fn any(filters: &LogFilters) -> bool {
    !pairs(filters, "").is_empty()
}

/// The words of the choice `token` among `choices`, the token itself when
/// none names it.
fn words_of(choices: &[(&str, AccessMsg)], token: &str) -> String {
    choices
        .iter()
        .find(|(value, _)| *value == token)
        .map_or_else(|| token.to_owned(), |(_, words)| tr(*words))
}

/// A select of `choices` with Any first.
fn select(
    id: &'static str,
    name: &'static str,
    label: String,
    choices: &[(&str, AccessMsg)],
    chosen: &str,
) -> Html {
    let any = h(format!(
        r#"<option value=""{}>{}</option>"#,
        flag("selected", chosen.is_empty()),
        t(tr(AccessMsg::FilterAny))
    ));
    let options: Html = std::iter::once(any)
        .chain(choices.iter().map(|(value, words)| {
            h(format!(
                r#"<option value="{}"{}>{}</option>"#,
                t(value),
                flag("selected", *value == chosen),
                t(tr(*words))
            ))
        }))
        .collect();
    Field::new(id, label).view(h(format!(
        r#"<select id="{id}" name="{name}" class="fui-select">{options}</select>"#
    )))
}

/// The outcome filter, `chosen` selected.
#[must_use]
pub fn outcome_select(id: &'static str, chosen: &str) -> Html {
    select(
        id,
        "outcome",
        tr(AccessMsg::FilterOutcome),
        &OUTCOMES,
        chosen,
    )
}

/// The ground filter, `chosen` selected.
#[must_use]
pub fn grounds_select(id: &'static str, chosen: &str) -> Html {
    select(
        id,
        "justification",
        tr(AccessMsg::FilterJustified),
        &GROUNDS,
        chosen,
    )
}

/// The words of an outcome or ground token, the token itself when neither
/// names it.
#[must_use]
pub fn outcome_words(token: &str) -> String {
    if OUTCOMES.iter().any(|(value, _)| *value == token) {
        words_of(&OUTCOMES, token)
    } else {
        words_of(&GROUNDS, token)
    }
}

/// The accessor, action and outcome cells of `entry` in a table row.
#[must_use]
pub fn row_parts(entry: &Entry) -> (Html, Html, Html) {
    (
        accessor(entry, false),
        coded(entry.action.as_ref()),
        outcome(entry),
    )
}

/// The filter toolbar, a `GET` form to `action`.
#[must_use]
pub fn toolbar(action: &str, filters: &LogFilters) -> Html {
    let include_self = check(
        "l-self",
        tr(AccessMsg::FilterSelf),
        h(format!(
            r#"<input type="checkbox" id="l-self" name="include_self" value="true"{}>"#,
            flag("checked", filters.include_self)
        )),
    );
    let tools = Html::join([
        select(
            "l-action",
            "action",
            tr(AccessMsg::FilterAction),
            &ACTIONS,
            &filters.action,
        ),
        outcome_select("l-outcome", &filters.outcome),
        select(
            "l-kind",
            "accessor_kind",
            tr(AccessMsg::FilterAccessor),
            &KINDS,
            &filters.accessor_kind,
        ),
        grounds_select("l-ground", &filters.justification),
        date_range(
            "l-when",
            tr(AccessMsg::FilterWhen),
            &filters.from,
            &filters.to,
        ),
        include_self,
    ]);
    filter_bar(action, &tools)
}

/// A `GET` filter form to `action` holding `tools` and Apply.
#[must_use]
pub fn filter_bar(action: &str, tools: &Html) -> Html {
    h(format!(
        r#"<form method="get" action="{}" class="fui-list-toolbar"><div class="fui-list-toolbar-start">{tools}<button type="submit" class="fui-button fui-button-secondary">{}{}</button></div></form>"#,
        t(action),
        ferrohealth_ui::icon::icon(Glyph::FUNNEL, "16"),
        t(tr(Msg::FilterApply))
    ))
}

/// The active filters as chips, each removing itself from `base`.
#[must_use]
pub fn chips(base: &str, filters: &LogFilters) -> Option<Html> {
    let mut chips: Vec<FilterChip> = [
        (
            "action",
            &filters.action,
            AccessMsg::ChipAction,
            &ACTIONS[..],
        ),
        (
            "outcome",
            &filters.outcome,
            AccessMsg::ChipOutcome,
            &OUTCOMES[..],
        ),
        (
            "accessor_kind",
            &filters.accessor_kind,
            AccessMsg::ChipAccessor,
            &KINDS[..],
        ),
        (
            "justification",
            &filters.justification,
            AccessMsg::ChipJustified,
            &GROUNDS[..],
        ),
    ]
    .into_iter()
    .filter(|(_, value, _, _)| !value.is_empty())
    .map(|(key, value, msg, choices)| FilterChip {
        label: tr_with(msg, &[("value", words_of(choices, value).into())]),
        remove: href_with(base, filters, key),
    })
    .collect();
    for (key, value, msg) in [
        ("from", &filters.from, Msg::ChipFrom),
        ("to", &filters.to, Msg::ChipTo),
    ] {
        if !value.is_empty() {
            chips.push(FilterChip {
                label: tr_with(msg, &[("value", value.clone().into())]),
                remove: href_with(base, filters, key),
            });
        }
    }
    if filters.include_self {
        chips.push(FilterChip {
            label: tr(AccessMsg::ChipSelf),
            remove: href_with(base, filters, "include_self"),
        });
    }
    (!chips.is_empty()).then(|| {
        active_filters(
            &current(),
            ActiveFilters {
                chips,
                clear: base.to_owned(),
                shown: None,
            },
            None,
        )
    })
}

/// The words of a coded value, or a muted dash.
fn coded(value: Option<&Coded>) -> Html {
    match value {
        Some(value) => t(value.words()),
        None => muted(tr(AccessMsg::Unknown)),
    }
}

/// `words`, muted.
fn muted(words: String) -> Html {
    h(format!(r#"<span class="fui-muted">{}</span>"#, t(words)))
}

/// The outcome of `entry` as a state, with its ground beside it.
fn outcome(entry: &Entry) -> Html {
    let words = entry.outcome.as_ref().map_or_else(
        || tr(AccessMsg::Unknown),
        |outcome| outcome.words().to_owned(),
    );
    let state = if entry.denied() {
        FlowState::Unreachable
    } else {
        FlowState::Up
    };
    let ground = entry.justified().map(|ground| {
        let (tone, msg) = if ground.is_emergency() {
            (FlowState::Degraded, AccessMsg::GroundEmergency)
        } else {
            (FlowState::Off, AccessMsg::GroundJustification)
        };
        state_mark(tone, &tr(msg))
    });
    state_mark(state, &words).then(ground.unwrap_or_else(Html::empty))
}

/// The accessor of `entry`: who, linked to their party when `linked`, and
/// as what kind. A table row is one link (FerroUI R9), so a row draws it
/// unlinked.
fn accessor(entry: &Entry, linked: bool) -> Html {
    let href = linked
        .then_some(())
        .and(entry.accessor_party.as_ref())
        .and_then(|(rm_type, id)| {
            PartyKind::from_rm_type(rm_type).map(|kind| party_href(kind, id))
        });
    let who = match (entry.accessor.is_empty(), href) {
        (true, _) => muted(tr(AccessMsg::Unknown)),
        (false, Some(href)) => h(format!(
            r#"<a href="{}">{}</a>"#,
            t(href),
            t(&entry.accessor)
        )),
        (false, None) => t(&entry.accessor),
    };
    match &entry.accessor_kind {
        Some(kind) => who.then(h(format!(
            r#"<span class="fui-small fui-muted"> · {}</span>"#,
            t(kind.words())
        ))),
        None => who,
    }
}

/// The objects `entry` reached, by their types.
fn reached(entry: &Entry) -> Html {
    let types: Vec<&str> = entry
        .items
        .iter()
        .map(|item| item.rm_type.as_str())
        .filter(|rm_type| !rm_type.is_empty())
        .collect();
    if types.is_empty() {
        muted(tr(AccessMsg::Unknown))
    } else {
        h(format!("<code>{}</code>", t(types.join(", "))))
    }
}

/// The entries as a table, each row one link to the page `href` names.
#[must_use]
pub fn table(entries: &[Entry], href: impl Fn(&Entry) -> String) -> Html {
    let columns = [
        Column::text("when", tr(AccessMsg::ColumnWhen)).role(Role::Title),
        Column::text("accessor", tr(AccessMsg::ColumnAccessor)).role(Role::Sub),
        Column::text("action", tr(AccessMsg::ColumnAction)).role(Role::Meta),
        Column::text("reached", tr(AccessMsg::ColumnReached)).role(Role::Meta),
        Column::text("outcome", tr(AccessMsg::ColumnOutcome)).role(Role::Meta),
    ];
    let rows = entries
        .iter()
        .map(|entry| {
            let link = href(entry);
            let (who, action, result) = row_parts(entry);
            h(format!(
                "<tr>{}{}{}{}{}</tr>",
                title_cell(None, Some(&link), ago(&entry.recorded), None),
                cell(&columns[1], who),
                cell(&columns[2], action),
                cell(&columns[3], reached(entry)),
                cell(&columns[4], result)
            ))
        })
        .collect();
    let caption = tr(AccessMsg::Caption);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// What a refused read of an access log says: who reads one for a `403`,
/// that the CDR keeps none for a `409`, that it serves none here for a
/// `404`, else the CDR's own refusal.
#[must_use]
pub fn refusal(error: ViewerError) -> Html {
    if is_status(&error, http::StatusCode::FORBIDDEN) {
        info_note(t(tr(AccessMsg::Forbidden)))
    } else if is_status(&error, http::StatusCode::CONFLICT) {
        info_note(t(tr(AccessMsg::StoreOff)))
    } else if is_status(&error, http::StatusCode::NOT_FOUND) {
        info_note(t(tr(AccessMsg::Unserved)))
    } else {
        Refused::new(tr(AccessMsg::Refused), error).view()
    }
}

/// The authorisations of `entry` as a table.
fn grounds(entry: &Entry) -> Html {
    if entry.authorisations.is_empty() {
        return muted(tr(AccessMsg::GroundsNone));
    }
    let columns = [
        Column::text("ground", tr(AccessMsg::ColumnGround)).role(Role::Title),
        Column::text("name", tr(AccessMsg::ColumnName)).role(Role::Sub),
        Column::text("version", tr(AccessMsg::ColumnPolicyVersion)).role(Role::Meta),
        Column::text("reason", tr(AccessMsg::FactStatedReason)).role(Role::Meta),
    ];
    let rows = entry
        .authorisations
        .iter()
        .map(|ground| {
            h(format!(
                r#"<tr><th scope="row" data-role="title">{}</th>{}{}{}</tr>"#,
                coded(ground.kind.as_ref()),
                cell(&columns[1], t(&ground.name)),
                cell(&columns[2], t(&ground.version)),
                cell(&columns[3], t(&ground.reason))
            ))
        })
        .collect();
    let caption = tr(AccessMsg::EntryGrounds);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The objects `entry` reached as a table.
fn items(entry: &Entry) -> Html {
    if entry.items.is_empty() {
        return muted(tr(AccessMsg::ReachedNone));
    }
    let columns = [
        Column::text("type", tr(AccessMsg::ColumnType)).role(Role::Title),
        Column::text("object", tr(AccessMsg::ColumnObject)).role(Role::Sub),
        Column::text("categories", tr(AccessMsg::ColumnCategories)).role(Role::Meta),
    ];
    let rows = entry
        .items
        .iter()
        .map(|item| {
            h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}</tr>"#,
                t(&item.rm_type),
                cell(&columns[1], h(format!("<code>{}</code>", t(&item.id)))),
                cell(&columns[2], t(item.categories.join(", ")))
            ))
        })
        .collect();
    let caption = tr(AccessMsg::EntryReached);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The words of a list of strings, or a muted dash.
fn listed(words: &[String]) -> Html {
    if words.is_empty() {
        muted(tr(AccessMsg::Unknown))
    } else {
        t(words.join(", "))
    }
}

/// One entry whole: its facts, its grounds with the stated reason, what it
/// reached and the entry as the CDR served it.
#[must_use]
pub fn entry_sections(entry: &Entry) -> Html {
    let purposes: Vec<String> = entry
        .purposes
        .iter()
        .map(|purpose| purpose.words().to_owned())
        .collect();
    let mut facts = vec![
        (tr(AccessMsg::ColumnWhen), when(&entry.recorded)),
        (tr(AccessMsg::ColumnAccessor), accessor(entry, true)),
        (tr(AccessMsg::FactProvider), t(&entry.provider)),
        (tr(AccessMsg::FactRoles), listed(&entry.roles)),
        (tr(AccessMsg::ColumnAction), coded(entry.action.as_ref())),
        (
            tr(AccessMsg::FactOperation),
            h(format!(
                "<code>{}</code>",
                t(entry
                    .operation
                    .as_ref()
                    .map(Coded::words)
                    .unwrap_or_default())
            )),
        ),
        (tr(AccessMsg::ColumnOutcome), outcome(entry)),
        (tr(AccessMsg::FactPurposes), listed(&purposes)),
    ];
    if let Some(reason) = &entry.outcome_reason {
        facts.push((tr(AccessMsg::FactReason), t(reason.words())));
    }
    if let Some(ground) = entry.justified() {
        facts.push((tr(AccessMsg::FactStatedReason), t(&ground.reason)));
    }
    Html::join([
        section(
            Some(tr(AccessMsg::EntryFacts)),
            None,
            None,
            definitions(facts),
        ),
        section(
            Some(tr(AccessMsg::EntryGrounds)),
            None,
            None,
            grounds(entry),
        ),
        section(Some(tr(AccessMsg::EntryReached)), None, None, items(entry)),
        document_section(
            tr(AccessMsg::EntryAsServed),
            Language::Json,
            entry.document.clone(),
            "access-log-entry",
        ),
    ])
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Retention: the periods the CDR keeps each kind of record for, the form
//! that declares or changes one, and what is due.
//!
//! The register and the due list come from the CDR's admin marks extension
//! ([`crate::bff::retention`]); a `403` and a `405` are drawn as the CDR
//! answered them. The CDR deletes nothing on a timer, and neither does the
//! page: the due list is a list. No openEHR specification governs
//! retention: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::empty::empty_quiet;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Header, page_heading, section_intro};

use crate::bff::retention::{
    DUE_LIMIT, DueEhr, RetentionPeriod, declare, fetch_due, fetch_periods,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::policy::PolicyMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    Refused, card_panel, count, info_note, is_status, section, short, when,
};
use crate::pages::ehr::ehr_href;
use crate::routes::Drawn;

/// The route of Retention.
pub const RETENTION_ROUTE: &str = "/retention";

/// The kinds a period applies to.
const KINDS: [&str; 4] = ["EHR", "COMPOSITION", "EHR_STATUS", "FOLDER"];

/// What a period runs from.
const ANCHORS: [&str; 3] = ["last_commit", "death", "majority"];

/// The refusal of a read, worded for a `403` and a `405`.
fn refusal_of(error: ViewerError) -> Refused {
    let summary = if matches!(error, ViewerError::Forbidden(_)) {
        PolicyMsg::RetentionForbidden
    } else if is_status(&error, http::StatusCode::METHOD_NOT_ALLOWED) {
        PolicyMsg::AdminOff
    } else {
        PolicyMsg::RetentionRefused
    };
    Refused::new(tr(summary), error)
}

/// The page's model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Model {
    /// The register.
    periods: Result<Vec<RetentionPeriod>, Refused>,
    /// What is due.
    due: Result<Vec<DueEhr>, Refused>,
    /// The declaration as posted, and the CDR's refusal of it.
    draft: Option<(RetentionPeriod, Refused)>,
}

/// The period a post declares.
fn posted_period(posted: &crate::posted::Posted) -> RetentionPeriod {
    let filled =
        |name: &str| Some(posted.text(name).trim().to_owned()).filter(|text| !text.is_empty());
    RetentionPeriod {
        kind: posted.text("kind"),
        jurisdiction: posted.text("jurisdiction").trim().to_owned(),
        category: filled("category"),
        origin: filled("origin"),
        period: posted.text("period").trim().to_owned(),
        anchor: posted.text("anchor"),
        source: posted.text("source").trim().to_owned(),
    }
}

/// What the page reads, after the declaration its post asks.
async fn read() -> Model {
    let mut draft = None;
    if let Some(posted) = crate::posted::posted().filter(|posted| posted.intent() == "declare") {
        let period = posted_period(&posted);
        match declare(&period).await {
            Ok(()) => redirect_with(RETENTION_ROUTE, &Done::ok(tr(PolicyMsg::RetentionDeclared))),
            Err(error) => {
                draft = Some((
                    period,
                    Refused::new(tr(PolicyMsg::RetentionDeclareRefused), error),
                ));
            }
        }
    }
    Model {
        periods: fetch_periods().await.map_err(refusal_of),
        due: fetch_due().await.map_err(refusal_of),
        draft,
    }
}

/// An optional key, or "every" when it names none.
fn every(value: Option<&String>) -> Html {
    value.map_or_else(
        || {
            h(format!(
                r#"<span class="fui-muted">{}</span>"#,
                t(tr(PolicyMsg::RetentionEvery))
            ))
        },
        |value| h(format!("<code>{}</code>", t(value))),
    )
}

/// The register as a table.
fn periods_table(periods: &[RetentionPeriod]) -> Html {
    let columns = [
        Column::text("kind", tr(PolicyMsg::RetentionKind)).role(Role::Title),
        Column::text("jurisdiction", tr(PolicyMsg::MarksJurisdiction)).role(Role::Sub),
        Column::text("category", tr(PolicyMsg::RetentionCategory)).role(Role::Meta),
        Column::text("origin", tr(PolicyMsg::RetentionOrigin)).role(Role::Meta),
        Column::text("period", tr(PolicyMsg::CarePeriod)).role(Role::Meta),
        Column::text("anchor", tr(PolicyMsg::RetentionAnchor)).role(Role::Meta),
        Column::text("source", tr(PolicyMsg::RetentionSource)).role(Role::Wide),
    ];
    let rows = periods
        .iter()
        .map(|period| {
            h(format!(
                "<tr>{}{}{}{}{}{}{}</tr>",
                title_cell(None, None, t(&period.kind), None),
                cell(&columns[1], t(&period.jurisdiction)),
                cell(&columns[2], every(period.category.as_ref())),
                cell(&columns[3], every(period.origin.as_ref())),
                cell(&columns[4], t(&period.period)),
                cell(
                    &columns[5],
                    h(format!("<code>{}</code>", t(&period.anchor)))
                ),
                cell(&columns[6], t(&period.source)),
            ))
        })
        .collect();
    let caption = tr(PolicyMsg::RetentionPeriods);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The due list as a table, each row opening its EHR.
fn due_table(due: &[DueEhr]) -> Html {
    let columns = [
        Column::text("ehr", tr(PolicyMsg::RetentionEhr)).role(Role::Title),
        Column::text("due", tr(PolicyMsg::RetentionDueAt)).role(Role::Sub),
        Column::text("kind", tr(PolicyMsg::RetentionKind)).role(Role::Meta),
        Column::text("category", tr(PolicyMsg::RetentionCategory)).role(Role::Meta),
        Column::text("objects", tr(PolicyMsg::RetentionObjectsDue)).role(Role::Meta),
        Column::text("held", tr(PolicyMsg::RetentionObjectsHeld)).role(Role::Meta),
        Column::text("source", tr(PolicyMsg::RetentionSource)).role(Role::Wide),
    ];
    let rows = due
        .iter()
        .map(|row| {
            h(format!(
                "<tr>{}{}{}{}{}{}{}</tr>",
                title_cell(None, Some(&ehr_href(&row.ehr_id)), short(&row.ehr_id), None),
                cell(&columns[1], when(&row.due_at)),
                cell(
                    &columns[2],
                    t(format!("{} · {}", row.kind, row.jurisdiction))
                ),
                cell(&columns[3], every(row.category.as_ref())),
                cell(&columns[4], t(row.objects_due.to_string())),
                cell(&columns[5], t(row.objects_held.to_string())),
                cell(&columns[6], t(&row.source)),
            ))
        })
        .collect();
    let caption = tr(PolicyMsg::RetentionDue);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// A select of `choices`, `chosen` selected.
fn select(id: &str, name: &str, label: PolicyMsg, choices: &[&str], chosen: &str) -> Html {
    let options = choices
        .iter()
        .map(|choice| {
            format!(
                r#"<option value="{choice}"{}>{choice}</option>"#,
                crate::markup::flag("selected", *choice == chosen)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    Field::new(id, tr(label)).view(h(format!(
        r#"<select id="{id}" name="{name}" class="fui-select">{options}</select>"#
    )))
}

/// A text field holding `value`.
fn text_field(
    id: &str,
    name: &str,
    label: PolicyMsg,
    help: Option<PolicyMsg>,
    value: &str,
) -> Html {
    Field::new(id, tr(label)).help_if(help.map(tr)).view(h(format!(
        r#"<input id="{id}" name="{name}" type="text" class="fui-input" autocomplete="off" value="{}">"#,
        t(value)
    )))
}

/// The declaration form, holding `draft` when a post was refused.
fn declare_form(draft: Option<&RetentionPeriod>) -> Html {
    let empty = RetentionPeriod::default();
    let draft = draft.unwrap_or(&empty);
    let fields = Html::join([
        select(
            "ret-kind",
            "kind",
            PolicyMsg::RetentionKind,
            &KINDS,
            &draft.kind,
        ),
        text_field(
            "ret-jurisdiction",
            "jurisdiction",
            PolicyMsg::MarksJurisdiction,
            Some(PolicyMsg::RetentionJurisdictionHelp),
            &draft.jurisdiction,
        ),
        text_field(
            "ret-category",
            "category",
            PolicyMsg::RetentionCategory,
            Some(PolicyMsg::RetentionCategoryHelp),
            draft.category.as_deref().unwrap_or_default(),
        ),
        text_field(
            "ret-origin",
            "origin",
            PolicyMsg::RetentionOrigin,
            Some(PolicyMsg::RetentionOriginHelp),
            draft.origin.as_deref().unwrap_or_default(),
        ),
        text_field(
            "ret-period",
            "period",
            PolicyMsg::CarePeriod,
            Some(PolicyMsg::RetentionPeriodHelp),
            &draft.period,
        ),
        select(
            "ret-anchor",
            "anchor",
            PolicyMsg::RetentionAnchor,
            &ANCHORS,
            &draft.anchor,
        ),
        text_field(
            "ret-source",
            "source",
            PolicyMsg::RetentionSource,
            Some(PolicyMsg::RetentionSourceHelp),
            &draft.source,
        ),
    ]);
    h(format!(
        r#"<form method="post" action="{RETENTION_ROUTE}" class="fui-form">{fields}<div class="fui-cluster"><button type="submit" name="intent" value="declare" class="fui-button fui-button-primary">{}</button></div></form>"#,
        t(tr(PolicyMsg::RetentionDeclare))
    ))
}

/// Retention.
pub async fn page() -> Drawn {
    let Model {
        periods,
        due,
        draft,
    } = read().await;
    let register = match &periods {
        Ok(periods) if periods.is_empty() => empty_quiet(
            Glyph::FILE_CLOCK,
            tr(PolicyMsg::RetentionNone),
            tr(PolicyMsg::RetentionNoneLine),
        ),
        Ok(periods) => periods_table(periods),
        Err(refusal) => refusal.view(),
    };
    let refused_admin = periods.is_err();
    let due_list = match &due {
        Ok(due) if due.is_empty() => empty_quiet(
            Glyph::FILE_CLOCK,
            tr(PolicyMsg::RetentionNothingDue),
            tr(PolicyMsg::RetentionNothingDueLine),
        ),
        Ok(due) => opt(
            (due.len() >= usize::try_from(DUE_LIMIT).unwrap_or(usize::MAX)).then(|| {
                info_note(t(tr_with(
                    PolicyMsg::GrantsTruncated,
                    &[("count", count(due.len()))],
                )))
            }),
        )
        .then(due_table(due)),
        Err(refusal) => refusal.view(),
    };
    let declare_section = (!refused_admin).then(|| {
        section(
            Some(tr(PolicyMsg::RetentionDeclareTitle)),
            None,
            None,
            section_intro(tr(PolicyMsg::RetentionDeclareIntro))
                .then(opt(draft.as_ref().map(|(_, refusal)| refusal.view())))
                .then(declare_form(draft.as_ref().map(|(period, _)| period))),
        )
    });
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavRetention).into(),
            subtitle: Some(tr(PolicyMsg::RetentionSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    let body = card_panel(Html::join([
        section(
            Some(tr(PolicyMsg::RetentionPeriods)),
            periods
                .as_ref()
                .ok()
                .and_then(|periods| u64::try_from(periods.len()).ok()),
            None,
            section_intro(tr(PolicyMsg::RetentionPeriodsIntro)).then(register),
        ),
        opt(declare_section),
        section(
            Some(tr(PolicyMsg::RetentionDue)),
            due.as_ref()
                .ok()
                .and_then(|due| u64::try_from(due.len()).ok()),
            None,
            section_intro(tr(PolicyMsg::RetentionDueIntro)).then(due_list),
        ),
    ]));
    Drawn::page(
        tr(Msg::NavRetention),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

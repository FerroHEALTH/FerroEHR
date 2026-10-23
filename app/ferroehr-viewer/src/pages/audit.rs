// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Audit log: the CDR's security audit records, who accessed what and
//! with what outcome, and one record as stored (FerroUI
//! `design/consoles/ferroehr/`, pages 20.1 to 20.5).
//!
//! The records are the IHE ATNA retrieval (ITI-81, `GET fhir/r4/AuditEvent`)
//! the CDR serves over its local store ([`crate::bff::audit`]); a CDR that
//! keeps no local store answers `404` and the page says so. The filters are a
//! `GET` form, the page position is kept in the browser's session
//! ([`crate::paging`]), and a record's page finds it again by the filters and
//! the page it was listed on, since ITI-81 offers no read by id. The patient
//! identifier is masked as everywhere. No openEHR spec governs the viewer's
//! pages: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer reads the stored AuditEvent JSON for its id"
)]

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::{Paging, StateSource, Window};
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_pager};
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Crumb, Header, definitions, page_heading};
use ferrohealth_ui::view::template::list::{
    ActiveFilters, FilterChip, ListCard, active_filters, list_card, list_empty,
};
use ferrohealth_ui::widget::identifier::{Domain, KEEP, Masked};
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;
use serde_json::Value;

use crate::bff::PAGE_SIZE;
use crate::bff::audit::{ACTIONS, AuditPage as Listed, AuditRow, OUTCOMES, search_audit};
use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{
    ago, awaited, card_panel, date_range, document_section, refused, section, state_mark, when,
};
use crate::paging::Listing;

/// The page sizes the audit log offers: the one the CDR is asked for.
const SIZES: &[u32] = &[PAGE_SIZE];

/// The filters the address names.
#[derive(Debug, Clone, Default)]
struct Filters {
    /// The principal.
    agent: String,
    /// The outcome code.
    outcome: String,
    /// The action code.
    action: String,
    /// The earliest day.
    from: String,
    /// The latest day.
    to: String,
    /// The patient identifier.
    patient: String,
}

impl Filters {
    /// The filters of the request.
    fn read() -> Self {
        let get = |key: &str| crate::request::query(key).unwrap_or_default();
        Self {
            agent: get("agent"),
            outcome: get("outcome"),
            action: get("action"),
            from: get("from"),
            to: get("to"),
            patient: get("patient"),
        }
    }

    /// The filters as address pairs, `without` left out.
    fn pairs(&self, without: &str) -> Vec<String> {
        [
            ("agent", &self.agent),
            ("outcome", &self.outcome),
            ("action", &self.action),
            ("from", &self.from),
            ("to", &self.to),
            ("patient", &self.patient),
        ]
        .into_iter()
        .filter(|(key, value)| *key != without && !value.is_empty())
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect()
    }

    /// The audit log with these filters but `without`.
    fn href_without(&self, without: &str) -> String {
        let pairs = self.pairs(without);
        if pairs.is_empty() {
            "/audit".to_owned()
        } else {
            format!("/audit?{}", pairs.join("&"))
        }
    }

    /// Whether any filter is set.
    fn any(&self) -> bool {
        !self.pairs("").is_empty()
    }

    /// Runs the search for the page starting at `offset`.
    async fn search(&self, offset: u32) -> Result<Listed, ViewerError> {
        search_audit(
            self.from.clone(),
            self.to.clone(),
            self.patient.clone(),
            self.agent.clone(),
            self.outcome.clone(),
            self.action.clone(),
            offset,
        )
        .await
    }
}

/// The stored id of an audit record.
fn record_id(row: &AuditRow) -> String {
    serde_json::from_str::<Value>(&row.raw)
        .ok()
        .and_then(|record| record.get("id").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

/// The page of a record, carrying the filters and the offset it was listed
/// at.
fn record_href(row: &AuditRow, filters: &Filters, offset: u32) -> String {
    let mut pairs = filters.pairs("");
    pairs.push(format!("offset={offset}"));
    format!(
        "/audit/{}?{}",
        urlencoding::encode(&record_id(row)),
        pairs.join("&")
    )
}

/// The masked patient identifier of a record.
fn patient(row: &AuditRow) -> AnyView {
    if row.patient.is_empty() {
        return view! { <span class="fui-muted">{tr(Msg::AuditNoPatient)}</span> }.into_any();
    }
    view! { <span class="fui-identifier"><code>{Masked::new(Domain::new(String::new()), &row.patient, KEEP).drawn()}</code></span> }.into_any()
}

/// The stored record with its patient identifier masked as the list masks
/// it.
fn masked_record(row: &AuditRow) -> String {
    let pretty = crate::bff::pretty_body(&row.raw, crate::format::ReprFormat::CanonicalJson);
    if row.patient.is_empty() {
        return pretty;
    }
    pretty.replace(&row.patient, &Masked::new(Domain::new(String::new()), &row.patient, KEEP).drawn())
}

/// The outcome of a record as a state.
fn outcome(row: &AuditRow) -> AnyView {
    if row.success {
        state_mark(FlowState::Up, &tr(Msg::AuditSuccess))
    } else {
        state_mark(FlowState::Unreachable, &row.outcome.clone())
    }
}

/// The event words of a record.
fn event(row: &AuditRow) -> String {
    if row.operation.is_empty() {
        row.family.clone()
    } else {
        row.operation.clone()
    }
}

/// The filter toolbar.
fn toolbar(filters: &Filters) -> AnyView {
    let select = |id: &'static str,
                  name: &'static str,
                  label: String,
                  choices: &[(&str, Msg)],
                  chosen: &str| {
        let options = choices
            .iter()
            .map(|(value, words)| view! { <option value=(*value).to_owned() selected=*value == chosen>{tr(*words)}</option> })
            .collect_view();
        Field::new(id, label)
            .view(view! { <select id=id name=name class="fui-select">{options}</select> })
    };
    let tools = view! {
        {select("a-outcome", "outcome", tr(Msg::AuditOutcome), &OUTCOMES, &filters.outcome)}
        {select("a-action", "action", tr(Msg::AuditAction), &ACTIONS, &filters.action)}
        {date_range("a-when", tr(Msg::AuditWhen), &filters.from, &filters.to)}
        <div class="fui-list-find">
            <label for="a-patient" class="fui-sr-only">{tr(Msg::AuditPatient)}</label>
            <input id="a-patient" name="patient" type="text" class="fui-input fui-mono" value=filters.patient.clone() placeholder=tr(Msg::AuditPatient) autocomplete="off" />
        </div>
        <button type="submit" class="fui-button fui-button-secondary">
            {ferrohealth_ui::icon::icon(Glyph::FUNNEL, "16")}
            {tr(Msg::FilterApply)}
        </button>
    };
    let label = tr(Msg::AuditFind);
    find_form(
        Find {
            id: "a-agent",
            name: "agent",
            label: &label,
            value: &filters.agent,
            source: StateSource::Url,
            action: Some("/audit"),
            fields: &[],
        },
        tools,
    )
}

/// The active filters as chips.
fn chips(filters: &Filters) -> Option<AnyView> {
    let chips: Vec<FilterChip> = [
        ("agent", &filters.agent, Msg::AuditChipAgent),
        ("outcome", &filters.outcome, Msg::AuditChipOutcome),
        ("action", &filters.action, Msg::AuditChipAction),
        ("from", &filters.from, Msg::ChipFrom),
        ("to", &filters.to, Msg::ChipTo),
        ("patient", &filters.patient, Msg::AuditChipPatient),
    ]
    .into_iter()
    .filter(|(_, value, _)| !value.is_empty())
    .map(|(key, value, msg)| FilterChip {
        label: tr_with(msg, &[("value", value.clone().into())]),
        remove: filters.href_without(key),
    })
    .collect();
    (!chips.is_empty()).then(|| {
        active_filters(
            ActiveFilters {
                chips,
                clear: "/audit".to_owned(),
                shown: None,
            },
            None,
        )
    })
}

/// The records of one page, as a table.
fn table(page: &Listed, filters: &Filters) -> AnyView {
    let columns = [
        Column::text("event", tr(Msg::AuditEvent)).role(Role::Title),
        Column::text("when", tr(Msg::AuditWhen)).role(Role::Sub),
        Column::text("principal", tr(Msg::AuditPrincipal)).role(Role::Meta),
        Column::text("patient", tr(Msg::AuditPatient)).role(Role::Meta),
        Column::text("outcome", tr(Msg::AuditOutcome)).role(Role::Meta),
    ];
    let rows = page
        .rows
        .iter()
        .map(|row| {
            let href = record_href(row, filters, page.offset);
            let principal = if row.principal.is_empty() {
                view! { <span class="fui-muted">{tr(Msg::AuditUnknown)}</span> }.into_any()
            } else {
                row.principal.clone().into_any()
            };
            view! {
                <tr>
                    {title_cell(Some(&href), event(row), None)}
                    {cell(&columns[1], ago(&row.recorded))}
                    {cell(&columns[2], principal)}
                    {cell(&columns[3], patient(row))}
                    {cell(&columns[4], outcome(row))}
                </tr>
            }
            .into_any()
        })
        .collect_view()
        .into_any();
    let caption = tr(Msg::AuditRecords);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows),
    })
    .into_any()
}

/// The heading of the Audit log.
fn heading() -> AnyView {
    page_heading(
        Header {
            title: tr(Msg::NavAudit).into(),
            subtitle: Some(tr(Msg::AuditSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
        },
        ActionGroup::none(),
    )
    .into_any()
}

/// The note of a CDR that keeps no local audit store.
fn store_off() -> AnyView {
    card_panel(view! {
        <div class="fui-panel-body fui-stack">
            <p>{tr(Msg::AuditStoreOff)}</p>
            <p class="fui-small fui-muted">{tr(Msg::AuditStoreOffLine)}</p>
        </div>
    })
}

/// The Audit log.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn AuditPage() -> impl IntoView {
    let filters = Filters::read();
    let kept = crate::paging::current(Listing::Audit);
    let fields = crate::paging::fields(Listing::Audit);
    let body = awaited(async move {
        let paging = Paging {
            page: kept.page,
            size: PAGE_SIZE,
        };
        let asked = Window::over(u32::MAX, paging);
        let mut page = match filters.search(asked.start).await {
            Ok(page) => page,
            Err(error) => return refused(tr(Msg::AuditRefused), &error, None),
        };
        if !page.available {
            return store_off();
        }
        let window = Window::over(page.total, paging);
        if window.start != asked.start {
            page = match filters.search(window.start).await {
                Ok(page) => page,
                Err(error) => return refused(tr(Msg::AuditRefused), &error, None),
            };
        }
        let list = if page.rows.is_empty() {
            list_empty(
                tr(if filters.any() {
                    Msg::AuditNoMatch
                } else {
                    Msg::AuditEmpty
                }),
                None,
            )
        } else {
            table(&page, &filters)
        };
        let pager = (window.pages > 1).then(|| {
            post_pager(
                PagerListing::new(tr(Msg::AuditPages)).sizes(SIZES),
                window,
                PostTo {
                    id: "audit-pager",
                    action: crate::paging::PAGING_ROUTE,
                    fields: &fields,
                },
            )
        });
        let card = list_card(ListCard {
            title: tr(Msg::AuditRecords),
            count: Some(u64::from(page.total)),
            order: None,
            tabs: None,
            actions: None,
            body: list,
            pager,
        });
        view! {
            <div class="fui-list-page">
                {toolbar(&filters)}
                {chips(&filters)}
                {card}
            </div>
        }
        .into_any()
    });
    view! {
        <Title text=tr(Msg::NavAudit) />
        {heading()}
        {body}
    }
}

/// One audit record.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn AuditRecordPage() -> impl IntoView {
    let id = use_params_map()
        .with_untracked(|params| params.get("id"))
        .unwrap_or_default();
    let filters = Filters::read();
    let offset = crate::request::query("offset")
        .and_then(|offset| offset.parse::<u32>().ok())
        .unwrap_or(0);
    let page = awaited(async move {
        let found = match filters.search(offset).await {
            Ok(page) if !page.available => return view! { {heading()} {store_off()} }.into_any(),
            Ok(page) => page.rows.into_iter().find(|row| record_id(row) == id),
            Err(error) => {
                return view! { {heading()} {refused(tr(Msg::AuditRefused), &error, None)} }
                    .into_any();
            }
        };
        let Some(row) = found else {
            crate::request::set_status(http::StatusCode::NOT_FOUND);
            return view! {
                {heading()}
                {crate::pages::common::info_note(tr(Msg::AuditRecordGone))}
            }
            .into_any();
        };
        let facts = definitions(vec![
            (tr(Msg::AuditWhen), when(&row.recorded)),
            (
                tr(Msg::AuditPrincipal),
                if row.principal.is_empty() {
                    tr(Msg::AuditUnknown)
                } else {
                    row.principal.clone()
                }
                .into_any(),
            ),
            (tr(Msg::AuditPatient), patient(&row)),
            (
                tr(Msg::AuditWhat),
                view! { <code>{row.resource.clone()}</code> }.into_any(),
            ),
            (tr(Msg::AuditAction), row.action.clone().into_any()),
            (tr(Msg::AuditOutcome), outcome(&row)),
        ]);
        let mut back = filters.pairs("");
        back.retain(|pair| !pair.is_empty());
        let back = if back.is_empty() {
            "/audit".to_owned()
        } else {
            format!("/audit?{}", back.join("&"))
        };
        let heading = page_heading(
            Header {
                title: event(&row).into(),
                subtitle: Some(tr(Msg::AuditStoredAs).into()),
                crumbs: vec![Crumb::new(tr(Msg::NavAudit), back)],
                mono: false,
                badge: None,
            },
            ActionGroup::none(),
        );
        view! {
            <Title text=event(&row) />
            {heading}
            <div class="fui-stack">
                {card_panel(view! {
                    {section(Some(tr(Msg::AuditTheEvent)), None, None, facts)}
                    {document_section(
                        tr(Msg::AuditAsStored),
                        Language::Json,
                        masked_record(&row),
                        "audit-record",
                    )}
                })}
            </div>
        }
        .into_any()
    });
    view! { {page} }
}

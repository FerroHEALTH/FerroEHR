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
//! the instant it was recorded at, since ITI-81 offers no read by id. The patient
//! identifier is masked as everywhere. No openEHR spec governs the viewer's
//! pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
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
use ferrohealth_ui::view::identifier::{Domain, KEEP, Masked};

use crate::bff::PAGE_SIZE;
use crate::bff::audit::search_audit;
use crate::bff::audit::{ACTIONS, AuditPage as Listed, AuditRow, OUTCOMES};
use crate::error::ViewerError;
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{flag, h, opt, t};
use crate::pages::common::{
    Refused, ago, card_panel, date_range, document_section, drawn, section, state_mark, when,
};
use crate::paging::Listing;
use crate::routes::Drawn;

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

/// The page of the record at `place` in the filtered log, carrying the
/// filters and the instant it was recorded at, which finds it again.
// NOTE: an ITI-81 AuditEvent carries no id the search can read back, so a record is found by its instant.
fn record_href(row: &AuditRow, filters: &Filters, place: u32) -> String {
    let mut pairs = filters.pairs("");
    pairs.push(format!("at={}", urlencoding::encode(&row.recorded)));
    format!("/audit/{place}?{}", pairs.join("&"))
}

/// The masked patient identifier of a record.
fn patient(row: &AuditRow) -> Html {
    if row.patient.is_empty() {
        return h(format!(
            r#"<span class="fui-muted">{}</span>"#,
            t(tr(ConsoleMsg::AuditNoPatient))
        ));
    }
    h(format!(
        r#"<span class="fui-identifier"><code>{}</code></span>"#,
        t(Masked::new(Domain::new(String::new()), &row.patient, KEEP).drawn())
    ))
}

/// The stored record with its patient identifier masked as the list masks
/// it.
fn masked_record(row: &AuditRow) -> String {
    let pretty = crate::bff::pretty_body(&row.raw, crate::format::ReprFormat::CanonicalJson);
    if row.patient.is_empty() {
        return pretty;
    }
    pretty.replace(
        &row.patient,
        &Masked::new(Domain::new(String::new()), &row.patient, KEEP).drawn(),
    )
}

/// The outcome of a record as a state.
fn outcome(row: &AuditRow) -> Html {
    if row.success {
        state_mark(FlowState::Up, &tr(ConsoleMsg::AuditSuccess))
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
fn toolbar(filters: &Filters) -> Html {
    let select = |id: &'static str,
                  name: &'static str,
                  label: String,
                  choices: &[(&str, ConsoleMsg)],
                  chosen: &str| {
        let options: Html = choices
            .iter()
            .map(|(value, words)| {
                h(format!(
                    r#"<option value="{}"{}>{}</option>"#,
                    t(value),
                    flag("selected", *value == chosen),
                    t(tr(*words))
                ))
            })
            .collect();
        Field::new(id, label).view(h(format!(
            r#"<select id="{id}" name="{name}" class="fui-select">{options}</select>"#
        )))
    };
    let tools = Html::join([
        select(
            "a-outcome",
            "outcome",
            tr(ConsoleMsg::AuditOutcome),
            &OUTCOMES,
            &filters.outcome,
        ),
        select(
            "a-action",
            "action",
            tr(ConsoleMsg::AuditAction),
            &ACTIONS,
            &filters.action,
        ),
        date_range(
            "a-when",
            tr(ConsoleMsg::AuditWhen),
            &filters.from,
            &filters.to,
        ),
        h(format!(
            r#"<div class="fui-list-find"><label for="a-patient" class="fui-sr-only">{label}</label><input id="a-patient" name="patient" type="text" class="fui-input fui-mono" value="{value}" placeholder="{label}" autocomplete="off"></div><button type="submit" class="fui-button fui-button-secondary">{glyph}{apply}</button>"#,
            label = t(tr(ConsoleMsg::AuditPatient)),
            value = t(&filters.patient),
            glyph = ferrohealth_ui::icon::icon(Glyph::FUNNEL, "16"),
            apply = t(tr(Msg::FilterApply)),
        )),
    ]);
    let label = tr(ConsoleMsg::AuditFind);
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
fn chips(filters: &Filters) -> Option<Html> {
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
            &current(),
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
fn table(page: &Listed, filters: &Filters) -> Html {
    let columns = [
        Column::text("event", tr(ConsoleMsg::AuditEvent)).role(Role::Title),
        Column::text("when", tr(ConsoleMsg::AuditWhen)).role(Role::Sub),
        Column::text("principal", tr(ConsoleMsg::AuditPrincipal)).role(Role::Meta),
        Column::text("patient", tr(ConsoleMsg::AuditPatient)).role(Role::Meta),
        Column::text("outcome", tr(ConsoleMsg::AuditOutcome)).role(Role::Meta),
    ];
    let rows = page
        .rows
        .iter()
        .zip(page.offset..)
        .map(|(row, place)| {
            let href = record_href(row, filters, place);
            let principal = if row.principal.is_empty() {
                h(format!(
                    r#"<span class="fui-muted">{}</span>"#,
                    t(tr(ConsoleMsg::AuditUnknown))
                ))
            } else {
                t(&row.principal)
            };
            h(format!(
                "<tr>{}{}{}{}{}</tr>",
                title_cell(Some(&href), t(event(row)), None),
                cell(&columns[1], ago(&row.recorded)),
                cell(&columns[2], principal),
                cell(&columns[3], patient(row)),
                cell(&columns[4], outcome(row))
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::AuditRecords);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows),
    })
}

/// The heading of the Audit log.
fn heading() -> Html {
    page_heading(
        &current(),
        Header {
            title: tr(Msg::NavAudit).into(),
            subtitle: Some(tr(ConsoleMsg::AuditSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    )
}

/// The note of a CDR that keeps no local audit store.
fn store_off() -> Html {
    card_panel(h(format!(
        r#"<div class="fui-panel-body fui-stack"><p>{}</p><p class="fui-small fui-muted">{}</p></div>"#,
        t(tr(ConsoleMsg::AuditStoreOff)),
        t(tr(ConsoleMsg::AuditStoreOffLine))
    )))
}

/// The Audit log's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LogModel {
    /// The page of records the kept position finds.
    Listed(Listed),
    /// The CDR keeps no local audit store.
    StoreOff,
    /// The CDR's refusal.
    Refused(Refused),
}

/// Loads the Audit log: the page of records the request's filters and the
/// kept position find, read again at the last page when the kept one is past
/// it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_log(
) -> Result<LogModel, ViewerError> {
    crate::session::require_session()?;
    Ok(read_log().await)
}

/// What [`load_log`] reads.
async fn read_log() -> LogModel {
    let filters = Filters::read();
    let paging = Paging {
        page: crate::paging::current(Listing::Audit).page,
        size: PAGE_SIZE,
    };
    let asked = Window::over(u32::MAX, paging);
    let refused = |error| LogModel::Refused(Refused::new(tr(ConsoleMsg::AuditRefused), error));
    let page = match filters.search(asked.start).await {
        Ok(page) => page,
        Err(error) => return refused(error),
    };
    if !page.available {
        return LogModel::StoreOff;
    }
    let window = Window::over(page.total, paging);
    if window.start == asked.start {
        return LogModel::Listed(page);
    }
    match filters.search(window.start).await {
        Ok(page) => LogModel::Listed(page),
        Err(error) => refused(error),
    }
}

/// The Audit log's list card over the page `page`.
fn log_card(page: &Listed, filters: &Filters) -> Html {
    let paging = Paging {
        page: crate::paging::current(Listing::Audit).page,
        size: PAGE_SIZE,
    };
    let fields = crate::paging::fields(Listing::Audit);
    let window = Window::over(page.total, paging);
    let list = if page.rows.is_empty() {
        list_empty(
            tr(if filters.any() {
                ConsoleMsg::AuditNoMatch
            } else {
                ConsoleMsg::AuditEmpty
            }),
            None,
        )
    } else {
        table(page, filters)
    };
    let pager = (window.pages > 1).then(|| {
        post_pager(
            &current(),
            PagerListing::new(tr(ConsoleMsg::AuditPages)).sizes(SIZES),
            window,
            PostTo {
                id: "audit-pager",
                action: crate::paging::PAGING_ROUTE,
                fields: &fields,
            },
        )
    });
    list_card(&current(), ListCard {
        title: tr(ConsoleMsg::AuditRecords),
        count: Some(ferrohealth_ui::view::template::list::HeadingCount::All(
            u64::from(page.total),
        )),
        order: None,
        tabs: None,
        actions: None,
        body: list,
        pager,
    })
}

/// The Audit log.
/// The audit log.
pub async fn page() -> Drawn {
    let body = drawn(load_log().await, |model| match model {
        LogModel::Listed(page) => {
            let filters = Filters::read();
            let card = log_card(&page, &filters);
            h(format!(
                r#"<div class="fui-list-page">{}{}{card}</div>"#,
                toolbar(&filters),
                opt(chips(&filters))
            ))
        }
        LogModel::StoreOff => store_off(),
        LogModel::Refused(refusal) => refusal.view(),
    });
    Drawn::page(tr(Msg::NavAudit), heading().then(body))
}

/// One audit record's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordModel {
    /// The record, and its stored form with the patient identifier masked.
    Found {
        /// The record.
        row: Box<AuditRow>,
        /// Its stored form, masked.
        masked: String,
    },
    /// No record answers the address.
    Gone,
    /// The CDR keeps no local audit store.
    StoreOff,
    /// The CDR's refusal.
    Refused(Refused),
}

/// Loads one audit record: found again by the filters and the instant it
/// was recorded at.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
async fn load_record(
    // The record's place the route names.
    id: Option<String>,
) -> Result<RecordModel, ViewerError> {
    crate::session::require_session()?;
    Ok(read_record(id).await)
}

/// What [`load_record`] reads.
async fn read_record(id: Option<String>) -> RecordModel {
    let place = id.and_then(|id| id.parse::<u32>().ok());
    if place.is_none() {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
        return RecordModel::Gone;
    }
    let at = crate::request::query("at").unwrap_or_default();
    let mut bounded = Filters::read();
    bounded.to.clone_from(&at);
    let found = match bounded.search(0).await {
        Ok(page) if !page.available => return RecordModel::StoreOff,
        Ok(page) => page.rows.into_iter().find(|row| row.recorded == at),
        Err(error) => {
            return RecordModel::Refused(Refused::new(tr(ConsoleMsg::AuditRefused), error));
        }
    };
    let Some(row) = found else {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
        return RecordModel::Gone;
    };
    let masked = masked_record(&row);
    RecordModel::Found {
        row: Box::new(row),
        masked,
    }
}

/// One audit record, drawn.
fn record(row: &AuditRow, masked: String) -> Html {
    let principal = if row.principal.is_empty() {
        tr(ConsoleMsg::AuditUnknown)
    } else {
        row.principal.clone()
    };
    let facts = definitions(vec![
        (tr(ConsoleMsg::AuditWhen), when(&row.recorded)),
        (tr(ConsoleMsg::AuditPrincipal), t(principal)),
        (tr(ConsoleMsg::AuditPatient), patient(row)),
        (
            tr(ConsoleMsg::AuditWhat),
            h(format!("<code>{}</code>", t(&row.resource))),
        ),
        (tr(ConsoleMsg::AuditAction), t(&row.action)),
        (tr(ConsoleMsg::AuditOutcome), outcome(row)),
    ]);
    let back = Filters::read().pairs("");
    let back = if back.is_empty() {
        "/audit".to_owned()
    } else {
        format!("/audit?{}", back.join("&"))
    };
    let heading = page_heading(
        &current(),
        Header {
            title: event(row).into(),
            subtitle: Some(tr(ConsoleMsg::AuditStoredAs).into()),
            crumbs: vec![Crumb::new(tr(Msg::NavAudit), back)],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let panel = card_panel(
        section(Some(tr(ConsoleMsg::AuditTheEvent)), None, None, facts).then(
            document_section(
                tr(ConsoleMsg::AuditAsStored),
                Language::Json,
                masked,
                "audit-record",
            ),
        ),
    );
    heading.then(h(format!(r#"<div class="fui-stack">{panel}</div>"#)))
}

/// One audit record.
/// One audit record.
pub async fn record_page() -> Drawn {
    let id = crate::routes::params().get("id");
    match load_record(id).await {
        Ok(RecordModel::Found { row, masked }) => Drawn::page(event(&row), record(&row, masked)),
        Ok(RecordModel::Gone) => Drawn::page(
            tr(Msg::NavAudit),
            heading().then(crate::pages::common::info_note(t(tr(
                ConsoleMsg::AuditRecordGone,
            )))),
        ),
        Ok(RecordModel::StoreOff) => Drawn::page(tr(Msg::NavAudit), heading().then(store_off())),
        Ok(RecordModel::Refused(refusal)) => {
            Drawn::page(tr(Msg::NavAudit), heading().then(refusal.view()))
        }
        Err(error) => Drawn::page(
            tr(Msg::NavAudit),
            heading().then(drawn(Err::<(), _>(error), |()| Html::empty())),
        ),
    }
}

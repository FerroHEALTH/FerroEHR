// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Access log console page: the review of the log across EHRs, and its
//! integrity (the seals, the newest checkpoint and a verification).
//!
//! The records are read from `{ext}/admin/access-log` in the full view, the
//! filters a `GET` form sent to the CDR as they are; the review states no
//! total, so it pages with Previous and Next alone, its position kept in the
//! browser's session ([`crate::paging`]). An entry opens on its EHR's access
//! log, where an administrator erases it. No openEHR spec governs the
//! viewer's pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::model::OpenWindow;
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Header, definitions, page_heading};
use ferrohealth_ui::view::tabs::{Tab, binder};
use ferrohealth_ui::view::template::list::{ActiveFilters, FilterChip, active_filters, list_empty};

use crate::bff::access_log::Coded;
use crate::bff::access_review::{
    Checkpoint, Record, ReviewFilters, Seal, Verification, newest_checkpoint, review, seals,
    sign_checkpoint, verify,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::access::AccessMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::access_log::{grounds_select, outcome_select, outcome_words, row_parts};
use crate::pages::common::{
    Refused, ago, date_range, document_section, info_note, is_status, section, short, warn_note,
    when,
};
use crate::pages::ehr::{EhrTab, tab_href};
use crate::paging::Listing;
use crate::routes::Drawn;

/// The route of the review.
const REVIEW_ROUTE: &str = "/access-log";

/// The route of the integrity tab.
const INTEGRITY_ROUTE: &str = "/access-log/integrity";

/// The widest span of entry ids the CDR answers seals for.
const SEAL_SPAN: i64 = 9999;

/// The text filters of the review: the address key, the field id and its
/// label.
const TEXT_FILTERS: [(&str, &str, AccessMsg); 5] = [
    ("ehr_id", "r-ehr", AccessMsg::ReviewFilterEhr),
    ("party_id", "r-party", AccessMsg::ReviewFilterParty),
    ("issuer", "r-issuer", AccessMsg::ReviewFilterIssuer),
    ("subject", "r-subject", AccessMsg::ReviewFilterSubject),
    ("request_id", "r-request", AccessMsg::ReviewFilterRequest),
];

/// The filters the address names.
fn read_filters() -> ReviewFilters {
    let get = |key: &str| crate::request::query(key).unwrap_or_default();
    ReviewFilters {
        ehr_id: get("ehr_id"),
        party_id: get("party_id"),
        issuer: get("issuer"),
        subject: get("subject"),
        request_id: get("request_id"),
        from: get("from"),
        to: get("to"),
        outcome: get("outcome"),
        justification: get("justification"),
    }
}

/// The value of the filter `key` of `filters`.
fn value_of<'a>(filters: &'a ReviewFilters, key: &str) -> &'a str {
    match key {
        "ehr_id" => &filters.ehr_id,
        "party_id" => &filters.party_id,
        "issuer" => &filters.issuer,
        "subject" => &filters.subject,
        "request_id" => &filters.request_id,
        "from" => &filters.from,
        "to" => &filters.to,
        "outcome" => &filters.outcome,
        "justification" => &filters.justification,
        _ => "",
    }
}

/// Every filter key, in the address's order.
const KEYS: [&str; 9] = [
    "ehr_id",
    "party_id",
    "issuer",
    "subject",
    "request_id",
    "from",
    "to",
    "outcome",
    "justification",
];

/// The review with these filters but `without`.
fn href_without(filters: &ReviewFilters, without: &str) -> String {
    let pairs: Vec<String> = KEYS
        .into_iter()
        .filter(|key| *key != without && !value_of(filters, key).is_empty())
        .map(|key| format!("{key}={}", urlencoding::encode(value_of(filters, key))))
        .collect();
    if pairs.is_empty() {
        REVIEW_ROUTE.to_owned()
    } else {
        format!("{REVIEW_ROUTE}?{}", pairs.join("&"))
    }
}

/// The heading of the page.
fn heading() -> Html {
    page_heading(
        &current(),
        Header {
            title: tr(AccessMsg::ReviewTitle).into(),
            subtitle: Some(tr(AccessMsg::ReviewSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    )
}

/// The page's tabs over `body`, `current` the one shown.
fn tabbed(current_tab: &str, body: &Html) -> Html {
    let tabs = vec![
        Tab::new("records", REVIEW_ROUTE, tr(AccessMsg::ReviewTabRecords)),
        Tab::new(
            "integrity",
            INTEGRITY_ROUTE,
            tr(AccessMsg::ReviewTabIntegrity),
        ),
    ];
    binder(
        &current(),
        tr(AccessMsg::ReviewViews),
        tabs,
        current_tab,
        h(format!(
            r#"<div class="fui-panel"><div class="fui-panel-body fui-stack">{body}</div></div>"#
        )),
    )
}

/// What a refused read of the review says: who reviews for a `403`, that the
/// admin group is off for a `405`, that the audit store is off for a `409`,
/// else the CDR's own refusal of what `summary` says.
fn refusal(error: ViewerError, summary: AccessMsg) -> Html {
    if is_status(&error, http::StatusCode::FORBIDDEN) {
        info_note(t(tr(AccessMsg::ReviewNotReviewer)))
    } else if is_status(&error, http::StatusCode::METHOD_NOT_ALLOWED) {
        info_note(t(tr(AccessMsg::ReviewAdminOff)))
    } else if is_status(&error, http::StatusCode::CONFLICT) {
        info_note(t(tr(AccessMsg::StoreOff)))
    } else {
        Refused::new(tr(summary), error).view()
    }
}

/// The filter toolbar, a `GET` form to the review.
fn toolbar(filters: &ReviewFilters) -> Html {
    let texts: Html = TEXT_FILTERS
        .into_iter()
        .map(|(key, id, label)| {
            Field::new(id, tr(label)).view(h(format!(
                r#"<input id="{id}" name="{key}" type="text" class="fui-input fui-mono" value="{}" autocomplete="off" spellcheck="false">"#,
                t(value_of(filters, key))
            )))
        })
        .collect();
    let tools = Html::join([
        texts,
        outcome_select("r-outcome", &filters.outcome),
        grounds_select("r-ground", &filters.justification),
        date_range(
            "r-when",
            tr(AccessMsg::FilterWhen),
            &filters.from,
            &filters.to,
        ),
        h(format!(
            r#"<button type="submit" class="fui-button fui-button-secondary">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::FUNNEL, "16"),
            t(tr(Msg::FilterApply))
        )),
    ]);
    h(format!(
        r#"<form method="get" action="{REVIEW_ROUTE}" class="fui-list-toolbar"><div class="fui-list-toolbar-start">{tools}</div></form>"#
    ))
}

/// The active filters as chips.
fn chips(filters: &ReviewFilters) -> Option<Html> {
    let mut chips: Vec<FilterChip> = TEXT_FILTERS
        .into_iter()
        .filter(|(key, _, _)| !value_of(filters, key).is_empty())
        .map(|(key, _, label)| FilterChip {
            label: tr_with(
                AccessMsg::ReviewChip,
                &[
                    ("name", tr(label).into()),
                    ("value", value_of(filters, key).to_owned().into()),
                ],
            ),
            remove: href_without(filters, key),
        })
        .collect();
    for (key, msg) in [("from", Msg::ChipFrom), ("to", Msg::ChipTo)] {
        if !value_of(filters, key).is_empty() {
            chips.push(FilterChip {
                label: tr_with(msg, &[("value", value_of(filters, key).to_owned().into())]),
                remove: href_without(filters, key),
            });
        }
    }
    for (key, msg) in [
        ("outcome", AccessMsg::ChipOutcome),
        ("justification", AccessMsg::ChipJustified),
    ] {
        if !value_of(filters, key).is_empty() {
            chips.push(FilterChip {
                label: tr_with(
                    msg,
                    &[("value", outcome_words(value_of(filters, key)).into())],
                ),
                remove: href_without(filters, key),
            });
        }
    }
    (!chips.is_empty()).then(|| {
        active_filters(
            &current(),
            ActiveFilters {
                chips,
                clear: REVIEW_ROUTE.to_owned(),
                shown: None,
            },
            None,
        )
    })
}

/// The records as a table: an entry opens on its EHR's access log, a
/// tombstone names its grounds.
fn table(records: &[Record]) -> Html {
    let columns = [
        Column::text("when", tr(AccessMsg::ColumnWhen)).role(Role::Title),
        Column::text("ehr", tr(AccessMsg::ColumnEhr)).role(Role::Sub),
        Column::text("accessor", tr(AccessMsg::ColumnAccessor)).role(Role::Meta),
        Column::text("action", tr(AccessMsg::ColumnAction)).role(Role::Meta),
        Column::text("outcome", tr(AccessMsg::ColumnOutcome)).role(Role::Meta),
    ];
    let rows = records
        .iter()
        .map(|record| match record {
            Record::Entry(entry, ehr) => {
                let href = ehr.as_ref().map(|ehr| {
                    format!(
                        "{}/{}",
                        tab_href(ehr, EhrTab::AccessLog),
                        urlencoding::encode(&entry.uid)
                    )
                });
                let (accessor, action, outcome) = row_parts(entry);
                h(format!(
                    "<tr>{}{}{}{}{}</tr>",
                    title_cell(None, href.as_deref(), ago(&entry.recorded), None),
                    cell(&columns[1], ehr.as_deref().map_or_else(Html::empty, short)),
                    cell(&columns[2], accessor),
                    cell(&columns[3], action),
                    cell(&columns[4], outcome)
                ))
            }
            Record::Tombstone(tombstone) => {
                let grounds: Vec<&str> = tombstone.grounds.iter().map(Coded::words).collect();
                h(format!(
                    r#"<tr>{}{}<td colspan="3">{}</td></tr>"#,
                    title_cell(None, None, ago(&tombstone.recorded), None),
                    cell(
                        &columns[1],
                        t(tr_with(
                            AccessMsg::TombstoneRow,
                            &[(
                                "when",
                                ferrohealth_ui::view::time::reading(&current(), &tombstone.removed)
                                    .into()
                            )]
                        ))
                    ),
                    t(tr_with(
                        AccessMsg::TombstoneGrounds,
                        &[("grounds", grounds.join(", ").into())]
                    ))
                ))
            }
        })
        .collect();
    let caption = tr(AccessMsg::Caption);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The review: the filters, the records and the pager.
async fn records_tab() -> Html {
    let filters = read_filters();
    let window = OpenWindow::of(crate::paging::current(Listing::AccessReview), 0);
    let records = match review(&filters, window.start, window.size).await {
        Ok(records) => records,
        Err(error) => return refusal(error, AccessMsg::Refused),
    };
    let held = u32::try_from(records.len()).unwrap_or(u32::MAX);
    let window = OpenWindow::of(crate::paging::current(Listing::AccessReview), held);
    let any = KEYS
        .into_iter()
        .any(|key| !value_of(&filters, key).is_empty());
    let list = if records.is_empty() && window.page == 1 {
        list_empty(
            tr(if any {
                AccessMsg::NoMatch
            } else {
                AccessMsg::Empty
            }),
            None,
        )
    } else {
        table(&records)
    };
    let pager = (window.page > 1 || window.has_next()).then(|| {
        let fields = crate::paging::fields(Listing::AccessReview);
        post_open_pager(
            &current(),
            PagerListing::new(tr(AccessMsg::Pages)),
            window,
            PostTo {
                id: "access-review-pager",
                action: crate::paging::PAGING_ROUTE,
                fields: &fields,
            },
        )
    });
    Html::join([
        h(format!(
            r#"<p class="fui-small fui-muted">{}</p>"#,
            t(tr(AccessMsg::ReviewIntro))
        )),
        toolbar(&filters),
        opt(chips(&filters)),
        list,
        opt(pager),
    ])
}

/// The Access log console page.
pub async fn page() -> Drawn {
    let body = tabbed("records", &records_tab().await);
    Drawn::page(tr(AccessMsg::ReviewTitle), heading().then(body))
}

/// The newest checkpoint, drawn, with Sign a checkpoint now.
fn checkpoint_section(checkpoint: Result<Option<Checkpoint>, ViewerError>) -> Html {
    let sign = h(format!(
        r#"<form method="post" action="{INTEGRITY_ROUTE}"><button type="submit" name="intent" value="sign" class="fui-button fui-button-secondary">{}{}</button></form>"#,
        ferrohealth_ui::icon::icon(Glyph::KEY_ROUND, "16"),
        t(tr(AccessMsg::CheckpointSign))
    ));
    let body = match checkpoint {
        Ok(None) => info_note(t(tr(AccessMsg::CheckpointNone))),
        Ok(Some(checkpoint)) => definitions(vec![
            (
                tr(AccessMsg::CheckpointSequence),
                t(checkpoint.sequence.to_string()),
            ),
            (
                tr(AccessMsg::CheckpointDigest),
                h(format!("<code>{}</code>", t(&checkpoint.digest))),
            ),
            (tr(AccessMsg::CheckpointAlgorithm), t(&checkpoint.algorithm)),
            (tr(AccessMsg::CheckpointSealed), when(&checkpoint.sealed)),
            (tr(AccessMsg::CheckpointSigner), t(&checkpoint.signer)),
        ])
        .then(document_section(
            tr(AccessMsg::CheckpointProof),
            Language::Plain,
            checkpoint.proof,
            "checkpoint-proof",
        )),
        Err(error) => refusal(error, AccessMsg::CheckpointRefused),
    };
    section(Some(tr(AccessMsg::CheckpointTitle)), None, Some(sign), body)
}

/// The verification: its result when one ran, and Verify the log.
fn verify_section(ran: Option<Result<Verification, ViewerError>>) -> Html {
    let run = h(format!(
        r#"<form method="post" action="{INTEGRITY_ROUTE}"><button type="submit" name="intent" value="verify" class="fui-button fui-button-secondary">{}{}</button></form>"#,
        ferrohealth_ui::icon::icon(Glyph::SHIELD_CHECK, "16"),
        t(tr(AccessMsg::VerifyRun))
    ));
    let body = match ran {
        None => Html::empty(),
        Some(Err(error)) => refusal(error, AccessMsg::VerifyRefused),
        Some(Ok(found)) if found.findings.is_empty() => match (found.intact, found.checkpoint) {
            (true, Some(sequence)) => info_note(t(tr_with(
                AccessMsg::VerifyIntact,
                &[("sequence", sequence.to_string().into())],
            ))),
            _ => warn_note(t(tr(AccessMsg::VerifyNoCheckpoint))),
        },
        Some(Ok(found)) => {
            let count = u64::try_from(found.findings.len()).unwrap_or(u64::MAX);
            let items: Html = found
                .findings
                .iter()
                .map(|finding| h(format!("<li>{}</li>", t(finding))))
                .collect();
            warn_note(t(tr_with(
                AccessMsg::VerifyFindings,
                &[("count", count.into())],
            )))
            .then(h(format!(r#"<ul class="fui-stack">{items}</ul>"#)))
        }
    };
    section(Some(tr(AccessMsg::VerifyTitle)), None, Some(run), body)
}

/// The span of entry ids the address names, the last [`SEAL_SPAN`] the
/// newest checkpoint covers by default.
fn seal_span(newest: Option<i64>) -> (i64, i64) {
    let read = |key: &str| crate::request::query(key).and_then(|value| value.parse::<i64>().ok());
    let to = read("to_id").or(newest).unwrap_or(SEAL_SPAN + 1).max(1);
    let from = read("from_id")
        .unwrap_or_else(|| to.saturating_sub(SEAL_SPAN))
        .clamp(1, to);
    (from, to)
}

/// The seals over `span`, with the form that names another span.
fn seals_section(span: (i64, i64), found: Result<Vec<Seal>, ViewerError>) -> Html {
    let field = |id: &str, name: &str, label: AccessMsg, value: i64| {
        Field::new(id, tr(label)).view(h(format!(
            r#"<input id="{id}" name="{name}" type="number" min="1" class="fui-input fui-mono" value="{value}">"#
        )))
    };
    let form = h(format!(
        r#"<form method="get" action="{INTEGRITY_ROUTE}" class="fui-list-toolbar"><div class="fui-list-toolbar-start">{}{}<button type="submit" class="fui-button fui-button-secondary">{}</button></div></form>"#,
        field("s-from", "from_id", AccessMsg::SealsFrom, span.0),
        field("s-to", "to_id", AccessMsg::SealsTo, span.1),
        t(tr(AccessMsg::SealsShow))
    ));
    let body = match found {
        Ok(found) if found.is_empty() => list_empty(tr(AccessMsg::SealsNone), None),
        Ok(found) => {
            let columns = [
                Column::text("window", tr(AccessMsg::ColumnWindow)).role(Role::Title),
                Column::text("span", tr(AccessMsg::ColumnSpan)).role(Role::Sub),
                Column::text("count", tr(AccessMsg::ColumnCount)).role(Role::Meta),
                Column::text("digest", tr(AccessMsg::ColumnDigest)).role(Role::Meta),
                Column::text("previous", tr(AccessMsg::ColumnPrevious)).role(Role::Meta),
                Column::text("folded", tr(AccessMsg::ColumnFolded)).role(Role::Meta),
            ];
            let rows = found
                .iter()
                .map(|seal| {
                    h(format!(
                        r#"<tr><th scope="row" data-role="title">{}</th>{}{}{}{}{}</tr>"#,
                        t(seal.window.to_string()),
                        cell(
                            &columns[1],
                            t(tr_with(
                                AccessMsg::SealSpan,
                                &[
                                    ("first", seal.first.to_string().into()),
                                    ("last", seal.last.to_string().into())
                                ]
                            ))
                        ),
                        cell(&columns[2], t(seal.entries.to_string())),
                        cell(&columns[3], short(&seal.digest)),
                        cell(&columns[4], short(&seal.previous)),
                        cell(
                            &columns[5],
                            t(tr(if seal.folded { Msg::Yes } else { Msg::No }))
                        )
                    ))
                })
                .collect();
            let caption = tr(AccessMsg::SealsCaption);
            data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
        }
        Err(error) => refusal(error, AccessMsg::SealsRefused),
    };
    section(Some(tr(AccessMsg::SealsTitle)), None, None, form.then(body))
}

/// The integrity tab: a post signs a checkpoint or runs a verification.
async fn integrity_tab() -> Html {
    let intent = crate::posted::posted().map(|posted| posted.intent());
    let mut acted = None;
    let mut ran = None;
    match intent.as_deref() {
        Some("sign") => match sign_checkpoint().await {
            Ok(_) => {
                redirect_with(INTEGRITY_ROUTE, &Done::ok(tr(AccessMsg::CheckpointSigned)));
                return Html::empty();
            }
            Err(error) => {
                acted = Some(Refused::new(tr(AccessMsg::CheckpointSignRefused), error).view());
            }
        },
        Some("verify") => ran = Some(verify().await),
        _ => {}
    }
    let checkpoint = newest_checkpoint().await;
    let newest = checkpoint
        .as_ref()
        .ok()
        .and_then(|found| found.as_ref())
        .map(|found| found.sequence);
    let span = seal_span(newest);
    let found = seals(span.0, span.1).await;
    Html::join([
        h(format!(
            r#"<p class="fui-small fui-muted">{}</p>"#,
            t(tr(AccessMsg::IntegrityIntro))
        )),
        opt(acted),
        checkpoint_section(checkpoint),
        verify_section(ran),
        seals_section(span, found),
    ])
}

/// The Access log console page's Integrity tab.
pub async fn integrity_page() -> Drawn {
    let body = tabbed("integrity", &integrity_tab().await);
    Drawn::page(tr(AccessMsg::ReviewTitle), heading().then(body))
}

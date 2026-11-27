// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server › Integrity: the sweep that checks the stored data against itself,
//! and the rebuild of what it finds damaged, behind a confirmation
//! ([`crate::bff::integrity`]).
//!
//! The scope is a posted form: Verify posts it and the page draws the
//! report; a report with findings offers Rebuild, which posts the same scope
//! after its confirmation and draws the rebuild's report. No openEHR spec
//! governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table};
use ferrohealth_ui::view::dialog::{Confirmation, Stakes, confirm_dialog};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::definitions;

use super::{ServerTab, Tabbed, load_tab, server_page};
use crate::bff::integrity::{REBUILD, Rebuild, Scope, VERIFY, Verification, run};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    Refused, info_note, is_status, plain_section, section, short, warn_note,
};
use crate::routes::Drawn;

/// The route of the tab.
const ROUTE: &str = "/server/integrity";

/// The id of the scope's form.
const FORM: &str = "integrity-scope";

/// The id of Rebuild's confirmation.
const REBUILD_DIALOG: &str = "rebuild-nodes";

/// What the post ran.
#[derive(Debug, Clone, PartialEq)]
enum Ran {
    /// Nothing yet.
    Nothing,
    /// The sweep, or its refusal.
    Verified(Result<Verification, Refused>),
    /// The rebuild, or its refusal.
    Rebuilt(Result<Rebuild, Refused>),
}

/// Integrity's model: the scope as typed and what the post ran over it.
#[derive(Debug, Clone, PartialEq)]
struct Integrity {
    /// The scope the form holds.
    scope: Scope,
    /// What ran.
    ran: Ran,
}

/// What the page reads, on the server.
async fn read_integrity() -> Tabbed<Integrity> {
    load_tab(ServerTab::Integrity, || async {
        let posted = crate::posted::posted();
        let text = |key: &str| {
            posted
                .as_ref()
                .map(|posted| posted.text(key))
                .unwrap_or_default()
        };
        let scope = Scope {
            ehr_id: text("ehr_id"),
            vo_id: text("vo_id"),
            sys_version: text("sys_version"),
            committed_since: text("committed_since"),
        };
        let intent = posted.as_ref().map(crate::posted::Posted::intent);
        let ran = match intent.as_deref() {
            Some("verify") => Ran::Verified(
                run::<Verification>(VERIFY, &scope)
                    .await
                    .map_err(|error| Refused::new(tr(ConsoleMsg::IntegrityVerifyRefused), error)),
            ),
            Some("rebuild") => Ran::Rebuilt(
                run::<Rebuild>(REBUILD, &scope)
                    .await
                    .map_err(|error| Refused::new(tr(ConsoleMsg::IntegrityRebuildRefused), error)),
            ),
            _ => Ran::Nothing,
        };
        Integrity { scope, ran }
    })
    .await
}

/// One scope field.
fn field(id: &str, name: &str, label: ConsoleMsg, value: &str) -> Html {
    Field::new(id, tr(label)).holds_id().view(h(format!(
        r#"<input id="{id}" name="{name}" type="text" class="fui-input fui-mono" value="{}" autocomplete="off">"#,
        t(value)
    )))
}

/// The scope's form with Verify, the refusal of the last run beside its
/// fields.
fn scope_form(scope: &Scope, refusal: Option<&Refused>) -> Html {
    let form = h(format!(
        r#"<form id="{FORM}" method="post" action="{ROUTE}" class="fui-transform-options">{ehr}{vo}{version}{since}<div class="fui-transform-run"><button type="submit" name="intent" value="verify" class="fui-button fui-button-primary">{glyph}{verify}</button></div></form>"#,
        ehr = field("i-ehr", "ehr_id", ConsoleMsg::IntegrityEhr, &scope.ehr_id),
        vo = field("i-vo", "vo_id", ConsoleMsg::IntegrityObject, &scope.vo_id),
        version = field(
            "i-version",
            "sys_version",
            ConsoleMsg::IntegrityVersion,
            &scope.sys_version
        ),
        since = field(
            "i-since",
            "committed_since",
            ConsoleMsg::IntegritySince,
            &scope.committed_since
        ),
        glyph = ferrohealth_ui::icon::icon(Glyph::CHECK, "16"),
        verify = t(tr(ConsoleMsg::IntegrityVerify)),
    ));
    section(
        Some(tr(ConsoleMsg::IntegrityScope)),
        None,
        None,
        ferrohealth_ui::view::surface::section_intro(tr(ConsoleMsg::IntegrityScopeIntro))
            .then(form)
            .then(refusal.map(Refused::view).unwrap_or_default()),
    )
}

/// The confirmation of Rebuild, which posts the scope's form.
fn rebuild_dialog() -> Html {
    confirm_dialog(
        &current(),
        Confirmation {
            id: REBUILD_DIALOG.into(),
            title: tr(ConsoleMsg::IntegrityRebuildTitle).into(),
            message: tr(ConsoleMsg::IntegrityRebuildMessage).into(),
        },
        Stakes::Plain,
        None,
        h(format!(
            r#"<button type="submit" form="{FORM}" name="intent" value="rebuild" class="fui-button fui-button-primary">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::ROTATE_CCW, "16"),
            t(tr(ConsoleMsg::IntegrityRebuildConfirm))
        )),
    )
}

/// The ordinal of a version, or the empty mark for rows with none.
fn ordinal(version: Option<u64>) -> Html {
    t(version.map_or_else(|| tr(Msg::NoValue), |version| version.to_string()))
}

/// The sweep's report, with Rebuild when it found anything.
fn verification(report: &Verification) -> Html {
    let facts = definitions(vec![
        (
            tr(ConsoleMsg::IntegrityChecked),
            t(report.versions_checked.to_string()),
        ),
        (
            tr(ConsoleMsg::IntegrityFindings),
            t(report.mismatch_count.to_string()),
        ),
        (
            tr(ConsoleMsg::IntegrityElapsed),
            t(tr_with(
                ConsoleMsg::IntegrityMilliseconds,
                &[("ms", report.elapsed_ms.into())],
            )),
        ),
    ]);
    if report.mismatch_count == 0 {
        return section(
            Some(tr(ConsoleMsg::IntegrityReport)),
            None,
            None,
            facts.then(info_note(t(tr(ConsoleMsg::IntegrityClean)))),
        );
    }
    let columns = [
        Column::text("object", tr(ConsoleMsg::IntegrityObject)).role(Role::Title),
        Column::text("version", tr(ConsoleMsg::IntegrityVersion)).role(Role::Meta),
        Column::text("domain", tr(ConsoleMsg::IntegrityDomain)).role(Role::Sub),
        Column::text("kind", tr(ConsoleMsg::IntegrityKind)).role(Role::Meta),
        Column::text("defect", tr(ConsoleMsg::IntegrityDefect)).role(Role::Sub),
    ];
    let rows = report
        .mismatches
        .iter()
        .map(|finding| {
            h(format!(
                r#"<tr><th scope="row" data-role="title">{}</th>{}{}{}{}</tr>"#,
                short(&finding.vo_id),
                cell(&columns[1], ordinal(finding.sys_version)),
                cell(&columns[2], t(&finding.domain)),
                cell(&columns[3], t(&finding.kind)),
                cell(
                    &columns[4],
                    h(format!("<code>{}</code>", t(&finding.defect)))
                ),
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::IntegrityFindings);
    let rebuild = h(format!(
        r#"<button type="button" class="fui-button fui-button-secondary" command="show-modal" commandfor="{REBUILD_DIALOG}" aria-haspopup="dialog">{}{}</button>"#,
        ferrohealth_ui::icon::icon(Glyph::ROTATE_CCW, "16"),
        t(tr(ConsoleMsg::IntegrityRebuild))
    ));
    Html::join([
        section(Some(tr(ConsoleMsg::IntegrityReport)), None, None, facts),
        section(
            Some(caption.clone()),
            Some(report.mismatch_count),
            Some(rebuild),
            opt(report
                .truncated
                .then(|| warn_note(t(tr(ConsoleMsg::IntegrityTruncated)))))
            .then(data_table(DataTable::new(
                &caption,
                &columns,
                Body::Rows(rows),
            ))),
        ),
        rebuild_dialog(),
    ])
}

/// The rebuild's report.
fn rebuilt(report: &Rebuild) -> Html {
    let facts = definitions(vec![
        (
            tr(ConsoleMsg::IntegrityChecked),
            t(report.versions_checked.to_string()),
        ),
        (
            tr(ConsoleMsg::IntegrityDamaged),
            t(report.versions_damaged.to_string()),
        ),
        (
            tr(ConsoleMsg::IntegrityRebuiltCount),
            t(report.versions_rebuilt.to_string()),
        ),
        (
            tr(ConsoleMsg::IntegrityRefusedCount),
            t(report.versions_refused.to_string()),
        ),
    ]);
    let columns = [
        Column::text("object", tr(ConsoleMsg::IntegrityObject)).role(Role::Title),
        Column::text("version", tr(ConsoleMsg::IntegrityVersion)).role(Role::Meta),
        Column::text("defect", tr(ConsoleMsg::IntegrityDefect)).role(Role::Sub),
        Column::text("outcome", tr(ConsoleMsg::IntegrityOutcome)).role(Role::Sub),
        Column::text("reason", tr(ConsoleMsg::IntegrityReason)).role(Role::Meta),
    ];
    let rows: Vec<Html> = report
        .records
        .iter()
        .map(|record| {
            h(format!(
                r#"<tr><th scope="row" data-role="title">{}</th>{}{}{}{}</tr>"#,
                short(&record.vo_id),
                cell(&columns[1], ordinal(record.sys_version)),
                cell(
                    &columns[2],
                    h(format!("<code>{}</code>", t(&record.defect)))
                ),
                cell(&columns[3], t(&record.outcome)),
                cell(&columns[4], t(record.reason.clone().unwrap_or_default())),
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::IntegrityRecords);
    let records = (!rows.is_empty()).then(|| {
        section(
            Some(caption.clone()),
            Some(u64::try_from(rows.len()).unwrap_or(u64::MAX)),
            None,
            opt(report
                .truncated
                .then(|| warn_note(t(tr(ConsoleMsg::IntegrityTruncated)))))
            .then(data_table(DataTable::new(
                &caption,
                &columns,
                Body::Rows(Html::join(rows)),
            ))),
        )
    });
    section(
        Some(tr(ConsoleMsg::IntegrityRebuildReport)),
        None,
        None,
        facts,
    )
    .then(opt(records))
}

/// Integrity, drawn.
fn integrity(model: Integrity) -> Html {
    let Integrity { scope, ran } = model;
    let refusal = match &ran {
        Ran::Verified(Err(refusal)) | Ran::Rebuilt(Err(refusal)) => Some(refusal),
        _ => None,
    };
    if refusal
        .is_some_and(|refusal| is_status(&refusal.error, http::StatusCode::METHOD_NOT_ALLOWED))
    {
        return plain_section(info_note(t(tr(ConsoleMsg::ServerNoAdmin))));
    }
    let report = match &ran {
        Ran::Verified(Ok(report)) => verification(report),
        Ran::Rebuilt(Ok(report)) => rebuilt(report),
        _ => Html::empty(),
    };
    scope_form(&scope, refusal).then(report)
}

/// Server › Integrity.
pub async fn page() -> Drawn {
    let tabbed = read_integrity().await;
    Drawn::page(
        tr(Msg::NavServer),
        server_page(ServerTab::Integrity, tabbed, integrity),
    )
}

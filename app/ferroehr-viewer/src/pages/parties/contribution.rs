// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One demographic contribution: its audit, the versions it made and its
//! record. No openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::surface::{Header, definitions, page_heading};

use super::{party_crumbs, uid_param};
use crate::bff::demographics::contribution::{ContributionState, fetch_demographic_contribution};
use crate::bff::demographics::{PartyKind, party_href, relationship_href};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::{
    Refused, card_panel, document_section, is_status, section, shortened, when,
};
use crate::routes::Drawn;
use crate::uid::version_number;

/// What the page reads, on the server.
async fn read_party_contribution(uid: String) -> Result<ContributionState, Refused> {
    match fetch_demographic_contribution(uid).await {
        Ok(mut state) => {
            state.body =
                crate::bff::pretty_body(&state.body, crate::format::ReprFormat::CanonicalJson);
            Ok(state)
        }
        Err(error) => {
            if is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            Err(Refused::new(tr(RecordsMsg::ContributionRefused), error))
        }
    }
}

/// A demographic contribution's audit, the versions it made and its record.
fn party_contribution_view(state: &ContributionState) -> Html {
    let mut rows = vec![
        (tr(RecordsMsg::FactCommitted), when(&state.time_committed)),
        (tr(RecordsMsg::FactCommitter), t(&state.committer)),
        (tr(RecordsMsg::FactChange), t(&state.change_type)),
    ];
    if !state.description.is_empty() {
        rows.push((tr(RecordsMsg::FactDescription), t(&state.description)));
    }
    let columns = [
        Column::text("object", tr(RecordsMsg::ColumnObject)).role(Role::Title),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Sub),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
    ];
    let made = state
        .versions
        .iter()
        .map(|version| {
            let object = crate::uid::container_uid_of(&version.version_uid);
            let href = PartyKind::from_rm_type(&version.rm_type)
                .map(|kind| party_href(kind, &object))
                .or_else(|| {
                    (version.rm_type == "PARTY_RELATIONSHIP").then(|| relationship_href(&object))
                });
            let title = title_cell(None, href.as_deref(), t(shortened(&object)), None);
            h(format!(
                "<tr>{title}{}{}</tr>",
                cell(&columns[1], t(&version.rm_type)),
                cell(&columns[2], t(version_number(&version.version_uid)))
            ))
        })
        .collect();
    let caption = tr(RecordsMsg::ContributionVersions);
    let table = data_table(DataTable::new(&caption, &columns, Body::Rows(made)));
    card_panel(Html::join([
        section(
            Some(tr(RecordsMsg::ContributionAudit)),
            None,
            None,
            definitions(rows),
        ),
        section(
            Some(tr(RecordsMsg::ContributionVersions)),
            u64::try_from(state.versions.len()).ok(),
            None,
            table,
        ),
        document_section(
            tr(RecordsMsg::ContributionRecord),
            Language::Json,
            state.body.clone(),
            "contribution-record",
        ),
    ]))
}

/// A demographic contribution.
/// One demographic contribution.
pub async fn party_contribution_page() -> Drawn {
    let uid = uid_param();
    let title = tr_with(
        RecordsMsg::ContributionTitle,
        &[("id", shortened(&uid).into())],
    );
    let heading = page_heading(
        &current(),
        Header {
            title: title.clone().into(),
            subtitle: Some(tr(RecordsMsg::ContributionSubtitle).into()),
            crumbs: party_crumbs(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    let body = match read_party_contribution(uid).await {
        Ok(state) => party_contribution_view(&state),
        Err(refusal) => refusal.view(),
    };
    Drawn::page(
        title,
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

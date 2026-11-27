// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Care tab: the care relationships of its subject.
//!
//! They are read by the subject reference its `EHR_STATUS` names, live ones on today or the day asked,
//! ended ones on request (`GET care_relationship?subject_id=&subject_namespace=`).
//! The CDR decides per subject what the caller sees. No openEHR
//! specification governs the tab: our own design.

use ferrohealth_ui::html::Html;

use crate::bff::care::{CareRow, Span, fetch_of_subject};
use crate::locale::policy::PolicyMsg;
use crate::locale::{tr, tr_with};
use crate::markup::{h, t};
use crate::pages::care::{Named, asked_span, care_list, refusal_of, span_form};
use crate::pages::common::{Refused, info_note, panel, plain_section, section, shortened};
use crate::pages::ehr::{EhrModel, EhrTab, ehr_id_param, ehr_view, tab_href};
use crate::pages::policy::grants::party_href;
use crate::routes::Drawn;

/// The Care tab's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Care {
    /// The relationships the span reads.
    Listed(Span, Vec<CareRow>),
    /// The EHR's subject carries no reference a relationship can name.
    NoSubject,
    /// The CDR's refusal of the status or of the list.
    Refused(Refused),
}

/// What the tab reads.
async fn read(ehr_id: String) -> EhrModel<Care> {
    crate::pages::ehr::load_ehr(&ehr_id, async |facts| {
        let status = match facts.status {
            Ok(status) => status,
            Err(error) => return Care::Refused(refusal_of(error)),
        };
        if status.subject.is_empty() || status.subject_namespace.is_empty() {
            return Care::NoSubject;
        }
        let span = asked_span();
        match fetch_of_subject(status.subject, status.subject_namespace, span.clone()).await {
            Ok(rows) => Care::Listed(span, rows),
            Err(error) => Care::Refused(refusal_of(error)),
        }
    })
    .await
}

/// The tab's panel.
fn care_panel(ehr_id: &str, care: Care) -> Html {
    let (span, rows) = match care {
        Care::Listed(span, rows) => (span, rows),
        Care::NoSubject => return panel(plain_section(info_note(t(tr(PolicyMsg::CareNoSubject))))),
        Care::Refused(refusal) => return panel(plain_section(refusal.view())),
    };
    let manage = rows
        .first()
        .and_then(|row| party_href(&row.subject))
        .map(|href| {
            plain_section(info_note(h(format!(
                r#"{} <a href="{}/care">{}</a>"#,
                t(tr(PolicyMsg::CareManageLine)),
                t(href),
                t(tr(PolicyMsg::CareManage))
            ))))
        })
        .unwrap_or_default();
    panel(
        manage.then(section(
            Some(tr(PolicyMsg::CareCaption)),
            u64::try_from(rows.len()).ok(),
            None,
            ferrohealth_ui::view::surface::section_intro(tr(PolicyMsg::CareEhrIntro))
                .then(span_form(&tab_href(ehr_id, EhrTab::Care), &span))
                .then(care_list(&rows, Named::Carer)),
        )),
    )
}

/// An EHR's Care tab.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(
        PolicyMsg::CareEhrTitle,
        &[("id", shortened(&ehr_id).into())],
    );
    let model = read(ehr_id.clone()).await;
    let body = ehr_view(&ehr_id, EhrTab::Care, model, |care| {
        care_panel(&ehr_id, care)
    });
    Drawn::page(title, body)
}

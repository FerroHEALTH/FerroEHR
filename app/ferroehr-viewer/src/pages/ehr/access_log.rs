// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Access log tab and one entry of it.
//!
//! The tab lists the EHR's access-log entries newest first (`GET
//! {ext}/ehr/{ehr_id}/access_log`), filtered by a `GET` form whose filters the
//! CDR takes as they are; the log states no total, so it pages with Previous
//! and Next alone, its position kept in the browser's session
//! ([`crate::paging`]). The CDR decides who reads which view, and the viewer
//! draws what it answers ([`crate::pages::access_log`]). No openEHR spec
//! governs the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::model::OpenWindow;
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading};
use ferrohealth_ui::view::template::list::list_empty;

use crate::bff::access_log::{Entry, LogFilters, Served, ehr_access_log, ehr_access_log_entry};
use crate::error::ViewerError;
use crate::locale::access::AccessMsg;
use crate::locale::{current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::access_log::{
    any, chips, entry_sections, href_with, read_filters, refusal, table, toolbar,
};
use crate::pages::common::{card_panel, info_note, is_status, shortened};
use crate::pages::ehr::{EhrModel, EhrTab, crumbs, ehr_id_param, ehr_view, tab_href};
use crate::paging::Listing;
use crate::routes::Drawn;

/// The tab's model: the filters and the page of entries, or why it is not
/// there.
#[derive(Debug, Clone)]
struct Panel {
    /// The filters the address names.
    filters: LogFilters,
    /// The entries of the page, or the CDR's refusal.
    entries: Result<Served<Vec<Entry>>, ViewerError>,
}

/// The page of the entry `uid` of the EHR `ehr_id`'s log.
fn entry_href(ehr_id: &str, uid: &str) -> String {
    format!(
        "{}/{}",
        tab_href(ehr_id, EhrTab::AccessLog),
        urlencoding::encode(uid)
    )
}

/// What the tab reads.
async fn read_page(ehr_id: String) -> EhrModel<Panel> {
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let filters = read_filters();
        let window = OpenWindow::of(crate::paging::current(Listing::AccessLog), 0);
        let entries = ehr_access_log(&ehr_id, &filters, window.start, window.size).await;
        Panel { filters, entries }
    })
    .await
}

/// The note saying the page is the person's own view of their log, when the
/// CDR served the subject view.
fn own_view<T>(served: &Served<T>) -> Option<Html> {
    served
        .is_subject_view()
        .then(|| info_note(t(tr(AccessMsg::OwnView))))
}

/// The tab's panel.
fn panel(ehr_id: &str, panel: Panel) -> Html {
    let Panel { filters, entries } = panel;
    let base = tab_href(ehr_id, EhrTab::AccessLog);
    let (own, entries) = match entries {
        Ok(served) => (own_view(&served), served.value),
        Err(error) => {
            return h(format!(
                r#"<div class="fui-panel"><div class="fui-panel-body fui-stack">{}</div></div>"#,
                refusal(error)
            ));
        }
    };
    let held = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    let window = OpenWindow::of(crate::paging::current(Listing::AccessLog), held);
    let list = if entries.is_empty() && window.page == 1 {
        list_empty(
            tr(if any(&filters) {
                AccessMsg::NoMatch
            } else {
                AccessMsg::Empty
            }),
            None,
        )
    } else {
        table(&entries, |entry| entry_href(ehr_id, &entry.uid))
    };
    let pager = (window.page > 1 || window.has_next()).then(|| {
        let fields = crate::paging::fields(Listing::AccessLog);
        post_open_pager(
            &current(),
            PagerListing::new(tr(AccessMsg::Pages)),
            window,
            PostTo {
                id: "access-log-pager",
                action: crate::paging::PAGING_ROUTE,
                fields: &fields,
            },
        )
    });
    h(format!(
        r#"<div class="fui-panel"><div class="fui-panel-body fui-stack">{}<p class="fui-small fui-muted">{}</p>{}{}</div>{list}{}</div>"#,
        opt(own),
        t(tr(AccessMsg::Intro)),
        toolbar(&base, &filters),
        opt(chips(&base, &filters)),
        opt(pager)
    ))
}

/// An EHR's Access log tab.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(AccessMsg::Title, &[("id", shortened(&ehr_id).into())]);
    let body = {
        let model = read_page(ehr_id.clone()).await;
        ehr_view(&ehr_id, EhrTab::AccessLog, model, |found| {
            panel(&ehr_id, found)
        })
    };
    Drawn::page(title, body)
}

/// One entry of an EHR's access log.
pub async fn entry_page() -> Drawn {
    let (ehr_id, uid) = crate::routes::params().with(|params| {
        (
            params.get("ehr_id").unwrap_or_default(),
            params.get("uid").unwrap_or_default(),
        )
    });
    let back = href_with(&tab_href(&ehr_id, EhrTab::AccessLog), &read_filters(), "");
    let mut trail = crumbs(&ehr_id);
    trail.push(Crumb::new(tr(AccessMsg::Tab), back));
    let (title, body) = match ehr_access_log_entry(&ehr_id, &uid).await {
        Ok(served) => {
            let own = own_view(&served);
            let entry = served.value;
            let reading = ferrohealth_ui::view::time::reading(&current(), &entry.recorded);
            let title = tr_with(AccessMsg::EntryTitle, &[("when", reading.into())]);
            (title, opt(own).then(card_panel(entry_sections(&entry))))
        }
        Err(error) if is_status(&error, http::StatusCode::NOT_FOUND) => {
            crate::request::set_status(http::StatusCode::NOT_FOUND);
            (tr(AccessMsg::Tab), info_note(t(tr(AccessMsg::EntryGone))))
        }
        Err(error) => (tr(AccessMsg::Tab), refusal(error)),
    };
    let heading = page_heading(
        &current(),
        Header {
            title: title.clone().into(),
            subtitle: Some(tr(AccessMsg::EntrySubtitle).into()),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    Drawn::page(
        title,
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

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
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::model::OpenWindow;
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action, Item, Menu};
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, Stakes, confirm_dialog};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading};
use ferrohealth_ui::view::template::list::list_empty;

use crate::bff::access_log::{Coded, Entry, LogFilters, ehr_access_log, ehr_access_log_entry};
use crate::bff::access_review::erase;
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::access::AccessMsg;
use crate::locale::{current, tr, tr_with};
use crate::markup::{flag, h, opt, t};
use crate::pages::access_log::{
    any, chips, entry_sections, href_with, read_filters, refusal, table, toolbar,
};
use crate::pages::common::{Refused, card_panel, info_note, is_status, shortened, warn_note};
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
    entries: Result<Vec<Entry>, ViewerError>,
    /// Whether the admin group is mounted, which offers the export.
    admin: bool,
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
    crate::pages::ehr::load_ehr(&ehr_id, async |facts| {
        let filters = read_filters();
        let window = OpenWindow::of(crate::paging::current(Listing::AccessLog), 0);
        let entries = ehr_access_log(&ehr_id, &filters, window.start, window.size).await;
        Panel {
            filters,
            entries,
            admin: facts.admin,
        }
    })
    .await
}

/// The tab's panel.
fn panel(ehr_id: &str, panel: Panel) -> Html {
    let Panel {
        filters,
        entries,
        admin,
    } = panel;
    let base = tab_href(ehr_id, EhrTab::AccessLog);
    let entries = match entries {
        Ok(entries) => entries,
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
    let export = admin.then(|| {
        ferrohealth_ui::view::button::link_button(
            crate::download::access_log_href(ehr_id),
            ferrohealth_ui::view::button::Variant::Secondary,
            Some(Glyph::DOWNLOAD),
            tr(AccessMsg::ExportAction),
        )
    });
    h(format!(
        r#"<div class="fui-panel"><div class="fui-panel-body fui-stack"><div class="fui-cluster"><p class="fui-small fui-muted">{}</p>{}</div>{}{}</div>{list}{}</div>"#,
        t(tr(AccessMsg::Intro)),
        opt(export),
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

/// The id of Erase's confirmation dialog.
const ERASE_DIALOG: &str = "erase-entry";

/// What a post to an entry's page asks, done: Erase. `Some` holds what it
/// did not do.
async fn act(ehr_id: &str, uid: &str) -> Option<Html> {
    let posted = crate::posted::posted()?;
    if posted.intent() != "erase" {
        return None;
    }
    let ground = Coded {
        value: posted.text("words"),
        terminology: posted.text("terminology"),
        code: posted.text("code"),
    };
    if ground.terminology.is_empty() || ground.code.is_empty() {
        return Some(warn_note(t(tr(AccessMsg::EraseNeedsGrounds))));
    }
    match erase(uid, &[ground]).await {
        Ok(_) => {
            redirect_with(
                &tab_href(ehr_id, EhrTab::AccessLog),
                &Done::ok(tr(AccessMsg::ErasedToast)).line(shortened(uid)),
            );
            None
        }
        Err(error) => Some(Refused::new(tr(AccessMsg::EraseRefused), error).view()),
    }
}

/// Erase's confirmation: the grounds as a coded term, and the decision.
fn erase_dialog(action: String) -> Html {
    let field = |id: &str, name: &str, label: AccessMsg, required: bool| {
        Field::new(id, tr(label)).view(h(format!(
            r#"<input id="{id}" name="{name}" type="text" class="fui-input fui-mono" autocomplete="off" spellcheck="false"{}>"#,
            flag("required", required)
        )))
    };
    confirm_dialog(
        &current(),
        Confirmation {
            id: ERASE_DIALOG.into(),
            title: tr(AccessMsg::EraseTitle).into(),
            message: tr(AccessMsg::EraseBody).into(),
        },
        Stakes::Danger,
        Some(ConfirmForm {
            action,
            fields: Html::join([
                h(r#"<input type="hidden" name="intent" value="erase">"#.to_owned()),
                field(
                    "e-terminology",
                    "terminology",
                    AccessMsg::EraseTerminology,
                    true,
                ),
                field("e-code", "code", AccessMsg::EraseCode, true),
                field("e-words", "words", AccessMsg::EraseWords, false),
            ]),
        }),
        h(format!(
            r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
            t(tr(AccessMsg::EraseConfirm))
        )),
    )
}

/// One entry of an EHR's access log, which an administrator erases.
pub async fn entry_page() -> Drawn {
    let (ehr_id, uid) = crate::routes::params().with(|params| {
        (
            params.get("ehr_id").unwrap_or_default(),
            params.get("uid").unwrap_or_default(),
        )
    });
    let acted = act(&ehr_id, &uid).await;
    let admin = crate::admin::renders_admin_ops(&crate::admin::probe_admin_api().await);
    let back = href_with(&tab_href(&ehr_id, EhrTab::AccessLog), &read_filters(), "");
    let mut trail = crumbs(&ehr_id);
    trail.push(Crumb::new(tr(AccessMsg::Tab), back));
    let (title, body, found) = match ehr_access_log_entry(&ehr_id, &uid).await {
        Ok(entry) => {
            let reading = ferrohealth_ui::view::time::reading(&current(), &entry.recorded);
            let title = tr_with(AccessMsg::EntryTitle, &[("when", reading.into())]);
            (title, card_panel(entry_sections(&entry)), true)
        }
        Err(error) if is_status(&error, http::StatusCode::NOT_FOUND) => {
            crate::request::set_status(http::StatusCode::NOT_FOUND);
            (
                tr(AccessMsg::Tab),
                info_note(t(tr(AccessMsg::EntryGone))),
                false,
            )
        }
        Err(error) => (tr(AccessMsg::Tab), refusal(error), false),
    };
    let erasable = admin && found;
    let actions = if erasable {
        ActionGroup::more_only(
            Menu::new("entry-more", tr(AccessMsg::EntryMore), Vec::new()).destructive(vec![
                Item::new(Action::new(
                    Some(Glyph::TRASH),
                    tr(AccessMsg::EraseAction),
                    Act::Dialog(ERASE_DIALOG.to_owned()),
                )),
            ]),
        )
    } else {
        ActionGroup::default()
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
        actions,
    );
    let dialog = erasable.then(|| erase_dialog(crate::request::path()));
    Drawn::page(
        title,
        Html::join([
            heading,
            h(format!(
                r#"<div class="fui-stack">{}{body}</div>"#,
                opt(acted)
            )),
            opt(dialog),
        ]),
    )
}

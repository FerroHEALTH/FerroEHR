// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Audit log › Delivery: each sink's cursor of each log and the records the
//! sinks parked ([`crate::bff::queues`]).
//!
//! A parked record is retried or dismissed, and a sink the configuration no
//! longer names is forgotten, each from its row through a confirmation the
//! page draws above the tables, so every act works with no script. No
//! openEHR spec governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading};

use crate::bff::queues::{
    Delivery, ParkedRef, delivery, dismiss_parked, forget_sink, retry_parked,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, tr};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    Refused, ago, card_panel, info_note, is_status, plain_section, section, state_mark, warn_note,
};
use crate::routes::Drawn;

/// The route of the page.
pub const DELIVERY_ROUTE: &str = "/audit/delivery";

/// An act on the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    /// Offer a parked record again.
    Retry,
    /// Remove a parked record for good.
    Dismiss,
    /// Remove a sink the configuration no longer names.
    Forget,
}

impl Act {
    /// The act a form or an address names.
    fn of(word: &str) -> Option<Self> {
        match word {
            "retry" => Some(Self::Retry),
            "dismiss" => Some(Self::Dismiss),
            "forget" => Some(Self::Forget),
            _ => None,
        }
    }

    /// The act's word in a form or an address.
    const fn word(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::Dismiss => "dismiss",
            Self::Forget => "forget",
        }
    }
}

/// The page as shown: the queue, the act the address asks to confirm, and a
/// refused act.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    /// The queue, `None` when the CDR serves none, or its refusal.
    queue: Result<Option<Delivery>, ViewerError>,
    /// The act to confirm and the record or sink it names.
    confirm: Option<(Act, ParkedRef)>,
    /// The act the CDR refused.
    refused: Option<Refused>,
}

/// The record or sink a form or an address names.
fn named(get: &dyn Fn(&str) -> String) -> ParkedRef {
    ParkedRef {
        sink: get("sink"),
        chain: get("chain"),
        record_id: get("record_id"),
    }
}

/// What the page reads, on the server; `None` when the post redirected.
async fn read() -> Option<Shown> {
    let mut refused = None;
    if let Some(posted) = crate::posted::posted()
        && let Some(act) = Act::of(&posted.intent())
    {
        let target = named(&|key| posted.text(key));
        let done = match act {
            Act::Retry => retry_parked(&target).await,
            Act::Dismiss => dismiss_parked(&target).await,
            Act::Forget => forget_sink(&target.sink).await,
        };
        let (ok, no) = match act {
            Act::Retry => (
                ConsoleMsg::DeliveryRetried,
                ConsoleMsg::DeliveryRetryRefused,
            ),
            Act::Dismiss => (
                ConsoleMsg::DeliveryDismissed,
                ConsoleMsg::DeliveryDismissRefused,
            ),
            Act::Forget => (
                ConsoleMsg::DeliveryForgotten,
                ConsoleMsg::DeliveryForgetRefused,
            ),
        };
        match done {
            Ok(()) => {
                redirect_with(DELIVERY_ROUTE, &Done::ok(tr(ok)));
                return None;
            }
            Err(error) => refused = Some(Refused::new(tr(no), error)),
        }
    }
    let confirm = crate::request::query("confirm")
        .as_deref()
        .and_then(Act::of)
        .map(|act| {
            (
                act,
                named(&|key| crate::request::query(key).unwrap_or_default()),
            )
        });
    Some(Shown {
        queue: delivery().await,
        confirm,
        refused,
    })
}

/// The address that asks to confirm `act` on `target`.
fn confirm_href(act: Act, target: &ParkedRef) -> String {
    format!(
        "{DELIVERY_ROUTE}?confirm={}&sink={}&chain={}&record_id={}",
        act.word(),
        urlencoding::encode(&target.sink),
        urlencoding::encode(&target.chain),
        urlencoding::encode(&target.record_id)
    )
}

/// The confirmation of `act` on `target`, a form that posts it.
fn confirmation(act: Act, target: &ParkedRef) -> Html {
    let (title, message) = match act {
        Act::Retry => (
            ConsoleMsg::DeliveryRetryTitle,
            ConsoleMsg::DeliveryRetryMessage,
        ),
        Act::Dismiss => (
            ConsoleMsg::DeliveryDismissTitle,
            ConsoleMsg::DeliveryDismissMessage,
        ),
        Act::Forget => (
            ConsoleMsg::DeliveryForgetTitle,
            ConsoleMsg::DeliveryForgetMessage,
        ),
    };
    let what = if act == Act::Forget {
        target.sink.clone()
    } else {
        format!("{} · {} · {}", target.sink, target.chain, target.record_id)
    };
    let hidden = |name: &str, value: &str| {
        format!(
            r#"<input type="hidden" name="{name}" value="{}">"#,
            t(value)
        )
    };
    warn_note(h(format!(
        r#"<p><strong>{title}</strong> <code>{what}</code> {message}</p><form method="post" action="{DELIVERY_ROUTE}" class="fui-cluster"><input type="hidden" name="intent" value="{intent}">{sink}{chain}{record}<button type="submit" class="fui-button fui-button-danger">{confirm}</button><a class="fui-button fui-button-secondary" href="{DELIVERY_ROUTE}">{cancel}</a></form>"#,
        title = t(tr(title)),
        what = t(what),
        message = t(tr(message)),
        intent = act.word(),
        sink = hidden("sink", &target.sink),
        chain = hidden("chain", &target.chain),
        record = hidden("record_id", &target.record_id),
        confirm = t(tr(ConsoleMsg::DeliveryConfirm)),
        cancel = t(tr(Msg::Cancel)),
    )))
}

/// A row action: a link to the confirmation of `act` on `target`.
fn row_act(act: Act, target: &ParkedRef, words: ConsoleMsg) -> String {
    format!(
        r#"<a class="fui-button fui-button-ghost fui-button-small" href="{}">{}</a>"#,
        t(confirm_href(act, target)),
        t(tr(words))
    )
}

/// The cursors and the parked records.
fn queue(view: &Delivery) -> Html {
    let columns = [
        Column::text("sink", tr(ConsoleMsg::DeliverySink)).role(Role::Title),
        Column::text("log", tr(ConsoleMsg::DeliveryLog)).role(Role::Sub),
        Column::text("through", tr(ConsoleMsg::DeliveryThrough))
            .numeric()
            .role(Role::Meta),
        Column::text("behind", tr(ConsoleMsg::DeliveryBehind))
            .numeric()
            .role(Role::Meta),
        Column::text("configured", tr(ConsoleMsg::DeliveryConfigured)).role(Role::Meta),
    ];
    let rows: Html = view
        .cursors
        .iter()
        .map(|cursor| {
            let configured = if cursor.configured {
                state_mark(FlowState::Up, &tr(ConsoleMsg::DeliveryInConfiguration))
            } else {
                let target = ParkedRef {
                    sink: cursor.sink.clone(),
                    ..ParkedRef::default()
                };
                state_mark(FlowState::Off, &tr(ConsoleMsg::DeliveryNotConfigured)).then(h(format!(
                    r#" <span class="fui-table-actions">{}</span>"#,
                    row_act(Act::Forget, &target, ConsoleMsg::DeliveryForget)
                )))
            };
            h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}{}{}</tr>"#,
                t(&cursor.sink),
                cell(&columns[1], t(&cursor.chain)),
                cell(&columns[2], t(cursor.through_id.to_string())),
                cell(&columns[3], t(cursor.behind.to_string())),
                cell(&columns[4], configured),
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::DeliveryCursors);
    let cursors = section(
        Some(caption.clone()),
        u64::try_from(view.cursors.len()).ok(),
        None,
        if view.cursors.is_empty() {
            info_note(t(tr(ConsoleMsg::DeliveryNoSink)))
        } else {
            data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
        },
    );
    let columns = [
        Column::text("record", tr(ConsoleMsg::DeliveryRecord)).role(Role::Title),
        Column::text("reason", tr(ConsoleMsg::DeliveryReason)).role(Role::Sub),
        Column::text("parked", tr(ConsoleMsg::DeliveryParkedAt)).role(Role::Meta),
        Column::text("actions", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let rows: Html = view
        .parked
        .iter()
        .map(|parked| {
            let target = ParkedRef {
                sink: parked.sink.clone(),
                chain: parked.chain.clone(),
                record_id: parked.record_id.to_string(),
            };
            let retry = if parked.retry {
                t(tr(ConsoleMsg::DeliveryRetryAsked)).to_string()
            } else {
                row_act(Act::Retry, &target, ConsoleMsg::DeliveryRetry)
            };
            h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{} · {} · {}</code></th>{}{}<td data-role="menu"><span class="fui-table-actions">{retry}{dismiss}</span></td></tr>"#,
                t(&parked.sink),
                t(&parked.chain),
                parked.record_id,
                cell(&columns[1], h(format!("<code>{}</code>", t(&parked.reason)))),
                cell(&columns[2], ago(&parked.at)),
                dismiss = row_act(Act::Dismiss, &target, ConsoleMsg::DeliveryDismiss),
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::DeliveryParked);
    let parked = section(
        Some(caption.clone()),
        u64::try_from(view.parked.len()).ok(),
        None,
        if view.parked.is_empty() {
            info_note(t(tr(ConsoleMsg::DeliveryNoneParked)))
        } else {
            data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
        },
    );
    cursors.then(parked)
}

/// Audit log › Delivery.
pub async fn page() -> Drawn {
    let Some(Shown {
        queue: read_queue,
        confirm,
        refused,
    }) = read().await
    else {
        return Drawn::default();
    };
    let title = tr(ConsoleMsg::DeliveryTitle);
    let heading = page_heading(
        &current(),
        Header {
            title: title.clone().into(),
            subtitle: Some(tr(ConsoleMsg::DeliverySubtitle).into()),
            crumbs: vec![Crumb::new(tr(Msg::NavAudit), "/audit")],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    let body = match read_queue {
        Ok(Some(view)) => Html::join([
            opt(confirm.map(|(act, target)| plain_section(confirmation(act, &target)))),
            queue(&view),
        ]),
        Ok(None) => plain_section(info_note(t(tr(ConsoleMsg::DeliveryAbsent)))),
        Err(error) if is_status(&error, http::StatusCode::METHOD_NOT_ALLOWED) => {
            plain_section(info_note(t(tr(ConsoleMsg::DeliveryNoAdmin))))
        }
        Err(error) => plain_section(Refused::new(tr(ConsoleMsg::DeliveryRefused), error).view()),
    };
    let refused = refused.map(|refusal| refusal.view());
    Drawn::page(
        title,
        heading.then(h(format!(
            r#"<div class="fui-stack">{}{}</div>"#,
            opt(refused),
            card_panel(body)
        ))),
    )
}

#[cfg(test)]
mod tests {
    use super::{Act, confirm_href};
    use crate::bff::queues::ParkedRef;

    /// An act's confirmation is an address naming the record.
    #[test]
    fn a_confirmation_names_its_record() {
        let target = ParkedRef {
            sink: "syslog".to_owned(),
            chain: "access_log".to_owned(),
            record_id: "7".to_owned(),
        };
        assert_eq!(
            confirm_href(Act::Dismiss, &target),
            "/audit/delivery?confirm=dismiss&sink=syslog&chain=access_log&record_id=7"
        );
        assert_eq!(Act::of("forget"), Some(Act::Forget));
        assert_eq!(Act::of("delete"), None);
    }
}

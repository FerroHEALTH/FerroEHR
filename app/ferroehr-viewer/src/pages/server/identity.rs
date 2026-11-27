// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server › Identity feed: the patient identity feed's refused messages
//! (IHE PMIR, ITI-93), each retried once its cause is fixed or settled
//! without applying it ([`crate::bff::queues`]).
//!
//! An act opens its confirmation above the list from the message's row, so
//! it works with no script; settling asks for the decision and its reason.
//! A CDR that runs no identity feed (`404`) offers none of it. No openEHR
//! spec governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table};
use ferrohealth_ui::view::form::Field;

use super::{ServerTab, Tabbed, load_tab, server_page};
use crate::bff::queues::{Message, messages, retry_message, settle_message};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, tr};
use crate::markup::{h, opt, t};
use crate::pages::common::{Refused, ago, info_note, is_status, plain_section, section, warn_note};
use crate::routes::Drawn;

/// The route of the tab.
const ROUTE: &str = "/server/identity";

/// The page's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Model {
    /// The post redirected.
    Left,
    /// The messages, the act to confirm, and a refused act.
    Shown {
        /// The messages, `None` when the CDR runs no feed, or their refusal.
        listed: Result<Option<Vec<Message>>, ViewerError>,
        /// The act to confirm (`retry` or `settle`) and its message.
        confirm: Option<(String, String)>,
        /// The act the CDR refused.
        refused: Option<Refused>,
    },
}

/// What the page reads, on the server.
async fn read() -> Tabbed<Model> {
    load_tab(ServerTab::Identity, || async {
        let mut refused = None;
        if let Some(posted) = crate::posted::posted() {
            let message = posted.text("message");
            let done = match posted.intent().as_str() {
                "retry" => Some((
                    retry_message(&message).await,
                    ConsoleMsg::IdentityRetried,
                    ConsoleMsg::IdentityRetryRefused,
                )),
                "settle" => Some((
                    settle_message(&message, &posted.text("decision"), &posted.text("reason"))
                        .await,
                    ConsoleMsg::IdentitySettled,
                    ConsoleMsg::IdentitySettleRefused,
                )),
                _ => None,
            };
            if let Some((done, ok, no)) = done {
                match done {
                    Ok(()) => {
                        redirect_with(ROUTE, &Done::ok(tr(ok)));
                        return Model::Left;
                    }
                    Err(error) => refused = Some(Refused::new(tr(no), error)),
                }
            }
        }
        let confirm = crate::request::query("confirm")
            .filter(|act| act == "retry" || act == "settle")
            .zip(crate::request::query("message"));
        Model::Shown {
            listed: messages().await,
            confirm,
            refused,
        }
    })
    .await
}

/// The confirmation of `act` on the message `message`: Retry's plain form,
/// or Settle's decision and reason.
fn confirmation(act: &str, message: &str) -> Html {
    let settle = act == "settle";
    let fields = if settle {
        let options: Html = [
            ("discard", ConsoleMsg::IdentityDiscard),
            ("lift-splits", ConsoleMsg::IdentityLiftSplits),
        ]
        .into_iter()
        .map(|(value, words)| {
            h(format!(
                r#"<option value="{value}">{}</option>"#,
                t(tr(words))
            ))
        })
        .collect();
        Field::new("i-decision", tr(ConsoleMsg::IdentityDecision))
            .holds_id()
            .view(h(format!(
                r#"<select id="i-decision" name="decision" class="fui-select">{options}</select>"#
            )))
            .then(Field::new("i-reason", tr(ConsoleMsg::IdentityReason)).holds_id().view(h(
                r#"<textarea id="i-reason" name="reason" class="fui-textarea" rows="2" maxlength="1000" required></textarea>"#
                    .to_owned(),
            )))
    } else {
        Html::empty()
    };
    let (title, line) = if settle {
        (
            ConsoleMsg::IdentitySettleTitle,
            ConsoleMsg::IdentitySettleMessage,
        )
    } else {
        (
            ConsoleMsg::IdentityRetryTitle,
            ConsoleMsg::IdentityRetryMessage,
        )
    };
    warn_note(h(format!(
        r#"<p><strong>{title}</strong> <code>{message}</code> {line}</p><form method="post" action="{ROUTE}" class="fui-stack"><input type="hidden" name="intent" value="{act}"><input type="hidden" name="message" value="{message}">{fields}<div class="fui-cluster"><button type="submit" class="fui-button fui-button-primary">{confirm}</button><a class="fui-button fui-button-secondary" href="{ROUTE}">{cancel}</a></div></form>"#,
        title = t(tr(title)),
        message = t(message),
        line = t(tr(line)),
        act = if settle { "settle" } else { "retry" },
        confirm = t(tr(if settle {
            ConsoleMsg::IdentitySettle
        } else {
            ConsoleMsg::IdentityRetry
        })),
        cancel = t(tr(Msg::Cancel)),
    )))
}

/// The refused messages, each with Retry and Settle.
fn listed(messages: &[Message]) -> Html {
    if messages.is_empty() {
        return info_note(t(tr(ConsoleMsg::IdentityNoneRefused)));
    }
    let columns = [
        Column::text("message", tr(ConsoleMsg::IdentityMessage)).role(Role::Title),
        Column::text("reason", tr(ConsoleMsg::IdentityRefusal)).role(Role::Sub),
        Column::text("refused", tr(ConsoleMsg::IdentityRefusedAt)).role(Role::Meta),
        Column::text("attempts", tr(ConsoleMsg::IdentityAttempts))
            .numeric()
            .role(Role::Meta),
        Column::text("splits", tr(ConsoleMsg::IdentitySplits))
            .numeric()
            .role(Role::Meta),
        Column::text("actions", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let link = |act: &str, id: &str, words: ConsoleMsg| {
        format!(
            r#"<a class="fui-button fui-button-ghost fui-button-small" href="{ROUTE}?confirm={act}&amp;message={}">{}</a>"#,
            t(urlencoding::encode(id)),
            t(tr(words))
        )
    };
    let rows: Html = messages
        .iter()
        .map(|message| {
            let splits: u64 = message
                .splits
                .iter()
                .map(|split| split.identifiers.saturating_add(split.subjects))
                .sum();
            h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}{}{}<td data-role="menu"><span class="fui-table-actions">{}{}</span></td></tr>"#,
                t(&message.id),
                cell(&columns[1], h(format!("<code>{}</code>", t(&message.reason)))),
                cell(&columns[2], ago(&message.refused_at)),
                cell(&columns[3], t(message.attempts.to_string())),
                cell(&columns[4], t(splits.to_string())),
                link("retry", &message.id, ConsoleMsg::IdentityRetry),
                link("settle", &message.id, ConsoleMsg::IdentitySettle),
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::IdentityRefusedMessages);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// Identity feed, drawn.
fn identity(model: Model) -> Html {
    let Model::Shown {
        listed: read,
        confirm,
        refused,
    } = model
    else {
        return Html::empty();
    };
    let refused = refused.map(|refusal| plain_section(refusal.view()));
    let body = match read {
        Ok(Some(messages)) => section(
            Some(tr(ConsoleMsg::IdentityRefusedMessages)),
            u64::try_from(messages.len()).ok(),
            None,
            opt(confirm.map(|(act, message)| confirmation(&act, &message))).then(listed(&messages)),
        ),
        Ok(None) => plain_section(info_note(t(tr(ConsoleMsg::IdentityAbsent)))),
        Err(error) if is_status(&error, http::StatusCode::METHOD_NOT_ALLOWED) => {
            plain_section(info_note(t(tr(ConsoleMsg::ServerNoAdmin))))
        }
        Err(error) => {
            plain_section(Refused::new(tr(ConsoleMsg::IdentityListRefused), error).view())
        }
    };
    opt(refused).then(body)
}

/// Server › Identity feed.
pub async fn page() -> Drawn {
    let tabbed = read().await;
    Drawn::page(
        tr(Msg::NavServer),
        server_page(ServerTab::Identity, tabbed, identity),
    )
}

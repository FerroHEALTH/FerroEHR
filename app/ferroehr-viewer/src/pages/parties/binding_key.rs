// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `PERSON`'s binding keys on its Overview, and an administrator's re-key
//! of one ([`crate::admin::rekey`]).
//!
//! A binding key is a `DV_IDENTIFIER` whose `issuer` a login issuer names;
//! the section lists every identifier the `PERSON` carries with an issuer.
//! Re-key opens a confirmation from the key's row, asking for the new id,
//! and the CDR decides it. No openEHR spec governs the binding: the CDR's
//! own extension.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's demographic JSON wire for a party's identifiers"
)]

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table};
use ferrohealth_ui::view::form::Field;

use crate::bff::demographics::{PartyKind, party_href};
use crate::flash::{Done, redirect_with};
use crate::locale::operations::OperationsMsg;
use crate::locale::{Msg, tr};
use crate::markup::{h, opt, t};
use crate::pages::common::{Refused, section, warn_note};

/// One identifier a party carries with an issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// Its `DV_IDENTIFIER.issuer`.
    pub issuer: String,
    /// Its `DV_IDENTIFIER.id`.
    pub id: String,
}

/// Every `DV_IDENTIFIER` with an issuer in the party document `body`, in
/// document order.
#[must_use]
pub fn keys(body: &str) -> Vec<Key> {
    fn walk(node: &serde_json::Value, found: &mut Vec<Key>) {
        match node {
            serde_json::Value::Object(fields) => {
                let identifier = fields.get("_type").and_then(serde_json::Value::as_str)
                    == Some("DV_IDENTIFIER");
                let text = |key: &str| {
                    fields
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                if identifier && !text("issuer").is_empty() {
                    found.push(Key {
                        issuer: text("issuer"),
                        id: text("id"),
                    });
                }
                for value in fields.values() {
                    walk(value, found);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    // NOTE: no openEHR spec governs this: our own design; a document that is no
    // JSON carries no identifier the section can list.
    if let Ok(document) = serde_json::from_str::<serde_json::Value>(body) {
        walk(&document, &mut found);
    }
    found
}

/// The confirmation of a re-key under `issuer`, asking for the new id.
fn confirmation(action: &str, issuer: &str) -> Html {
    warn_note(h(format!(
        r#"<p><strong>{title}</strong> <code>{issuer}</code>. {message}</p><form method="post" action="{action}" class="fui-stack"><input type="hidden" name="intent" value="rekey"><input type="hidden" name="issuer" value="{issuer}">{field}<div class="fui-cluster"><button type="submit" class="fui-button fui-button-danger">{confirm}</button><a class="fui-button fui-button-secondary" href="{action}">{cancel}</a></div></form>"#,
        title = t(tr(OperationsMsg::RekeyTitle)),
        issuer = t(issuer),
        message = t(tr(OperationsMsg::RekeyMessage)),
        action = t(action),
        field = Field::new("k-id", tr(OperationsMsg::RekeyNewId))
            .holds_id()
            .view(h(
                r#"<input id="k-id" name="id" type="text" class="fui-input fui-mono" autocomplete="off" required>"#
                    .to_owned()
            )),
        confirm = t(tr(OperationsMsg::Rekey)),
        cancel = t(tr(Msg::Cancel)),
    )))
}

/// The Binding keys section of the `PERSON` `uid` whose document is `body`.
///
/// It lists each key, with Re-key while the admin group is mounted, and the
/// re-key the address asks to confirm; nothing for another kind of party.
#[must_use]
pub fn keys_section(kind: PartyKind, uid: &str, body: &str, admin: bool) -> Option<Html> {
    if kind != PartyKind::Person {
        return None;
    }
    let action = party_href(kind, uid);
    let listed = keys(body);
    let asked = crate::request::query("rekey")
        .filter(|issuer| admin && listed.iter().any(|key| key.issuer == *issuer));
    let columns = [
        Column::text("issuer", tr(OperationsMsg::KeyIssuer)).role(Role::Title),
        Column::text("id", tr(OperationsMsg::KeyId)).role(Role::Sub),
        Column::text("actions", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let rows: Html = listed
        .iter()
        .map(|key| {
            let rekey = if admin {
                format!(
                    r#"<a class="fui-button fui-button-ghost fui-button-small" href="{}?rekey={}">{}</a>"#,
                    t(&action),
                    t(urlencoding::encode(&key.issuer)),
                    t(tr(OperationsMsg::RekeyAsk))
                )
            } else {
                String::new()
            };
            h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}<td data-role="menu"><span class="fui-table-actions">{rekey}</span></td></tr>"#,
                t(&key.issuer),
                cell(&columns[1], h(format!("<code>{}</code>", t(&key.id)))),
            ))
        })
        .collect();
    let caption = tr(OperationsMsg::BindingKeys);
    let body = if listed.is_empty() {
        crate::pages::common::info_note(t(tr(OperationsMsg::NoBindingKey)))
    } else {
        data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
    };
    Some(section(
        Some(caption),
        u64::try_from(listed.len()).ok(),
        None,
        opt(asked.map(|issuer| confirmation(&action, &issuer))).then(body),
    ))
}

/// Answers a re-key posted to the `PERSON` `uid`.
///
/// `None` when the post is no re-key; a success redirects back with a toast,
/// and a refusal is returned in the CDR's words.
pub async fn act(uid: &str, posted: &crate::posted::Posted) -> Option<Option<Refused>> {
    if posted.intent() != "rekey" {
        return None;
    }
    Some(
        match crate::admin::rekey(uid, &posted.text("issuer"), &posted.text("id")).await {
            Ok(()) => {
                redirect_with(
                    &party_href(PartyKind::Person, uid),
                    &Done::ok(tr(OperationsMsg::Rekeyed)),
                );
                None
            }
            Err(error) => Some(Refused::new(tr(OperationsMsg::RekeyRefused), error)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{Key, keys};

    /// Every identifier with an issuer is listed, nested or not; one with
    /// none is not.
    #[test]
    fn the_identifiers_with_an_issuer_are_the_keys() {
        let body = r#"{"_type":"PERSON","details":{"items":[{"value":{"_type":"DV_IDENTIFIER","issuer":"urn:example:staff-login","id":"5b0f","type":"login"}},{"value":{"_type":"DV_IDENTIFIER","id":"no-issuer"}}]}}"#;
        assert_eq!(
            keys(body),
            [Key {
                issuer: "urn:example:staff-login".to_owned(),
                id: "5b0f".to_owned()
            }]
        );
        assert_eq!(keys("not json"), []);
    }
}

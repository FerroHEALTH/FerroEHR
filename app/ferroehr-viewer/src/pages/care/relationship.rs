// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One care relationship: its facts, its versions and its writes.
//!
//! Its versions carry the audit details of each. A change of its role or period and its end (the
//! period closed today) commit the next version with `If-Match` naming the
//! version read; a delete removes one written in error. An older version
//! opens read-only by its version uid. No openEHR specification governs the
//! page: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::dialog::{Confirmation, Stakes, confirm_dialog};
use ferrohealth_ui::view::surface::{Crumb, Header, definitions, page_heading, section_intro};

use crate::bff::care::{CareRow, changed_document, delete, fetch_one, fetch_versions, update};
use crate::bff::grants::Validity;
use crate::bff::wire::VersionEntry;
use crate::flash::{Done, redirect_with};
use crate::locale::policy::PolicyMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::care::{
    care_href, path_words, period_fields, period_words, posted_period, posted_role, refusal_of,
    role_coding, role_select,
};
use crate::pages::common::{
    Refused, Unsaved, card_panel, document_section, info_note, plain_section, section, short,
};
use crate::pages::policy::grants::party_href;
use crate::pages::versions::History;
use crate::routes::Drawn;
use crate::uid::version_number;

/// The id of the relationship's change form.
const CHANGE_FORM: &str = "care-change";

/// The id of End's confirmation.
const END_DIALOG: &str = "care-end";

/// The id of Delete's confirmation.
const DELETE_DIALOG: &str = "care-delete";

/// The relationship's page model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Model {
    /// The relationship, the version read, its versions and the write the
    /// CDR refused.
    Shown {
        /// The version read.
        row: Box<CareRow>,
        /// Whether it is the current version.
        current: bool,
        /// Its versions, or the CDR's refusal of them.
        versions: Result<Vec<VersionEntry>, Refused>,
        /// The write the CDR refused.
        unsaved: Option<Unsaved>,
    },
    /// The relationship is deleted.
    Deleted,
    /// The CDR's refusal.
    Refused(Refused),
}

/// Today in UTC, as a `DV_DATE` bound.
fn today() -> String {
    crate::request::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}

/// What the post asks, done: the change, the end or the delete. `Some`
/// holds the CDR's refusal.
async fn act(id: &str, row: &CareRow) -> Option<Unsaved> {
    let posted = crate::posted::posted()?;
    let replaces = Some(posted.text("version"))
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| row.version_uid.clone());
    let (written, done, refused) = match posted.intent().as_str() {
        "change" => {
            let document = changed_document(
                &row.body,
                &posted_role(&posted.text("role")),
                &posted_period(&posted),
            );
            let written = match document {
                Ok(document) => update(id.to_owned(), replaces, document).await,
                Err(error) => Err(error),
            };
            (
                written,
                PolicyMsg::CareChanged,
                PolicyMsg::CareChangeRefused,
            )
        }
        "end" => {
            let ended = Validity {
                lower: row.period.lower.clone(),
                upper: Some(today()),
            };
            let written = match changed_document(&row.body, &row.role, &ended) {
                Ok(document) => update(id.to_owned(), replaces, document).await,
                Err(error) => Err(error),
            };
            (written, PolicyMsg::CareEnded, PolicyMsg::CareEndRefused)
        }
        "delete" => (
            delete(id.to_owned(), replaces).await,
            PolicyMsg::CareDeleted,
            PolicyMsg::CareDeleteRefused,
        ),
        _ => return None,
    };
    match written {
        Ok(()) => {
            redirect_with(&care_href(id), &Done::ok(tr(done)));
            None
        }
        Err(error) => Some(Unsaved::new(tr(refused), error)),
    }
}

/// What the page reads.
async fn read(uid: String) -> Model {
    let row = match fetch_one(uid.clone()).await {
        Ok(Some(row)) => row,
        Ok(None) => return Model::Deleted,
        Err(error) => {
            if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            return Model::Refused(refusal_of(error));
        }
    };
    let versions = fetch_versions(row.id.clone())
        .await
        .map_err(|error| Refused::new(tr(PolicyMsg::CareVersionsRefused), error));
    let latest = versions
        .as_ref()
        .ok()
        .and_then(|versions| versions.first())
        .map(|entry| entry.version_id.clone());
    let current = latest
        .as_deref()
        .is_none_or(|latest| latest == row.version_uid);
    let unsaved = if current {
        act(&row.id, &row).await
    } else {
        None
    };
    Model::Shown {
        row: Box::new(row),
        current,
        versions,
        unsaved,
    }
}

/// A confirmation whose confirm submits the change form with `intent`.
fn confirm(id: &str, title: PolicyMsg, message: PolicyMsg, intent: &str, stakes: Stakes) -> Html {
    let class = match stakes {
        Stakes::Danger => "fui-button fui-button-destructive",
        Stakes::Plain => "fui-button fui-button-primary",
    };
    confirm_dialog(
        &current(),
        Confirmation {
            id: id.to_owned().into(),
            title: tr(title).into(),
            message: tr(message).into(),
        },
        stakes,
        None,
        h(format!(
            r#"<button type="submit" form="{CHANGE_FORM}" name="intent" value="{intent}" formnovalidate class="{class}">{}</button>"#,
            t(tr(title))
        )),
    )
}

/// The relationship's facts.
fn facts(row: &CareRow) -> Html {
    let link = |party: &crate::bff::grants::PartyRef| {
        let words = h(format!("{} {}", t(&party.rm_type), short(&party.id)));
        party_href(party).map_or(words.clone(), |href| {
            h(format!(r#"<a href="{}">{words}</a>"#, t(href)))
        })
    };
    definitions(vec![
        (tr(PolicyMsg::CareSubject), link(&row.subject)),
        (tr(PolicyMsg::CareCarer), link(&row.carer)),
        (tr(PolicyMsg::CareRole), role_coding(&row.role)),
        (
            tr(PolicyMsg::CareProvider),
            h(format!(
                "<code>{}</code> {}",
                t(&row.provider.0),
                t(&row.provider.1)
            )),
        ),
        (tr(PolicyMsg::CarePeriod), t(period_words(&row.period))),
        (tr(PolicyMsg::CarePath), t(path_words(row.path()))),
        (
            tr(PolicyMsg::FactVersion),
            t(version_number(&row.version_uid)),
        ),
    ])
}

/// The change form: the role and the period, saved over the version read,
/// with End and Delete behind their confirmations.
fn change_form(row: &CareRow) -> Html {
    let buttons = format!(
        r#"<div class="fui-cluster"><button type="submit" name="intent" value="change" class="fui-button fui-button-primary">{save}</button><button type="button" class="fui-button fui-button-secondary" command="show-modal" commandfor="{END_DIALOG}" aria-haspopup="dialog">{end}</button><button type="button" class="fui-button fui-button-destructive" command="show-modal" commandfor="{DELETE_DIALOG}" aria-haspopup="dialog">{delete}</button></div>"#,
        save = t(tr(PolicyMsg::CareSave)),
        end = t(tr(PolicyMsg::CareEnd)),
        delete = t(tr(PolicyMsg::CareDelete)),
    );
    h(format!(
        r#"<form id="{CHANGE_FORM}" method="post" action="{action}" class="fui-form"><input type="hidden" name="version" value="{version}">{role}{period}{buttons}</form>"#,
        action = t(care_href(&row.id)),
        version = t(&row.version_uid),
        role = role_select("care-role", &row.role),
        period = period_fields("care", &row.period),
    ))
    .then(confirm(
        END_DIALOG,
        PolicyMsg::CareEnd,
        PolicyMsg::CareEndMessage,
        "end",
        Stakes::Plain,
    ))
    .then(confirm(
        DELETE_DIALOG,
        PolicyMsg::CareDelete,
        PolicyMsg::CareDeleteMessage,
        "delete",
        Stakes::Danger,
    ))
}

/// The versions, newest first, each opening by its version uid.
fn versions_view(id: &str, versions: &[VersionEntry], read: &str) -> Html {
    let reading = version_number(read);
    crate::pages::versions::history(
        History {
            versions,
            reading: Some(reading.as_str()),
            deleted: false,
            contribution: t(tr(PolicyMsg::OwnContribution)),
            caption: tr(PolicyMsg::VersionsCaption),
            at_action: care_href(id),
            at_opens: tr(PolicyMsg::AtTimeOpens),
        },
        |number| match number {
            None => care_href(id),
            Some(number) => versions
                .iter()
                .find(|entry| version_number(&entry.version_id) == number)
                .map_or_else(|| care_href(id), |entry| care_href(&entry.version_id)),
        },
    )
}

/// The page's body.
fn body(model: Model) -> Html {
    let (row, current, versions, unsaved) = match model {
        Model::Shown {
            row,
            current,
            versions,
            unsaved,
        } => (*row, current, versions, unsaved),
        Model::Deleted => return card_panel(plain_section(info_note(t(tr(PolicyMsg::CareGone))))),
        Model::Refused(refusal) => return card_panel(plain_section(refusal.view())),
    };
    let top = unsaved.map(|unsaved| {
        let words = tr(PolicyMsg::ReloadNone);
        plain_section(unsaved.view(&care_href(&row.id), words))
    });
    let older = (!current).then(|| plain_section(info_note(t(tr(PolicyMsg::CareOlder)))));
    let write = current.then(|| {
        section(
            Some(tr(PolicyMsg::CareChange)),
            None,
            None,
            section_intro(tr(PolicyMsg::CareChangeIntro)).then(change_form(&row)),
        )
    });
    let history = match &versions {
        Ok(versions) => versions_view(&row.id, versions, &row.version_uid),
        Err(refusal) => refusal.view(),
    };
    card_panel(Html::join([
        opt(top),
        opt(older),
        section(Some(tr(PolicyMsg::CareThis)), None, None, facts(&row)),
        opt(write),
        section(
            Some(tr(PolicyMsg::VersionsCaption)),
            versions
                .as_ref()
                .ok()
                .and_then(|versions| u64::try_from(versions.len()).ok()),
            None,
            section_intro(tr(PolicyMsg::CareVersionsIntro)).then(history),
        ),
        document_section(
            tr_with(
                PolicyMsg::CareDocument,
                &[("version", version_number(&row.version_uid).into())],
            ),
            Language::Json,
            row.body.clone(),
            "care-document",
        ),
    ]))
}

/// A care relationship's page.
pub async fn page() -> Drawn {
    let uid = crate::routes::params().get("uid").unwrap_or_default();
    let model = read(uid.clone()).await;
    let heading = page_heading(
        &current(),
        Header {
            title: tr(PolicyMsg::CareTitle).into(),
            subtitle: Some(crate::pages::common::shortened(&uid).into()),
            crumbs: vec![Crumb::new(tr(Msg::NavParties), "/parties")],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    Drawn::page(
        tr(PolicyMsg::CareTitle),
        heading.then(h(format!(
            r#"<div class="fui-stack">{}</div>"#,
            body(model)
        ))),
    )
}

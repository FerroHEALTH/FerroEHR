// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access policy: the controller's `ACCESS_POLICY` in force, by section, its
//! versions, and its editor.
//!
//! The policy is read whole from the CDR's admin extension
//! ([`crate::bff::access_policy`]); the CDR decides who may read and write
//! it, so a `403` says the account may not and a `405` says the admin API is
//! off. While no version is committed the CDR tightens nothing, and the page
//! says so. An older version opens read-only from the Versions tab. No
//! openEHR specification governs the viewer's pages: our own design.

pub mod edit;

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::versions::older_version;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action};
use ferrohealth_ui::view::coding::{Coding, Layout, coding};
use ferrohealth_ui::view::empty::empty_state;
use ferrohealth_ui::view::layout::bullet_list;
use ferrohealth_ui::view::surface::{Header, definitions, page_heading, section_intro};
use ferrohealth_ui::view::tabs::{Tab, binder};

use crate::bff::access_policy::{
    PolicyVersion, PolicyView, Shown, fetch_history, fetch_policy, read_policy,
};
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    Refused, document_section, is_status, panel, plain_section, section, when,
};
use crate::pages::configuration::label_of;
use crate::pages::versions::History;
use crate::routes::Drawn;
use crate::uid::version_number;

/// The route of the policy in force.
pub const POLICY_ROUTE: &str = "/access-policy";

/// The route of the policy's versions.
const VERSIONS_ROUTE: &str = "/access-policy/versions";

/// The route of the policy's editor.
pub const EDIT_ROUTE: &str = "/access-policy/edit";

/// The tabs of Access policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyTab {
    /// The policy, by section.
    Policy,
    /// Its versions.
    Versions,
}

impl PolicyTab {
    /// Every tab, in order.
    const ALL: [Self; 2] = [Self::Policy, Self::Versions];

    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Versions => "versions",
        }
    }

    /// The tab's page.
    const fn href(self) -> &'static str {
        match self {
            Self::Policy => POLICY_ROUTE,
            Self::Versions => VERSIONS_ROUTE,
        }
    }

    /// The tab's label.
    fn label(self) -> String {
        tr(match self {
            Self::Policy => PolicyMsg::TabPolicy,
            Self::Versions => PolicyMsg::TabVersions,
        })
    }
}

/// Access policy's frame: its heading with `actions`, its tabs with `tab`
/// marked, and `body` in the tab's panel.
fn frame(tab: PolicyTab, actions: ActionGroup, body: Html) -> Html {
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavAccessPolicy).into(),
            subtitle: Some(tr(PolicyMsg::PolicySubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        actions,
    );
    let tabs = PolicyTab::ALL
        .into_iter()
        .map(|each| Tab::new(each.key(), each.href(), each.label()))
        .collect();
    let binder = binder(
        &current(),
        tr(PolicyMsg::PolicySections),
        tabs,
        tab.key(),
        body,
    );
    heading.then(h(format!(r#"<div class="fui-stack">{binder}</div>"#)))
}

/// Why the CDR shows no policy: none is committed, the account may not read
/// it, or the admin API is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// No version is committed: the CDR tightens nothing.
    NoneCommitted,
    /// The CDR's refusal.
    Refused(Refused),
}

/// The refusal of a read of the policy, worded for a `403` and a `405`.
#[must_use]
pub fn refusal_of(error: ViewerError) -> Refused {
    let summary = if matches!(error, ViewerError::Forbidden(_)) {
        PolicyMsg::PolicyForbidden
    } else if is_status(&error, http::StatusCode::METHOD_NOT_ALLOWED) {
        PolicyMsg::AdminOff
    } else {
        PolicyMsg::PolicyRefused
    };
    Refused::new(tr(summary), error)
}

/// The policy tab's model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reading {
    /// The version shown.
    version_uid: String,
    /// Its document, pretty-printed.
    body: String,
    /// The number of the version asked for, when it is not the newest.
    older: Option<String>,
    /// How many versions there are.
    count: u32,
}

/// What the policy tab reads: the policy in force, or the version asked for
/// read from the history.
async fn read_policy_tab() -> Result<Reading, Unread> {
    let asked = crate::request::query("version");
    let current = match fetch_policy().await {
        Ok(Some(current)) => current,
        Ok(None) => return Err(Unread::NoneCommitted),
        Err(error) => return Err(Unread::Refused(refusal_of(error))),
    };
    let latest = version_number(&current.version_uid);
    match asked.filter(|number| *number != latest) {
        None => Ok(Reading {
            count: latest.parse().unwrap_or(1),
            version_uid: current.version_uid,
            body: current.body,
            older: None,
        }),
        Some(number) => {
            let versions = fetch_history()
                .await
                .map_err(|error| Unread::Refused(refusal_of(error)))?;
            let count = u32::try_from(versions.len()).unwrap_or(u32::MAX);
            let found = versions
                .into_iter()
                .find(|version| version_number(&version.entry.version_id) == number);
            let Some(PolicyVersion { entry, body }) = found else {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
                return Err(Unread::Refused(Refused::new(
                    tr(PolicyMsg::VersionMissing),
                    ViewerError::Invalid(tr_with(
                        PolicyMsg::VersionMissingLine,
                        &[("version", number.into())],
                    )),
                )));
            };
            Ok(Reading {
                version_uid: entry.version_id,
                body,
                older: Some(number),
                count,
            })
        }
    }
}

/// A shown value of the policy, drawn.
fn shown(value: &Shown) -> Html {
    match value {
        Shown::Text(text) if text.is_empty() => none(),
        Shown::Text(text) => t(text),
        Shown::Coded {
            rubric,
            terminology,
            code,
        } => coding(
            &current(),
            Coding::new(code.clone(), rubric.clone()).system(terminology.clone()),
            Layout::Inline,
        ),
        Shown::Flag(yes) => t(tr(if *yes { Msg::Yes } else { Msg::No })),
        Shown::List(items) if items.is_empty() => none(),
        Shown::List(items) => bullet_list(items.iter().map(shown).collect()),
        Shown::Fields { name, fields, .. } => {
            let rows = definitions(
                fields
                    .iter()
                    .map(|(key, value)| (label_of(key), shown(value)))
                    .collect(),
            );
            match name {
                Some(name) => h(format!("<strong>{}</strong>{rows}", t(name))),
                None => rows,
            }
        }
    }
}

/// The words of a value the policy leaves empty.
fn none() -> Html {
    h(format!(
        r#"<span class="fui-muted">{}</span>"#,
        t(tr(Msg::RelationshipNone))
    ))
}

/// The sections of the policy, each its title, its intro and the attributes
/// it draws, in the order the page draws them.
const SECTIONS: [(PolicyMsg, PolicyMsg, &[&str]); 10] = [
    (
        PolicyMsg::SectionClassifications,
        PolicyMsg::SectionClassificationsIntro,
        &["classifications"],
    ),
    (
        PolicyMsg::SectionRules,
        PolicyMsg::SectionRulesIntro,
        &["rules"],
    ),
    (
        PolicyMsg::SectionCatalogue,
        PolicyMsg::SectionCatalogueIntro,
        &["catalogue"],
    ),
    (
        PolicyMsg::SectionCare,
        PolicyMsg::SectionCareIntro,
        &[
            "care_relationship_required",
            "relationship_grace",
            "self_declared_care",
            "attested_care",
            "caseload",
            "episode_shared",
        ],
    ),
    (
        PolicyMsg::SectionPurpose,
        PolicyMsg::SectionPurposeIntro,
        &["purpose_required"],
    ),
    (
        PolicyMsg::SectionConflicts,
        PolicyMsg::SectionConflictsIntro,
        &["conflict_rules"],
    ),
    (
        PolicyMsg::SectionConcealed,
        PolicyMsg::SectionConcealedIntro,
        &["concealed_confidentialities"],
    ),
    (
        PolicyMsg::SectionLogRestrictions,
        PolicyMsg::SectionLogRestrictionsIntro,
        &["log_restrictions"],
    ),
    (
        PolicyMsg::SectionValueSets,
        PolicyMsg::SectionValueSetsIntro,
        &["value_sets"],
    ),
    (PolicyMsg::SectionOther, PolicyMsg::SectionOtherIntro, &[]),
];

/// Every attribute a named section draws.
fn sectioned() -> impl Iterator<Item = &'static str> {
    SECTIONS
        .iter()
        .flat_map(|(_, _, keys)| keys.iter().copied())
}

/// One section of the policy: a single list attribute drawn as its items,
/// several attributes as a definition list, each attribute the policy
/// leaves out read as its strictest default.
fn policy_section(view: &PolicyView, title: PolicyMsg, intro: PolicyMsg, keys: &[&str]) -> Html {
    let other: Vec<(String, &Shown)>;
    let body = if keys.is_empty() {
        other = view
            .attributes
            .iter()
            .filter(|(key, _)| key != "controller" && !sectioned().any(|known| known == key))
            .map(|(key, value)| (key.clone(), value))
            .collect();
        if other.is_empty() {
            return Html::empty();
        }
        definitions(
            other
                .iter()
                .map(|(key, value)| (label_of(key), shown(value)))
                .collect(),
        )
    } else if let [key] = keys {
        match view.get(key) {
            Some(Shown::List(items)) if !items.is_empty() => {
                bullet_list(items.iter().map(shown).collect())
            }
            Some(Shown::List(_)) | None => h(format!(
                r#"<p class="fui-small fui-muted">{}</p>"#,
                t(tr(PolicyMsg::LeftOut))
            )),
            Some(value) => definitions(vec![(label_of(key), shown(value))]),
        }
    } else {
        definitions(
            keys.iter()
                .map(|key| {
                    let value = view.get(key).map_or_else(
                        || {
                            h(format!(
                                r#"<span class="fui-muted">{}</span>"#,
                                t(tr(PolicyMsg::LeftOutShort))
                            ))
                        },
                        shown,
                    );
                    (label_of(key), value)
                })
                .collect(),
        )
    };
    let count = match keys {
        [key] => match view.get(key) {
            Some(Shown::List(items)) => u64::try_from(items.len()).ok(),
            _ => None,
        },
        _ => None,
    };
    section(
        Some(tr(title)),
        count,
        None,
        section_intro(tr(intro)).then(body),
    )
}

/// The policy drawn by section.
#[must_use]
pub fn policy_view(view: &PolicyView) -> Html {
    let head = definitions(vec![
        (tr(PolicyMsg::FactName), t(&view.name)),
        (
            tr(PolicyMsg::FactController),
            view.get("controller").map_or_else(none, shown),
        ),
    ]);
    Html::join(
        std::iter::once(section(Some(tr(PolicyMsg::ThisPolicy)), None, None, head)).chain(
            SECTIONS
                .iter()
                .map(|(title, intro, keys)| policy_section(view, *title, *intro, keys)),
        ),
    )
}

/// What an administrator may do from the page: edit the policy in force,
/// or write the first version.
fn actions(first: bool) -> ActionGroup {
    ActionGroup::primary(Action::new(
        Some(if first { Glyph::PLUS } else { Glyph::PENCIL }),
        tr(if first {
            PolicyMsg::WriteFirst
        } else {
            PolicyMsg::EditPolicy
        }),
        Act::Link(EDIT_ROUTE.to_owned()),
    ))
}

/// The policy tab's panel.
fn policy_panel(read: Result<Reading, Unread>) -> (ActionGroup, Html) {
    let shown = match read {
        Ok(shown) => shown,
        Err(Unread::NoneCommitted) => {
            return (
                actions(true),
                panel(plain_section(empty_state(
                    Glyph::SHIELD,
                    tr(PolicyMsg::NoneCommitted),
                    tr(PolicyMsg::NoneCommittedLine),
                    ferrohealth_ui::view::button::link_button(
                        EDIT_ROUTE.to_owned(),
                        ferrohealth_ui::view::button::Variant::Primary,
                        Some(Glyph::PLUS),
                        tr(PolicyMsg::WriteFirst),
                    ),
                ))),
            );
        }
        Err(Unread::Refused(refusal)) => {
            return (ActionGroup::default(), panel(plain_section(refusal.view())));
        }
    };
    let Reading {
        version_uid,
        body,
        older,
        count,
    } = shown;
    let notice = older
        .as_ref()
        .map(|number| plain_section(older_version(&current(), number, count, POLICY_ROUTE)));
    let drawn = match read_policy(&body) {
        Ok(view) => policy_view(&view),
        Err(error) => plain_section(Refused::new(tr(PolicyMsg::PolicyRefused), error).view()),
    };
    let document = document_section(
        tr_with(
            PolicyMsg::DocumentTitle,
            &[("version", version_number(&version_uid).into())],
        ),
        Language::Json,
        body,
        "policy-document",
    );
    let actions = if older.is_some() {
        ActionGroup::default()
    } else {
        actions(false)
    };
    (actions, panel(opt(notice).then(drawn).then(document)))
}

/// Access policy › Policy.
pub async fn page() -> Drawn {
    let (actions, body) = policy_panel(read_policy_tab().await);
    Drawn::page(
        tr(Msg::NavAccessPolicy),
        frame(PolicyTab::Policy, actions, body),
    )
}

/// The versions tab's panel.
fn versions_panel(read: Result<Vec<PolicyVersion>, Refused>) -> Html {
    let versions = match read {
        Ok(versions) => versions,
        Err(refusal) => return panel(plain_section(refusal.view())),
    };
    if versions.is_empty() {
        return panel(plain_section(empty_state(
            Glyph::SHIELD,
            tr(PolicyMsg::NoneCommitted),
            tr(PolicyMsg::NoneCommittedLine),
            Html::empty(),
        )));
    }
    let entries: Vec<_> = versions.into_iter().map(|version| version.entry).collect();
    let at = crate::request::query("at");
    let reading = at
        .as_deref()
        .and_then(|at| committed_at(&entries, at))
        .map(|entry| version_number(&entry.version_id));
    let history = crate::pages::versions::history(
        History {
            versions: &entries,
            reading: reading.as_deref(),
            deleted: false,
            contribution: t(tr(PolicyMsg::OwnContribution)),
            caption: tr(PolicyMsg::VersionsCaption),
            at_action: VERSIONS_ROUTE.to_owned(),
            at_opens: tr(PolicyMsg::AtTimeOpens),
        },
        |number| match number {
            None => POLICY_ROUTE.to_owned(),
            Some(number) => format!("{POLICY_ROUTE}?version={number}"),
        },
    );
    let newest = entries.first().map(|entry| {
        definitions(vec![
            (
                tr(PolicyMsg::FactVersion),
                t(version_number(&entry.version_id)),
            ),
            (tr(PolicyMsg::FactCommitter), t(&entry.committer)),
            (tr(PolicyMsg::FactCommitted), when(&entry.committed)),
        ])
    });
    panel(
        section(Some(tr(PolicyMsg::InForce)), None, None, opt(newest)).then(section(
            Some(tr(PolicyMsg::VersionsCaption)),
            u64::try_from(entries.len()).ok(),
            None,
            section_intro(tr(PolicyMsg::VersionsIntro)).then(history),
        )),
    )
}

/// The newest version committed at or before `at`, an RFC 3339 instant or a
/// date and time as the At a time field posts it.
fn committed_at<'a>(
    entries: &'a [crate::bff::wire::VersionEntry],
    at: &str,
) -> Option<&'a crate::bff::wire::VersionEntry> {
    let instant = |text: &str| {
        text.parse::<jiff::Timestamp>().ok().or_else(|| {
            text.parse::<jiff::civil::DateTime>()
                .ok()
                .and_then(|time| time.to_zoned(jiff::tz::TimeZone::UTC).ok())
                .map(|zoned| zoned.timestamp())
        })
    };
    let at = instant(at)?;
    entries
        .iter()
        .find(|entry| instant(&entry.committed).is_some_and(|committed| committed <= at))
}

/// Access policy › Versions.
pub async fn versions_page() -> Drawn {
    let read = fetch_history().await.map_err(refusal_of);
    Drawn::page(
        tr(Msg::NavAccessPolicy),
        frame(
            PolicyTab::Versions,
            ActionGroup::default(),
            versions_panel(read),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::{committed_at, policy_view};
    use crate::bff::access_policy::read_policy;
    use crate::bff::wire::VersionEntry;

    fn entry(number: u32, committed: &str) -> VersionEntry {
        VersionEntry {
            version_id: format!("a::ferroehr.local::{number}"),
            committed: committed.to_owned(),
            change_type: "modification".to_owned(),
            committer: "Admin".to_owned(),
        }
    }

    #[test]
    fn at_a_time_opens_the_newest_version_committed_by_then() {
        let entries = [
            entry(2, "2026-10-01T09:00:02Z"),
            entry(1, "2026-10-01T09:00:01Z"),
        ];
        let found = |at| committed_at(&entries, at).map(|entry| entry.version_id.clone());
        assert_eq!(
            Some("a::ferroehr.local::1".to_owned()),
            found("2026-10-01T09:00:01Z")
        );
        assert_eq!(
            Some("a::ferroehr.local::2".to_owned()),
            found("2026-10-02T00:00")
        );
        assert_eq!(None, found("2026-09-30T00:00"));
        assert_eq!(None, found("not a time"));
    }

    #[test]
    fn a_policy_draws_every_section_with_its_coded_values() {
        let view = read_policy(
            r#"{"_type":"ACCESS_POLICY","name":{"_type":"DV_TEXT","value":"Hospital"},
                "care_relationship_required":true,"purpose_required":false,
                "time_zone":"Europe/Amsterdam",
                "concealed_confidentialities":[{"_type":"DV_CODED_TEXT","value":"very restricted",
                  "defining_code":{"_type":"CODE_PHRASE",
                    "terminology_id":{"_type":"TERMINOLOGY_ID","value":"HL7_Confidentiality"},
                    "code_string":"V"}}]}"#,
        )
        .expect("a policy");
        let html = String::from(policy_view(&view));
        assert!(html.contains("very restricted"), "{html}");
        assert!(html.contains("HL7_Confidentiality"), "{html}");
        assert!(html.contains("Care relationship required"), "{html}");
        assert!(
            html.contains("Europe/Amsterdam"),
            "the other attributes: {html}"
        );
    }
}

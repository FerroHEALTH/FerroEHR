// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server › Overview: how the CDR runs, its licence and its support period.
//!
//! For a session that may read the full status document it shows the
//! licence in force, the deployment gaps and the support period (CRA Art.
//! 13(19)), then the management surface's build information. No openEHR spec
//! governs these surfaces: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table};
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::surface::definitions;

use super::{ServerTab, Tabbed, load_tab, server_page};
use crate::deployment::{Licence, Standing, StatusDocument, Support, gap_text};
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, tr, tr_with};
use crate::management::{BuildInfoView, fetch_build_info};
use crate::markup::{h, opt, t};
use crate::pages::common::{Refused, info_note, plain_section, section, state_mark, warn_note};
use crate::routes::Drawn;

/// How the CDR's status document says it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Running {
    /// `UP`.
    Up,
    /// Any other status word, or none.
    Degraded(Option<String>),
    /// A status that is no status document.
    Unknown,
    /// No status read.
    Unreachable,
}

/// One deployment gap: its words and whether the operator accepted it by
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Gap {
    /// The gap in plain words.
    words: String,
    /// Whether `deployment_accepts` names it.
    accepted: bool,
}

/// What only the full status document carries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Full {
    /// The licence in force.
    licence: Licence,
    /// The support period, when the document carries one.
    support: Option<Support>,
    /// Every deployment gap, open or accepted.
    gaps: Vec<Gap>,
}

/// Overview's model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Overview {
    /// How the CDR runs.
    running: Running,
    /// Whether it runs the sandbox deployment profile.
    sandbox: bool,
    /// Its version, empty when it names none.
    version: String,
    /// The full document's facts, absent when the session reads the public
    /// one.
    full: Option<Full>,
    /// The day the page is drawn on, in UTC.
    today: jiff::civil::Date,
    /// The management surface's build information, or its refusal.
    build: Result<Option<BuildInfoView>, Refused>,
}

/// The full document's facts of `document`, when it is the full one.
fn full_of(document: &StatusDocument) -> Option<Full> {
    let licence = document.licence.clone()?;
    let gaps = document
        .deployment
        .as_ref()
        .map(|deployment| {
            deployment
                .gaps
                .iter()
                .map(|code| Gap {
                    words: gap_text(code),
                    accepted: deployment.accepted.contains(code),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Full {
        licence,
        support: document.support.clone(),
        gaps,
    })
}

/// What the page reads, on the server.
async fn read_overview() -> Tabbed<Overview> {
    load_tab(ServerTab::Overview, || async {
        let status = crate::shell::status().await;
        let document = status
            .as_ref()
            .ok()
            .and_then(|body| StatusDocument::parse(body));
        let running = match (&status, &document) {
            (Ok(_), Some(document)) if document.status.as_deref() == Some("UP") => Running::Up,
            (Ok(_), Some(document)) => Running::Degraded(document.status.clone()),
            (Ok(_), None) => Running::Unknown,
            (Err(_), _) => Running::Unreachable,
        };
        let sandbox = document
            .as_ref()
            .and_then(StatusDocument::profile)
            .is_some_and(|profile| profile == "sandbox");
        Overview {
            running,
            sandbox,
            version: document
                .as_ref()
                .and_then(|document| document.server_version.clone())
                .unwrap_or_default(),
            full: document.as_ref().and_then(full_of),
            today: crate::request::now()
                .to_zoned(jiff::tz::TimeZone::UTC)
                .date(),
            build: fetch_build_info()
                .await
                .map_err(|error| Refused::new(tr(ConsoleMsg::ServerBuildRefused), error)),
        }
    })
    .await
}

/// The warnings the full document earns on `today`: a licence that is not in
/// force, and a support period that has ended.
fn warnings(full: &Full, today: jiff::civil::Date) -> Vec<String> {
    let licence = match full.licence.standing(today) {
        Standing::InForce => None,
        Standing::Expired => Some(tr_with(
            ConsoleMsg::ServerLicenceExpired,
            &[(
                "day",
                full.licence
                    .not_after
                    .map(|day| day.to_string())
                    .unwrap_or_default()
                    .into(),
            )],
        )),
        Standing::Refused => Some(tr(ConsoleMsg::ServerLicenceRefused)),
        Standing::Unlicensed => Some(tr(ConsoleMsg::ServerLicenceNone)),
    };
    let support = full
        .support
        .as_ref()
        .filter(|support| support.ended())
        .map(|support| {
            tr_with(
                ConsoleMsg::ServerSupportEnded,
                &[("month", support.ends.clone().unwrap_or_default().into())],
            )
        });
    licence.into_iter().chain(support).collect()
}

/// A fact the document may leave out, as its text or the empty mark.
fn or_none(value: Option<&String>) -> Html {
    t(value.cloned().unwrap_or_else(|| tr(Msg::NoValue)))
}

/// The licence in force, drawn.
fn licence(licence: &Licence, today: jiff::civil::Date) -> Html {
    let state = match licence.standing(today) {
        Standing::InForce => state_mark(FlowState::Up, &tr(ConsoleMsg::ServerLicensed)),
        Standing::Expired => state_mark(FlowState::Degraded, &tr(ConsoleMsg::ServerExpired)),
        Standing::Refused => state_mark(FlowState::Degraded, &tr(ConsoleMsg::ServerLicensed)),
        Standing::Unlicensed => {
            state_mark(FlowState::Unreachable, &tr(ConsoleMsg::ServerUnlicensed))
        }
    };
    let permitted = licence
        .permitted_use
        .as_deref()
        .map(|permitted| match permitted {
            "commercial" => tr(ConsoleMsg::ServerUseCommercial),
            "non-commercial" => tr(ConsoleMsg::ServerUseNonCommercial),
            other => other.to_owned(),
        });
    section(
        Some(tr(ConsoleMsg::ServerLicence)),
        None,
        None,
        definitions(vec![
            (tr(ConsoleMsg::ServerState), state),
            (
                tr(ConsoleMsg::ServerLicenceUse),
                or_none(permitted.as_ref()),
            ),
            (
                tr(ConsoleMsg::ServerLicensee),
                or_none(licence.licensee.as_ref()),
            ),
            (
                tr(ConsoleMsg::ServerNotAfter),
                or_none(licence.not_after.map(|day| day.to_string()).as_ref()),
            ),
        ]),
    )
}

/// The support period, drawn.
fn support(support: &Support) -> Html {
    let state = match support.status.as_str() {
        "supported" => state_mark(FlowState::Up, &tr(ConsoleMsg::ServerSupported)),
        "ended" => state_mark(FlowState::Degraded, &tr(ConsoleMsg::ServerSupportOver)),
        "unreleased" => state_mark(FlowState::NotChecked, &tr(ConsoleMsg::ServerUnreleased)),
        other => state_mark(FlowState::NotChecked, other),
    };
    section(
        Some(tr(ConsoleMsg::ServerSupport)),
        None,
        None,
        definitions(vec![
            (tr(ConsoleMsg::ServerState), state),
            (
                tr(ConsoleMsg::ServerSupportEnds),
                or_none(support.ends.as_ref()),
            ),
            (
                tr(ConsoleMsg::ServerReleased),
                or_none(support.release_date.as_ref()),
            ),
        ]),
    )
}

/// The deployment gaps, each open or accepted by name; nothing when there
/// are none.
fn gaps(gaps: &[Gap], sandbox: bool) -> Option<Html> {
    if gaps.is_empty() {
        return None;
    }
    let columns = [
        Column::text("gap", tr(ConsoleMsg::ServerSafeguard)).role(Role::Title),
        Column::text("state", tr(ConsoleMsg::ServerState)).role(Role::Sub),
    ];
    let rows = gaps
        .iter()
        .map(|gap| {
            let state = if gap.accepted {
                state_mark(FlowState::Off, &tr(ConsoleMsg::ServerGapAccepted))
            } else {
                state_mark(FlowState::Degraded, &tr(ConsoleMsg::ServerGapOpen))
            };
            h(format!(
                r#"<tr><th scope="row" data-role="title">{}</th>{}</tr>"#,
                t(&gap.words),
                cell(&columns[1], state)
            ))
        })
        .collect();
    let caption = tr(ConsoleMsg::ServerSafeguardsOff);
    let intro = tr(if sandbox {
        ConsoleMsg::ServerSandboxIntro
    } else {
        ConsoleMsg::ServerGapsIntro
    });
    Some(section(
        Some(caption.clone()),
        u64::try_from(gaps.len()).ok(),
        None,
        ferrohealth_ui::view::surface::section_intro(intro).then(data_table(DataTable::new(
            &caption,
            &columns,
            Body::Rows(rows),
        ))),
    ))
}

/// The management surface's build information, drawn.
fn build(build: Result<Option<BuildInfoView>, Refused>) -> Html {
    match build {
        Ok(Some(info)) => {
            let mut parts = vec![section(
                Some(tr(ConsoleMsg::ServerBuild)),
                None,
                None,
                definitions(
                    info.facts
                        .iter()
                        .map(|(key, value)| (key.clone(), t(value)))
                        .collect(),
                ),
            )];
            for group in &info.sections {
                parts.push(section(
                    Some(group.key.clone()),
                    None,
                    None,
                    definitions(
                        group
                            .rows
                            .iter()
                            .map(|(key, value)| (key.clone(), t(value)))
                            .collect(),
                    ),
                ));
            }
            Html::join(parts)
        }
        Ok(None) => plain_section(info_note(t(tr(ConsoleMsg::ServerNoBuildInfo)))),
        Err(refusal) => plain_section(refusal.view()),
    }
}

/// Overview, drawn.
fn overview(model: Overview) -> Html {
    let Overview {
        running,
        sandbox,
        version,
        full,
        today,
        build: built,
    } = model;
    let state = match running {
        Running::Up => state_mark(FlowState::Up, &tr(ConsoleMsg::ServerUp)),
        Running::Degraded(word) => state_mark(
            FlowState::Degraded,
            &word.unwrap_or_else(|| tr(ConsoleMsg::ServerUnknown)),
        ),
        Running::Unknown => state_mark(FlowState::NotChecked, &tr(ConsoleMsg::ServerUnknown)),
        Running::Unreachable => {
            state_mark(FlowState::Unreachable, &tr(ConsoleMsg::ServerUnreachable))
        }
    };
    let facts = definitions(vec![
        (tr(ConsoleMsg::ServerState), state),
        (
            tr(ConsoleMsg::ServerDeployment),
            t(tr(if sandbox {
                ConsoleMsg::ServerSandbox
            } else {
                ConsoleMsg::ServerProduction
            })),
        ),
        (tr(ConsoleMsg::ServerVersion), t(version)),
        (
            tr(ConsoleMsg::ServerConformance),
            h(format!(r#"<a href="/about">{}</a>"#, t(tr(Msg::NavAbout)))),
        ),
    ]);
    let full = match full {
        Some(full) => {
            let warned = warnings(&full, today);
            Html::join([
                opt((!warned.is_empty()).then(|| {
                    plain_section(Html::join(
                        warned.into_iter().map(|line| warn_note(t(line))),
                    ))
                })),
                licence(&full.licence, today),
                opt(full.support.as_ref().map(support)),
                opt(gaps(&full.gaps, sandbox)),
            ])
        }
        None => plain_section(info_note(t(tr(ConsoleMsg::ServerFullStatusWithheld)))),
    };
    Html::join([
        section(Some(tr(ConsoleMsg::ServerStatus)), None, None, facts),
        full,
        build(built),
    ])
}

/// Server › Overview.
pub async fn page() -> Drawn {
    let tabbed = read_overview().await;
    Drawn::page(
        tr(Msg::NavServer),
        server_page(ServerTab::Overview, tabbed, overview),
    )
}

#[cfg(test)]
mod tests {
    use super::{Full, Gap, warnings};
    use crate::deployment::{Licence, Support};

    fn day(text: &str) -> jiff::civil::Date {
        text.parse().expect("a fixture day")
    }

    fn full(licence: Licence, status: &str) -> Full {
        Full {
            licence,
            support: Some(Support {
                release_date: Some("2026-11-01".to_owned()),
                ends: Some("2031-11".to_owned()),
                status: status.to_owned(),
            }),
            gaps: vec![Gap {
                words: "x".to_owned(),
                accepted: false,
            }],
        }
    }

    fn licensed(not_after: &str, token: &str) -> Licence {
        Licence {
            state: "licensed".to_owned(),
            permitted_use: Some("commercial".to_owned()),
            licensee: Some("Example Hospital".to_owned()),
            not_after: Some(day(not_after)),
            configured_token: Some(token.to_owned()),
        }
    }

    /// A licence in force and a supported release earn no warning; an
    /// expired, refused or absent licence and an ended support period each
    /// earn one.
    #[test]
    fn only_a_licence_out_of_force_or_an_ended_support_period_warns() {
        let today = day("2027-03-01");
        assert_eq!(
            0,
            warnings(
                &full(licensed("2027-12-31", "in-force"), "supported"),
                today
            )
            .len()
        );
        assert_eq!(
            warnings(
                &full(licensed("2027-01-31", "in-force"), "supported"),
                today
            ),
            ["The licence expired on 2027-01-31."]
        );
        assert_eq!(
            1,
            warnings(&full(licensed("2027-12-31", "refused"), "supported"), today).len()
        );
        let none = Licence {
            state: "none".to_owned(),
            permitted_use: None,
            licensee: None,
            not_after: None,
            configured_token: Some("absent".to_owned()),
        };
        assert_eq!(2, warnings(&full(none, "ended"), today).len());
    }
}

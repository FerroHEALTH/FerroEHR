// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Legal marks tab.
//!
//! It shows the restriction of processing, the research objection, the
//! access-delay holds and the retention anchor and holds, each read from the
//! CDR's admin marks extension, and the forms that set them.
//!
//! Every write that changes what others may read waits behind a
//! confirmation. A `403` and a `405` are drawn as the CDR answered them.
//! No openEHR specification governs the marks: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table, title_cell};
use ferrohealth_ui::view::dialog::{Confirmation, Stakes, confirm_dialog};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{definitions, section_intro};

use crate::bff::marks::{
    Anchor, DelayHold, MarkWrite, Objection, Restriction, RetentionHold, fetch_anchor,
    fetch_delays, fetch_objection, fetch_restrictions, fetch_retention_holds, write_mark,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::policy::PolicyMsg;
use crate::locale::{current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    Refused, is_status, panel, plain_section, section, short, shortened, when,
};
use crate::pages::ehr::{EhrModel, EhrTab, ehr_id_param, ehr_view, tab_href};
use crate::routes::Drawn;

/// The grounds of GDPR Art. 18(1) a restriction rests on, and `national`.
const GROUNDS: [&str; 5] = [
    "gdpr-18-1-a",
    "gdpr-18-1-b",
    "gdpr-18-1-c",
    "gdpr-18-1-d",
    "national",
];

/// The refusal of a read of a mark, worded for a `403` and a `405`.
fn refusal_of(error: ViewerError) -> Refused {
    let summary = if matches!(error, ViewerError::Forbidden(_)) {
        PolicyMsg::MarksForbidden
    } else if is_status(&error, http::StatusCode::METHOD_NOT_ALLOWED) {
        PolicyMsg::AdminOff
    } else {
        PolicyMsg::MarksRefused
    };
    Refused::new(tr(summary), error)
}

/// The tab's model: each register, or the CDR's refusal of it, and the write
/// the CDR refused.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Marks {
    /// The restriction register.
    restrictions: Result<Vec<Restriction>, Refused>,
    /// The research objection.
    objection: Result<Objection, Refused>,
    /// The access-delay holds.
    delays: Result<Vec<DelayHold>, Refused>,
    /// The retention anchor.
    anchor: Result<Option<Anchor>, Refused>,
    /// The objects held from disposal.
    holds: Result<Vec<RetentionHold>, Refused>,
    /// The write the CDR refused.
    refused: Option<Refused>,
}

/// The text a posted field holds, `None` when it is empty.
fn filled(posted: &crate::posted::Posted, name: &str) -> Option<String> {
    Some(posted.text(name).trim().to_owned()).filter(|text| !text.is_empty())
}

/// The write a post asks for, by its intent.
fn posted_write(posted: &crate::posted::Posted) -> Option<MarkWrite> {
    Some(match posted.intent().as_str() {
        "restrict" => MarkWrite::Restrict {
            vo_id: filled(posted, "vo_id"),
            ground: posted.text("ground"),
            note: filled(posted, "note"),
        },
        "lift" => MarkWrite::Lift {
            vo_id: filled(posted, "vo_id"),
        },
        "object" => MarkWrite::Objection {
            objected: true,
            ground: None,
        },
        "override" => MarkWrite::Objection {
            objected: true,
            ground: filled(posted, "ground"),
        },
        "withdraw" => MarkWrite::Objection {
            objected: false,
            ground: None,
        },
        "delay" => MarkWrite::Delay {
            vo_id: posted.text("vo_id").trim().to_owned(),
            until: filled(posted, "until"),
            note: filled(posted, "note"),
        },
        "release" => MarkWrite::Release {
            vo_id: posted.text("vo_id").trim().to_owned(),
        },
        "anchor" => MarkWrite::Anchor(Anchor {
            jurisdiction: posted.text("jurisdiction").trim().to_owned(),
            anchored_at: filled(posted, "anchored_at"),
            hold_at: filled(posted, "hold_at"),
            hold_ground: filled(posted, "hold_ground"),
        }),
        "hold" | "unhold" => MarkWrite::Hold {
            vo_id: posted.text("vo_id").trim().to_owned(),
            held: posted.intent() == "hold",
        },
        _ => return None,
    })
}

/// What the tab reads, after the write its post asks.
async fn read(ehr_id: String) -> EhrModel<Marks> {
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let mut refused = None;
        if let Some(mark) = crate::posted::posted().as_ref().and_then(posted_write) {
            match write_mark(&ehr_id, &mark).await {
                Ok(()) => {
                    redirect_with(
                        &tab_href(&ehr_id, EhrTab::Marks),
                        &Done::ok(tr(PolicyMsg::MarksSaved)),
                    );
                }
                Err(error) => refused = Some(Refused::new(tr(PolicyMsg::MarksSaveRefused), error)),
            }
        }
        Marks {
            restrictions: fetch_restrictions(&ehr_id).await.map_err(refusal_of),
            objection: fetch_objection(&ehr_id).await.map_err(refusal_of),
            delays: fetch_delays(&ehr_id).await.map_err(refusal_of),
            anchor: fetch_anchor(&ehr_id).await.map_err(refusal_of),
            holds: fetch_retention_holds(&ehr_id).await.map_err(refusal_of),
            refused,
        }
    })
    .await
}

/// A form posting to the tab whose submit waits behind a confirmation.
fn confirmed_form(
    action: &str,
    id: &str,
    fields: &Html,
    intent: &str,
    words: PolicyMsg,
    message: PolicyMsg,
) -> Html {
    let dialog = format!("{id}-confirm");
    h(format!(
        r#"<form id="{id}" method="post" action="{action}" class="fui-form">{fields}<div class="fui-cluster"><button type="button" class="fui-button fui-button-secondary" command="show-modal" commandfor="{dialog}" aria-haspopup="dialog">{label}</button></div></form>"#,
        action = t(action),
        label = t(tr(words)),
    ))
    .then(confirm_dialog(
        &current(),
        Confirmation {
            id: dialog.into(),
            title: tr(words).into(),
            message: tr(message).into(),
        },
        Stakes::Plain,
        None,
        h(format!(
            r#"<button type="submit" form="{id}" name="intent" value="{intent}" class="fui-button fui-button-primary">{}</button>"#,
            t(tr(words))
        )),
    ))
}

/// A text field.
fn field(id: &str, name: &str, label: PolicyMsg, help: Option<PolicyMsg>) -> Html {
    Field::new(id, tr(label)).help_if(help.map(tr)).view(h(format!(
        r#"<input id="{id}" name="{name}" type="text" class="fui-input fui-mono" spellcheck="false" autocomplete="off">"#
    )))
}

/// A table under `caption` of `columns`, each its key, its header and its
/// role, and `rows`, each row's cells in that order, the first its title.
fn table(
    caption: PolicyMsg,
    columns: &[(&'static str, PolicyMsg, Role)],
    rows: Vec<Vec<Html>>,
) -> Html {
    let columns: Vec<Column> = columns
        .iter()
        .map(|(key, words, role)| Column::text(key, tr(*words)).role(*role))
        .collect();
    let body = rows.into_iter().map(|cells| {
        let drawn = cells
            .into_iter()
            .enumerate()
            .map(|(at, value)| match columns.get(at) {
                Some(column) if at > 0 => cell(column, value),
                _ => title_cell(None, None, value, None),
            });
        h(format!("<tr>{}</tr>", Html::join(drawn)))
    });
    let caption = tr(caption);
    data_table(DataTable::new(
        &caption,
        &columns,
        Body::Rows(Html::join(body)),
    ))
}

/// An object or the whole EHR, in words.
fn object_words(vo_id: Option<&str>) -> Html {
    vo_id.map_or_else(|| t(tr(PolicyMsg::MarksWholeEhr)), short)
}

/// A register or its refusal.
fn drawn<T>(read: &Result<T, Refused>, draw: impl FnOnce(&T) -> Html) -> Html {
    match read {
        Ok(value) => draw(value),
        Err(refusal) => refusal.view(),
    }
}

/// The restriction section.
fn restriction_section(action: &str, read: &Result<Vec<Restriction>, Refused>) -> Html {
    let list = drawn(read, |rows| {
        if rows.is_empty() {
            return t(tr(PolicyMsg::MarksNoRestriction));
        }
        let body = rows
            .iter()
            .map(|row| {
                vec![
                    object_words(row.vo_id.as_deref()),
                    h(format!("<code>{}</code>", t(&row.ground))),
                    when(&row.requested_at),
                    row.lifted_at
                        .as_deref()
                        .map_or_else(|| t(tr(PolicyMsg::MarksInForce)), when),
                    t(row.note.clone().unwrap_or_default()),
                ]
            })
            .collect();
        table(
            PolicyMsg::MarksRestrictions,
            &[
                ("object", PolicyMsg::MarksObject, Role::Title),
                ("ground", PolicyMsg::MarksGround, Role::Sub),
                ("placed", PolicyMsg::MarksPlaced, Role::Meta),
                ("lifted", PolicyMsg::MarksLifted, Role::Meta),
                ("note", PolicyMsg::MarksNote, Role::Wide),
            ],
            body,
        )
    });
    let grounds: String = GROUNDS
        .iter()
        .map(|ground| format!(r#"<option value="{ground}">{ground}</option>"#))
        .collect::<Vec<_>>()
        .concat();
    let place = confirmed_form(
        action,
        "mark-restrict",
        &Html::join([
            field("restrict-vo", "vo_id", PolicyMsg::MarksObjectId, Some(PolicyMsg::MarksObjectIdHelp)),
            Field::new("restrict-ground", tr(PolicyMsg::MarksGround)).view(h(format!(
                r#"<select id="restrict-ground" name="ground" class="fui-select">{grounds}</select>"#
            ))),
            field("restrict-note", "note", PolicyMsg::MarksNote, None),
        ]),
        "restrict",
        PolicyMsg::MarksRestrict,
        PolicyMsg::MarksRestrictMessage,
    );
    let lift_form = confirmed_form(
        action,
        "mark-lift",
        &field(
            "lift-vo",
            "vo_id",
            PolicyMsg::MarksObjectId,
            Some(PolicyMsg::MarksObjectIdHelp),
        ),
        "lift",
        PolicyMsg::MarksLift,
        PolicyMsg::MarksLiftMessage,
    );
    section(
        Some(tr(PolicyMsg::MarksRestrictions)),
        None,
        None,
        section_intro(tr(PolicyMsg::MarksRestrictionsIntro))
            .then(list)
            .then(place)
            .then(lift_form),
    )
}

/// The research objection section.
fn objection_section(action: &str, read: &Result<Objection, Refused>) -> Html {
    let state = drawn(read, |objection| {
        let standing = match (
            objection.objected,
            &objection.ground,
            &objection.objected_at,
        ) {
            (true, _, _) => tr(PolicyMsg::MarksObjected),
            (false, Some(_), _) => tr(PolicyMsg::MarksOverridden),
            (false, None, _) => tr(PolicyMsg::MarksNoObjection),
        };
        definitions(vec![
            (tr(PolicyMsg::MarksStanding), t(standing)),
            (
                tr(PolicyMsg::MarksPlaced),
                objection
                    .objected_at
                    .as_deref()
                    .map_or_else(Html::empty, when),
            ),
            (
                tr(PolicyMsg::MarksGround),
                t(objection.ground.clone().unwrap_or_default()),
            ),
        ])
    });
    let object = confirmed_form(
        action,
        "mark-object",
        &Html::empty(),
        "object",
        PolicyMsg::MarksRecordObjection,
        PolicyMsg::MarksObjectMessage,
    );
    let override_form = confirmed_form(
        action,
        "mark-override",
        &field(
            "override-ground",
            "ground",
            PolicyMsg::MarksOverrideGround,
            None,
        ),
        "override",
        PolicyMsg::MarksOverride,
        PolicyMsg::MarksOverrideMessage,
    );
    let withdraw = confirmed_form(
        action,
        "mark-withdraw",
        &Html::empty(),
        "withdraw",
        PolicyMsg::MarksWithdraw,
        PolicyMsg::MarksWithdrawMessage,
    );
    section(
        Some(tr(PolicyMsg::MarksObjection)),
        None,
        None,
        section_intro(tr(PolicyMsg::MarksObjectionIntro))
            .then(state)
            .then(object)
            .then(override_form)
            .then(withdraw),
    )
}

/// The access-delay section.
fn delay_section(action: &str, read: &Result<Vec<DelayHold>, Refused>) -> Html {
    let list = drawn(read, |rows| {
        if rows.is_empty() {
            return t(tr(PolicyMsg::MarksNoDelay));
        }
        let body = rows
            .iter()
            .map(|row| {
                vec![
                    short(&row.vo_id),
                    t(&row.basis),
                    when(&row.withheld_until),
                    row.released_at
                        .as_deref()
                        .map_or_else(|| t(tr(PolicyMsg::MarksHeld)), when),
                    t(row.note.clone().unwrap_or_default()),
                ]
            })
            .collect();
        table(
            PolicyMsg::MarksDelays,
            &[
                ("object", PolicyMsg::MarksObject, Role::Title),
                ("basis", PolicyMsg::MarksBasis, Role::Sub),
                ("until", PolicyMsg::MarksUntil, Role::Meta),
                ("released", PolicyMsg::MarksReleased, Role::Meta),
                ("note", PolicyMsg::MarksNote, Role::Wide),
            ],
            body,
        )
    });
    let place = confirmed_form(
        action,
        "mark-delay",
        &Html::join([
            field("delay-vo", "vo_id", PolicyMsg::MarksCompositionId, None),
            field(
                "delay-until",
                "until",
                PolicyMsg::MarksUntil,
                Some(PolicyMsg::MarksUntilHelp),
            ),
            field("delay-note", "note", PolicyMsg::MarksNote, None),
        ]),
        "delay",
        PolicyMsg::MarksDelay,
        PolicyMsg::MarksDelayMessage,
    );
    let release = confirmed_form(
        action,
        "mark-release",
        &field("release-vo", "vo_id", PolicyMsg::MarksCompositionId, None),
        "release",
        PolicyMsg::MarksRelease,
        PolicyMsg::MarksReleaseMessage,
    );
    section(
        Some(tr(PolicyMsg::MarksDelays)),
        None,
        None,
        section_intro(tr(PolicyMsg::MarksDelaysIntro))
            .then(list)
            .then(place)
            .then(release),
    )
}

/// The retention section: the anchor and the held objects.
fn retention_section(
    action: &str,
    anchor: &Result<Option<Anchor>, Refused>,
    holds: &Result<Vec<RetentionHold>, Refused>,
) -> Html {
    let anchor = drawn(anchor, |anchor| match anchor {
        None => t(tr(PolicyMsg::MarksNoAnchor)),
        Some(anchor) => definitions(vec![
            (tr(PolicyMsg::MarksJurisdiction), t(&anchor.jurisdiction)),
            (
                tr(PolicyMsg::MarksAnchoredAt),
                anchor.anchored_at.as_deref().map_or_else(Html::empty, when),
            ),
            (
                tr(PolicyMsg::MarksHoldAt),
                anchor.hold_at.as_deref().map_or_else(Html::empty, when),
            ),
            (
                tr(PolicyMsg::MarksHoldGround),
                t(anchor.hold_ground.clone().unwrap_or_default()),
            ),
        ]),
    });
    let holds = drawn(holds, |rows| {
        if rows.is_empty() {
            return t(tr(PolicyMsg::MarksNoHold));
        }
        let body = rows
            .iter()
            .map(|row| vec![short(&row.vo_id), when(&row.held_at)])
            .collect();
        table(
            PolicyMsg::MarksHolds,
            &[
                ("object", PolicyMsg::MarksObject, Role::Title),
                ("held", PolicyMsg::MarksPlaced, Role::Sub),
            ],
            body,
        )
    });
    let set_anchor = confirmed_form(
        action,
        "mark-anchor",
        &Html::join([
            field(
                "anchor-jurisdiction",
                "jurisdiction",
                PolicyMsg::MarksJurisdiction,
                None,
            ),
            field(
                "anchor-at",
                "anchored_at",
                PolicyMsg::MarksAnchoredAt,
                Some(PolicyMsg::MarksInstantHelp),
            ),
            field(
                "anchor-hold",
                "hold_at",
                PolicyMsg::MarksHoldAt,
                Some(PolicyMsg::MarksInstantHelp),
            ),
            field(
                "anchor-ground",
                "hold_ground",
                PolicyMsg::MarksHoldGround,
                None,
            ),
        ]),
        "anchor",
        PolicyMsg::MarksSetAnchor,
        PolicyMsg::MarksAnchorMessage,
    );
    let hold = confirmed_form(
        action,
        "mark-hold",
        &field("hold-vo", "vo_id", PolicyMsg::MarksObjectId, None),
        "hold",
        PolicyMsg::MarksHold,
        PolicyMsg::MarksHoldMessage,
    );
    let unhold = confirmed_form(
        action,
        "mark-unhold",
        &field("unhold-vo", "vo_id", PolicyMsg::MarksObjectId, None),
        "unhold",
        PolicyMsg::MarksUnhold,
        PolicyMsg::MarksUnholdMessage,
    );
    section(
        Some(tr(PolicyMsg::MarksRetention)),
        None,
        None,
        section_intro(tr(PolicyMsg::MarksRetentionIntro))
            .then(anchor)
            .then(set_anchor)
            .then(holds)
            .then(hold)
            .then(unhold),
    )
}

/// The tab's panel.
fn marks_panel(ehr_id: &str, marks: Marks) -> Html {
    let action = tab_href(ehr_id, EhrTab::Marks);
    panel(Html::join([
        opt(marks.refused.map(|refusal| plain_section(refusal.view()))),
        restriction_section(&action, &marks.restrictions),
        objection_section(&action, &marks.objection),
        delay_section(&action, &marks.delays),
        retention_section(&action, &marks.anchor, &marks.holds),
    ]))
}

/// An EHR's Legal marks tab.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(PolicyMsg::MarksTitle, &[("id", shortened(&ehr_id).into())]);
    let model = read(ehr_id.clone()).await;
    let body = ehr_view(&ehr_id, EhrTab::Marks, model, |marks| {
        marks_panel(&ehr_id, marks)
    });
    Drawn::page(title, body)
}

/// The treating professional's release of the composition `uid` to the
/// person, from the clinical route; `Some` holds the CDR's refusal.
pub async fn release(ehr_id: &str, uid: &str) -> Option<Refused> {
    let vo_id = crate::uid::container_uid_of(uid);
    match crate::bff::marks::release_as_professional(ehr_id, &vo_id).await {
        Ok(()) => {
            redirect_with(
                &crate::pages::composition::composition_href(ehr_id, &vo_id),
                &Done::ok(tr(PolicyMsg::MarksReleaseDone)),
            );
            None
        }
        Err(error) => Some(Refused::new(tr(PolicyMsg::MarksReleaseRefused), error)),
    }
}

/// The access delay of the composition `uid` on its page: the hold in
/// force, where the caller may read the register, and the treating
/// professional's release.
pub async fn hold_section(ehr_id: &str, uid: &str) -> Html {
    let vo_id = crate::uid::container_uid_of(uid);
    // NOTE: no specification governs this: our own design; a caller the CDR does
    // not let read the register still releases, so its refusal shows no hold.
    let held = fetch_delays(ehr_id).await.ok().and_then(|rows| {
        rows.into_iter()
            .find(|row| row.vo_id == vo_id && row.released_at.is_none())
    });
    let shown = held.map(|row| {
        plain_section(crate::pages::common::warn_note(t(tr_with(
            PolicyMsg::MarksHoldShown,
            &[("until", row.withheld_until.into())],
        ))))
    });
    let action = crate::pages::composition::composition_href(ehr_id, &vo_id);
    crate::pages::common::card_panel(section(
        Some(tr(PolicyMsg::MarksDelays)),
        None,
        None,
        opt(shown).then(section_intro(tr(PolicyMsg::MarksHoldNote))).then(h(format!(
            r#"<form method="post" action="{}"><button type="submit" name="intent" value="release-access-delay" class="fui-button fui-button-secondary">{}</button></form>"#,
            t(action),
            t(tr(PolicyMsg::MarksRelease))
        ))),
    ))
}

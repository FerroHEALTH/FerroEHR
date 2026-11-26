// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access to an EHR on a justification.
//!
//! The purpose and the reason a clinician states when the CDR refuses an
//! access no care relationship covers (`refused-reason-missing`, book
//! `access-control.md` §Justified access outside the care relationships).
//!
//! The justification is kept in the server-side session record
//! (`ferrohealth-session`'s memory store) for one EHR and
//! [`LIFETIME`], never in the browser, since the reason may hold health data
//! (GDPR Art. 5(1)(c) and (f)). While it holds, every CDR call drawing that
//! EHR's pages declares its purpose and states its reason in the headers the
//! CDR's purposes route names; the CDR alone decides whether the
//! justification admits the access, and whether it is an emergency. No
//! openEHR spec governs the viewer: our own design.

use std::time::Duration;

use ferrohealth_secret::secret::Secret;
use ferrohealth_session::justification::{Justification, Justified};

use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::form::Field;

use crate::error::ViewerError;
use crate::locale::access::AccessMsg;
use crate::locale::tr;
use crate::markup::{h, t};
use crate::pages::common::{card_panel, info_note, section, warn_note};

/// How long a stated justification holds.
pub const LIFETIME: Duration = Duration::from_mins(15);

/// The longest reason the CDR takes, in characters (book `access-control.md`,
/// Justified access outside the care relationships).
const MAX_REASON: usize = 1000;

/// The intent of the form that states a justification.
pub const JUSTIFY: &str = "justify";

/// The intent of the form that ends one.
pub const END: &str = "end-justification";

/// The refusal codes a justification can answer: the access needs a care
/// relationship the caller lacks, or rests on a justification and states no
/// reason (draft access control decision procedure, Refusals).
const JUSTIFIABLE: [&str; 2] = ["refused-no-care-relationship", "refused-reason-missing"];

/// Whether `error` is a refusal a justification can answer.
#[must_use]
pub fn asks_reason(error: &ViewerError) -> bool {
    error.status_code() == Some(http::StatusCode::FORBIDDEN)
        && error
            .detail()
            .is_some_and(|detail| JUSTIFIABLE.iter().any(|code| detail.contains(code)))
}

/// The EHR the page being drawn is about, from its route.
#[must_use]
pub fn page_ehr() -> Option<String> {
    crate::routes::params()
        .get("ehr_id")
        .filter(|ehr| !ehr.is_empty())
}

/// The justification the session holds for the EHR `ehr_id` and that has
/// not ended.
#[must_use]
pub fn held(ehr_id: &str) -> Option<Justification> {
    let state = crate::request::state()?;
    let headers = crate::request::headers();
    // NOTE: no specification governs this: our own design; an unusable store holds
    // no justification, so the CDR asks for the reason again.
    state
        .sign_in
        .sessions()
        .justification(&headers, ehr_id)
        .ok()
        .flatten()
}

/// The justification of the EHR the page being drawn is about, which its
/// CDR calls carry.
#[must_use]
pub fn of_page() -> Option<Justification> {
    held(&page_ehr()?)
}

/// What a reason typed into the form is: the reason, or why the CDR would
/// refuse it.
fn checked_reason(typed: &str) -> Result<String, AccessMsg> {
    let reason = typed.trim();
    let length = reason.chars().count();
    if length == 0 || length > MAX_REASON {
        return Err(AccessMsg::JustifyReasonLength);
    }
    if reason.chars().any(char::is_control) {
        return Err(AccessMsg::JustifyReasonControl);
    }
    Ok(reason.to_owned())
}

/// Answers a post of the justification form on an EHR's page.
///
/// It keeps or ends the justification and sends the browser back to the
/// page with a toast; `false` when the post is no justification of the
/// page's EHR.
#[must_use]
pub fn answer_post() -> bool {
    let Some(posted) = crate::posted::posted() else {
        return false;
    };
    let intent = posted.intent();
    let Some(ehr_id) = page_ehr().filter(|_| intent == JUSTIFY || intent == END) else {
        return false;
    };
    let back = crate::request::path();
    let Some(state) = crate::request::state() else {
        return false;
    };
    let headers = crate::request::headers();
    let sessions = state.sign_in.sessions();
    if intent == END {
        let done = match sessions.end_justification(&headers, &ehr_id) {
            Ok(()) => crate::flash::Done::ok(tr(AccessMsg::JustifyEnded)),
            Err(_) => crate::flash::Done::warn(tr(AccessMsg::JustifyStoreRefused)),
        };
        crate::flash::redirect_with(&back, &done);
        return true;
    }
    let purpose = posted.text("purpose");
    let done = match checked_reason(&posted.text("reason")) {
        Err(why) => crate::flash::Done::warn(tr(why)),
        Ok(_) if purpose.trim().is_empty() => {
            crate::flash::Done::warn(tr(AccessMsg::JustifyPurposeMissing))
        }
        Ok(reason) => match sessions.justify(
            &headers,
            &ehr_id,
            purpose.trim(),
            Secret::new(reason),
            LIFETIME,
        ) {
            Ok(Justified::Kept(_)) => crate::flash::Done::ok(tr(AccessMsg::JustifyKept)),
            Ok(_) | Err(_) => crate::flash::Done::warn(tr(AccessMsg::JustifyStoreRefused)),
        },
    };
    crate::flash::redirect_with(&back, &done);
    true
}

/// The refusal `drawn` of `error`, followed, when the CDR asks for the
/// reason of a justified access on an EHR's page, by the form that states
/// one, or by why this viewer cannot.
#[must_use]
pub fn after_refusal(error: &ViewerError, drawn: Html) -> Html {
    if !asks_reason(error) || page_ehr().is_none() {
        return drawn;
    }
    drawn.then(offer())
}

/// The justification form, or why the viewer offers none.
fn offer() -> Html {
    let offered = crate::request::state().and_then(|state| state.cdr.purposes().last_offered());
    match offered {
        Some(offered) if offered.reason_header.is_some() => form(&offered),
        _ => info_note(t(tr(AccessMsg::JustifyUnoffered))),
    }
}

/// The form that states a justification: the purpose of use among those the
/// CDR lists, and the reason.
fn form(offered: &crate::purpose::Offered) -> Html {
    let options: Html = offered
        .codes
        .iter()
        .map(|code| {
            let words = code.display.as_deref().map_or_else(
                || code.code.clone(),
                |display| format!("{display} ({})", code.code),
            );
            h(format!(
                r#"<option value="{}">{}</option>"#,
                t(&code.code),
                t(words)
            ))
        })
        .collect();
    let purpose = if offered.codes.is_empty() {
        h(r#"<input id="j-purpose" name="purpose" type="text" class="fui-input fui-mono" required autocomplete="off" spellcheck="false">"#.to_owned())
    } else {
        h(format!(
            r#"<select id="j-purpose" name="purpose" class="fui-select" required>{options}</select>"#
        ))
    };
    let reason = Field::new("j-reason", tr(AccessMsg::JustifyReason))
        .help(tr(AccessMsg::JustifyReasonHelp))
        .view(h(format!(
            r#"<textarea id="j-reason" name="reason" class="fui-textarea" rows="3" maxlength="{MAX_REASON}" required aria-describedby="{}"></textarea>"#,
            ferrohealth_ui::view::form::help_id("j-reason")
        )));
    let body = h(format!(
        r#"<form method="post" action="{}" class="fui-stack"><input type="hidden" name="intent" value="{JUSTIFY}"><p>{}</p>{}{reason}<p class="fui-small fui-muted">{}</p><div class="fui-cluster"><button type="submit" class="fui-button fui-button-primary">{}</button></div></form>"#,
        t(crate::request::path()),
        t(tr(AccessMsg::JustifyIntro)),
        Field::new("j-purpose", tr(AccessMsg::JustifyPurpose)).view(purpose),
        t(tr(AccessMsg::JustifyHolds)),
        t(tr(AccessMsg::JustifySubmit))
    ));
    card_panel(section(Some(tr(AccessMsg::JustifyTitle)), None, None, body))
}

/// `body` under the notice of the justification the page's EHR is opened
/// on, with End now, while one holds.
#[must_use]
pub fn with_banner(body: Html) -> Html {
    let Some(justification) = of_page() else {
        return body;
    };
    let until = jiff::Timestamp::from_second(justification.ends_at())
        .map(|instant| {
            ferrohealth_ui::view::time::reading(&crate::locale::current(), &instant.to_string())
        })
        .unwrap_or_default();
    let words = crate::locale::tr_with(
        AccessMsg::JustifyBanner,
        &[
            ("purpose", justification.purpose().to_owned().into()),
            ("until", until.into()),
            ("reason", justification.reason().expose().to_owned().into()),
        ],
    );
    let end = h(format!(
        r#"<form method="post" action="{}" class="fui-inline"><input type="hidden" name="intent" value="{END}"><button type="submit" class="fui-button fui-button-secondary fui-button-small">{}</button></form>"#,
        t(crate::request::path()),
        t(tr(AccessMsg::JustifyEnd))
    ));
    warn_note(t(words).then(end)).then(body)
}

/// The reason as the reason header carries it: percent-encoded UTF-8 (RFC
/// 3986 §2.1), so any language travels in an HTTP header.
#[must_use]
pub fn encoded(reason: &str) -> String {
    urlencoding::encode(reason).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{AccessMsg, asks_reason, checked_reason, encoded};
    use crate::error::ViewerError;

    #[test]
    fn a_reason_is_one_to_a_thousand_characters_with_no_control() {
        assert_eq!(
            checked_reason("  Unresponsive  "),
            Ok("Unresponsive".to_owned())
        );
        assert_eq!(checked_reason("   "), Err(AccessMsg::JustifyReasonLength));
        assert_eq!(
            checked_reason(&"é".repeat(1001)),
            Err(AccessMsg::JustifyReasonLength)
        );
        assert_eq!(
            checked_reason(&"é".repeat(1000)).map(|r| r.chars().count()),
            Ok(1000)
        );
        assert_eq!(
            checked_reason("a\u{7}b"),
            Err(AccessMsg::JustifyReasonControl)
        );
    }

    #[test]
    fn the_reason_travels_percent_encoded() {
        assert_eq!(
            encoded("Bewusstlos bei Ankunft"),
            "Bewusstlos%20bei%20Ankunft"
        );
        assert_eq!(encoded("ä"), "%C3%A4");
    }

    #[test]
    fn only_a_refusal_a_justification_answers_offers_one() {
        let asks = ViewerError::Cdr {
            status: 403,
            message: "access denied: refused-reason-missing: the access rests on a justification"
                .to_owned(),
        };
        assert!(asks_reason(&asks));
        let uncovered = ViewerError::Cdr {
            status: 403,
            message: "access denied: refused-no-care-relationship: the access policy requires a \
                      care relationship"
                .to_owned(),
        };
        assert!(asks_reason(&uncovered));
        let other = ViewerError::Cdr {
            status: 403,
            message: "access denied: refused-consent-denied".to_owned(),
        };
        assert!(!asks_reason(&other));
    }
}

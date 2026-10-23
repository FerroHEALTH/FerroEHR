// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The small pieces every page draws the same way: a refusal from a viewer
//! error, a relative time, a shortened identifier, a panel section, an
//! editor's heading and its docked decision bar.
//!
//! Each composes FerroUI's parts (`view::refusal`, `view::surface`,
//! `view::dock`) and the viewer's catalog; none draws a component of its own.
//! No openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::button::Variant;
use ferrohealth_ui::view::dock::{Decision, DecisionBar, DecisionState, DockForm, DockLine};
use ferrohealth_ui::view::refusal::{Issue, Refusal, Severity, refusal};
use ferrohealth_ui::view::surface::{SectionHead, panel_section};
use leptos::prelude::*;

use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};

/// The view `answer` draws once it resolves, awaited on the server.
///
/// Every route renders `SsrMode::Async`, so the page reaches the browser
/// whole, with no streamed fragment and no WebAssembly needed.
#[must_use]
pub fn awaited<F>(answer: F) -> AnyView
where
    F: Future<Output = AnyView> + Send + 'static,
{
    view! { <Suspense fallback=|| ()>{Suspend::new(answer)}</Suspense> }.into_any()
}

/// The refusal of `error`, with the sentence `summary` saying what did not
/// happen and the request it answered, when known.
#[must_use]
pub fn refused(summary: String, error: &ViewerError, request: Option<String>) -> AnyView {
    let (status, issues) = match error {
        ViewerError::Cdr { status, message } => (
            Some(*status),
            vec![Issue::new(Severity::Error, status_words(*status)).text(message.clone())],
        ),
        ViewerError::CdrUnauthorized(message) => (
            Some(401),
            vec![Issue::new(Severity::Error, status_words(401)).text(message.clone())],
        ),
        ViewerError::Forbidden(message) => (
            Some(403),
            vec![Issue::new(Severity::Error, status_words(403)).text(message.clone())],
        ),
        ViewerError::CdrUnreachable(message) => (
            None,
            vec![Issue::new(Severity::Error, tr(Msg::CdrUnreachableCode)).text(message.clone())],
        ),
        ViewerError::Invalid(message) => (
            None,
            vec![Issue::new(Severity::Error, tr(Msg::InvalidCode)).text(message.clone())],
        ),
        ViewerError::Unauthenticated => (
            None,
            vec![Issue::new(Severity::Error, tr(Msg::SessionEndedCode))],
        ),
        ViewerError::Internal(message) => (
            None,
            vec![Issue::new(Severity::Error, tr(Msg::InternalCode)).text(message.clone())],
        ),
    };
    refusal(&Refusal {
        status,
        summary,
        issues,
        request,
        evidence: None,
    })
    .into_any()
}

/// A status code with its reason phrase, as an issue names it ("412
/// Precondition Failed").
#[must_use]
pub fn status_words(status: u16) -> String {
    http::StatusCode::from_u16(status).map_or_else(
        |_| status.to_string(),
        |code| match code.canonical_reason() {
            Some(reason) => format!("{status} {reason}"),
            None => status.to_string(),
        },
    )
}

/// Whether `error` is the CDR's answer `status`.
#[must_use]
pub fn is_status(error: &ViewerError, status: http::StatusCode) -> bool {
    error.status_code() == Some(status)
}

/// A state as its tone and its word (family rule 12): FerroUI's pill, whose
/// dot and word say it together, never colour alone.
#[must_use]
pub fn state_mark(state: ferrohealth_ui::view::flow::FlowState, word: &str) -> AnyView {
    ferrohealth_ui::view::status::pill(state.tone(), word).into_any()
}

/// An identifier shortened to its first eight and last four characters, the
/// whole of it in the tooltip; an identifier of twelve characters or fewer
/// stays whole.
#[must_use]
pub fn short(id: &str) -> AnyView {
    let id = id.to_owned();
    let shown = shortened(&id);
    view! { <code title=id class="fui-nowrap">{shown}</code> }.into_any()
}

/// The words [`short`] shows for `id`.
#[must_use]
pub fn shortened(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 12 {
        return id.to_owned();
    }
    let head: String = chars.iter().take(8).collect();
    let tail: String = chars.iter().skip(chars.len().saturating_sub(4)).collect();
    format!("{head}…{tail}")
}

/// A relative time for a list (family rule 11): "13 s ago", "4 min ago",
/// with the full timestamp in its tooltip and the moment in `datetime`.
///
/// It is drawn by the server alone, against the server's clock when the page
/// is drawn; a value that is no instant is drawn as the reading form.
#[must_use]
pub fn ago(raw: &str) -> AnyView {
    let reading = ferrohealth_ui::view::time::reading(raw);
    let words = relative_words(raw, jiff::Timestamp::now()).unwrap_or_else(|| reading.clone());
    let raw = raw.to_owned();
    view! {
        <time class="fui-time" datetime=raw title=reading>
            {words}
        </time>
    }
    .into_any()
}

/// The relative words for the instant `raw` as seen at `now`, or `None` when
/// `raw` is no instant.
#[must_use]
pub fn relative_words(raw: &str, now: jiff::Timestamp) -> Option<String> {
    let then: jiff::Timestamp = raw.trim().parse().ok()?;
    let seconds = now.as_second().saturating_sub(then.as_second());
    if seconds < 0 {
        return Some(tr(Msg::JustNow));
    }
    let seconds = u64::try_from(seconds).ok()?;
    let (msg, count) = match seconds {
        0..60 => (Msg::SecondsAgo, seconds),
        60..3_600 => (Msg::MinutesAgo, seconds.div_euclid(60)),
        3_600..86_400 => (Msg::HoursAgo, seconds.div_euclid(3_600)),
        86_400..2_592_000 => (Msg::DaysAgo, seconds.div_euclid(86_400)),
        _ => return Some(ferrohealth_ui::view::time::reading(raw)),
    };
    Some(tr_with(msg, &[("count", count.into())]))
}

/// An absolute time for a record (family rule 11), as FerroUI draws it.
#[must_use]
pub fn when(raw: &str) -> AnyView {
    ferrohealth_ui::view::time::timestamp(raw).into_any()
}

/// One part of a tab panel (B10) titled `title`, with its count and actions.
#[must_use]
pub fn section(
    title: Option<String>,
    count: Option<u64>,
    actions: Option<AnyView>,
    body: impl IntoView + 'static,
) -> AnyView {
    panel_section(
        title.map(|title| SectionHead {
            title,
            count,
            actions,
        }),
        body,
    )
}

/// A part of a tab panel with no heading.
#[must_use]
pub fn plain_section(body: impl IntoView + 'static) -> AnyView {
    panel_section(None, body)
}

/// The panel that holds a tab's sections, edge to edge (B10).
#[must_use]
pub fn panel(sections: impl IntoView + 'static) -> AnyView {
    view! { <div class="fui-panel">{sections.into_any()}</div> }.into_any()
}

/// The sections of a page with no tabs, in one card (B10).
#[must_use]
pub fn card_panel(sections: impl IntoView + 'static) -> AnyView {
    view! { <div class="fui-card fui-panel">{sections.into_any()}</div> }.into_any()
}

/// One quiet line where an answer or a list will be.
#[must_use]
pub fn quiet(line: impl Into<String>) -> AnyView {
    ferrohealth_ui::view::transform::transform_quiet(line)
}

/// A one-line notice in the info tone, as a page opens over an older version
/// or a reading.
#[must_use]
pub fn info_note(body: impl IntoView + 'static) -> AnyView {
    ferrohealth_ui::view::notice::notice(
        ferrohealth_ui::tone::Tone::Info,
        ferrohealth_ui::view::notice::Announce::Status,
        None,
        body.into_any(),
    )
    .into_any()
}

/// A one-line notice in the warn tone.
#[must_use]
pub fn warn_note(body: impl IntoView + 'static) -> AnyView {
    ferrohealth_ui::view::notice::notice(
        ferrohealth_ui::tone::Tone::Warn,
        ferrohealth_ui::view::notice::Announce::Status,
        None,
        body.into_any(),
    )
    .into_any()
}

/// Today, by the server's clock in UTC, as a date picker counts days.
///
/// # Panics
///
/// Never: every civil date jiff holds is a day the calendar accepts.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "jiff's civil date is always a day of the proleptic Gregorian calendar with a year of 1 to 9999, which calendar::Day::new accepts"
)]
pub fn today() -> ferrohealth_ui::calendar::Day {
    let date = jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
    let day = u16::try_from(date.year())
        .ok()
        .zip(u8::try_from(date.month()).ok())
        .zip(u8::try_from(date.day()).ok())
        .and_then(|((year, month), day)| ferrohealth_ui::calendar::Day::new(year, month, day));
    day.expect("today should be a day of the calendar")
}

/// The first day of the week this reader asked for, Monday when the
/// language decides.
#[must_use]
pub fn first_day() -> ferrohealth_ui::calendar::Weekday {
    crate::prefs::current()
        .week_start
        .first_day(ferrohealth_ui::calendar::Weekday::Monday)
}

/// A history filter's date range (R22): one button naming the range once
/// the page hydrates, its From and To fields (`from`, `to`) before, posted
/// or sent with the form around it.
#[must_use]
pub fn date_range(id: &str, label: String, from: &str, to: &str) -> AnyView {
    use ferrohealth_ui::calendar::{Bounds, Preset};
    use ferrohealth_ui::widget::date::{DateRange, DateWords, RangeEnd, RangePreset};
    let today = today();
    let range = DateRange {
        id: id.to_owned(),
        label,
        from: RangeEnd {
            id: format!("{id}-from"),
            name: "from".to_owned(),
            label: tr(Msg::FilterFrom),
            value: from.to_owned(),
        },
        to: RangeEnd {
            id: format!("{id}-to"),
            name: "to".to_owned(),
            label: tr(Msg::FilterTo),
            value: to.to_owned(),
        },
        today,
        bounds: Bounds {
            earliest: None,
            latest: Some(today),
        },
        first_day: first_day(),
        presets: [
            Preset::Today,
            Preset::LastDays(7),
            Preset::LastDays(30),
            Preset::ThisMonth,
        ]
        .into_iter()
        .map(RangePreset::of_page)
        .collect(),
        words: DateWords::of_page(false),
    };
    view! { <ferrohealth_ui::island::DateRangePicker range /> }.into_any()
}

/// A date field (R23) with its picker (R25), a date and a time when
/// `with_time`: the typed text is the field's value and posts with its form.
#[must_use]
pub fn date_field(id: &str, name: &str, label: String, value: &str, with_time: bool) -> AnyView {
    use ferrohealth_ui::calendar::Bounds;
    use ferrohealth_ui::widget::date::{DateField, DateWords};
    let today = today();
    let field = DateField {
        id: id.to_owned(),
        name: name.to_owned(),
        label,
        value: value.to_owned(),
        help: None,
        error: None,
        today,
        bounds: Bounds {
            earliest: None,
            latest: Some(today),
        },
        first_day: first_day(),
        with_time,
        quick: Vec::new(),
        chosen: None,
        words: DateWords::of_page(with_time),
    };
    view! { <ferrohealth_ui::island::DatePicker field /> }.into_any()
}

/// The At a time row under a version history (E2): a date and time field
/// and Open, a `GET` form to `action` that opens what `opens` says.
#[must_use]
pub fn at_time_row(action: String, value: &str, opens: String) -> ferrohealth_ui::view::versions::AtTime {
    ferrohealth_ui::view::versions::AtTime {
        action,
        field: date_field(
            "at",
            "at",
            ferrohealth_ui::view::versions::at_time_label().text().to_owned(),
            value,
            true,
        ),
        opens,
    }
}

/// A document as the CDR sent it, numbered and never wrapped, as one part of
/// a tab panel (B1 in B10): its title, its Copy, and the lines.
#[must_use]
pub fn document_section(
    title: String,
    language: ferrohealth_ui::highlight::Language,
    text: String,
    copy: &str,
) -> AnyView {
    use ferrohealth_ui::view::code::{Code, CodeHead, code_section};
    let code = Code {
        label: title.clone(),
        language,
        text,
        ids: Some(format!("{copy}-L")),
        current: None,
        diagnostics: Vec::new(),
    };
    code_section(
        CodeHead {
            title,
            tags: Vec::new(),
            digest: None,
            copy: Some(copy.to_owned()),
            actions: None,
        },
        &code,
    )
}

/// The number of the version `version_uid` names: its last part, `3` of
/// `uid::system::3`.
#[must_use]
pub fn version_number(version_uid: &str) -> String {
    version_uid.rsplit("::").next().unwrap_or_default().to_owned()
}

/// The `OBJECT_VERSION_ID` of version `number` of the object whose version
/// `known` names: `known` with its last part replaced.
#[must_use]
pub fn version_uid_at(known: &str, number: &str) -> String {
    match known.rsplit_once("::") {
        Some((object, _)) => format!("{object}::{number}"),
        None => known.to_owned(),
    }
}

/// What an editor's docked bar says and offers.
#[derive(Debug, Clone)]
pub struct EditorDock {
    /// The id of the page's form the decisions submit.
    pub form: String,
    /// The line beside the decisions.
    pub line: String,
    /// The line's glyph.
    pub glyph: Glyph,
    /// The primary decision's words, such as "Save as version 4".
    pub save: String,
    /// Why the primary decision cannot be taken now, if it cannot.
    pub blocked: Option<String>,
}

/// Docks an editor's decision bar at the foot of the page: its line, Discard
/// and the primary decision, both submits of the page's form (R4).
pub fn dock_editor(editor: EditorDock) {
    let EditorDock {
        form,
        line,
        glyph,
        save,
        blocked,
    } = editor;
    let state = blocked.map_or(DecisionState::Ready, DecisionState::Blocked);
    crate::shell::dock(DecisionBar {
        id: "editor-dock".to_owned(),
        label: tr(Msg::DockLabel),
        form: DockForm::Page(form),
        fields: Vec::new(),
        line: Some(DockLine::State {
            glyph: Some(glyph),
            said: line,
        }),
        decisions: vec![
            Decision {
                name: "intent".to_owned(),
                value: "discard".to_owned(),
                label: tr(Msg::Discard),
                variant: Variant::Secondary,
                glyph: Some(Glyph::X),
                shortcut: None,
                state: DecisionState::Ready,
            },
            Decision {
                name: "intent".to_owned(),
                value: "save".to_owned(),
                label: save,
                variant: Variant::Primary,
                glyph: Some(Glyph::SAVE),
                shortcut: None,
                state,
            },
        ],
    });
}

/// Whether the post is the editor's Discard.
#[must_use]
pub fn discarded(posted: Option<&crate::posted::Posted>) -> bool {
    posted.is_some_and(|posted| posted.intent() == "discard")
}

/// A count as a message argument.
#[must_use]
pub fn count(n: usize) -> ferrohealth_ui::i18n::Arg {
    n.into()
}

/// The days of an activity chart: each day's count by its place, and the
/// first and the last day.
#[derive(Debug, Clone)]
pub struct LastDays {
    /// Each day's place from 0 and its count, every day of the window.
    pub counts: Vec<(u32, u32)>,
    /// The window's first day, `YYYY-MM-DD`.
    pub first: String,
    /// The window's last day, today, `YYYY-MM-DD`.
    pub last: String,
}

/// The last `days` days up to today, each with the count `points` holds for
/// it and 0 for a day it holds none.
#[must_use]
pub fn last_days(points: &[crate::activity::ActivityPoint], days: u16) -> LastDays {
    last_days_to(points, days, jiff::Zoned::now().date())
}

/// [`last_days`] up to `today`.
fn last_days_to(points: &[crate::activity::ActivityPoint], days: u16, today: jiff::civil::Date) -> LastDays {
    let span = i64::from(days.max(1)) - 1;
    let first = today
        .checked_sub(jiff::Span::new().days(span))
        .unwrap_or(today);
    let mut counts = Vec::with_capacity(usize::from(days));
    let mut day = first;
    for place in 0..u32::from(days.max(1)) {
        let key = day.to_string();
        let held = points
            .iter()
            .filter(|point| point.day == key)
            .map(|point| point.count)
            .sum();
        counts.push((place, held));
        day = match day.tomorrow() {
            Ok(next) => next,
            Err(_) => break,
        };
    }
    LastDays {
        counts,
        first: first.to_string(),
        last: today.to_string(),
    }
}

/// The CDR's refusal of a document as findings beside its editor (B3).
///
/// One finding for each line of the diagnostic, placed on the line of `text`
/// the path it names reaches; `None` when the refusal is not about the
/// document (anything but a `400` or a `422`).
#[must_use]
pub fn findings_of(
    error: &ViewerError,
    text: &str,
    field: &str,
) -> Option<(String, ferrohealth_ui::view::diagnostics::FileFindings)> {
    use ferrohealth_ui::view::code::Gravity;
    use ferrohealth_ui::view::diagnostics::{FileFindings, Finding};
    let ViewerError::Cdr { status, message } = error else {
        return None;
    };
    let status = http::StatusCode::from_u16(*status).ok()?;
    if status != http::StatusCode::BAD_REQUEST && status != http::StatusCode::UNPROCESSABLE_ENTITY {
        return None;
    }
    let findings: Vec<Finding> = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| Finding {
            gravity: Gravity::Error,
            code: status_words(status.as_u16()),
            at: path_in(line).and_then(|path| locate(text, &path)),
            message: line.to_owned(),
            mapping: None,
        })
        .collect();
    let total = findings.len();
    Some((
        tr_with(Msg::EditorRefusedHeading, &[("count", count(total))]),
        FileFindings {
            file: field.to_owned(),
            href: None,
            lines: format!("#{field}-L"),
            current: None,
            findings,
            all: None,
        },
    ))
}

/// The gutter marks of the lines `findings` place a finding on.
#[must_use]
pub fn marks_of(
    findings: Option<&(String, ferrohealth_ui::view::diagnostics::FileFindings)>,
) -> Vec<ferrohealth_ui::widget::editor::LineMark> {
    use ferrohealth_ui::widget::editor::{LineMark, MarkKind};
    findings.map_or_else(Vec::new, |(_, file)| {
        file.findings
            .iter()
            .filter_map(|finding| finding.at)
            .map(|at| LineMark {
                line: at.line,
                kind: MarkKind::Error,
            })
            .collect()
    })
}

/// The first RM path a diagnostic line names: a run that starts with `/`.
fn path_in(line: &str) -> Option<String> {
    let start = line.find('/')?;
    let path: String = line
        .get(start..)?
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '[' | ']' | '-' | '.'))
        .collect();
    (path.len() > 1).then_some(path)
}

/// The line of the JSON document `text` a path such as `/context/setting`
/// reaches: each key found in turn after the one before it. A position the
/// walk cannot reach is `None`, and the finding is about the whole document.
fn locate(text: &str, path: &str) -> Option<ferrohealth_ui::view::diagnostics::Position> {
    let mut from = 0;
    let mut found = None;
    for step in path.split('/').filter(|step| !step.is_empty()) {
        let key = step.split('[').next().unwrap_or(step);
        if key.is_empty() || key.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let needle = format!("\"{key}\"");
        let at = text.get(from..)?.find(&needle)?.checked_add(from)?;
        from = at.checked_add(needle.len())?;
        found = Some(at);
    }
    let at = found?;
    let before = text.get(..at)?;
    let line = u32::try_from(before.matches('\n').count()).ok()?.checked_add(1)?;
    let column = u32::try_from(before.rsplit('\n').next().map_or(0, |part| part.chars().count()))
        .ok()?
        .checked_add(1)?;
    Some(ferrohealth_ui::view::diagnostics::Position { line, column })
}

#[cfg(test)]
mod tests {
    use super::{findings_of, last_days_to, locate, path_in, relative_words, shortened, status_words};
    use crate::activity::ActivityPoint;
    use crate::error::ViewerError;

    #[test]
    fn an_identifier_is_shortened_to_its_ends() {
        assert_eq!(
            "0199c2a4…1a01",
            shortened("0199c2a4-7e1b-4c3e-9a51-4f2d8b6e1a01")
        );
        assert_eq!("short-id", shortened("short-id"));
    }

    #[test]
    fn a_status_reads_with_its_reason() {
        assert_eq!("412 Precondition Failed", status_words(412));
        assert_eq!("299", status_words(299));
    }

    #[test]
    fn a_time_in_a_list_reads_relative_and_an_unknown_value_is_none() {
        let now: jiff::Timestamp = "2026-10-21T09:42:11Z".parse().expect("an instant");
        assert!(relative_words("2026-10-21T09:41:58Z", now).is_some());
        assert!(relative_words("not a time", now).is_none());
    }

    #[test]
    fn the_activity_window_counts_every_day_and_zero_for_a_quiet_one() {
        let today = jiff::civil::date(2026, 10, 23);
        let points = vec![
            ActivityPoint { day: "2026-10-23".to_owned(), count: 3 },
            ActivityPoint { day: "2026-10-20".to_owned(), count: 1 },
            ActivityPoint { day: "2026-08-01".to_owned(), count: 9 },
        ];
        let days = last_days_to(&points, 30, today);
        assert_eq!(days.counts.len(), 30);
        assert_eq!(days.first, "2026-09-24");
        assert_eq!(days.last, "2026-10-23");
        assert_eq!(days.counts.last(), Some(&(29, 3)));
        assert_eq!(days.counts.get(26), Some(&(26, 1)));
        let total: u32 = days.counts.iter().map(|(_, count)| *count).sum();
        assert_eq!(total, 4, "a day before the window is left out");
    }

    #[test]
    fn a_diagnostic_lands_on_the_line_its_path_reaches() {
        let text = "{\n  \"context\": {\n    \"setting\": {}\n  }\n}";
        assert_eq!(path_in("/context/setting needs its defining_code"), Some("/context/setting".to_owned()));
        assert_eq!(path_in("no path here"), None);
        let at = locate(text, "/context/setting").expect("the path is in the document");
        assert_eq!((at.line, at.column), (3, 5));
        assert!(locate(text, "/territory").is_none());
        let refused = ViewerError::Cdr {
            status: 422,
            message: "/context/setting needs its defining_code\nthe composer is missing".to_owned(),
        };
        let (_, file) = findings_of(&refused, text, "document").expect("a 422 is about the document");
        assert_eq!(file.findings.len(), 2);
        assert_eq!(file.findings.first().and_then(|finding| finding.at.as_ref()).map(|at| at.line), Some(3));
        let elsewhere = ViewerError::Cdr { status: 409, message: "conflict".to_owned() };
        assert!(findings_of(&elsewhere, text, "document").is_none(), "a 409 is not about the document");
    }
}

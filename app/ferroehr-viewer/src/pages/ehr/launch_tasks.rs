// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART launch Tasks of an EHR's patient on the EHR's page, and the
//! launch of one ([`crate::launch_tasks`]).
//!
//! Only an EHR whose subject namespace is one of `[launch]
//! patient_namespaces` has a FHIR `Patient` the facade knows, so only such
//! an EHR lists Tasks. A Task whose app URL is a configured app's launch URL
//! gets a Launch button; any other is listed with no button, and a post
//! naming it is refused. The post names only the Task's id: the Task is read
//! again at the facade, and the browser goes to the configured URL, never
//! one the Task carries. No openEHR spec governs this: our own design.

use ferrohealth_session::resource::ForResource;
use ferrohealth_ui::html::Html;

use crate::bff::ehr_detail::status::EhrStatusState;
use crate::error::ViewerError;
use crate::launch::{Launcher, TaskContext};
use crate::launch_tasks::{LaunchTask, TaskKind};
use crate::locale::server::ServerMsg;
use crate::locale::{tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::{Refused, quiet, section, warn_note};
use crate::pages::ehr::ehr_href;

/// What the Tasks part of an EHR's page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Panel {
    /// The Tasks, each with the configured app that launches it, if any.
    Listed(Vec<(LaunchTask, Option<String>)>),
    /// The session holds no grant a facade token comes from.
    NoToken,
    /// The facade or FerroSMART refused, said in one line.
    Failed(String),
}

/// A launch from a Task that did not happen.
#[derive(Debug, thiserror::Error)]
enum TaskLaunchError {
    /// The session holds no grant a facade token comes from.
    #[error("the session holds no grant a facade token comes from")]
    NoToken,
    /// FerroSMART refused the facade token.
    #[error("FerroSMART gave no token for the FHIR facade")]
    Token,
    /// The Task is off its profile.
    #[error("the Task does not hold to its SMART launch profile")]
    Profile(#[source] crate::launch_tasks::TaskError),
    /// The Task is for another patient than the EHR's.
    #[error("the Task is not for this EHR's patient")]
    OtherPatient,
    /// The Task names an app the viewer does not launch.
    #[error("the Task's app is not a configured app")]
    Unconfigured,
    /// The configured app's launch URL is no URL.
    #[error("the configured app's launch URL is no URL")]
    LaunchUrl,
}

/// The launcher and the FHIR `Patient` id of the EHR whose status is
/// `status`, when the viewer lists Tasks for it.
fn patient_of(status: &EhrStatusState) -> Option<(std::sync::Arc<Launcher>, String)> {
    let launcher = crate::request::state().and_then(|state| state.launch)?;
    let listed = launcher.fhir_base().is_some()
        && !status.subject.is_empty()
        && launcher.names_patients(&status.subject_namespace);
    listed.then(|| (launcher, status.subject.clone()))
}

/// The person's own token for the facade at `base`, from FerroSMART on the
/// session's grant (RFC 8707 §2.2).
async fn token(base: &str) -> Result<String, TaskLaunchError> {
    let state = crate::request::state().ok_or(TaskLaunchError::NoToken)?;
    let party = state.sign_in.party().ok_or(TaskLaunchError::NoToken)?;
    let headers = crate::request::headers();
    match state
        .sign_in
        .sessions()
        .token_for(party, &headers, base)
        .await
    {
        Ok(ForResource::Issued(token)) => Ok(token.expose().to_owned()),
        Ok(ForResource::Ended(_)) => {
            crate::session_client::note_if_over(&ViewerError::Unauthenticated);
            Err(TaskLaunchError::Token)
        }
        Ok(ForResource::Unavailable) => Err(TaskLaunchError::NoToken),
        Ok(_) | Err(_) => Err(TaskLaunchError::Token),
    }
}

/// Reads the Tasks of the EHR whose status is `status`, or `None` when the
/// viewer lists none for it.
pub async fn read(status: &EhrStatusState) -> Option<Panel> {
    let (launcher, patient) = patient_of(status)?;
    let base = launcher.fhir_base()?.to_owned();
    let token = match token(&base).await {
        Ok(token) => token,
        Err(TaskLaunchError::NoToken) => return Some(Panel::NoToken),
        Err(error) => return Some(Panel::Failed(error.to_string())),
    };
    match crate::launch_tasks::search(launcher.http(), &base, &token, &patient).await {
        Ok(found) => Some(Panel::Listed(
            found
                .into_iter()
                .filter_map(|task| match task {
                    Ok(task) => {
                        let app = launcher
                            .app_launched_at(&task.app_url)
                            .map(|app| app.name.clone());
                        Some((task, app))
                    }
                    // NOTE: SMART App Launch 2.2.0, Task profile: a Task off its profile asks
                    // for no SMART launch, so it is logged and not listed.
                    Err(error) => {
                        tracing::warn!(error = %error, "a Task at the facade is off its SMART launch profile");
                        None
                    }
                })
                .collect(),
        )),
        Err(error) => {
            tracing::warn!(error = %crate::error::error_chain(&error), "the launch Tasks could not be read");
            Some(Panel::Failed(error.to_string()))
        }
    }
}

/// The Tasks part of the EHR `ehr_id`'s page.
#[must_use]
pub fn view(ehr_id: &str, panel: Panel) -> Html {
    let title = Some(tr(ServerMsg::LaunchTasks));
    match panel {
        Panel::NoToken => section(title, None, None, quiet(tr(ServerMsg::LaunchTasksNoToken))),
        Panel::Failed(why) => section(
            title,
            None,
            None,
            warn_note(t(tr_with(
                ServerMsg::LaunchTasksFailed,
                &[("why", why.into())],
            ))),
        ),
        Panel::Listed(tasks) if tasks.is_empty() => {
            section(title, Some(0), None, quiet(tr(ServerMsg::LaunchTasksNone)))
        }
        Panel::Listed(tasks) => {
            let count = u64::try_from(tasks.len()).ok();
            let rows = tasks
                .iter()
                .map(|(task, app)| row(ehr_id, task, app.as_deref()))
                .fold(h(r#"<ul class="fui-stack">"#.to_owned()), Html::then)
                .then(h("</ul>".to_owned()));
            section(title, count, None, rows)
        }
    }
}

/// One Task: its description and status, and a Launch button into a new
/// window when the configured app `app` launches it.
fn row(ehr_id: &str, task: &LaunchTask, app: Option<&str>) -> Html {
    let text = if task.description.is_empty() {
        task.app_url.clone()
    } else {
        task.description.clone()
    };
    let action = app.map_or_else(
        || quiet(tr(ServerMsg::LaunchTaskUnconfigured)),
        |name| {
            h(format!(
                r#"<form method="post" action="{}" target="_blank" rel="noopener"><input type="hidden" name="intent" value="launch-task"><input type="hidden" name="task" value="{}"><button type="submit" class="fui-button fui-button-secondary">{}</button></form>"#,
                Html::text(ehr_href(ehr_id)),
                Html::text(&task.id),
                Html::text(tr_with(ServerMsg::LaunchApp, &[("name", name.to_owned().into())])),
            ))
        },
    );
    h("<li>".to_owned())
        .then(t(format!("{text} ({})", task.status)))
        .then(action)
        .then(h("</li>".to_owned()))
}

/// Launches the app of the Task `task_id` for the EHR `ehr_id` whose
/// status is `status`, or returns why it did not.
pub async fn launch(ehr_id: &str, status: &EhrStatusState, task_id: &str) -> Option<Refused> {
    let refused = |error: TaskLaunchError| {
        tracing::warn!(ehr = %ehr_id, error = %error, "a SMART launch Task was not launched");
        Some(Refused::new(
            tr(ServerMsg::LaunchRefused),
            ViewerError::failed(tr(ServerMsg::LaunchRefused), error),
        ))
    };
    let Some((launcher, patient)) = patient_of(status) else {
        return refused(TaskLaunchError::Unconfigured);
    };
    let Some(base) = launcher.fhir_base().map(str::to_owned) else {
        return refused(TaskLaunchError::Unconfigured);
    };
    let token = match token(&base).await {
        Ok(token) => token,
        Err(error) => return refused(error),
    };
    let task = match crate::launch_tasks::read(launcher.http(), &base, &token, task_id).await {
        Ok(Ok(task)) => task,
        Ok(Err(error)) => return refused(TaskLaunchError::Profile(error)),
        Err(error) => {
            return Some(Refused::new(
                tr(ServerMsg::LaunchRefused),
                ViewerError::failed(tr(ServerMsg::LaunchRefused), error),
            ));
        }
    };
    if task.patient.as_deref() != Some(patient.as_str()) {
        return refused(TaskLaunchError::OtherPatient);
    }
    let Some(app) = launcher.app_launched_at(&task.app_url).cloned() else {
        return refused(TaskLaunchError::Unconfigured);
    };
    let away = match task.kind {
        // NOTE: SMART App Launch 2.2.0, Task profile: an EHR launch carries the Task's
        // patient, encounter and appContext to the app through the launch.
        TaskKind::Ehr => {
            let context = TaskContext {
                patient: task.patient.as_deref(),
                encounter: task.encounter.as_deref(),
                app_context: task.app_context.as_deref(),
            };
            match launcher.launch_task(&app, context).await {
                Ok(url) => url,
                Err(error) => {
                    return Some(Refused::new(
                        tr(ServerMsg::LaunchRefused),
                        ViewerError::failed(tr(ServerMsg::LaunchRefused), error),
                    ));
                }
            }
        }
        // NOTE: SMART App Launch 2.2.0, Launch App: Standalone Launch: the app is
        // opened and asks for its own authorization.
        TaskKind::Standalone => match url::Url::parse(&app.launch_url) {
            Ok(url) => url,
            Err(_bad) => return refused(TaskLaunchError::LaunchUrl),
        },
    };
    crate::request::launch_away(away);
    None
}

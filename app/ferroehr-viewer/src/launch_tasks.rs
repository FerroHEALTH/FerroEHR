// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART launch Tasks of the patient in view, read at the FHIR facade
//! under the person's own authorization.
//!
//! SMART App Launch 2.2.0, Task profile for requesting a SMART launch: a
//! `Task` of the `task-ehr-launch` or `task-standalone-launch` profile asks
//! staff to launch an app, named by its URL in the `smartonfhir-application`
//! input, with an optional `smartonfhir-appcontext`, `for` (the patient) and
//! `encounter`. The viewer searches them with the R4 Task search by
//! `patient` and `code` (FHIR R4, Task, Search Parameters), checks each
//! against its profile, and launches only a Task whose URL is a configured
//! app's launch URL ([`crate::launch::Launcher::app_launched_at`]). The
//! facade token is the person's own, from FerroSMART on the session's grant
//! with RFC 8707 `resource` on refresh. Which Tasks the viewer lists and
//! launches is our own design.

use fhir_types::codec::{Object, Path};
use fhir_types::r4::bundle::Bundle;
use fhir_types::r4::codeable_concept::CodeableConcept;
use fhir_types::r4::reference::Reference;
use fhir_types::r4::resource::Resource;
use fhir_types::r4::task::{Task, TaskInputValue};

/// The SMART codes system of the profiles' codes and input types.
pub const SMART_CODES: &str = "http://hl7.org/fhir/smart-app-launch/CodeSystem/smart-codes";

/// The FHIR resource type of a launch Task, its REST path segment.
const TASK: &str = "Task";

/// The FHIR resource type of a search answer.
const BUNDLE: &str = "Bundle";

/// How many pages of one search the viewer follows before it refuses the
/// rest rather than drop it silently.
const MAX_PAGES: usize = 20;

/// The launch a Task asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// `launch-app-ehr`: an EHR launch with the Task's context.
    Ehr,
    /// `launch-app-standalone`: the app is opened and starts on its own.
    Standalone,
}

/// One SMART launch Task, checked against its profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTask {
    /// The Task's id.
    pub id: String,
    /// The launch it asks for.
    pub kind: TaskKind,
    /// Its `status`.
    pub status: String,
    /// Its `description`, or the display of its code.
    pub description: String,
    /// The app's URL, the `smartonfhir-application` input.
    pub app_url: String,
    /// The `smartonfhir-appcontext` input.
    pub app_context: Option<String>,
    /// The FHIR `Patient` id `for` names.
    pub patient: Option<String>,
    /// The FHIR `Encounter` id `encounter` names.
    pub encounter: Option<String>,
}

/// A Task that does not hold to a SMART launch Task profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TaskError {
    /// It has no `id`.
    #[error("the Task has no id")]
    NoId,
    /// Its `code` names neither SMART launch.
    #[error("the Task's code is neither launch-app-ehr nor launch-app-standalone")]
    Code,
    /// It carries no single `smartonfhir-application` input with a
    /// `valueUrl`.
    #[error("the Task carries no single smartonfhir-application input with a valueUrl")]
    Application,
    /// It carries more than one `smartonfhir-appcontext` input, or one
    /// without a `valueString`.
    #[error("the Task's smartonfhir-appcontext input is repeated or carries no valueString")]
    AppContext,
}

/// Returns whether `concept` holds the SMART code `code`, as a profile's
/// `patternCodeableConcept` asks (FHIR R4, `ElementDefinition.pattern[x]`:
/// the instance's codings include the pattern's).
fn holds_code(concept: Option<&CodeableConcept>, code: &str) -> bool {
    concept.is_some_and(|concept| {
        concept.coding.iter().any(|coding| {
            coding.system.as_ref().and_then(|s| s.value.as_deref()) == Some(SMART_CODES)
                && coding.code.as_ref().and_then(|c| c.value.as_deref()) == Some(code)
        })
    })
}

/// The id of the `kind` resource `reference` names, read from its literal
/// reference, relative or absolute (FHIR R4, References, Literal
/// references).
fn referenced(reference: Option<&Reference>, kind: &str) -> Option<String> {
    let literal = reference?.reference.as_ref()?.value.as_deref()?;
    let (_, tail) = literal.rsplit_once(&format!("{kind}/"))?;
    let id = tail.split('/').next().unwrap_or_default();
    (!id.is_empty()).then(|| id.to_owned())
}

/// Reads `task` as a SMART launch Task, held to the differential of its
/// profile.
///
/// The rules: `code` is `launch-app-ehr` or `launch-app-standalone`, one
/// `smartonfhir-application` input with a `valueUrl` (the `launchurl`
/// slice, 1..1), and at most one `smartonfhir-appcontext` with a
/// `valueString` (the `launchcontext` slice, 0..1) (SMART App Launch 2.2.0,
/// `StructureDefinition` `task-ehr-launch`, `task-standalone-launch`). The
/// base R4 Task's `status` and `intent` are held by the model.
///
/// # Errors
///
/// The [`TaskError`] of the first rule it breaks.
pub fn read_task(task: &Task) -> Result<LaunchTask, TaskError> {
    let id = task.id.clone().ok_or(TaskError::NoId)?;
    let code = task.code.as_ref();
    let kind = if holds_code(code, "launch-app-ehr") {
        TaskKind::Ehr
    } else if holds_code(code, "launch-app-standalone") {
        TaskKind::Standalone
    } else {
        return Err(TaskError::Code);
    };
    let typed = |wanted: &str| {
        task.input
            .iter()
            .filter(|input| holds_code(Some(&input.r#type), wanted))
            .map(|input| &input.value)
            .collect::<Vec<_>>()
    };
    let app_url = match typed("smartonfhir-application").as_slice() {
        [TaskInputValue::Url(url)] => url.value.clone().ok_or(TaskError::Application)?,
        _ => return Err(TaskError::Application),
    };
    let app_context = match typed("smartonfhir-appcontext").as_slice() {
        [] => None,
        [TaskInputValue::String(text)] => Some(text.value.clone().ok_or(TaskError::AppContext)?),
        _ => return Err(TaskError::AppContext),
    };
    let description = task
        .description
        .as_ref()
        .and_then(|text| text.value.clone())
        .or_else(|| {
            code.and_then(|code| {
                code.coding
                    .iter()
                    .find_map(|coding| coding.display.as_ref().and_then(|d| d.value.clone()))
            })
        })
        .unwrap_or_default();
    Ok(LaunchTask {
        id,
        kind,
        status: task.status.value.clone().unwrap_or_default(),
        description,
        app_url,
        app_context,
        patient: referenced(task.r#for.as_ref(), "Patient"),
        encounter: referenced(task.encounter.as_ref(), "Encounter"),
    })
}

/// A read of the facade that did not answer the Tasks.
#[derive(Debug, thiserror::Error)]
pub enum FacadeError {
    /// The facade could not be reached.
    #[error("the FHIR facade could not be reached")]
    Unreachable(#[source] reqwest::Error),
    /// It answered another status than `200`.
    #[error("the FHIR facade answered {0}")]
    Status(http::StatusCode),
    /// Its body is no JSON object.
    #[error("the FHIR facade answered a body that is no JSON object")]
    Body(#[source] serde_json::Error),
    /// Its body is no R4 Bundle or Task.
    #[error("the FHIR facade answered a body that is no R4 {0}")]
    Resource(&'static str, #[source] fhir_types::codec::DecodeError),
    /// A `next` link is no URL under the facade's base.
    #[error("the FHIR facade's next link is not under its base")]
    Link,
    /// The search spans more pages than the viewer follows.
    #[error("the Task search spans more than {MAX_PAGES} pages")]
    TooManyPages,
}

/// Reads `url` with the bearer `token` and returns it as the R4 `T`,
/// named `name` in an error.
async fn get<T: fhir_types::codec::Json>(
    http: &reqwest::Client,
    url: url::Url,
    token: &str,
    name: &'static str,
) -> Result<T, FacadeError> {
    let response = http
        .get(url)
        .bearer_auth(token)
        .header(http::header::ACCEPT, "application/fhir+json")
        .send()
        .await
        .map_err(FacadeError::Unreachable)?;
    let status = response.status();
    if status != http::StatusCode::OK {
        return Err(FacadeError::Status(status));
    }
    let body = response.bytes().await.map_err(FacadeError::Unreachable)?;
    let object: Object = serde_json::from_slice(&body).map_err(FacadeError::Body)?;
    T::from_json(&object, &mut Path::root(name)).map_err(|error| FacadeError::Resource(name, error))
}

/// The URL `{base}/{path}`.
fn under(base: &str, path: &str) -> Result<url::Url, FacadeError> {
    url::Url::parse(&format!("{}/{path}", base.trim_end_matches('/')))
        .map_err(|_bad| FacadeError::Link)
}

/// Searches the facade at `base` for the SMART launch Tasks of the FHIR
/// `Patient` `patient`, following the Bundle's `next` links, and returns
/// each entry read against its profile, in the facade's order.
///
/// # Errors
///
/// [`FacadeError`] when a page cannot be read, a `next` link leaves the
/// base, or the search spans more than twenty pages.
pub async fn search(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    patient: &str,
) -> Result<Vec<Result<LaunchTask, TaskError>>, FacadeError> {
    let mut url = under(base, TASK)?;
    // NOTE: FHIR R4, Task, Search Parameters: `patient` and `code`, the code a
    // token list with the system (search.html §3.1.1.4.9).
    url.query_pairs_mut()
        .append_pair("patient", patient)
        .append_pair(
            "code",
            &format!("{SMART_CODES}|launch-app-ehr,{SMART_CODES}|launch-app-standalone"),
        );
    let mut tasks = Vec::new();
    let mut next = Some(url);
    let mut pages = 0_usize;
    let prefix = base.trim_end_matches('/');
    while let Some(page) = next.take() {
        pages = pages.saturating_add(1);
        if pages > MAX_PAGES {
            return Err(FacadeError::TooManyPages);
        }
        let bundle: Bundle = get(http, page, token, BUNDLE).await?;
        for entry in &bundle.entry {
            // NOTE: FHIR R4, Bundle, search.mode: an `include` entry is no match.
            let mode = entry
                .search
                .as_ref()
                .and_then(|search| search.mode.as_ref());
            if mode.and_then(|mode| mode.value.as_deref()) == Some("include") {
                continue;
            }
            if let Some(Resource::Task(task)) = &entry.resource {
                tasks.push(read_task(task));
            }
        }
        next = bundle
            .link
            .iter()
            .find(|link| link.relation.value.as_deref() == Some("next"))
            .and_then(|link| link.url.value.as_deref())
            .map(|link| {
                url::Url::parse(link)
                    .ok()
                    .filter(|link| link.as_str().starts_with(prefix))
                    .ok_or(FacadeError::Link)
            })
            .transpose()?;
    }
    Ok(tasks)
}

/// Reads the Task `id` at the facade at `base` against its profile.
///
/// # Errors
///
/// [`FacadeError`] when the facade does not answer it.
pub async fn read(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    id: &str,
) -> Result<Result<LaunchTask, TaskError>, FacadeError> {
    let mut url = under(base, TASK)?;
    url.path_segments_mut()
        .map_err(|()| FacadeError::Link)?
        .push(id);
    let task: Task = get(http, url, token, TASK).await?;
    Ok(read_task(&task))
}

#[cfg(test)]
mod tests {
    use fhir_types::codec::{Json as _, Object, Path};
    use fhir_types::r4::task::Task;

    use super::{LaunchTask, TaskError, TaskKind, read_task};

    /// The IG's example `name`, as `scripts/vendor/smart-app-launch.sh`
    /// vendors it, read as an R4 Task.
    fn example(name: &str) -> Task {
        let path = format!(
            "{}/../../docs/specs/smart-app-launch/package/example/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).expect("the example reads");
        let object: Object = serde_json::from_str(&text).expect("the example is JSON");
        Task::from_json(&object, &mut Path::root("Task")).expect("the example is an R4 Task")
    }

    #[test]
    fn the_igs_ehr_launch_example_reads_with_its_context() {
        let task = read_task(&example("Task-task-for-ehr-launch.json"));
        assert_eq!(
            task,
            Ok(LaunchTask {
                id: "task-for-ehr-launch".to_owned(),
                kind: TaskKind::Ehr,
                status: "requested".to_owned(),
                description: "Launch application using the SMART EHR launch".to_owned(),
                app_url: "https://www.example.org/myapplication".to_owned(),
                app_context: Some(r#"{"field1":"value"}"#.to_owned()),
                patient: Some("123".to_owned()),
                encounter: Some("456".to_owned()),
            })
        );
    }

    #[test]
    fn the_igs_standalone_launch_example_reads() {
        let task = read_task(&example("Task-task-for-standalone-launch.json"));
        assert!(
            matches!(&task, Ok(task) if task.kind == TaskKind::Standalone && task.app_context.is_none()),
            "{task:?}"
        );
    }

    #[test]
    fn a_task_off_its_profile_is_refused() {
        let mut no_url = example("Task-task-for-ehr-launch.json");
        no_url.input.remove(0);
        assert_eq!(read_task(&no_url), Err(TaskError::Application));
        let mut other_code = example("Task-task-for-ehr-launch.json");
        if let Some(coding) = other_code
            .code
            .as_mut()
            .and_then(|code| code.coding.first_mut())
        {
            coding.code = Some(String::from("something-else").into());
        }
        assert_eq!(read_task(&other_code), Err(TaskError::Code));
        let mut twice = example("Task-task-for-ehr-launch.json");
        let context = twice.input.get(1).cloned().expect("the context input");
        twice.input.push(context);
        assert_eq!(read_task(&twice), Err(TaskError::AppContext));
    }
}

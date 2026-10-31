// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The seam to the `openehr-adl` engine: the stack it runs on, and the
//! refusals of an operational template it cannot compile.

use openehr_adl::opt::OptError;
use openehr_adl::validate::catalogue::ValidationCode;
use openehr_base::validate::InvariantViolation;

use crate::service::error::{ErrorChain, ServiceError, Violation};

/// The refusal of an operational template that cannot be compiled.
pub(super) fn not_compiled(e: OptError) -> ServiceError {
    ServiceError::content_invalid(
        Violation::new(format!(
            "cannot compile operational template: {}",
            ErrorChain::new(&e)
        ))
        .with_source(e),
    )
}

/// The refusal of a stored template whose constituent archetypes `missing`
/// are not stored: one `VARXR` violation per archetype id, the id alone as
/// its message, so a client can name and offer each one.
///
/// AOM2 VARXR (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
/// §Validity Rules: `C_ARCHETYPE_ROOT`) requires every external reference to
/// resolve in the repository, and the `422` is the ITS-REST semantic refusal
/// (`ITS-REST/specifications/responses/422.yaml`). The message carrying the
/// bare id is no openEHR spec's: our own design.
pub(super) fn missing_constituents(missing: &[String]) -> ServiceError {
    ServiceError::ValidationFailed(
        missing
            .iter()
            .map(|id| InvariantViolation::at(ValidationCode::Varxr.mnemonic(), id.clone()))
            .collect(),
    )
}

/// The stack the AOM engine runs on.
///
/// Parsing, flattening and validating an archetype recurse over the artefact
/// and over the stored repository (slot fillers, specialisation parents), and
/// a well-stocked store crossed a tokio worker's 2 MiB stack (#3062). The
/// reservation is virtual: pages are committed only as the stack grows.
const ENGINE_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Run `f` on a dedicated thread with [`ENGINE_STACK_BYTES`] of stack and await
/// its result without blocking the runtime.
///
/// # Errors
/// [`ServiceError::Internal`] when the thread cannot be spawned or ends without
/// delivering a result (a panic inside `f` surfaces as that, never as an abort
/// of the process).
pub(super) async fn on_engine_stack<T, F>(f: F) -> Result<T, ServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("adl2-engine".to_owned())
        .stack_size(ENGINE_STACK_BYTES)
        .spawn(move || {
            // The receiver is gone only when the request was cancelled, in
            // which case the result has no reader left.
            drop(tx.send(f()));
        })
        .map_err(|e| ServiceError::internal("spawning the ADL2 engine thread", e))?;
    rx.await
        .map_err(|e| ServiceError::internal("the ADL2 engine thread ended without a result", e))
}

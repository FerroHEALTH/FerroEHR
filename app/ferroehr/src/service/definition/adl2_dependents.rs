// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stored ADL2 artefacts that depend on another one, which the delete
//! refuses to leave without it.
//!
//! An artefact depends on another when it compiles to its operational
//! template with the other stored and does not compile without it: a
//! template or archetype that fills a slot with it or names it through
//! `use_archetype`, a specialisation child, and every artefact built on one of
//! those. AOM2 VARXR (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules: `C_ARCHETYPE_ROOT`) requires each archetype reference to
//! "refer to an artefact that can be found in the current repository", and
//! OPT2 (`AM/docs/OPT2/master03-opt_raw.adoc` §Archetype References) resolves
//! every one of them.

use openehr_adl::opt::create_opt;
use sqlx::PgConnection;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;

use super::adl2_engine::on_engine_stack;
use super::adl2_resolution::{Candidate, repository_for};

impl FerroEhrService {
    /// Refuse the delete of the stored ADL2 artefact `hrid` while another
    /// stored artefact depends on it, naming every one of them.
    ///
    /// NOTE: ITS-REST `specifications/docs/overview/Requests_and_responses.md`
    /// §HTTP status codes, `409` "might generate … a conflict"; no openEHR spec
    /// governs this delete (the Definition API declares none): our own design.
    ///
    /// # Errors
    ///
    /// - Another stored artefact depends on `hrid` → `Conflict` (`409`).
    /// - A database failure, or the engine thread failing → `500`.
    pub(super) async fn adl2_refuse_in_use(
        &self,
        conn: &mut PgConnection,
        hrid: &str,
    ) -> Result<(), ServiceError> {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT hrid, kind, adl FROM adl2_artefact ORDER BY hrid")
                .fetch_all(conn)
                .await?;
        let target = hrid.to_owned();
        let dependents = on_engine_stack(move || dependents_of(&target, rows)).await?;
        if dependents.is_empty() {
            return Ok(());
        }
        Err(ServiceError::conflict(format!(
            "ADL2 artefact '{hrid}' is still needed by {} stored artefact(s): {}; \
             delete those before deleting it",
            dependents.len(),
            dependents.join(", ")
        )))
    }
}

/// The HRIDs, in the order of `rows`, of every stored artefact that compiles
/// with `target` stored and does not compile without it.
///
/// `rows` is every stored `(hrid, kind, adl)`. An artefact that does not
/// compile even with `target` stored is not counted: the delete takes nothing
/// from it. Each artefact compiles against the releases its own references
/// resolve to ([`repository_for`]), so with another release of `target`'s
/// major version stored, the delete leaves its users resolving to that one.
fn dependents_of(target: &str, rows: Vec<(String, String, String)>) -> Vec<String> {
    let is_target = |hrid: &str| hrid.eq_ignore_ascii_case(target);
    let mut users = Vec::new();
    let mut with = Vec::new();
    let mut without = Vec::new();
    for (hrid, kind, adl) in rows {
        let Some(stored) = Candidate::parse(&hrid, &adl) else {
            continue;
        };
        // A stored `operational_template` inlines every constituent and so
        // needs none of them stored (OPT2 `master02-overview.adoc`: a
        // top-level standalone artefact).
        if kind != "operational_template" && !is_target(&hrid) {
            users.push((hrid.clone(), stored.artefact().clone()));
        }
        if !is_target(&hrid) {
            without.push(stored.clone());
        }
        with.push(stored);
    }
    users
        .into_iter()
        .filter(|(_, a)| {
            create_opt(a, &repository_for(a, &without)).is_err()
                && create_opt(a, &repository_for(a, &with)).is_ok()
        })
        .map(|(hrid, _)| hrid)
        .collect()
}

/// Take the transaction-scoped advisory lock every ADL2 upload and delete
/// holds from its repository read to its commit.
///
/// The delete refuses while a stored artefact depends on its target, and an
/// upload is refused while a constituent it needs is not stored; each decides
/// on a read the other's write changes, so they run one after the other, the
/// second deciding on what the first committed. PostgreSQL releases the lock at
/// the end of the transaction (`pg_advisory_xact_lock`, PostgreSQL
/// documentation §13.3.5 Advisory Locks). No openEHR spec governs
/// storage integrity: our own design.
///
/// # Errors
///
/// A database failure → `500`.
pub(super) async fn adl2_write_lock(tx: &mut PgConnection) -> Result<(), ServiceError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('ferroehr.adl2_artefact', 0))")
        .execute(tx)
        .await?;
    Ok(())
}

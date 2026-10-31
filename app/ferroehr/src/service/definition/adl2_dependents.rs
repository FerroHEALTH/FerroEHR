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

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use sqlx::PgConnection;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;

use super::adl2_engine::on_engine_stack;

/// One stored ADL2 artefact as the store row holds it.
struct Stored {
    /// The case-preserved `ARCHETYPE_HRID`.
    hrid: String,
    /// The parsed artefact, absent when the stored source no longer parses.
    artefact: Option<Archetype>,
    /// True for a stored `operational_template`, which inlines every
    /// constituent and so needs none of them stored (OPT2 `master02-overview.adoc`:
    /// a top-level standalone artefact).
    is_opt: bool,
}

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
/// from it.
fn dependents_of(target: &str, rows: Vec<(String, String, String)>) -> Vec<String> {
    let stored: Vec<Stored> = rows
        .into_iter()
        .map(|(hrid, kind, adl)| Stored {
            hrid,
            // NOTE: a stored source that no longer parses is skipped, as the
            // engine repository skips it; it was engine-valid at its upload.
            artefact: parse_artefact(&adl, Dialect::Adl2).ok(),
            is_opt: kind == "operational_template",
        })
        .collect();
    let is_target = |s: &Stored| s.hrid.eq_ignore_ascii_case(target);
    let mut with = ArchetypeRepository::new();
    let mut without = ArchetypeRepository::new();
    for s in &stored {
        if let Some(artefact) = &s.artefact {
            with.insert(artefact.clone());
            if !is_target(s) {
                without.insert(artefact.clone());
            }
        }
    }
    stored
        .iter()
        .filter(|s| !s.is_opt && !is_target(s))
        .filter_map(|s| s.artefact.as_ref().map(|a| (s, a)))
        .filter(|(_, a)| create_opt(a, &without).is_err() && create_opt(a, &with).is_ok())
        .map(|(s, _)| s.hrid.clone())
        .collect()
}

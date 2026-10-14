// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The subjects of EHRs as the ABAC patient scope sees them.
//!
//! The query scope applies the subject-identity rule inside its SQL
//! ([`crate::aql::sql::subject`]); this module reads the same rule back per EHR
//! for the two callers that decide outside a query: the direct-route patient
//! gate and the query post-check's patient attribute. No openEHR spec governs
//! the patient scope — our own extension.

use std::collections::BTreeMap;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::error::{AqlError, ExecError};
use super::sql::subject::{SHARED_COLUMN, lookup};
use crate::db::acquire::acquire;
use crate::ids::EhrId;

/// The subject of one EHR (RM ehr master04 §EHR Status, `EHR_STATUS.subject`),
/// as a patient claim can address it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EhrSubject {
    /// The EHR records no subject, so no patient claim names it.
    Absent,
    /// The subject id is held under this one namespace, so the id names it.
    Unique {
        /// `subject.external_ref.id.value`.
        id: String,
        /// `subject.external_ref.namespace`.
        namespace: Option<String>,
    },
    /// Another namespace holds the same subject id, so the id names no subject.
    Shared {
        /// `subject.external_ref.id.value`.
        id: String,
        /// `subject.external_ref.namespace`.
        namespace: Option<String>,
    },
}

impl EhrSubject {
    /// Whether the patient claim `claim` names this subject: only a
    /// [`Self::Unique`] subject whose id equals it.
    #[must_use]
    pub fn is_named_by(&self, claim: &str) -> bool {
        matches!(self, Self::Unique { id, .. } if id == claim)
    }
}

/// Reads the subject of every existing EHR among `ids`; an id no EHR has is
/// absent from the map.
///
/// # Errors
/// [`AqlError::Exec`] when the database fails the lookup or a row does not
/// decode.
pub async fn ehr_subjects(
    pool: &PgPool,
    ids: &[EhrId],
) -> Result<BTreeMap<EhrId, EhrSubject>, AqlError> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let uuids: Vec<Uuid> = ids.iter().map(|id| id.0).collect();
    let (sql, values) = lookup(&uuids);
    let rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
        .persistent(false)
        .fetch_all(&mut *acquire(pool).await.map_err(ExecError::from)?)
        .await
        .map_err(ExecError::from)?;
    let mut out = BTreeMap::new();
    for row in &rows {
        let ehr: Uuid = row.try_get(0).map_err(ExecError::from)?;
        let id: Option<String> = row.try_get(1).map_err(ExecError::from)?;
        let namespace: Option<String> = row.try_get(2).map_err(ExecError::from)?;
        let shared: bool = row.try_get(SHARED_COLUMN).map_err(ExecError::from)?;
        let subject = match (id, shared) {
            (None, _) => EhrSubject::Absent,
            (Some(id), false) => EhrSubject::Unique { id, namespace },
            (Some(id), true) => EhrSubject::Shared { id, namespace },
        };
        out.insert(EhrId(ehr), subject);
    }
    Ok(out)
}

/// Reads the subject of the EHR `id`, or `None` when no EHR has that id.
///
/// # Errors
/// As [`ehr_subjects`].
pub async fn ehr_subject(pool: &PgPool, id: EhrId) -> Result<Option<EhrSubject>, AqlError> {
    Ok(ehr_subjects(pool, &[id]).await?.remove(&id))
}

/// The patient attribute of a set of touched EHRs, for the query post-check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TouchedSubjects {
    /// The distinct subject ids of the touched EHRs a claim can name.
    pub ids: Vec<String>,
    /// Whether a touched EHR records no subject.
    pub absent: bool,
    /// The distinct subject ids of touched EHRs another namespace also holds.
    pub shared: Vec<String>,
}

impl TouchedSubjects {
    /// Folds the subjects `subjects` of the touched EHRs into the attribute.
    #[must_use]
    pub fn from_subjects<'a>(subjects: impl IntoIterator<Item = &'a EhrSubject>) -> Self {
        let mut ids = std::collections::BTreeSet::new();
        let mut shared = std::collections::BTreeSet::new();
        let mut absent = false;
        for subject in subjects {
            match subject {
                EhrSubject::Absent => absent = true,
                EhrSubject::Unique { id, .. } => {
                    ids.insert(id.clone());
                }
                EhrSubject::Shared { id, .. } => {
                    shared.insert(id.clone());
                }
            }
        }
        Self {
            ids: ids.into_iter().collect(),
            absent,
            shared: shared.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_unique_subject_is_named_by_its_id() {
        let unique = EhrSubject::Unique {
            id: "p-1".to_owned(),
            namespace: Some("a".to_owned()),
        };
        let shared = EhrSubject::Shared {
            id: "p-1".to_owned(),
            namespace: Some("a".to_owned()),
        };
        assert!(unique.is_named_by("p-1"));
        assert!(!unique.is_named_by("p-2"));
        assert!(!shared.is_named_by("p-1"));
        assert!(!EhrSubject::Absent.is_named_by("p-1"));
    }

    #[test]
    fn touched_subjects_fold_by_kind() {
        let subjects = [
            EhrSubject::Unique {
                id: "b".to_owned(),
                namespace: None,
            },
            EhrSubject::Unique {
                id: "a".to_owned(),
                namespace: None,
            },
            EhrSubject::Unique {
                id: "a".to_owned(),
                namespace: None,
            },
            EhrSubject::Shared {
                id: "x".to_owned(),
                namespace: None,
            },
            EhrSubject::Absent,
        ];
        let touched = TouchedSubjects::from_subjects(&subjects);
        assert_eq!(touched.ids, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(touched.shared, vec!["x".to_owned()]);
        assert!(touched.absent);
    }
}

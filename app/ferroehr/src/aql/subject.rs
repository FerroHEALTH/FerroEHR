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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EhrSubject {
    /// The EHR records no subject, so no patient claim names it.
    Absent,
    /// No other namespace holds this subject id.
    Unique {
        /// `subject.external_ref.id.value`.
        id: String,
        /// `subject.external_ref.namespace`.
        namespace: Option<String>,
    },
    /// Another namespace holds the same subject id.
    Shared {
        /// `subject.external_ref.id.value`.
        id: String,
        /// `subject.external_ref.namespace`.
        namespace: Option<String>,
    },
}

impl EhrSubject {
    /// Whether the patient claim `claim`, read in the configured patient
    /// `namespace`, names this subject.
    ///
    /// With a namespace the subject's (id, namespace) pair must equal
    /// (`claim`, `namespace`); without one only a [`Self::Unique`] subject whose
    /// id equals `claim` is named, because a shared id names no subject.
    #[must_use]
    pub fn is_named_by(&self, claim: &str, namespace: Option<&str>) -> bool {
        match (self, namespace) {
            (
                Self::Unique { id, namespace: own } | Self::Shared { id, namespace: own },
                Some(configured),
            ) => id == claim && own.as_deref() == Some(configured),
            (Self::Unique { id, .. }, None) => id == claim,
            (Self::Absent, _) | (Self::Shared { .. }, None) => false,
        }
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
    /// Whether a touched EHR has a subject no claim can name: none recorded,
    /// or, with a configured namespace, one under another namespace.
    pub unnamed: bool,
    /// The distinct subject ids, held under several namespaces, that no claim
    /// can name because no namespace is configured.
    pub shared: Vec<String>,
}

impl TouchedSubjects {
    /// Folds the subjects of the touched EHRs into the attribute, reading
    /// subject ids in the configured patient `namespace`, as
    /// [`EhrSubject::is_named_by`] does.
    #[must_use]
    pub fn fold<'a>(
        subjects: impl IntoIterator<Item = &'a EhrSubject>,
        namespace: Option<&str>,
    ) -> Self {
        let mut ids = std::collections::BTreeSet::new();
        let mut shared = std::collections::BTreeSet::new();
        let mut unnamed = false;
        for subject in subjects {
            match (subject, namespace) {
                (EhrSubject::Absent, _) => unnamed = true,
                (
                    EhrSubject::Unique { id, namespace: own }
                    | EhrSubject::Shared { id, namespace: own },
                    Some(configured),
                ) => {
                    if own.as_deref() == Some(configured) {
                        ids.insert(id.clone());
                    } else {
                        unnamed = true;
                    }
                }
                (EhrSubject::Unique { id, .. }, None) => {
                    ids.insert(id.clone());
                }
                (EhrSubject::Shared { id, .. }, None) => {
                    shared.insert(id.clone());
                }
            }
        }
        Self {
            ids: ids.into_iter().collect(),
            unnamed,
            shared: shared.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(id: &str, namespace: &str) -> EhrSubject {
        EhrSubject::Unique {
            id: id.to_owned(),
            namespace: Some(namespace.to_owned()),
        }
    }

    fn shared(id: &str, namespace: &str) -> EhrSubject {
        EhrSubject::Shared {
            id: id.to_owned(),
            namespace: Some(namespace.to_owned()),
        }
    }

    #[test]
    fn without_a_namespace_only_a_unique_subject_is_named_by_its_id() {
        assert!(unique("p-1", "a").is_named_by("p-1", None));
        assert!(!unique("p-1", "a").is_named_by("p-2", None));
        assert!(!shared("p-1", "a").is_named_by("p-1", None));
        assert!(!EhrSubject::Absent.is_named_by("p-1", None));
    }

    #[test]
    fn with_a_namespace_the_pair_must_match() {
        assert!(unique("p-1", "a").is_named_by("p-1", Some("a")));
        assert!(!unique("p-1", "a").is_named_by("p-1", Some("b")));
        assert!(shared("p-1", "a").is_named_by("p-1", Some("a")));
        assert!(!shared("p-1", "b").is_named_by("p-1", Some("a")));
        assert!(!EhrSubject::Absent.is_named_by("p-1", Some("a")));
    }

    #[test]
    fn touched_subjects_fold_without_a_namespace() {
        let subjects = [
            unique("b", "a"),
            unique("a", "a"),
            shared("x", "a"),
            shared("x", "b"),
            EhrSubject::Absent,
        ];
        let touched = TouchedSubjects::fold(&subjects, None);
        assert_eq!(touched.ids, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(touched.shared, vec!["x".to_owned()]);
        assert!(touched.unnamed);
    }

    #[test]
    fn touched_subjects_fold_with_a_namespace() {
        let subjects = [unique("b", "a"), shared("x", "a"), shared("x", "other")];
        let touched = TouchedSubjects::fold(&subjects, Some("a"));
        assert_eq!(touched.ids, vec!["b".to_owned(), "x".to_owned()]);
        assert!(touched.shared.is_empty());
        assert!(touched.unnamed, "the other namespace's subject is unnamed");
    }
}

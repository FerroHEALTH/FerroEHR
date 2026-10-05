// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Storage-layer error surface.
//!
//! No openEHR spec governs the physical storage — these are our own
//! codec/repository errors. Semantic outcomes (version conflict, not-found,
//! validation) belong to the versioning/service layer and are NOT modelled
//! here; storage reports only what the codec and the SQL row I/O can observe.

use uuid::Uuid;

/// Errors produced by the node-storage codec and the row repositories.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The root value handed to [`crate::storage::codec::decompose`] is not a
    /// decomposable versioned-object tree (its `_type` is not a versioned root).
    #[error("root object has no structure _type (found {0:?})")]
    NotAStructureRoot(Option<String>),

    /// A canonical-JSON array mixed structure and non-structure elements, which
    /// canonical RM JSON never does.
    #[error("array {attribute:?} mixes structure and non-structure elements")]
    MixedArray {
        /// The RM attribute name whose array held the mixed elements.
        attribute: String,
    },

    /// Reassembly received rows that do not form one tree rooted at `num = 0`.
    #[error("invalid node rows: {0}")]
    InvalidRows(String),

    /// A client-supplied CONTRIBUTION uid is already in use (the insert hit
    /// `ON CONFLICT (id) DO NOTHING`). ITS-REST `contribution_create`: "if the
    /// `uid` is supplied it must not already be in use" — a duplicate is a
    /// conflict, never an overwrite. Mapped to 409 by the versioning layer.
    #[error("CONTRIBUTION uid {0:?} is already in use")]
    ContributionUidInUse(Option<Uuid>),

    /// The EHR row insert violated the one-EHR-per-subject unique index
    /// (`uq_ehr_subject`) — another EHR already names this subject
    /// `(id, namespace)` (RM ehr master04 §EHR Status: the subject 0..1
    /// identifies the EHR). Carries `(subject_id, namespace)` so the service
    /// layer can build the ITS-REST `409_EHR` body. Distinct from a duplicate
    /// EHR-id conflict (that surfaces as an absent `RETURNING` row).
    #[error("an EHR already exists for subject {0}@{1}")]
    SubjectInUse(String, String),

    /// An archive record would take a TRUNK position of a version container
    /// that another creating system already holds. RM common master06
    /// §Distributed Versioning identifies a version globally by
    /// `{object_id, creating_system_id, version_tree_id}`, but the trunk line
    /// itself is one global sequence: §Copying §Subsequent Local Modifications
    /// makes a second system BRANCH rather than extend the trunk, and §Moving
    /// Version Containers continues the trunk increment under the new system's
    /// id. Two trunk rows at one position are therefore not a schema detail but
    /// a broken version tree — reported as a typed conflict rather than as the
    /// bare unique-index violation the load would otherwise surface.
    #[error(
        "versioned object {vo_id} already holds trunk version {trunk_version} \
         (created by {held_by:?}); a trunk position is unique across creating systems"
    )]
    TrunkPositionInUse {
        /// The version container the archive record targets.
        vo_id: Uuid,
        /// The trunk position both rows claim.
        trunk_version: i32,
        /// The `creating_system_id` of the row already holding the position.
        held_by: String,
    },

    /// A version container holds branch versions but no TRUNK version, so it
    /// has no current version to record. RM common
    /// `master06-change_control_package.adoc` §Copying §Subsequent Local
    /// Modifications rules the state out: branch versions are never copied
    /// without their trunk versions.
    #[error(
        "versioned object {vo_id} holds branch versions but no trunk version, so it has no \
         current version to record ({count} such container(s) in this batch)"
    )]
    TrunklessContainer {
        /// The first container found in this state.
        vo_id: Uuid,
        /// How many containers of the batch are in it.
        count: usize,
    },

    /// A driver/pool/query error from `sqlx`.
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),

    /// A stored `version.body` text failed to parse as JSON — storage
    /// corruption, never a caller error (the column holds the canonical bytes
    /// the commit serialized).
    #[error("stored canonical body does not parse: {0}")]
    BodyDecode(#[source] serde_json::Error),
}

impl From<StorageError> for crate::service::status::SmError {
    /// Bridge a storage failure to the SM call-status model. Constraint/
    /// concurrency detail is preserved via `classify_sqlx` for the raw
    /// `sqlx` case; the typed conflicts keep their specific `409` meaning; codec
    /// faults are genuine server faults (`exception` → `500`). No openEHR spec
    /// governs the persistence mechanism; the
    /// HTTP mapping the resulting status drives is the ITS-REST-governed one
    /// (overview §HTTP status codes).
    fn from(e: StorageError) -> Self {
        use crate::service::status::{CallStatusType, SmError};
        match e {
            // A raw driver/pool/query error carries SQLSTATE + constraint detail
            // — classify it instead of collapsing to a blanket 500.
            StorageError::Database(db) => classify_sqlx(&db).with_source(db),
            // Two conflicts with content this repository already holds →
            // `409`: a client-supplied CONTRIBUTION uid already in use
            // (normally intercepted at the versioning call site; this is the
            // correct fallback should it propagate), and an archive record
            // claiming an occupied trunk position of a version container.
            StorageError::ContributionUidInUse(_) | StorageError::TrunkPositionInUse { .. } => {
                SmError::new(CallStatusType::Conflict, e.to_string())
            }
            // The one-EHR-per-subject index → `409` (normally intercepted in
            // `service::ehr` to build the ITS-REST `409_EHR` body).
            StorageError::SubjectInUse(_, _) => {
                SmError::new(CallStatusType::EhrForSubjectAlreadyExists, e.to_string())
            }
            // Codec faults (a non-structure root, a mixed array, malformed node
            // rows) are our own bugs, not client errors → `500`. Their text
            // names RM attributes and the internal node-row shape, so the wire
            // body carries the curated message and the detail is traced.
            StorageError::NotAStructureRoot(_)
            | StorageError::MixedArray { .. }
            | StorageError::InvalidRows(_)
            | StorageError::TrunklessContainer { .. }
            | StorageError::BodyDecode(_) => {
                tracing::error!(
                    error = %e,
                    "storage bridge: node-codec invariant violated → 500"
                );
                SmError::new(CallStatusType::Exception, CODEC_MESSAGE.to_owned())
            }
        }
    }
}

/// The client-visible message of a database-integrity conflict. Deliberately
/// opaque: a driver string names the constraint, table and column that failed,
/// which is server-internal schema detail with no client action attached to it.
/// The full detail goes to `tracing` instead (one structured record per
/// classification).
const CONFLICT_MESSAGE: &str = "the request conflicts with data already stored by this server";

/// The client-visible message of a serialization/deadlock failure — the one
/// database conflict a client can genuinely act on, by retrying.
const RETRYABLE_CONFLICT_MESSAGE: &str =
    "a concurrent transaction conflicted with this request; retry it";

/// The client-visible message of an internal database failure (a violated
/// server-side invariant, or an unclassified driver error). Opaque by design:
/// the client cannot act on it, and the diagnosis belongs in the server's own
/// logs.
const INTERNAL_MESSAGE: &str = "the server encountered an internal database error";

/// The client-visible message of a node-codec fault — a versioned-object tree
/// the decomposer could not take apart, or stored node rows that do not
/// reassemble into one tree. Opaque for the same reason the driver string is:
/// the error text names RM attribute names and the internal nested-set row
/// shape, which is server-internal detail with no client action attached. The
/// full detail goes to `tracing` at the bridge below.
const CODEC_MESSAGE: &str = "the server encountered an internal storage-codec error";

/// Classify a raw [`sqlx::Error`] at the storage boundary into the SM call
/// status it should surface, emitting **one** structured trace record so the
/// SQLSTATE code + constraint name are never lost. No openEHR spec governs the
/// persistence mechanism or server-overload behaviour — this is our own design;
/// the resulting HTTP status codes are the ITS-REST-governed ones (overview
/// §HTTP status codes: `409 Conflict`; RFC 9110 §15.6.4: `503 Service
/// Unavailable`).
///
/// No driver string ever reaches a client body: every branch returns one of the
/// three curated constants above, and the SQLSTATE, constraint, table and
/// driver text are recorded on the trace record only, a PostgreSQL message
/// naming schema objects the client can neither use nor be trusted with.
///
/// The SQLSTATE split follows the PostgreSQL error-code table
/// (<https://www.postgresql.org/docs/18/errcodes-appendix.html>) and answers
/// whose invariant broke:
///
/// - `23505` `unique_violation`, `23503` `foreign_key_violation`, `23001`
///   `restrict_violation`, `23P01` `exclusion_violation` →
///   `CallStatusType::Conflict` (`409`): collisions with data the repository
///   already holds, which is what `409` means (ITS-REST overview §HTTP status
///   codes).
/// - `23514` `check_violation`, `23502` `not_null_violation`, the generic
///   `23000` and any other class-23 code → `CallStatusType::Exception` (`500`):
///   a CHECK or NOT NULL reaching the driver is a violated server-side
///   invariant, either a value the service layer failed to refuse or a drift
///   between schema and code, and presenting it as `409` would invite an endless
///   retry against an optimistic-lock failure that never happened.
/// - `40001` `serialization_failure` and `40P01` `deadlock_detected` →
///   `CallStatusType::Conflict` (`409`, retryable).
/// - [`sqlx::Error::PoolTimedOut`] → `CallStatusType::ServiceOverloaded` (`503`
///   plus `Retry-After`, the admission contract).
/// - anything else → `CallStatusType::Exception` (`500`, a genuine fault).
pub(crate) fn classify_sqlx(e: &sqlx::Error) -> crate::service::status::SmError {
    use crate::service::status::{CallStatusType, SmError};
    match e {
        sqlx::Error::PoolTimedOut => {
            tracing::warn!(
                error = %e,
                "storage bridge: connection-pool acquire timed out — shedding as service-overloaded (503)"
            );
            SmError::new(
                CallStatusType::ServiceOverloaded,
                "the server is temporarily overloaded; retry shortly".to_owned(),
            )
        }
        sqlx::Error::Database(db) => {
            let sqlstate = db.code();
            let sqlstate = sqlstate.as_deref().unwrap_or("");
            let constraint = db.constraint();
            let table = db.table();
            match sqlstate {
                // Collisions with stored data — client-caused, retryable only
                // by changing the request.
                "23505" | "23503" | "23001" | "23P01" => {
                    tracing::warn!(
                        sqlstate,
                        constraint = ?constraint,
                        table = ?table,
                        error = %e,
                        "storage bridge: integrity-constraint violation → 409 conflict"
                    );
                    SmError::new(CallStatusType::Conflict, CONFLICT_MESSAGE.to_owned())
                }
                "40001" | "40P01" => {
                    tracing::warn!(
                        sqlstate,
                        constraint = ?constraint,
                        table = ?table,
                        error = %e,
                        "storage bridge: serialization/deadlock failure → 409 conflict (retryable)"
                    );
                    SmError::new(
                        CallStatusType::Conflict,
                        RETRYABLE_CONFLICT_MESSAGE.to_owned(),
                    )
                }
                // A CHECK / NOT NULL / generic integrity violation reaching the
                // driver is OUR broken invariant, not the client's conflict.
                other if other.starts_with("23") => {
                    tracing::error!(
                        sqlstate,
                        constraint = ?constraint,
                        table = ?table,
                        error = %e,
                        "storage bridge: server-side integrity invariant violated \
                         (CHECK/NOT NULL reached the driver) → 500"
                    );
                    SmError::new(CallStatusType::Exception, INTERNAL_MESSAGE.to_owned())
                }
                _ => {
                    tracing::error!(
                        sqlstate,
                        constraint = ?constraint,
                        table = ?table,
                        error = %e,
                        "storage bridge: unclassified database error → 500"
                    );
                    SmError::new(CallStatusType::Exception, INTERNAL_MESSAGE.to_owned())
                }
            }
        }
        other => {
            tracing::error!(
                error = %other,
                "storage bridge: non-database sqlx error → 500"
            );
            SmError::new(CallStatusType::Exception, INTERNAL_MESSAGE.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error as StdError;
    use std::fmt;

    use sqlx::error::{DatabaseError, ErrorKind};

    use crate::service::status::{CallStatusType, SmError};

    use super::{StorageError, classify_sqlx};

    /// A driver-error double carrying one SQLSTATE plus the schema names a real
    /// PostgreSQL error would carry, so the classification and the
    /// no-leak assertions can be exercised without a live server.
    #[derive(Debug)]
    struct PgErrorDouble {
        sqlstate: &'static str,
    }

    /// The verbatim shape of a PostgreSQL integrity message: it names the
    /// constraint and the table, which is exactly what must not reach a client.
    const DRIVER_TEXT: &str =
        "new row for relation \"item_tag\" violates check constraint \"ck_item_tag_target_type\"";

    impl fmt::Display for PgErrorDouble {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(DRIVER_TEXT)
        }
    }

    impl StdError for PgErrorDouble {}

    impl DatabaseError for PgErrorDouble {
        fn message(&self) -> &str {
            DRIVER_TEXT
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.sqlstate))
        }

        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }

        fn constraint(&self) -> Option<&str> {
            Some("ck_item_tag_target_type")
        }

        fn table(&self) -> Option<&str> {
            Some("item_tag")
        }

        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    fn db_error(sqlstate: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(PgErrorDouble { sqlstate }))
    }

    #[test]
    fn pool_timeout_is_service_overloaded() {
        // Pool exhaustion under load → 503 semantics (our admission
        // contract), never a blanket 500.
        let sm = classify_sqlx(&sqlx::Error::PoolTimedOut);
        assert_eq!(sm.status, CallStatusType::ServiceOverloaded);
    }

    /// The bridge keeps the driver error as the call status's source: the
    /// message stays curated for the wire, and a log or a test walking the
    /// chain reaches the SQLSTATE and the driver text (#3250). Asserted by
    /// downcast, because `source().is_some()` cannot tell the error from the
    /// smart pointer around it.
    #[test]
    fn the_curated_status_keeps_the_driver_error_as_its_source() {
        use std::error::Error as _;
        let sm: SmError = StorageError::Database(sqlx::Error::PoolTimedOut).into();
        let source = sm.source().expect("the driver error travels as the source");
        assert!(
            source.downcast_ref::<sqlx::Error>().is_some(),
            "the source is the sqlx error itself, not a wrapper: {source}"
        );
        assert_eq!(
            sm.message,
            "the server is temporarily overloaded; retry shortly"
        );
    }

    #[test]
    fn non_database_error_is_exception() {
        // A row-not-found or other non-database sqlx error is a genuine fault.
        let sm = classify_sqlx(&sqlx::Error::RowNotFound);
        assert_eq!(sm.status, CallStatusType::Exception);
    }

    #[test]
    fn collisions_with_stored_data_stay_conflicts() {
        // unique / foreign-key / restrict / exclusion are genuinely
        // client-caused collisions with data the repository already holds.
        for sqlstate in ["23505", "23503", "23001", "23P01"] {
            let sm = classify_sqlx(&db_error(sqlstate));
            assert_eq!(
                sm.status,
                CallStatusType::Conflict,
                "SQLSTATE {sqlstate} should stay a conflict"
            );
        }
    }

    #[test]
    fn serialization_and_deadlock_stay_retryable_conflicts() {
        for sqlstate in ["40001", "40P01"] {
            let sm = classify_sqlx(&db_error(sqlstate));
            assert_eq!(sm.status, CallStatusType::Conflict);
            assert!(
                sm.message.contains("retry"),
                "the retryable conflict should tell the client to retry, got {:?}",
                sm.message
            );
        }
    }

    #[test]
    fn check_and_not_null_violations_are_internal_faults() {
        // A CHECK or NOT NULL reaching the driver is a violated SERVER-side
        // invariant — never an optimistic-lock conflict the client can resolve.
        for sqlstate in ["23514", "23502", "23000"] {
            let sm = classify_sqlx(&db_error(sqlstate));
            assert_eq!(
                sm.status,
                CallStatusType::Exception,
                "SQLSTATE {sqlstate} is a server-side invariant failure, not a 409"
            );
        }
    }

    #[test]
    fn no_codec_internal_shape_reaches_the_client_message() {
        // A node-codec fault is a violated SERVER-side invariant: the client
        // gets the curated `500` message, never the RM attribute name or the
        // internal node-row shape the error text carries.
        let faults = [
            StorageError::NotAStructureRoot(Some("DV_TEXT".to_owned())),
            StorageError::MixedArray {
                attribute: "content".to_owned(),
            },
            StorageError::InvalidRows("row num 7 has no parent at num 3".to_owned()),
        ];
        for fault in faults {
            let rendered = fault.to_string();
            let sm = SmError::from(fault);
            assert_eq!(
                sm.status,
                CallStatusType::Exception,
                "a codec fault is a server fault, not a client error"
            );
            for leaked in ["DV_TEXT", "content", "num", "structure", "array", "rows"] {
                assert!(
                    !sm.message.contains(leaked),
                    "codec fault {rendered:?} leaked {leaked:?} into the client message {:?}",
                    sm.message
                );
            }
        }
    }

    #[test]
    fn no_driver_string_reaches_the_client_message() {
        // The message a client sees never names a constraint, a table, or any
        // other schema object, on ANY classification branch.
        for sqlstate in [
            "23505", "23503", "23001", "23P01", "23514", "23502", "23000", "40001", "40P01",
            "42P01", "08006",
        ] {
            let sm = classify_sqlx(&db_error(sqlstate));
            for leaked in [
                "ck_item_tag_target_type",
                "item_tag",
                "violates",
                "relation",
            ] {
                assert!(
                    !sm.message.contains(leaked),
                    "SQLSTATE {sqlstate} leaked {leaked:?} into the client message {:?}",
                    sm.message
                );
            }
        }
    }
}

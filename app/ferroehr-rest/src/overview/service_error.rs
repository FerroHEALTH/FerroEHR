// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one mapping of a service failure onto the ITS-REST wire: each
//! [`ServiceError`] variant is the HTTP class of its refusal (ITS-REST overview
//! `Requests_and_responses.md` §HTTP status codes), and a server fault answers
//! with the curated internal message, its diagnosis on the trace record.
//!
//! A free function rather than `impl From<ServiceError> for ApiError`, because
//! both types are foreign to this crate and the orphan rule forbids that impl.

use ferroehr::service::error::ServiceError;
use openehr_its::rest::runtime::{ApiError, ValidationError};

use crate::overview::error::{internal_fault, internal_fault_caused};

/// The wire error a service failure answers.
///
/// A query timeout maps to the `500` row here; [`crate::overview::error::RestError`]
/// marks it with its own problem type, which answers `408`.
pub(crate) fn api_error(e: ServiceError) -> ApiError {
    match e {
        ServiceError::NotFound(e) => ApiError::NotFound(e.message),
        ServiceError::BadRequest(e) => ApiError::BadRequest(e.message),
        ServiceError::Conflict(e) => ApiError::Conflict(e.message),
        ServiceError::VersionConflict(e) => ApiError::PreconditionFailed(e.message),
        // The ONE rendering point of a `Violation` on the wire.
        ServiceError::Unprocessable { violation, .. } => {
            ApiError::Unprocessable(violation.to_string())
        }
        // The ONE place the RM violation data becomes the protocol's
        // `validationErrors[]` shape (the two carry the same `{path,
        // message}` facts).
        ServiceError::ValidationFailed(v) => ApiError::ValidationFailed(
            v.into_iter()
                .map(|e| ValidationError {
                    path: e.path,
                    message: e.message,
                })
                .collect(),
        ),
        // Storage/DB failures carry SQLSTATE/constraint detail: classified
        // (integrity/serialization conflict → 409, nothing ran → 503) rather
        // than blanket-500. A genuine fault stays 500.
        e @ (ServiceError::Storage(_) | ServiceError::Database(_) | ServiceError::Acquire(_)) => {
            api_error(e.classified())
        }
        ServiceError::Overloaded(err) => ApiError::ServiceUnavailable(err.message.to_owned()),
        // A malformed payload the server was asked to read is a client defect
        // → `400`, and the body names it (that IS the strict reader's
        // contract). A failure to serialize the server's own data is a server
        // fault → `500` with the curated opaque message; the serde diagnostic
        // rides the trace record only.
        ServiceError::JsonRead(e) => ApiError::BadRequest(e.to_string()),
        ServiceError::JsonWrite(e) => internal_fault_caused("serialize a JSON payload", &e),
        // Signing/integrity failures and generic faults are server-side
        // (`500`): the carried text is the log detail, the body is the curated
        // message.
        ServiceError::Signing(m) => internal_fault("sign or verify a version", &m),
        e @ (ServiceError::SigningFailed(_) | ServiceError::SigningCanonical(_)) => {
            internal_fault_caused("sign or verify a version", &e)
        }
        ServiceError::Internal(err) => internal_fault_caused("complete the request", &err),
        ServiceError::Fault(err) => ApiError::Internal(err.message.to_owned()),
        // Fail-closed: the result is withheld because its access record was
        // not taken. Our own contract (no openEHR spec governs the audit
        // trail's availability); RFC 9110 §15.6.4 is the HTTP authority for
        // the 503.
        ServiceError::Unrecorded(err) => ApiError::ServiceUnavailable(err.message),
        // Our own refusal (no openEHR spec governs restriction of processing);
        // RFC 9110 §15.5.4 is the HTTP authority for the 403.
        ServiceError::Restricted(err) | ServiceError::Forbidden(err) => {
            ApiError::Forbidden(err.message)
        }
        ServiceError::Timeout(err) => ApiError::Internal(err.message),
    }
}

#[cfg(test)]
mod tests {
    use ferroehr::service::error::{ServiceError, Violation};
    use ferroehr::service::refusal::{Clash, Invalid, Malformed, Missing, Refusal};
    use http::StatusCode as C;
    use openehr_base::validate::InvariantViolation;
    use openehr_its::rest::runtime::ApiError;

    use super::api_error;

    /// A stand-in underlying failure whose `Display` is exactly the kind of
    /// internal detail a `500` body must never carry.
    fn cause() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the node table is not readable",
        )
    }

    /// Every `404` kind.
    const MISSING: [Missing; 9] = [
        Missing::ObjectVersion,
        Missing::VersionedObject,
        Missing::Ehr,
        Missing::Party,
        Missing::Composition,
        Missing::Contribution,
        Missing::Artefact,
        Missing::Template,
        Missing::Version,
    ];

    /// Every `422` kind.
    const INVALID: [Invalid; 3] = [Invalid::Archetype, Invalid::Template, Invalid::Content];

    /// Every refusal the service raises, with the HTTP status its row
    /// prescribes; `with_cause` attaches [`cause`] where the row can carry one.
    fn rows(with_cause: bool) -> Vec<(String, ServiceError, C)> {
        fn refusal<K>(kind: K, with_cause: bool) -> Refusal<K> {
            let r = Refusal::new(kind, "m");
            if with_cause {
                r.with_source(cause())
            } else {
                r
            }
        }
        let mut rows: Vec<(String, ServiceError, C)> = vec![(
            "Precondition".to_owned(),
            ServiceError::BadRequest(refusal(Malformed::Precondition, with_cause)),
            C::BAD_REQUEST,
        )];
        for kind in MISSING {
            rows.push((
                format!("{kind:?}"),
                ServiceError::NotFound(refusal(kind, with_cause)),
                C::NOT_FOUND,
            ));
        }
        for kind in [
            Clash::DuplicateEhrId,
            Clash::CompositionExists,
            Clash::EhrForSubject,
            Clash::State,
        ] {
            rows.push((
                format!("{kind:?}"),
                ServiceError::Conflict(refusal(kind, with_cause)),
                C::CONFLICT,
            ));
        }
        for kind in INVALID {
            let violation = Violation::new("m");
            let violation = if with_cause {
                violation.with_source(cause())
            } else {
                violation
            };
            rows.push((
                format!("{kind:?}"),
                ServiceError::Unprocessable { kind, violation },
                C::UNPROCESSABLE_ENTITY,
            ));
        }
        rows.push((
            "VersionConflict".to_owned(),
            ServiceError::VersionConflict(refusal((), with_cause)),
            C::PRECONDITION_FAILED,
        ));
        rows.push((
            "Restricted".to_owned(),
            ServiceError::Restricted(refusal((), with_cause)),
            C::FORBIDDEN,
        ));
        rows.push((
            "Forbidden".to_owned(),
            ServiceError::Forbidden(refusal((), with_cause)),
            C::FORBIDDEN,
        ));
        rows.push((
            "Unrecorded".to_owned(),
            ServiceError::Unrecorded(refusal((), with_cause)),
            C::SERVICE_UNAVAILABLE,
        ));
        rows.push((
            "Internal".to_owned(),
            if with_cause {
                ServiceError::internal("m", cause())
            } else {
                ServiceError::exception("m")
            },
            C::INTERNAL_SERVER_ERROR,
        ));
        rows.push((
            "Fault".to_owned(),
            if with_cause {
                ServiceError::fault_caused("m", cause())
            } else {
                ServiceError::fault("m")
            },
            C::INTERNAL_SERVER_ERROR,
        ));
        rows
    }

    /// Every refusal lands on the HTTP status its row prescribes.
    #[test]
    fn service_error_routes_to_the_expected_http_status() {
        for (row, error, expected) in rows(false) {
            assert_eq!(api_error(error).status(), expected, "row {row} diverged");
        }
    }

    /// Every refusal carried with a cause lands on exactly the wire outcome,
    /// status AND body text, its cause-less twin produces: the chain is a new
    /// fact for the operator, never a change to what the client is told.
    #[test]
    fn a_carried_cause_changes_no_wire_outcome() {
        for ((row, flat, _), (_, caused, _)) in rows(false).into_iter().zip(rows(true)) {
            let (flat, caused) = (api_error(flat), api_error(caused));
            assert_eq!(
                caused.status(),
                flat.status(),
                "row {row} diverged on status"
            );
            assert_eq!(
                caused.to_string(),
                flat.to_string(),
                "row {row} diverged on body text"
            );
        }
    }

    /// A `ValidationFailed` keeps its violations as data on the wire, and
    /// answers the same `422` a flat content refusal does.
    #[test]
    fn validation_failed_keeps_its_violations_and_its_status() {
        let violations = vec![
            InvariantViolation::at("/content[0]/data", "missing mandatory attribute"),
            InvariantViolation::at("/context/start_time", "not a valid DV_DATE_TIME"),
        ];
        match api_error(ServiceError::ValidationFailed(violations.clone())) {
            ApiError::ValidationFailed(v) => {
                assert_eq!(v.len(), 2);
                assert_eq!(v[0].path, "/content[0]/data");
                assert_eq!(v[1].message, "not a valid DV_DATE_TIME");
            }
            other => panic!("expected ValidationFailed, got {other:?}"),
        }
        assert_eq!(
            api_error(ServiceError::ValidationFailed(violations)).status(),
            api_error(ServiceError::content_invalid(Violation::new("x"))).status(),
        );
    }

    /// A resolver failure while acquiring answers `503` on the wire, while the
    /// same driver identity on an acquired session stays `500` (#3749).
    #[tokio::test]
    async fn the_acquire_phase_decides_503_versus_500() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_lazy_with(
                sqlx::postgres::PgConnectOptions::new()
                    .host("ferroehr-acquire-test.invalid")
                    .username("nobody")
                    .database("nothing"),
            );
        let failure = ferroehr::db::acquire::acquire(&pool)
            .await
            .expect_err("the `.invalid` top-level domain never resolves");
        assert_eq!(
            api_error(ServiceError::from(failure)).status(),
            C::SERVICE_UNAVAILABLE
        );

        let in_session = ServiceError::Database(sqlx::Error::Protocol("unexpected".to_owned()));
        assert_eq!(api_error(in_session).status(), C::INTERNAL_SERVER_ERROR);
    }

    /// The JSON twins: a payload the server was asked to READ names its own
    /// defect on a `400`, while a failure to serialize the server's OWN data is
    /// a `500` whose body says nothing about serde's diagnosis.
    #[test]
    fn json_read_names_its_defect_and_json_write_stays_opaque() {
        let markers = ["invalid type", "line", "column"];
        let parse_failure = || {
            let failure: Result<std::collections::BTreeMap<String, i32>, _> =
                serde_json::from_str("{\"committer\":\"not an integer\"}");
            failure.expect_err("the fixture should fail to deserialize")
        };
        let rendered = parse_failure().to_string();
        assert!(
            markers.iter().any(|m| rendered.contains(m)),
            "the fixture must actually produce a serde diagnostic, got {rendered:?}"
        );

        let api = api_error(ServiceError::JsonRead(parse_failure()));
        assert_eq!(api.status(), C::BAD_REQUEST);
        assert!(
            markers.iter().any(|m| api.to_string().contains(m)),
            "a parse refusal must name its defect, got {:?}",
            api.to_string()
        );

        let api = api_error(ServiceError::JsonWrite(parse_failure()));
        assert_eq!(api.status(), C::INTERNAL_SERVER_ERROR);
        for leaked in markers {
            assert!(
                !api.to_string().contains(leaked),
                "the wire 500 body leaked {leaked:?}: {:?}",
                api.to_string()
            );
        }
    }

    /// A `500`-class failure carries its diagnosis to the LOG, never to the
    /// client: whatever the fault site wrote, and whatever its cause says,
    /// stays out of the body.
    #[test]
    fn internal_faults_never_render_their_diagnosis_on_the_wire() {
        let detail = "typing the ORIGINAL_VERSION: unknown field `_kind` at line 3 column 12";
        let markers = [
            "ORIGINAL_VERSION",
            "unknown field",
            "_kind",
            "line 3",
            "not readable",
        ];
        let canonical = || {
            let failure = serde_json::from_str::<std::collections::BTreeMap<String, i32>>("{")
                .expect_err("truncated");
            openehr_rm::v1_2::common::change_control::version_impl::CanonicalFormError::Canonicalize(
                failure,
            )
        };
        for error in [
            ServiceError::exception(detail),
            ServiceError::Signing(detail.to_owned()),
            ServiceError::internal("read a stored version row", cause()),
            ServiceError::SigningCanonical(canonical()),
        ] {
            let api = api_error(error);
            assert_eq!(api.status(), C::INTERNAL_SERVER_ERROR);
            for leaked in markers {
                assert!(
                    !api.to_string().contains(leaked),
                    "the wire 500 body leaked {leaked:?}: {:?}",
                    api.to_string()
                );
            }
        }
    }

    /// A [`Violation`] with a carried cause renders on the wire EXACTLY as one
    /// without: the 422 body is the refusal's own facts.
    #[test]
    fn a_violation_cause_is_not_on_the_wire() {
        let plain =
            Violation::new("is not a valid PARTY_PROXY").with_path("AUDIT_DETAILS.committer");
        let carried = Violation::new("is not a valid PARTY_PROXY")
            .with_path("AUDIT_DETAILS.committer")
            .with_source(cause());
        assert_eq!(
            api_error(ServiceError::content_invalid(carried)).to_string(),
            api_error(ServiceError::content_invalid(plain)).to_string()
        );
    }
}

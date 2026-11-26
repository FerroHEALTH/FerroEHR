// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's legal marks over the CDR's admin marks extension.
//!
//! The marks are a restriction of processing (GDPR Art. 18), an objection to
//! research processing (Art. 21(6)), the access-delay holds of the person's
//! own access (EHDS Art. 3(3)) and the retention anchor and holds. Each is
//! read and set under `{ext}/admin/…`; the treating professional releases a
//! held composition from the clinical route. No openEHR specification
//! governs the marks: FerroEHR's own design.

use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::tr;

/// One row of the restriction register.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Restriction {
    /// The restricted object, or `None` for the whole EHR.
    pub vo_id: Option<String>,
    /// The GDPR Art. 18(1) point, or `national`.
    pub ground: String,
    /// The controller's note.
    pub note: Option<String>,
    /// When it was requested.
    pub requested_at: String,
    /// When it was lifted, or `None` while in force.
    pub lifted_at: Option<String>,
}

/// The research objection of an EHR.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Objection {
    /// Whether an objection stands unoverridden.
    pub objected: bool,
    /// When the subject objected.
    pub objected_at: Option<String>,
    /// The controller's override ground.
    pub ground: Option<String>,
}

/// One access-delay hold.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DelayHold {
    /// The held composition.
    pub vo_id: String,
    /// `category` or `professional`.
    pub basis: String,
    /// When the hold began.
    pub withheld_at: String,
    /// When it ends by itself.
    pub withheld_until: String,
    /// When a professional released it.
    pub released_at: Option<String>,
    /// The professional's note.
    pub note: Option<String>,
}

/// The retention anchor of an EHR.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Anchor {
    /// The jurisdiction whose periods apply.
    pub jurisdiction: String,
    /// The anchor event, once known.
    pub anchored_at: Option<String>,
    /// When an EHR-wide hold was placed.
    pub hold_at: Option<String>,
    /// The ground of that hold.
    pub hold_ground: Option<String>,
}

/// One object held from disposal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionHold {
    /// The held object.
    pub vo_id: String,
    /// When the hold was placed.
    pub held_at: String,
}

/// Reads `path` of the admin extension with `ehr_id` as its query.
async fn read(path: &str, ehr_id: &str) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = format!(
        "{}?ehr_id={}",
        cdr.extensions_v1(path),
        urlencoding::encode(ehr_id)
    );
    Ok(cdr.get(&url).accept("application/json").ok().await?.body)
}

/// Posts (or puts, with `put`) the JSON `body` to `path` of the admin
/// extension.
async fn write(path: &str, body: String, put: bool) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(path);
    let call = if put { cdr.put(&url) } else { cdr.post(&url) };
    call.accept("application/json")
        .body("application/json", body)
        .ok()
        .await?;
    Ok(())
}

/// The text at `key` of `value`, `None` when it is absent or null.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the admin marks extension's JSON registers"
)]
pub(crate) fn text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The list at `key` of a JSON body, `what` naming the body in a failure.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the admin marks extension's JSON registers"
)]
pub(crate) fn rows(
    body: &str,
    key: &str,
    what: PolicyMsg,
) -> Result<Vec<serde_json::Value>, ViewerError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| ViewerError::failed(tr(what), error))?;
    Ok(value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// The restriction register of an EHR, newest request first
/// (`GET admin/restriction`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not JSON.
pub async fn fetch_restrictions(ehr_id: &str) -> Result<Vec<Restriction>, ViewerError> {
    let body = read("admin/restriction", ehr_id).await?;
    parse_restrictions(&body)
}

/// The restrictions a register body lists.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
pub fn parse_restrictions(body: &str) -> Result<Vec<Restriction>, ViewerError> {
    Ok(rows(body, "restrictions", PolicyMsg::MarksJson)?
        .iter()
        .map(|row| Restriction {
            vo_id: text(row, "vo_id"),
            ground: text(row, "ground").unwrap_or_default(),
            note: text(row, "note"),
            requested_at: text(row, "requested_at").unwrap_or_default(),
            lifted_at: text(row, "lifted_at"),
        })
        .collect())
}

/// The research objection of an EHR (`GET admin/research-objection`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the admin marks extension's JSON registers"
)]
pub async fn fetch_objection(ehr_id: &str) -> Result<Objection, ViewerError> {
    let body = read("admin/research-objection", ehr_id).await?;
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::MarksJson), error))?;
    Ok(Objection {
        objected: value
            .get("objected")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_default(),
        objected_at: text(&value, "objected_at"),
        ground: text(&value, "ground"),
    })
}

/// The access-delay holds of an EHR, newest first (`GET admin/access-delay`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not JSON.
pub async fn fetch_delays(ehr_id: &str) -> Result<Vec<DelayHold>, ViewerError> {
    let body = read("admin/access-delay", ehr_id).await?;
    Ok(rows(&body, "access_delays", PolicyMsg::MarksJson)?
        .iter()
        .map(|row| DelayHold {
            vo_id: text(row, "vo_id").unwrap_or_default(),
            basis: text(row, "basis").unwrap_or_default(),
            withheld_at: text(row, "withheld_at").unwrap_or_default(),
            withheld_until: text(row, "withheld_until").unwrap_or_default(),
            released_at: text(row, "released_at"),
            note: text(row, "note"),
        })
        .collect())
}

/// The retention anchor of an EHR, `None` while none is recorded
/// (`GET admin/retention/anchor`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the admin marks extension's JSON registers"
)]
pub async fn fetch_anchor(ehr_id: &str) -> Result<Option<Anchor>, ViewerError> {
    let body = read("admin/retention/anchor", ehr_id).await?;
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::MarksJson), error))?;
    Ok(value
        .get("anchor")
        .filter(|anchor| anchor.is_object())
        .map(|anchor| Anchor {
            jurisdiction: text(anchor, "jurisdiction").unwrap_or_default(),
            anchored_at: text(anchor, "anchored_at"),
            hold_at: text(anchor, "hold_at"),
            hold_ground: text(anchor, "hold_ground"),
        }))
}

/// The objects of an EHR held from disposal (`GET admin/retention/hold`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not JSON.
pub async fn fetch_retention_holds(ehr_id: &str) -> Result<Vec<RetentionHold>, ViewerError> {
    let body = read("admin/retention/hold", ehr_id).await?;
    Ok(rows(&body, "holds", PolicyMsg::MarksJson)?
        .iter()
        .map(|row| RetentionHold {
            vo_id: text(row, "vo_id").unwrap_or_default(),
            held_at: text(row, "held_at").unwrap_or_default(),
        })
        .collect())
}

/// A write of one mark, as the admin extension takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkWrite {
    /// Restrict the EHR, or one object of it, on a ground.
    Restrict {
        /// The object, or `None` for the whole EHR.
        vo_id: Option<String>,
        /// The ground.
        ground: String,
        /// The note.
        note: Option<String>,
    },
    /// Lift the restrictions at a grain.
    Lift {
        /// The object, or `None` for the whole EHR.
        vo_id: Option<String>,
    },
    /// Record the objection, its override with a ground, or its withdrawal.
    Objection {
        /// Whether the objection stands.
        objected: bool,
        /// The override ground.
        ground: Option<String>,
    },
    /// Hold a composition from the person.
    Delay {
        /// The composition.
        vo_id: String,
        /// The end, or `None` for the register's limit.
        until: Option<String>,
        /// The note.
        note: Option<String>,
    },
    /// Release a held composition to the person.
    Release {
        /// The composition.
        vo_id: String,
    },
    /// Record the retention anchor and any EHR-wide hold.
    Anchor(Anchor),
    /// Place or release a per-object retention hold.
    Hold {
        /// The object.
        vo_id: String,
        /// Whether it is held.
        held: bool,
    },
}

impl MarkWrite {
    /// The route and the JSON body of this write on the EHR `ehr_id`, and
    /// whether it is a `PUT`.
    #[expect(
        clippy::disallowed_types,
        reason = "the viewer writes the admin marks extension's JSON bodies"
    )]
    #[must_use]
    pub fn request(&self, ehr_id: &str) -> (&'static str, String, bool) {
        use serde_json::json;
        let (path, body, put) = match self {
            Self::Restrict {
                vo_id,
                ground,
                note,
            } => (
                "admin/restriction",
                json!({ "ehr_id": ehr_id, "vo_id": vo_id, "ground": ground, "note": note }),
                false,
            ),
            Self::Lift { vo_id } => (
                "admin/restriction/lift",
                json!({ "ehr_id": ehr_id, "vo_id": vo_id }),
                false,
            ),
            Self::Objection { objected, ground } => (
                "admin/research-objection",
                json!({ "ehr_id": ehr_id, "objected": objected, "ground": ground }),
                false,
            ),
            Self::Delay { vo_id, until, note } => (
                "admin/access-delay",
                json!({ "ehr_id": ehr_id, "vo_id": vo_id, "until": until, "note": note }),
                false,
            ),
            Self::Release { vo_id } => (
                "admin/access-delay/release",
                json!({ "ehr_id": ehr_id, "vo_id": vo_id }),
                false,
            ),
            Self::Anchor(anchor) => (
                "admin/retention/anchor",
                json!({ "ehr_id": ehr_id, "jurisdiction": anchor.jurisdiction,
                        "anchored_at": anchor.anchored_at, "hold_at": anchor.hold_at,
                        "hold_ground": anchor.hold_ground }),
                true,
            ),
            Self::Hold { vo_id, held } => (
                "admin/retention/hold",
                json!({ "vo_id": vo_id, "held": held }),
                false,
            ),
        };
        let body = match body {
            serde_json::Value::Object(object) => serde_json::Value::Object(
                object
                    .into_iter()
                    .filter(|(_, value)| !value.is_null())
                    .collect(),
            ),
            other => other,
        };
        (path, body.to_string(), put)
    }
}

/// Writes `mark` on the EHR `ehr_id`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); the CDR's `400`,
/// `403`, `404` and `405` normalize via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn write_mark(ehr_id: &str, mark: &MarkWrite) -> Result<(), ViewerError> {
    let (path, body, put) = mark.request(ehr_id);
    write(path, body, put).await
}

/// Releases a held composition to the person from the clinical route, as
/// the treating professional
/// (`POST ehr/{ehr_id}/versioned_composition/{uid}/access-delay/release`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `403` of a
/// request that may be the person's included.
pub async fn release_as_professional(ehr_id: &str, vo_id: &str) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!(
        "ehr/{}/versioned_composition/{}/access-delay/release",
        urlencoding::encode(ehr_id),
        urlencoding::encode(vo_id)
    ));
    cdr.post(&url).ok().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MarkWrite, parse_restrictions};

    #[test]
    fn a_whole_ehr_restriction_leaves_out_its_object() {
        let (path, body, put) = MarkWrite::Restrict {
            vo_id: None,
            ground: "gdpr-18-1-a".to_owned(),
            note: None,
        }
        .request("e1");
        assert_eq!("admin/restriction", path);
        assert!(!put);
        assert_eq!(r#"{"ehr_id":"e1","ground":"gdpr-18-1-a"}"#, body);
    }

    #[test]
    fn the_anchor_is_put() {
        let (path, _, put) = MarkWrite::Anchor(super::Anchor {
            jurisdiction: "CH".to_owned(),
            ..super::Anchor::default()
        })
        .request("e1");
        assert_eq!("admin/retention/anchor", path);
        assert!(put);
    }

    #[test]
    fn the_register_reads_a_lift() {
        let rows = parse_restrictions(
            r#"{"restrictions":[{"id":"r","ehr_id":"e1","vo_id":null,"ground":"national",
                "requested_at":"2026-11-01T00:00:00Z","lifted_at":"2026-11-02T00:00:00Z",
                "note":null}]}"#,
        )
        .expect("rows");
        assert_eq!(1, rows.len());
        assert_eq!(None, rows.first().and_then(|row| row.vo_id.clone()));
        assert!(rows.first().is_some_and(|row| row.lifted_at.is_some()));
    }
}

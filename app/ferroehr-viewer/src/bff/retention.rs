// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention register and what is due, over the CDR's admin marks
//! extension (`{ext}/admin/retention/policy`, `/due`).
//!
//! The register declares how long each kind of record is kept, keyed on a
//! jurisdiction and optionally an EHDS priority category and an origin; the
//! due list says which EHRs' periods have run, with the objects due and
//! held. No openEHR specification governs the register: FerroEHR's own
//! design over GDPR Art. 5(1)(e) and Art. 30(1)(f).

use crate::bff::marks::{rows, text};
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;

/// The most rows the due list reads.
pub const DUE_LIMIT: u32 = 100;

/// One declared retention period.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionPeriod {
    /// The RM kind it applies to.
    pub kind: String,
    /// The ISO 3166-1 alpha-2 jurisdiction whose rule it is.
    pub jurisdiction: String,
    /// The EHDS category it is keyed on, or `None` for every one.
    pub category: Option<String>,
    /// The origin it is keyed on, or `None` for every one.
    pub origin: Option<String>,
    /// The period, as the CDR renders it.
    pub period: String,
    /// What the period runs from.
    pub anchor: String,
    /// The legal citation it rests on.
    pub source: String,
}

/// One EHR whose period has run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DueEhr {
    /// The EHR.
    pub ehr_id: String,
    /// The jurisdiction whose period ran out.
    pub jurisdiction: String,
    /// The kind it covers.
    pub kind: String,
    /// The category it is keyed on.
    pub category: Option<String>,
    /// The origin it is keyed on.
    pub origin: Option<String>,
    /// The citation.
    pub source: String,
    /// When the period ran out.
    pub due_at: String,
    /// The objects due.
    pub objects_due: i64,
    /// The objects held from disposal.
    pub objects_held: i64,
}

/// The periods a register body declares.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
pub fn parse_periods(body: &str) -> Result<Vec<RetentionPeriod>, ViewerError> {
    Ok(rows(body, "policies", PolicyMsg::RetentionJson)?
        .iter()
        .map(|row| RetentionPeriod {
            kind: text(row, "kind").unwrap_or_default(),
            jurisdiction: text(row, "jurisdiction").unwrap_or_default(),
            category: text(row, "category"),
            origin: text(row, "origin"),
            period: text(row, "period").unwrap_or_default(),
            anchor: text(row, "anchor").unwrap_or_default(),
            source: text(row, "source").unwrap_or_default(),
        })
        .collect())
}

/// The rows a due body lists, in its order (oldest first).
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the admin marks extension's JSON registers"
)]
pub fn parse_due(body: &str) -> Result<Vec<DueEhr>, ViewerError> {
    let count = |row: &serde_json::Value, key: &str| {
        row.get(key)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default()
    };
    Ok(rows(body, "due", PolicyMsg::RetentionJson)?
        .iter()
        .map(|row| DueEhr {
            ehr_id: text(row, "ehr_id").unwrap_or_default(),
            jurisdiction: text(row, "jurisdiction").unwrap_or_default(),
            kind: text(row, "kind").unwrap_or_default(),
            category: text(row, "category"),
            origin: text(row, "origin"),
            source: text(row, "source").unwrap_or_default(),
            due_at: text(row, "due_at").unwrap_or_default(),
            objects_due: count(row, "objects_due"),
            objects_held: count(row, "objects_held"),
        })
        .collect())
}

/// Every declared period (`GET admin/retention/policy`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `403` and `405`
/// included; [`ViewerError::Failed`] when the answer is not JSON.
pub async fn fetch_periods() -> Result<Vec<RetentionPeriod>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/retention/policy");
    parse_periods(&cdr.get(&url).accept("application/json").ok().await?.body)
}

/// What is due, oldest first (`GET admin/retention/due`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not JSON.
pub async fn fetch_due() -> Result<Vec<DueEhr>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = format!(
        "{}?limit={DUE_LIMIT}",
        cdr.extensions_v1("admin/retention/due")
    );
    parse_due(&cdr.get(&url).accept("application/json").ok().await?.body)
}

/// Declares or changes `period` (`PUT admin/retention/policy`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); the CDR's `400`
/// normalizes via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
#[expect(
    clippy::disallowed_types,
    reason = "the viewer writes the admin marks extension's JSON body"
)]
pub async fn declare(period: &RetentionPeriod) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/retention/policy");
    let mut body = serde_json::json!({
        "kind": period.kind, "jurisdiction": period.jurisdiction, "period": period.period,
        "anchor": period.anchor, "source": period.source,
    });
    if let Some(object) = body.as_object_mut() {
        for (key, value) in [("category", &period.category), ("origin", &period.origin)] {
            if let Some(value) = value {
                object.insert(key.to_owned(), value.clone().into());
            }
        }
    }
    cdr.put(&url)
        .accept("application/json")
        .body("application/json", body.to_string())
        .ok()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_due, parse_periods};

    #[test]
    fn the_register_and_the_due_list_read_their_rows() {
        let periods = parse_periods(
            r#"{"policies":[{"kind":"EHR","jurisdiction":"CH","period":"20 years",
                "anchor":"last_commit","source":"EPDV","category":null,"origin":null}]}"#,
        )
        .expect("periods");
        assert_eq!(
            "20 years",
            periods
                .first()
                .map(|p| p.period.as_str())
                .unwrap_or_default()
        );
        let due = parse_due(
            r#"{"due":[{"ehr_id":"e1","jurisdiction":"CH","kind":"EHR","source":"EPDV",
                "due_at":"2026-01-01T00:00:00Z","objects_due":3,"objects_held":1}]}"#,
        )
        .expect("due");
        assert_eq!(
            Some((3, 1)),
            due.first().map(|row| (row.objects_due, row.objects_held))
        );
    }
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The facts digest of a stored entry, which its seal covers (#6322). How a
//! record is stored is our own design; no openEHR spec governs it.

use super::assemble::RowColumns;

/// The version of the stored form this build writes and its renderer reads
/// (#6322).
///
/// The form is the row part, the shared part and the columns as
/// [`super::assemble::whole`] joins them. A reader refuses a form it has no
/// renderer for.
pub const FORM: i16 = 1;

/// The tag the facts digest of a stored form starts with.
const FACTS_TAG: &str = "ferroehr.access_log.facts.v1";

/// The content digest of a stored entry: the SHA-256 of its stored facts
/// (#6322).
///
/// The facts are the form, the columns, the shared part's digest (lower-case
/// hex) and the row part, each written as its length, a colon and its text on
/// a line of its own, an absent one as `-`. The time is the column's
/// microsecond count, or `-` when the row part keeps the time's own text,
/// which its column then only approximates. The seal covers this digest, so
/// a rendering of the entry is never what is sealed.
///
/// # Errors
/// A [`serde_json::Error`] when the row part leaves the time to a column
/// finer than the microsecond it keeps.
pub fn facts_digest(
    form: i16,
    row: &str,
    shared: Option<&str>,
    columns: &RowColumns<'_>,
) -> Result<[u8; 32], serde_json::Error> {
    use sha2::Digest as _;
    let at = columns.time_recorded;
    let on_row = super::Members::of(row)
        .is_some_and(|mut members| members.any(|(name, _)| super::is_name(name, "time_recorded")));
    if !on_row && at.subsec_nanosecond().rem_euclid(1000) != 0 {
        return Err(serde::ser::Error::custom(
            "a stored time is kept to the microsecond",
        ));
    }
    let mut hasher = sha2::Sha256::new();
    let mut field = |value: Option<&[u8]>| match value {
        Some(bytes) => {
            let mut length = Line::default();
            length.put(bytes.len());
            hasher.update(length.bytes());
            hasher.update(b":");
            hasher.update(bytes);
            hasher.update(b"\n");
        }
        None => hasher.update(b"-\n"),
    };
    let mut form_text = Line::default();
    form_text.put(form);
    let mut at_text = Line::default();
    at_text.put(at.as_microsecond());
    let mut uid = uuid::Uuid::encode_buffer();
    let mut ehr = uuid::Uuid::encode_buffer();
    field(Some(FACTS_TAG.as_bytes()));
    field(Some(form_text.bytes()));
    field(Some(columns.record_type.as_bytes()));
    field(Some(
        columns.uid.hyphenated().encode_lower(&mut uid).as_bytes(),
    ));
    field((!on_row).then(|| at_text.bytes()));
    field(
        columns
            .ehr_id
            .map(|id| &*id.hyphenated().encode_lower(&mut ehr))
            .map(str::as_bytes),
    );
    field(columns.index.request_id.map(str::as_bytes));
    field(columns.index.resource_id.map(str::as_bytes));
    field(shared.map(str::as_bytes));
    field(Some(row.as_bytes()));
    Ok(hasher.finalize().into())
}

/// A number written in decimal on the stack, so a field of the facts digest
/// costs no allocation.
#[derive(Debug, Default)]
struct Line {
    buffer: [u8; 24],
    length: usize,
}

impl Line {
    /// Writes `value` in decimal, replacing what the line held.
    fn put(&mut self, value: impl std::fmt::Display) {
        use std::io::Write as _;
        let mut cursor = std::io::Cursor::new(&mut self.buffer[..]);
        // NOTE: the buffer holds the 20 digits and sign of any 64-bit integer, so the write fits.
        let _fits = write!(cursor, "{value}");
        self.length = usize::try_from(cursor.position()).unwrap_or(0);
    }

    /// The written digits.
    fn bytes(&self) -> &[u8] {
        self.buffer.get(..self.length).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{FORM, facts_digest};
    use crate::access_log::stored_form::parts::assemble::RowColumns;
    use crate::access_log::stored_form::parts::index::IndexColumns;

    /// The columns of an entry recorded at `at`.
    fn columns(at: &str) -> RowColumns<'static> {
        RowColumns {
            record_type: "ACCESS_LOG_ENTRY",
            uid: uuid::Uuid::nil(),
            time_recorded: at.parse().expect("an instant"),
            ehr_id: None,
            index: IndexColumns {
                request_id: None,
                resource_id: None,
            },
        }
    }

    /// A time finer than the microsecond, which the row part keeps as its own
    /// text, is digested by that text alone; one the row part leaves to its
    /// column must be whole microseconds.
    #[test]
    fn a_time_the_row_keeps_is_digested_by_its_text() {
        let fine = columns("2026-11-23T06:41:44.123456789Z");
        let kept = r#"{"time_recorded":{"_type":"DV_DATE_TIME","value":"2026-11-23T06:41:44.123456789Z"}}"#;
        assert!(facts_digest(FORM, kept, None, &fine).is_ok());
        assert!(facts_digest(FORM, "{}", None, &fine).is_err());
        assert!(facts_digest(FORM, "{}", None, &columns("2026-11-23T06:41:44.123456Z")).is_ok());
    }
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A stored body's text, held in the row that carried it (#6328).
//!
//! A read keeps the column's bytes where the driver received them, so a
//! response body takes them with no copy and no UTF-8 check; a consumer that
//! parses the text reads it as bytes, or checks it once with
//! [`StoredText::text`]. No openEHR spec governs how the server moves stored
//! bytes: our own design.

use std::str::Utf8Error;

use bytes::Bytes;
use sqlx::postgres::PgRow;
use sqlx::{Column as _, Row, ValueRef as _};

use crate::storage::error::StorageError;

/// The text of a stored body, shared without copying.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredText(Bytes);

impl StoredText {
    /// The text of the column `column` of `row`, `None` when it is `NULL`;
    /// the row is kept for as long as the text is.
    ///
    /// # Errors
    /// [`StorageError::Database`] when `row` has no such column.
    pub fn of_row(row: PgRow, column: &str) -> Result<Option<Self>, StorageError> {
        let at = row.try_column(column)?.ordinal();
        if row.try_get_raw(at)?.is_null() {
            return Ok(None);
        }
        Ok(Some(Self(Bytes::from_owner(Column { row, at }))))
    }

    /// The bytes of the text.
    #[must_use]
    pub fn bytes(&self) -> &Bytes {
        &self.0
    }

    /// The text as a string.
    ///
    /// # Errors
    /// The [`Utf8Error`] of bytes that are no UTF-8, which the store never
    /// holds in a `text` column.
    pub fn text(&self) -> Result<&str, Utf8Error> {
        std::str::from_utf8(&self.0)
    }

    /// The length of the text in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for StoredText {
    fn from(text: String) -> Self {
        Self(Bytes::from(text))
    }
}

impl From<&str> for StoredText {
    fn from(text: &str) -> Self {
        Self(Bytes::copy_from_slice(text.as_bytes()))
    }
}

impl std::fmt::Debug for StoredText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NOTE: our own design (#6328), a stored body is clinical content and never formatted.
        f.debug_tuple("StoredText").field(&self.0.len()).finish()
    }
}

/// A non-null column of a row, as the bytes the driver received.
struct Column {
    row: PgRow,
    at: usize,
}

impl AsRef<[u8]> for Column {
    #[expect(
        clippy::expect_used,
        reason = "StoredText::of_row builds a Column only for a column it read as present and \
                  non-null, and a row's columns never change"
    )]
    fn as_ref(&self) -> &[u8] {
        self.row
            .try_get_raw(self.at)
            .ok()
            .and_then(|value| value.as_bytes().ok())
            .expect("the column should have been read as non-null")
    }
}

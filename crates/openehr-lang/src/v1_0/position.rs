// @generated-from-template templates/openehr-lang/position.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Source-text position arithmetic — one home for this generation's parsers.
//!
//! Every reader in the LANG family reports defects at a byte offset in the
//! source it was handed, and needs to present that offset to a human as a line
//! and column. Which readers a generation carries differs — Release-1.0.0
//! defines an ODIN-only grammar set, the 1.1.0 line adds BEL and the ADL/cADL
//! front end — but the mapping is a property of the TEXT, not of any one
//! grammar, so it lives here once rather than being re-derived per parser or
//! per generation.

/// The 1-based line and column of a byte `offset` in `src`, counting COLUMNS
/// IN CHARACTERS.
///
/// Columns count characters rather than bytes so a line of clinical text with
/// non-ASCII content reports the column an author sees in an editor. An offset
/// past the end of `src` clamps to the end, so a defect reported at
/// end-of-input still names a real position.
#[must_use]
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let clamped = offset.min(src.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for (idx, ch) in src.char_indices() {
        if idx >= clamped {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::line_col;

    /// Both axes are 1-based, and a multi-byte character advances the column
    /// by one, not by its byte length.
    #[test]
    fn line_and_column_are_one_based_and_count_characters() {
        let src = "ab\ncdé/f";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 3), (2, 1));
        let slash = src.find('/').expect("the fixture contains a slash");
        assert_eq!(line_col(src, slash), (2, 4));
    }

    /// An offset past the end clamps rather than running off the text.
    #[test]
    fn an_offset_past_the_end_clamps() {
        let src = "ab\ncd";
        assert_eq!(line_col(src, src.len()), (2, 3));
        assert_eq!(line_col(src, src.len() + 100), (2, 3));
    }
}

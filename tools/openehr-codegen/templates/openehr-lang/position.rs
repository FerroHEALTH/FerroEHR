// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Byte offset to line and column, shared by every reader of the generation.

/// The 1-based line and column of byte `offset` in `src`, columns in
/// characters; an offset past the end clamps to the end.
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

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `master03` string-escape decoder — one home for this generation.
//!
//! `LANG/docs/odin/master03-basics.adoc` §File Encoding (verbatim twin:
//! `AM/docs/ADL2/master03-file_encoding.adoc` §File Encoding) defines the
//! ASCII encoding of unicode as "`\u` escaped UTF-16" in two spellings:
//!
//! - `\uHHHH` — "4 hex digits which will be the same (possibly 0-filled on the
//!   left) as the unicode code-point number expressed in hexadecimal; this
//!   applies to unicode codepoints in the range `U+0000` - `U+FFFF` (the 'base
//!   multi-lingual plane', BMP)";
//! - `\uHHHHHHHH` — "8 hex digits to encode unicode code-points in the range
//!   `U+10000` through `U+10FFFF` (non-BMP planes); the algorithm is described
//!   in IETF RFC 2781".
//!
//! plus the six customary quoted forms `\r \n \t \\ \" \'` of §Special
//! Character Sequences.
//!
//! Those eight forms are the whole set: §Special Character Sequences closes it
//! with "Any other character combination starting with a backslash is illegal",
//! so every other backslash sequence is a typed decode defect here. Regular
//! expressions never reach this decoder — a cADL string constraint's PERL
//! classes "should not be treated as anything other than literal strings"
//! (same §), so `openehr-adl` decodes only the `;"assumed"` suffix.
//!
//! NOTE: the released text contradicts itself — §File Encoding sanctions the
//! two `\u` spellings while §Special Character Sequences' closing sentence bans
//! them — adjudicated for §File Encoding, the specific rule, since the general
//! sentence would make that whole provision dead text.
//!
//! NOTE: no openEHR spec governs which UTF-16 spelling the eight digits carry —
//! our own design/extension: both the RFC 2781 surrogate-pair and the
//! zero-filled scalar readings decode, disambiguated by the first four digits
//! (`D800`-`DBFF` against `0000`-`0001`, disjoint by construction).
//!
//! NOTE: `master03-basics.adoc` is byte-identical in Release-1.0.0, so one
//! decoder is correct for every generation.

/// A `master03` escape-sequence defect.
///
/// Every variant is a decode failure with no sound fallback: the alternative
/// would be to emit a replacement character or pass the escape through
/// verbatim, both of which turn an authoring defect into silently wrong text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EscapeError {
    /// A backslash sequence outside the sanctioned set — the six customary
    /// quoted forms of §Special Character Sequences plus the two `\u`
    /// spellings of §File Encoding. "Any other character combination starting
    /// with a backslash is illegal; to get the effect of a literal backslash,
    /// the `\\` sequence should always be used" (§Special Character
    /// Sequences).
    #[error(
        "the escape sequence '{sequence}' at byte {at} is illegal; the legal forms are \\r \\n \\t \\\\ \\\" \\' and \\u"
    )]
    IllegalEscape {
        /// The two-character sequence, as authored.
        sequence: String,
        /// Its byte offset in the undelimited string body.
        at: usize,
    },
    /// A string body ending in an unpaired backslash. A backslash carries no
    /// meaning on its own: "to get the effect of a literal backslash, the
    /// `\\` sequence should always be used" (§Special Character Sequences).
    #[error(
        "the string body ends with an unpaired backslash at byte {at}; a literal backslash is written \\\\"
    )]
    DanglingBackslash {
        /// The byte offset of the unpaired backslash.
        at: usize,
    },
    /// A `\u` escape carrying neither of the two digit counts §File Encoding
    /// defines (4 for the BMP form, 8 for the non-BMP form).
    #[error(
        "the unicode escape at byte {at} carries {digits} hex digits; the \\u forms take 4 (BMP) or 8 (non-BMP)"
    )]
    MalformedUnicodeEscape {
        /// How many hex digits followed the `\u`.
        digits: usize,
        /// The byte offset of the backslash opening the escape.
        at: usize,
    },
    /// A `\uHHHHHHHH` escape read as a zero-filled scalar whose code point
    /// falls outside `U+10000`-`U+10FFFF`, the only range the 8-digit form
    /// encodes (§File Encoding).
    #[error(
        "the 8-digit unicode escape '\\u{escape}' denotes U+{code_point:04X}, outside the U+10000-U+10FFFF range the 8-digit form encodes"
    )]
    NonBmpEscapeOutOfRange {
        /// The eight hex digits, as authored.
        escape: String,
        /// The code point they denote.
        code_point: u32,
    },
    /// A `\uHHHHHHHH` escape opening with a UTF-16 high surrogate whose second
    /// half is not a low surrogate (`DC00`-`DFFF`, RFC 2781 §2.1).
    #[error(
        "the 8-digit unicode escape '\\u{escape}' opens with a high surrogate but its second half is not a low surrogate (DC00-DFFF)"
    )]
    MalformedSurrogatePair {
        /// The eight hex digits, as authored.
        escape: String,
    },
    /// A `\uHHHH` escape naming a UTF-16 surrogate code point. A surrogate is
    /// not a Unicode scalar value and appears only as half of a pair
    /// (RFC 2781 §2.1), so on its own it denotes no character.
    #[error(
        "the unicode escape '\\u{escape}' names the surrogate code point U+{code_point:04X}, which is not a character"
    )]
    LoneSurrogate {
        /// The four hex digits, as authored.
        escape: String,
        /// The surrogate code point they name.
        code_point: u32,
    },
}

/// The lowest UTF-16 high surrogate (RFC 2781 §2.1).
const HIGH_SURROGATE_FIRST: u32 = 0xD800;
/// The highest UTF-16 high surrogate.
const HIGH_SURROGATE_LAST: u32 = 0xDBFF;
/// The lowest UTF-16 low surrogate.
const LOW_SURROGATE_FIRST: u32 = 0xDC00;
/// The highest UTF-16 low surrogate.
const LOW_SURROGATE_LAST: u32 = 0xDFFF;
/// The first non-BMP code point — the low end of the 8-digit form's range.
const NON_BMP_FIRST: u32 = 0x0001_0000;
/// The last Unicode code point — the high end of the 8-digit form's range.
const NON_BMP_LAST: u32 = 0x0010_FFFF;

/// Decode the `master03` escape sequences of an undelimited string body.
///
/// # Errors
/// [`EscapeError`] for any backslash sequence the two chapters do not
/// sanction — an unknown letter after the backslash, an unpaired trailing
/// backslash, a `\u` with neither 4 nor 8 hex digits — and for a `\u` escape
/// that denotes no character: an 8-digit form outside `U+10000`-`U+10FFFF`, a
/// malformed surrogate pair, or a lone surrogate.
pub fn decode(inner: &str) -> Result<String, EscapeError> {
    if !inner.contains('\\') {
        return Ok(inner.to_owned());
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    // The byte offset of the character `chars` is about to yield, so an error
    // can name where in the body the offending sequence sits.
    let mut at = 0usize;
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            at += c.len_utf8();
            continue;
        }
        match chars.next() {
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            // The `\u` arm consumes its own hex digits on top of the two bytes
            // the backslash and the `u` take.
            Some('u') => at += decode_unicode(&mut chars, &mut out, at)?,
            // "Any other character combination starting with a backslash is
            // illegal" (§Special Character Sequences). Passing it through
            // verbatim would make an authoring defect silently readable text.
            Some(other) => {
                return Err(EscapeError::IllegalEscape {
                    sequence: format!("\\{other}"),
                    at,
                });
            }
            None => return Err(EscapeError::DanglingBackslash { at }),
        }
        // Every arm above consumed the backslash and one ASCII selector.
        at += 2;
    }
    Ok(out)
}

/// Check that every `master03` escape sequence of an undelimited string body
/// decodes, discarding the decoded text.
///
/// The lexers call this so a semantically-unsound `\u` fails the LEX, next to
/// the structural escape check, rather than surfacing later.
///
/// # Errors
/// As [`decode`].
pub fn validate(inner: &str) -> Result<(), EscapeError> {
    decode(inner).map(|_| ())
}

/// Decode a double-quoted `STRING` literal — strip the `"` delimiters, then
/// decode the body with [`decode`].
///
/// The delimiters are `master03`'s (`LANG/docs/odin/master03-basics` §Special
/// Character Sequences: `\"` is the escape *because* `"` delimits), and every
/// reader of a `master03` string literal strips them the same way — whichever
/// readers this generation carries — so the rule lives here once. A slice that
/// is not delimited is decoded as-is rather than losing a character.
///
/// # Errors
/// As [`decode`].
pub fn decode_string_literal(raw: &str) -> Result<String, EscapeError> {
    decode(strip_delimiters(raw, '"'))
}

/// Decode a single-quoted `CHARACTER` literal — strip the `'` delimiters, then
/// decode the body with [`decode`].
///
/// Returns a `String` rather than a `char` because the caller decides what an
/// empty literal means; the grammars admit exactly one character, so a
/// well-formed literal yields exactly one.
///
/// # Errors
/// As [`decode`].
pub fn decode_character_literal(raw: &str) -> Result<String, EscapeError> {
    decode(strip_delimiters(raw, '\''))
}

/// The body of a `delimiter`-quoted literal, or the whole slice when it is not
/// delimited.
fn strip_delimiters(raw: &str, delimiter: char) -> &str {
    raw.strip_prefix(delimiter)
        .and_then(|s| s.strip_suffix(delimiter))
        .unwrap_or(raw)
}
/// Decode one `\u…` escape, whose `\u` prefix `chars` has already yielded, and
/// report how many bytes of hex digits it consumed (4 or 8 — every digit is
/// ASCII, so the byte and character counts coincide).
///
/// Consumes exactly the hex digits the chosen spelling uses, so any trailing
/// hex text stays literal. `at` is the byte offset of the backslash that opens
/// the escape, carried only so a defect can name its position.
fn decode_unicode(
    chars: &mut std::str::Chars<'_>,
    out: &mut String,
    at: usize,
) -> Result<usize, EscapeError> {
    let digits: String = chars
        .clone()
        .take(8)
        .take_while(char::is_ascii_hexdigit)
        .collect();
    let digit_count = digits.chars().count();
    let first: String = digits.chars().take(4).collect();
    let first_value = if digit_count >= 4 {
        hex_value(&first)
    } else {
        None
    };
    let Some(first_value) = first_value else {
        // Fewer than four hex digits: neither of the two spellings §File
        // Encoding defines, so the escape names no character.
        return Err(EscapeError::MalformedUnicodeEscape {
            digits: digit_count,
            at,
        });
    };
    let has_eight = digit_count >= 8;

    if has_eight && (HIGH_SURROGATE_FIRST..=HIGH_SURROGATE_LAST).contains(&first_value) {
        let second: String = digits.chars().skip(4).take(4).collect();
        let low = hex_value(&second)
            .filter(|low| (LOW_SURROGATE_FIRST..=LOW_SURROGATE_LAST).contains(low));
        let Some(low) = low else {
            return Err(EscapeError::MalformedSurrogatePair { escape: digits });
        };
        // RFC 2781 §2.1: the pair encodes `0x10000 + (hi - 0xD800) * 0x400 +
        // (lo - 0xDC00)`, which is total over the two surrogate ranges.
        let code_point = NON_BMP_FIRST
            + (first_value - HIGH_SURROGATE_FIRST) * 0x400
            + (low - LOW_SURROGATE_FIRST);
        push_non_bmp(code_point, digits, out)?;
        advance(chars, 8);
        return Ok(8);
    }

    if has_eight && first_value <= 0x0001 {
        let Some(code_point) = hex_value(&digits) else {
            return Err(EscapeError::MalformedUnicodeEscape {
                digits: digit_count,
                at,
            });
        };
        push_non_bmp(code_point, digits, out)?;
        advance(chars, 8);
        return Ok(8);
    }

    // The 4-digit BMP form.
    if (HIGH_SURROGATE_FIRST..=LOW_SURROGATE_LAST).contains(&first_value) {
        return Err(EscapeError::LoneSurrogate {
            escape: first,
            code_point: first_value,
        });
    }
    let Some(ch) = char::from_u32(first_value) else {
        return Err(EscapeError::LoneSurrogate {
            escape: first,
            code_point: first_value,
        });
    };
    out.push(ch);
    advance(chars, 4);
    Ok(4)
}

/// Push the character an 8-digit escape denotes, refusing anything outside the
/// `U+10000`-`U+10FFFF` range that form encodes.
fn push_non_bmp(code_point: u32, escape: String, out: &mut String) -> Result<(), EscapeError> {
    if !(NON_BMP_FIRST..=NON_BMP_LAST).contains(&code_point) {
        return Err(EscapeError::NonBmpEscapeOutOfRange { escape, code_point });
    }
    let Some(ch) = char::from_u32(code_point) else {
        return Err(EscapeError::NonBmpEscapeOutOfRange { escape, code_point });
    };
    out.push(ch);
    Ok(())
}

/// The value of a hex-digit string, or `None` if it is empty, over-long, or not
/// all hex.
fn hex_value(digits: &str) -> Option<u32> {
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// Drop `n` characters from `chars`.
fn advance(chars: &mut std::str::Chars<'_>, n: usize) {
    for _ in 0..n {
        chars.next();
    }
}

#[cfg(test)]
mod tests {
    use super::{EscapeError, decode};

    /// Decode `inner`, failing the test with the defect if it does not.
    fn decoded(inner: &str) -> String {
        decode(inner).expect("the fixture should decode")
    }

    /// The six customary quoted forms of §Special Character Sequences.
    #[test]
    fn the_six_quoted_forms_decode() {
        assert_eq!(decoded(r"a\rb\nc\td\\e"), "a\rb\nc\td\\e");
        assert_eq!(decoded(r#"\""#), "\"");
        assert_eq!(decoded(r"\'"), "'");
    }

    /// The 4-digit form covers the BMP (`U+0000`-`U+FFFF`).
    #[test]
    fn four_digit_escapes_decode_bmp_code_points() {
        assert_eq!(decoded(r"\u0041"), "A");
        assert_eq!(decoded(r"\u00E9t\u00e9"), "\u{e9}t\u{e9}");
        assert_eq!(decoded(r"\uFFFD"), "\u{FFFD}");
    }

    /// RFC 2781 §2.1 surrogate pairs, at both ends of the non-BMP range.
    #[test]
    fn surrogate_pairs_decode_non_bmp_code_points() {
        assert_eq!(decoded(r"\uD800DC00"), "\u{10000}");
        assert_eq!(decoded(r"\uDBFFDFFF"), "\u{10FFFF}");
        assert_eq!(decoded(r"\uD83DDE00"), "\u{1F600}");
    }

    /// The zero-filled 8-digit spelling, whose first four digits are `0001`.
    #[test]
    fn zero_filled_eight_digit_escapes_decode() {
        assert_eq!(decoded(r"\u00010000"), "\u{10000}");
        assert_eq!(decoded(r"\u0001F600"), "\u{1F600}");
    }

    /// An 8-digit spelling of a BMP code point is outside the 8-digit form's
    /// stated range, so it is a defect rather than a silent BMP character.
    #[test]
    fn an_eight_digit_bmp_value_is_refused() {
        assert_eq!(
            decode(r"\u0000FFFF"),
            Err(EscapeError::NonBmpEscapeOutOfRange {
                escape: "0000FFFF".to_owned(),
                code_point: 0xFFFF,
            })
        );
    }

    /// A high surrogate whose second half is not a low surrogate.
    #[test]
    fn a_high_surrogate_without_a_low_half_is_refused() {
        assert_eq!(
            decode(r"\uD8000041"),
            Err(EscapeError::MalformedSurrogatePair {
                escape: "D8000041".to_owned(),
            })
        );
    }

    /// A lone surrogate, and the reversed pair whose first half is a LOW
    /// surrogate, are both lone-surrogate defects.
    #[test]
    fn lone_and_reversed_surrogates_are_refused() {
        assert_eq!(
            decode(r"\uD800"),
            Err(EscapeError::LoneSurrogate {
                escape: "D800".to_owned(),
                code_point: 0xD800,
            })
        );
        assert_eq!(
            decode(r"\uDC00D800"),
            Err(EscapeError::LoneSurrogate {
                escape: "DC00".to_owned(),
                code_point: 0xDC00,
            })
        );
    }

    /// Eight hex digits opening outside both disambiguating windows read as the
    /// 4-digit form followed by literal hex text.
    #[test]
    fn hex_text_after_a_four_digit_escape_stays_literal() {
        assert_eq!(decoded(r"\u00410042"), "A0042");
    }

    /// "Any other character combination starting with a backslash is illegal"
    /// (§Special Character Sequences) — including the PERL regex classes that
    /// are legal only INSIDE a cADL regex literal, which is never decoded.
    #[test]
    fn a_backslash_sequence_outside_the_sanctioned_set_is_refused() {
        for (body, sequence, at) in [
            (r"\q", r"\q", 0),
            (r"a\d", r"\d", 1),
            (r"ab\s.*", r"\s", 2),
            (r"\0", r"\0", 0),
            (r"\ ", r"\ ", 0),
            (r"\U0041", r"\U", 0),
        ] {
            assert_eq!(
                decode(body),
                Err(EscapeError::IllegalEscape {
                    sequence: sequence.to_owned(),
                    at,
                }),
                "{body:?} must be refused"
            );
        }
    }

    /// The offset an illegal escape reports counts BYTES of the AUTHORED body,
    /// so multi-byte text before it does not shift the position it names, and a
    /// preceding escape counts its two authored characters rather than the one
    /// it decodes to.
    #[test]
    fn the_illegal_escape_offset_counts_authored_bytes() {
        assert_eq!(
            decode("\u{e9}\\q"),
            Err(EscapeError::IllegalEscape {
                sequence: r"\q".to_owned(),
                at: 2,
            })
        );
        assert_eq!(
            decode(r"\t\q"),
            Err(EscapeError::IllegalEscape {
                sequence: r"\q".to_owned(),
                at: 2,
            })
        );
        assert_eq!(
            decode("\\u00E9\\q"),
            Err(EscapeError::IllegalEscape {
                sequence: r"\q".to_owned(),
                at: 6,
            })
        );
    }

    /// A backslash carries no meaning on its own: "to get the effect of a
    /// literal backslash, the `\\` sequence should always be used".
    #[test]
    fn an_unpaired_trailing_backslash_is_refused() {
        assert_eq!(decode("\\"), Err(EscapeError::DanglingBackslash { at: 0 }));
        assert_eq!(
            decode(r"a\\b\"),
            Err(EscapeError::DanglingBackslash { at: 4 })
        );
    }

    /// §File Encoding defines exactly two `\u` spellings, of 4 and 8 hex
    /// digits; anything shorter names no character.
    #[test]
    fn a_unicode_escape_shorter_than_four_digits_is_refused() {
        for (body, digits) in [(r"\u", 0), (r"\u4", 1), (r"\u00", 2), (r"\uFFF", 3)] {
            assert_eq!(
                decode(body),
                Err(EscapeError::MalformedUnicodeEscape { digits, at: 0 }),
                "{body:?} must be refused"
            );
        }
    }

    /// The whole sanctioned set decodes in one body: the six quoted forms, the
    /// 4-digit spelling, the surrogate pair and the zero-filled 8-digit form.
    #[test]
    fn the_whole_sanctioned_set_decodes_together() {
        assert_eq!(
            decoded("\\r\\n\\t\\\\\\\"\\'\\u00E9\\uD83DDE00\\u0001F600"),
            "\r\n\t\\\"'\u{e9}\u{1F600}\u{1F600}"
        );
    }
}

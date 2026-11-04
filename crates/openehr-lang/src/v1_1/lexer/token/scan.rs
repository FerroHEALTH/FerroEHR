// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Loop scanners for the delimited free-text tokens: `STRING`, the `--`
//! comment, the `<# … #>` plug-in block, `EMBEDDED_URI` and
//! `CONTAINED_REGEXP`.
//!
//! `logos` 0.16 renders each DFA state as a function that tail-calls the
//! next, and a debug build does not eliminate those calls, so every
//! transition its fast self-loop does not absorb (a multi-byte character, an
//! escape) pushes a stack frame: a long token needed a stack proportional to
//! its length. These tokens can be as long as the text an author writes, so
//! the DFA matches only their opening delimiter and a callback here scans
//! the rest in one loop, in constant stack. Each scanner accepts exactly the
//! language of the regular expression its token carried before, with the same
//! longest-match end and the same failure extent, so the token stream and its
//! spans are unchanged. No openEHR specification governs this: it is our own
//! implementation of the vendored `base_lexer.g4` patterns.
//!
//! The scanners work on bytes. Every delimiter and escape they look for is
//! ASCII and a UTF-8 continuation byte is never ASCII, so a byte scan ends
//! only on a character boundary.

use logos::{Lexer, Logos, Skip};

/// The rest of a `STRING` after its opening `"`, as `"([^"\\]|\\.)*"`.
///
/// Returns the length through the closing quote, or, when the literal never
/// closes, the length the DFA consumed before it stuck: to the end of the
/// input, or up to a line feed that follows a backslash (`.` excludes it).
fn string_rest(rest: &[u8]) -> Result<usize, usize> {
    let mut at = 0;
    loop {
        match rest.get(at) {
            None => return Err(at),
            Some(b'"') => return Ok(at + 1),
            Some(b'\\') => match rest.get(at + 1) {
                None | Some(b'\n') => return Err(at + 1),
                Some(_) => at += 2,
            },
            Some(_) => at += 1,
        }
    }
}

/// The rest of a plug-in block after its `<`, as `#([^#]|#[^>])*#*#>`.
///
/// Returns the length of the longest match, or `None` when there is none.
/// The loop is the minimal DFA of that expression: `Hash` follows a `#` that
/// opens a unit, `HashHash` a `#` that could also close one; `#>` read from
/// `Hash` ends every match, while from `HashHash` it is one match the block
/// may still extend past.
pub(crate) fn plug_in_rest(rest: &[u8]) -> Option<usize> {
    #[derive(Clone, Copy)]
    enum State {
        Unit,
        Hash,
        HashHash,
    }
    let body = rest.strip_prefix(b"#")?;
    let mut state = State::Unit;
    let mut longest = None;
    for (at, byte) in body.iter().enumerate() {
        state = match (state, byte) {
            (State::Unit | State::HashHash, b'#') => State::Hash,
            (State::Hash, b'>') => return Some(at + 2),
            (State::Hash, b'#') => State::HashHash,
            (State::HashHash, b'>') => {
                longest = Some(at + 2);
                State::Unit
            }
            _ => State::Unit,
        };
    }
    longest
}

/// The first offset at or after `at` that is not `[ \t\r]`.
fn skip_blanks(rest: &[u8], mut at: usize) -> usize {
    while matches!(rest.get(at), Some(b' ' | b'\t' | b'\r')) {
        at += 1;
    }
    at
}

/// The rest of an `EMBEDDED_URI` after its `<`, as
/// `[ \t\r\n]*[a-zA-Z][a-zA-Z0-9+.\-]*:[^>]*>`.
///
/// Returns the length through the first `>`, or `None` when the text is no
/// embedded URI. The expression has one parse, so its only match is the
/// longest.
pub(crate) fn embedded_uri_rest(rest: &[u8]) -> Option<usize> {
    let mut at = 0;
    while matches!(rest.get(at), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        at += 1;
    }
    if !rest.get(at)?.is_ascii_alphabetic() {
        return None;
    }
    at += 1;
    while rest
        .get(at)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-'))
    {
        at += 1;
    }
    if rest.get(at) != Some(&b':') {
        return None;
    }
    let close = rest.get(at..)?.iter().position(|byte| *byte == b'>')?;
    Some(at + close + 1)
}

/// The rest of a `CONTAINED_REGEXP` after its `{`, as
/// `[ \t\r]*(/([^/\r\n\\]|\\.)+/|\^([^\^\r\n\\]|\\.)+\^)[ \t\r]*(;[ \t\r]*"([^"\\]|\\.)*")?[ \t\r]*\}`.
///
/// Returns the length through the closing `}`, or `None` when the text is
/// no contained regexp. The expression has one parse, so its only match is
/// the longest.
pub(crate) fn contained_regexp_rest(rest: &[u8]) -> Option<usize> {
    let mut at = skip_blanks(rest, 0);
    let delimiter = *rest.get(at).filter(|byte| matches!(byte, b'/' | b'^'))?;
    at += 1;
    let body = at;
    loop {
        match *rest.get(at)? {
            byte if byte == delimiter => break,
            b'\\' if rest.get(at + 1).is_some_and(|next| *next != b'\n') => at += 2,
            b'\\' | b'\r' | b'\n' => return None,
            _ => at += 1,
        }
    }
    if at == body {
        return None;
    }
    at = skip_blanks(rest, at + 1);
    if rest.get(at) == Some(&b';') {
        at = skip_blanks(rest, at + 1);
        if rest.get(at) != Some(&b'"') {
            return None;
        }
        at += 1 + string_rest(rest.get(at + 1..)?).ok()?;
    }
    at = skip_blanks(rest, at);
    (rest.get(at) == Some(&b'}')).then_some(at + 1)
}

/// Finish a `STRING` whose opening `"` the DFA matched.
///
/// # Errors
/// Returns `Err(())` when the literal never closes, with the token extended
/// over what the DFA consumed before it stuck.
pub(crate) fn string<'s, T>(lex: &mut Lexer<'s, T>) -> Result<(), ()>
where
    T: Logos<'s, Source = str>,
{
    let scanned = string_rest(lex.remainder().as_bytes());
    let (Ok(length) | Err(length)) = scanned;
    lex.bump(length);
    scanned.map(|_closed| ()).map_err(|_stuck| ())
}

/// Skip a `--` comment to the end of its line (`CMT_LINE`, `--[^\n]*`).
pub(crate) fn comment<'s, T>(lex: &mut Lexer<'s, T>) -> Skip
where
    T: Logos<'s, Source = str>,
{
    let rest = lex.remainder();
    lex.bump(rest.find('\n').unwrap_or(rest.len()));
    Skip
}

/// One token a one-character DFA match may open: the scanner of the rest of
/// its text and the variant that carries it.
pub(crate) type Opening<T> = (fn(&[u8]) -> Option<usize>, fn(String) -> T);

/// Extend the one-character token the DFA matched into the first of
/// `openings` whose scanner accepts the text after it, or leave it as
/// `otherwise` when none does.
pub(crate) fn opening_or<'s, T>(lex: &mut Lexer<'s, T>, openings: &[Opening<T>], otherwise: T) -> T
where
    T: Logos<'s, Source = str>,
{
    let rest = lex.remainder().as_bytes();
    for &(scan, token) in openings {
        if let Some(length) = scan(rest) {
            lex.bump(length);
            return token(lex.slice().to_owned());
        }
    }
    otherwise
}

#[cfg(test)]
mod tests {
    use super::{contained_regexp_rest, embedded_uri_rest, plug_in_rest, string_rest};

    #[test]
    fn a_string_closes_at_its_first_unescaped_quote() {
        assert_eq!(string_rest(br#"ab\"c" d""#), Ok(6));
        assert_eq!(string_rest(b"a\nb\""), Ok(4));
    }

    #[test]
    fn an_open_string_reports_where_the_dfa_stuck() {
        assert_eq!(string_rest(b"abc"), Err(3));
        assert_eq!(string_rest(b"a\\"), Err(2));
        assert_eq!(string_rest(b"a\\\nb\""), Err(2));
    }

    #[test]
    fn a_plug_in_block_takes_the_longest_match() {
        assert_eq!(plug_in_rest(b"#a#>"), Some(4));
        assert_eq!(plug_in_rest(b"# a #> b #>"), Some(6));
        assert_eq!(plug_in_rest(b"#a##>b#>"), Some(8));
        assert_eq!(plug_in_rest(b"#a##>b"), Some(5));
        assert_eq!(plug_in_rest(b"##>"), Some(3));
        assert_eq!(plug_in_rest(b"#>"), None);
        assert_eq!(plug_in_rest(b"#a"), None);
        assert_eq!(plug_in_rest(b" #a#>"), None);
    }

    #[test]
    fn a_contained_regexp_closes_at_its_brace() {
        assert_eq!(contained_regexp_rest(b"/a/}"), Some(4));
        assert_eq!(contained_regexp_rest(b" ^a\\^^ ; \"x}\" } x"), Some(15));
        assert_eq!(contained_regexp_rest(b"//}"), None);
        assert_eq!(contained_regexp_rest(b"/a\nb/}"), None);
        assert_eq!(contained_regexp_rest(b"/a/ ;x}"), None);
        assert_eq!(contained_regexp_rest(b"/a/"), None);
    }

    #[test]
    fn an_embedded_uri_closes_at_its_first_angle() {
        assert_eq!(embedded_uri_rest(b"http://a/b>c>"), Some(11));
        assert_eq!(embedded_uri_rest(b"\n a+b.c-d:x\ny>"), Some(14));
        assert_eq!(embedded_uri_rest(b"1a:b>"), None);
        assert_eq!(embedded_uri_rest(b"a_b:c>"), None);
        assert_eq!(embedded_uri_rest(b"a:b"), None);
    }
}

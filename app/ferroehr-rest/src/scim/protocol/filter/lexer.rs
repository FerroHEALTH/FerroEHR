// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The tokens of the filter grammar (RFC 7644 Figure 1).
//!
//! A filter is brackets, JSON string literals and words: an attribute path,
//! an operator, or a JSON literal name or number. A word runs to the next
//! whitespace, bracket or quote, so `eq"x"` is the word `eq` and a string.

/// One token and the byte offset it starts at.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Token<'a> {
    pub(super) kind: Kind<'a>,
    pub(super) at: usize,
}

/// What a token is.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Kind<'a> {
    Open,
    Close,
    OpenBracket,
    CloseBracket,
    /// A JSON string literal, unescaped.
    Str(String),
    /// A run of characters that is none of the above.
    Word(&'a str),
}

/// A filter text that does not split into tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LexError {
    pub(super) at: usize,
    pub(super) message: &'static str,
}

/// Splits `text` into tokens.
pub(super) fn lex(text: &str) -> Result<Vec<Token<'_>>, LexError> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(at, c)) = chars.peek() {
        let kind = match c {
            _ if is_space(c) => {
                chars.next();
                continue;
            }
            '(' => Kind::Open,
            ')' => Kind::Close,
            '[' => Kind::OpenBracket,
            ']' => Kind::CloseBracket,
            '"' => {
                let end = string_end(text, at).ok_or(LexError {
                    at,
                    message: "an unterminated string",
                })?;
                let literal = text.get(at..end).ok_or(LexError {
                    at,
                    message: "an unterminated string",
                })?;
                let value = serde_json::from_str::<String>(literal).map_err(|_error| LexError {
                    at,
                    message: "a string that is not a JSON string",
                })?;
                while chars.peek().is_some_and(|&(next, _)| next < end) {
                    chars.next();
                }
                tokens.push(Token {
                    kind: Kind::Str(value),
                    at,
                });
                continue;
            }
            _ => {
                let mut end = text.len();
                while let Some(&(next, n)) = chars.peek() {
                    if is_space(n) || matches!(n, '(' | ')' | '[' | ']' | '"') {
                        end = next;
                        break;
                    }
                    chars.next();
                }
                let word = text.get(at..end).ok_or(LexError {
                    at,
                    message: "a word that is not text",
                })?;
                tokens.push(Token {
                    kind: Kind::Word(word),
                    at,
                });
                continue;
            }
        };
        chars.next();
        tokens.push(Token { kind, at });
    }
    Ok(tokens)
}

/// The separators of the grammar: `SP` (RFC 7644 Figure 1), read liberally
/// as any ASCII whitespace.
const fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Returns the byte offset just past the quote that closes the string opened
/// at `start`.
fn string_end(text: &str, start: usize) -> Option<usize> {
    let body = start + '"'.len_utf8();
    let mut escaped = false;
    for (at, c) in text.get(body..)?.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(body + at + '"'.len_utf8());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Kind, lex};

    fn kinds(text: &str) -> Vec<Kind<'_>> {
        lex(text)
            .expect("lexes")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn a_quote_ends_a_word() {
        assert_eq!(
            kinds(r#"value eq"x""#),
            [
                Kind::Word("value"),
                Kind::Word("eq"),
                Kind::Str("x".to_owned())
            ]
        );
    }

    #[test]
    fn brackets_and_dots_split_a_value_path() {
        assert_eq!(
            kinds(r#"emails[type eq "work"].value"#),
            [
                Kind::Word("emails"),
                Kind::OpenBracket,
                Kind::Word("type"),
                Kind::Word("eq"),
                Kind::Str("work".to_owned()),
                Kind::CloseBracket,
                Kind::Word(".value"),
            ]
        );
    }

    #[test]
    fn a_string_is_unescaped_as_json() {
        assert_eq!(kinds(r#""a\"bé""#), [Kind::Str("a\"b\u{e9}".to_owned())]);
        assert!(lex(r#""unterminated"#).is_err());
        assert!(
            lex("\"tab\there\"").is_err(),
            "RFC 8259 §7: control characters are escaped"
        );
    }

    #[test]
    fn multi_byte_text_splits_on_character_boundaries() {
        assert_eq!(
            kinds("naïve eq \"é\""),
            [
                Kind::Word("naïve"),
                Kind::Word("eq"),
                Kind::Str("é".to_owned())
            ]
        );
    }
}

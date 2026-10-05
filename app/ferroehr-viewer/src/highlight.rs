// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Syntax tokenizer for the shared document viewer — pure Rust, hand-rolled.
//!
//! The viewer ships ZERO authored JavaScript, so a browser highlighter is not
//! an option, and a general highlighter crate would drag `regex`/grammar
//! machinery into the WASM bundle for two grammars we already know: canonical
//! JSON (the Simplified Formats and Web Templates are JSON too) and canonical
//! XML (compositions, operational templates).
//!
//! Two properties make the output hydration-safe and honest:
//!
//! * **Total** — concatenating every token's text reproduces the input byte for
//!   byte (asserted by `tokens_reproduce_the_input`), so the pane still shows
//!   the exact wire document; a body the grammar does not fit degrades to plain
//!   text instead of losing characters.
//! * **Deterministic** — a pure function of the input string: no timestamps, no
//!   randomness, no locale-sensitive comparison, one linear pass. The server
//!   pass and client hydration therefore emit identical markup.

use std::iter::Peekable;
use std::str::Chars;

/// Which grammar a document body is tokenized as, decided from its first
/// non-whitespace character rather than from a media type.
///
/// The pane is handed a string, and FLAT / STRUCTURED / Web Template /
/// stored-query bodies all arrive through the same prop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// Canonical JSON and every JSON-shaped simplified format.
    Json,
    /// Canonical XML (also the operational-template XML).
    Xml,
    /// Anything else (AQL, an error body, free text) — not highlighted.
    Plain,
}

impl Language {
    /// Detect the grammar of `body`.
    #[must_use]
    pub fn detect(body: &str) -> Self {
        match body.trim_start().chars().next() {
            Some('{' | '[') => Self::Json,
            Some('<') => Self::Xml,
            _ => Self::Plain,
        }
    }
}

/// The semantic class of one token — what the pane turns into a CSS class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Unclassified text: whitespace, XML character data, stray input.
    Plain,
    /// A JSON object member name (a string whose next significant character
    /// is `:`).
    Key,
    /// A quoted string: a JSON string value or an XML attribute value.
    Str,
    /// A JSON number.
    Number,
    /// A JSON literal keyword (`true`, `false`, `null`).
    Keyword,
    /// Structural punctuation: `{}[],:` in JSON, the angle brackets, slashes
    /// and `=` of XML markup.
    Punctuation,
    /// An XML element name.
    Tag,
    /// An XML attribute name.
    Attribute,
    /// An XML comment, DOCTYPE declaration or CDATA section.
    Comment,
}

/// One token: its class plus the exact input text it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token's semantic class.
    pub kind: TokenKind,
    /// The verbatim input slice this token covers.
    pub text: String,
}

/// Bodies longer than this are not tokenized at all — a single
/// [`TokenKind::Plain`] token carries the whole text.
///
/// A multi-megabyte document would otherwise emit hundreds of thousands of DOM
/// nodes that the browser has to hydrate one by one. The threshold is a byte
/// length of the body, so server and client always make the same decision.
pub const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;

/// Tokenize `body` for display.
///
/// An empty body yields no tokens; an oversized one (see
/// [`MAX_HIGHLIGHT_BYTES`]) or a body with no recognized grammar yields exactly
/// one plain token.
#[must_use]
pub fn tokenize(body: &str) -> Vec<Token> {
    if body.is_empty() {
        return Vec::new();
    }
    if body.len() > MAX_HIGHLIGHT_BYTES {
        return vec![plain(body)];
    }
    match Language::detect(body) {
        Language::Json => tokenize_json(body),
        Language::Xml => tokenize_xml(body),
        Language::Plain => vec![plain(body)],
    }
}

/// The whole input as one unhighlighted token.
fn plain(body: &str) -> Token {
    Token {
        kind: TokenKind::Plain,
        text: body.to_owned(),
    }
}

/// Append a token, coalescing runs so a pretty-printed document costs roughly
/// one span per syntactic item instead of one per character class: adjacent
/// same-kind tokens merge, and whitespace merges into an adjacent punctuation
/// run (indentation carries no colour of its own).
fn push(out: &mut Vec<Token>, kind: TokenKind, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.kind == kind {
            last.text.push_str(&text);
            return;
        }
        let blank = |t: &str| t.chars().all(char::is_whitespace);
        if kind == TokenKind::Plain && last.kind == TokenKind::Punctuation && blank(text.as_str()) {
            last.text.push_str(&text);
            return;
        }
        if kind == TokenKind::Punctuation
            && last.kind == TokenKind::Plain
            && blank(last.text.as_str())
        {
            last.kind = TokenKind::Punctuation;
            last.text.push_str(&text);
            return;
        }
    }
    out.push(Token { kind, text });
}

/// Consume and return the leading run of characters `accept`s.
fn take_while(chars: &mut Peekable<Chars<'_>>, accept: impl Fn(char) -> bool) -> String {
    let mut text = String::new();
    while let Some(&c) = chars.peek() {
        if !accept(c) {
            break;
        }
        text.push(c);
        chars.next();
    }
    text
}

/// Consume a quoted run starting at the current quote character, honouring
/// backslash escapes; the returned text includes both quotes (or stops at the
/// end of input for an unterminated literal).
fn take_quoted(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut text = String::new();
    let Some(quote) = chars.next() else {
        return text;
    };
    text.push(quote);
    let mut escaped = false;
    for c in chars.by_ref() {
        text.push(c);
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == quote {
            break;
        }
    }
    text
}

/// Consume up to and including `terminator` (or to the end of input).
fn take_until_inclusive(chars: &mut Peekable<Chars<'_>>, terminator: &str) -> String {
    let mut text = String::new();
    for c in chars.by_ref() {
        text.push(c);
        if text.ends_with(terminator) {
            break;
        }
    }
    text
}

/// Consume exactly one character.
fn take_one(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut text = String::new();
    if let Some(c) = chars.next() {
        text.push(c);
    }
    text
}

/// The next non-whitespace character without consuming anything.
fn next_significant(chars: &Peekable<Chars<'_>>) -> Option<char> {
    chars.clone().find(|c| !c.is_whitespace())
}

/// Whether the remaining input starts with `prefix` (no allocation).
fn starts_with(chars: &Peekable<Chars<'_>>, prefix: &str) -> bool {
    chars
        .clone()
        .take(prefix.chars().count())
        .eq(prefix.chars())
}

/// JSON structural punctuation.
fn is_json_punctuation(c: char) -> bool {
    matches!(c, '{' | '}' | '[' | ']' | ',' | ':')
}

/// A character that can appear inside a JSON number.
fn is_number_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')
}

/// A character that can appear in an XML element or attribute name (including
/// namespace prefixes).
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, ':' | '_' | '-' | '.')
}

/// Tokenize a JSON body: member names are distinguished from string values by
/// the next significant character being `:`.
fn tokenize_json(body: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == '"' {
            let text = take_quoted(&mut chars);
            let kind = if next_significant(&chars) == Some(':') {
                TokenKind::Key
            } else {
                TokenKind::Str
            };
            push(&mut out, kind, text);
        } else if c.is_whitespace() {
            let text = take_while(&mut chars, char::is_whitespace);
            push(&mut out, TokenKind::Plain, text);
        } else if is_json_punctuation(c) {
            let text = take_while(&mut chars, is_json_punctuation);
            push(&mut out, TokenKind::Punctuation, text);
        } else if c == '-' || c.is_ascii_digit() {
            let text = take_while(&mut chars, is_number_char);
            push(&mut out, TokenKind::Number, text);
        } else if c.is_ascii_alphabetic() {
            let text = take_while(&mut chars, |c| c.is_ascii_alphabetic());
            push(&mut out, TokenKind::Keyword, text);
        } else {
            let text = take_one(&mut chars);
            push(&mut out, TokenKind::Plain, text);
        }
    }
    out
}

/// Tokenize an XML body: character data outside markup stays plain; inside a
/// tag the element name, attribute names and quoted values are classified.
fn tokenize_xml(body: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c != '<' {
            let text = take_while(&mut chars, |c| c != '<');
            push(&mut out, TokenKind::Plain, text);
            continue;
        }
        if starts_with(&chars, "<!--") {
            let text = take_until_inclusive(&mut chars, "-->");
            push(&mut out, TokenKind::Comment, text);
            continue;
        }
        if starts_with(&chars, "<![CDATA[") {
            let text = take_until_inclusive(&mut chars, "]]>");
            push(&mut out, TokenKind::Comment, text);
            continue;
        }
        if starts_with(&chars, "<!") {
            let text = take_until_inclusive(&mut chars, ">");
            push(&mut out, TokenKind::Comment, text);
            continue;
        }
        let opener = take_while(&mut chars, |c| matches!(c, '<' | '/' | '?'));
        push(&mut out, TokenKind::Punctuation, opener);
        let name = take_while(&mut chars, is_name_char);
        push(&mut out, TokenKind::Tag, name);
        tokenize_xml_tag_body(&mut chars, &mut out);
    }
    out
}

/// Tokenize the inside of one XML tag, up to and including its closing `>`.
fn tokenize_xml_tag_body(chars: &mut Peekable<Chars<'_>>, out: &mut Vec<Token>) {
    while let Some(&c) = chars.peek() {
        if c == '>' {
            let text = take_one(chars);
            push(out, TokenKind::Punctuation, text);
            return;
        }
        if c.is_whitespace() {
            let text = take_while(chars, char::is_whitespace);
            push(out, TokenKind::Plain, text);
        } else if c == '"' || c == '\'' {
            let text = take_quoted(chars);
            push(out, TokenKind::Str, text);
        } else if is_name_char(c) {
            let text = take_while(chars, is_name_char);
            push(out, TokenKind::Attribute, text);
        } else {
            // `=`, `/`, `?` and anything else a malformed tag carries: one
            // character at a time, so the scan always advances.
            let text = take_one(chars);
            push(out, TokenKind::Punctuation, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::highlight::{Language, MAX_HIGHLIGHT_BYTES, Token, TokenKind, tokenize};

    /// Every token of `kind`, concatenated in order.
    fn text_of(tokens: &[Token], kind: TokenKind) -> String {
        tokens
            .iter()
            .filter(|t| t.kind == kind)
            .map(|t| t.text.as_str())
            .collect()
    }

    #[test]
    fn language_detection_uses_the_first_significant_character() {
        assert_eq!(Language::detect("  {\"a\":1}"), Language::Json);
        assert_eq!(Language::detect("[1,2]"), Language::Json);
        assert_eq!(Language::detect("\n<?xml version=\"1.0\"?>"), Language::Xml);
        assert_eq!(
            Language::detect("SELECT c FROM COMPOSITION c"),
            Language::Plain
        );
        assert_eq!(Language::detect(""), Language::Plain);
    }

    #[test]
    fn json_keys_strings_numbers_and_keywords_are_classified() {
        let body = "{\n  \"_type\": \"DV_QUANTITY\",\n  \"magnitude\": -72.5,\n  \"ok\": true,\n  \"note\": null\n}";
        let tokens = tokenize(body);
        assert_eq!(
            text_of(&tokens, TokenKind::Key),
            "\"_type\"\"magnitude\"\"ok\"\"note\""
        );
        assert_eq!(text_of(&tokens, TokenKind::Str), "\"DV_QUANTITY\"");
        assert_eq!(text_of(&tokens, TokenKind::Number), "-72.5");
        assert_eq!(text_of(&tokens, TokenKind::Keyword), "truenull");
    }

    #[test]
    fn a_json_string_containing_a_colon_is_still_a_value() {
        let tokens = tokenize("{\"uid\": \"a::1\"}");
        assert_eq!(text_of(&tokens, TokenKind::Key), "\"uid\"");
        assert_eq!(text_of(&tokens, TokenKind::Str), "\"a::1\"");
    }

    #[test]
    fn an_escaped_quote_does_not_end_a_json_string() {
        let tokens = tokenize("{\"a\": \"x\\\"y\", \"b\": 1}");
        assert_eq!(text_of(&tokens, TokenKind::Str), "\"x\\\"y\"");
        assert_eq!(text_of(&tokens, TokenKind::Key), "\"a\"\"b\"");
    }

    #[test]
    fn xml_tags_attributes_values_and_comments_are_classified() {
        let body = "<!-- gen --><composition xmlns=\"http://schemas.openehr.org/v1\" archetype_node_id='at0000'>\n  <name>Pulse</name>\n</composition>";
        let tokens = tokenize(body);
        assert_eq!(
            text_of(&tokens, TokenKind::Tag),
            "compositionnamenamecomposition"
        );
        assert_eq!(
            text_of(&tokens, TokenKind::Attribute),
            "xmlnsarchetype_node_id"
        );
        assert_eq!(
            text_of(&tokens, TokenKind::Str),
            "\"http://schemas.openehr.org/v1\"'at0000'"
        );
        assert_eq!(text_of(&tokens, TokenKind::Comment), "<!-- gen -->");
        assert!(text_of(&tokens, TokenKind::Plain).contains("Pulse"));
    }

    #[test]
    fn tokens_reproduce_the_input() {
        // The load-bearing invariant: the pane must still show the byte-exact
        // wire document, and SSR/hydration must agree on every character.
        let bodies = [
            "{\n  \"_type\": \"COMPOSITION\",\n  \"content\": [ {\"a\": [1, 2.5e3, false]} ]\n}",
            "<?xml version=\"1.0\"?>\n<x a=\"1\" b='2'><!--c--><![CDATA[<raw>]]>text &amp; more</x>",
            "SELECT c/uid/value FROM COMPOSITION c WHERE c/name/value = 'Pulse'",
            "{\"unterminated\": \"oops",
            "<broken attr=",
            "",
        ];
        for body in bodies {
            let joined: String = tokenize(body).into_iter().map(|t| t.text).collect();
            assert_eq!(joined, body, "tokenization lost or rewrote input");
        }
    }

    #[test]
    fn tokenization_is_stable_across_calls() {
        // Hydration safety in one assertion: same input, same output, always.
        let body = "{\"a\": 1, \"b\": [true, null]}";
        assert_eq!(tokenize(body), tokenize(body));
    }

    #[test]
    fn an_oversized_body_is_left_as_one_plain_token() {
        let body = format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_HIGHLIGHT_BYTES));
        let tokens = tokenize(&body);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens.first().map(|t| t.kind), Some(TokenKind::Plain));
        assert_eq!(tokens.into_iter().map(|t| t.text).collect::<String>(), body);
    }

    #[test]
    fn indentation_merges_into_the_punctuation_run() {
        // One span per syntactic item, not one per character class.
        let tokens = tokenize("{\n  \"a\": 1,\n  \"b\": 2\n}");
        assert!(
            tokens.len() <= 9,
            "expected a coalesced token stream, got {tokens:#?}"
        );
    }
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `_default` value of an object node (`ADL2/master06-default_values.adoc`).
//!
//! `cadl2.g4` reads it as an optionally typed ODIN object block or an
//! `INCLUDED_LANGUAGE_FRAGMENT` (`base_lexer.g4`), the `(json) <# … #>` form
//! §Syntax shows. Both land in `C_OBJECT.default_value` as canonical JSON.

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use crate::error::SyntaxErrorCode;
use crate::odin::{is_interval, odin_to_json};
use crate::parse::{PResult, Parser};
use openehr_lang::v1_1::lexer::Token;

impl Parser<'_> {
    /// `default_value : SYM_DEFAULT SYM_EQ ( object_value_block |
    /// INCLUDED_LANGUAGE_FRAGMENT )` (`cadl2.g4`, adl-antlr `d85ef269b`,
    /// SPECPR-146): an optionally typed ODIN block, parsed by
    /// `openehr_lang::v1_1::odin`, or a `(syntax) <# … #>` block, each read
    /// into canonical JSON.
    ///
    /// # Errors
    /// A malformed block, or a block in a syntax with no canonical JSON form.
    pub(super) fn parse_default_value(&mut self) -> PResult<serde_json::Value> {
        self.pos += 1; // SYM_DEFAULT (`_default`)
        self.expect(
            |t| matches!(t, Token::SymEq),
            SyntaxErrorCode::Sadf,
            "expecting '=' after '_default'",
        )?;
        if let Some(syntax) = self.included_language_head() {
            return self.parse_included_language_fragment(&syntax);
        }
        // The optional RM-type cast of the typed form
        // `_default = (DV_CODED_TEXT) < … >` (`master06-default_values.adoc`
        // §Syntax); it lands as the value's `_type` in the canonical JSON.
        let mut cast: Option<String> = None;
        if self.eat(|t| matches!(t, Token::LParen)) {
            let Some(Token::AlphaUcId(name)) = self.peek() else {
                return self.err(
                    SyntaxErrorCode::Sadf,
                    "expecting an RM type name in the '_default = (TYPE)' cast",
                );
            };
            cast = Some(name.clone());
            self.pos += 1;
            self.expect(
                |t| matches!(t, Token::RParen),
                SyntaxErrorCode::Sadf,
                "expecting ')' closing the '_default' type cast",
            )?;
        }
        // The whole `<…>` block, delimiters included, is what the ODIN reader
        // is handed: `odin_text` is `attr_vals | object_value_block`
        // (`LANG/docs/odin/master05-content.adoc` §General Structure), so a
        // block whose content is a bare leaf or interval (`<|0..5|>`, `<5>`)
        // only parses with its delimiters in place.
        let start_byte = self.cur_span().start;
        self.expect(
            |t| matches!(t, Token::SymLt),
            SyntaxErrorCode::Sadf,
            "expecting '<' opening the default value",
        )?;
        // Capture the balanced `<…>` span by `<`/`>` depth. A `<` or `>` inside
        // an interval is a bound operator, not a block delimiter
        // (`LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered
        // Primitive Types: `|>N..<M|`), so the `|…|` pairs are tracked and
        // their contents skipped.
        let mut depth = 1usize;
        let mut end_byte = start_byte;
        let mut in_interval = false;
        while depth > 0 {
            match self.bump() {
                Some(Token::SymIvlDelim) => in_interval = !in_interval,
                Some(Token::SymLt) if !in_interval => depth += 1,
                Some(Token::SymGt) if !in_interval => {
                    depth -= 1;
                    if depth == 0 {
                        end_byte = self.span_at(self.pos - 1).end;
                    }
                }
                Some(_) => {}
                None => return self.err(SyntaxErrorCode::Sadf, "unterminated default value block"),
            }
        }
        let text = self.src.get(start_byte..end_byte).unwrap_or_default();
        match openehr_lang::v1_1::odin::parse(text) {
            Ok(v) => {
                let mut json = match odin_to_json(&v) {
                    Ok(json) => json,
                    Err(e) => {
                        self.push(SyntaxErrorCode::Sdinv, e.to_string(), start_byte..end_byte);
                        return Err(());
                    }
                };
                // An interval already carries its own canonical `_type`
                // (`Point_interval`/`Proper_interval`); the cast on one names
                // the generic slot type (`Interval<Quantity>`), which is not a
                // canonical-JSON class tag.
                if let Some(t) = cast
                    && !is_interval(&v)
                    && let serde_json::Value::Object(m) = &mut json
                {
                    m.insert("_type".to_owned(), serde_json::Value::String(t));
                }
                Ok(json)
            }
            Err(e) => {
                self.push(
                    SyntaxErrorCode::Sadf,
                    format!("invalid ODIN default value: {e}"),
                    start_byte..end_byte,
                );
                Err(())
            }
        }
    }

    /// The syntax name of an `INCLUDED_LANGUAGE_FRAGMENT` starting at the
    /// cursor: `'(' ALPHANUM_CHAR+ ')' [ \t\r\n]* '<#' .*? '#>'`
    /// (`base_lexer.g4`). The lexer reads it as `(`, the name, `)` and the
    /// `<# … #>` block, with only white space allowed before the block.
    fn included_language_head(&self) -> Option<String> {
        let (Some(Token::LParen), Some(Token::AlphaUcId(name) | Token::AlphaLcId(name))) =
            (self.peek(), self.peek_at(1))
        else {
            return None;
        };
        if !matches!(self.peek_at(2), Some(Token::RParen))
            || !matches!(self.peek_at(3), Some(Token::PlugInBlock(_)))
            || !name.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return None;
        }
        let gap = self.span_at(self.pos + 2).end..self.span_at(self.pos + 3).start;
        self.src
            .get(gap)
            .is_some_and(|between| {
                between
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
            })
            .then(|| name.clone())
    }

    /// Reads the `(syntax) <# … #>` block the cursor stands on.
    ///
    /// NOTE: `ADL2/master06-default_values.adoc` §Syntax names ODIN and JSON
    /// as the default value syntaxes; a block in any other has no canonical
    /// JSON form here and is refused (our own design).
    fn parse_included_language_fragment(&mut self, syntax: &str) -> PResult<serde_json::Value> {
        let span = self.span_at(self.pos + 3);
        let Some(Token::PlugInBlock(raw)) = self.peek_at(3).cloned() else {
            return self.err(SyntaxErrorCode::Sadf, "expecting a '<# … #>' block");
        };
        self.pos += 4;
        if !syntax.eq_ignore_ascii_case("json") {
            self.push(
                SyntaxErrorCode::Sadf,
                format!(
                    "a '_default' block in syntax '{syntax}' cannot be read; ADL2 default \
                     values are ODIN or JSON"
                ),
                span,
            );
            return Err(());
        }
        let body = raw
            .strip_prefix("<#")
            .and_then(|inner| inner.strip_suffix("#>"))
            .unwrap_or(&raw);
        match serde_json::from_str(body) {
            Ok(value) => Ok(value),
            Err(e) => {
                self.push(
                    SyntaxErrorCode::Sdinv,
                    format!("invalid JSON default value: {e}"),
                    span,
                );
                Err(())
            }
        }
    }
}

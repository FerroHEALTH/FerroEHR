// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cADL value machinery every primitive production shares.
//!
//! Interval kind-classification, value lists with their `, ...` continuation
//! marker, the `|…|` interval forms of `odin_values.g4` and their endpoints
//! (including the `infinity` / `-infinity` / `*` unbounded markers), the
//! signed numeric readers, and the `CadlValue` value-kind trait that types
//! them. One `impl` block over the `Parser` state of [`crate::parse`].

use openehr_base::prelude::{Interval, Iso8601Date, Iso8601DateTime, Iso8601Duration, Iso8601Time};

use crate::aom::build::{point_interval, proper_interval};
use crate::error::SyntaxErrorCode;
use crate::parse::patterns::iso_has_timezone;
use crate::parse::{PResult, Parser};
use openehr_lang::v1_1::lexer::Token;

// ── value lists, intervals, endpoints ─────────────────────────────────────
impl Parser<'_> {
    /// Peek inside a `|…|` interval to classify its value kind by the first
    /// endpoint token (skipping relational operators, signs, and the unbounded
    /// endpoint markers — `|-infinity..5.0|` is typed by its UPPER endpoint).
    pub(crate) fn classify_bar_kind(&mut self) -> PResult<PrimKind> {
        let mut i = self.pos + 1;
        while matches!(
            self.toks.get(i).map(|s| &s.token),
            Some(
                Token::SymGt
                    | Token::SymGe
                    | Token::SymLt
                    | Token::SymLe
                    | Token::SymPlus
                    | Token::SymMinus
                    | Token::SymInfinity
                    | Token::SymStar
                    | Token::SymIvlSep
            )
        ) {
            i += 1;
        }
        match self.toks.get(i).map(|s| &s.token) {
            Some(Token::Integer(_)) => Ok(PrimKind::Integer),
            Some(Token::Real(_)) => Ok(PrimKind::Real),
            Some(Token::Iso8601Date(_)) => Ok(PrimKind::Date),
            Some(Token::Iso8601Time(_)) => Ok(PrimKind::Time),
            Some(Token::Iso8601DateTime(_)) => Ok(PrimKind::DateTime),
            Some(Token::Iso8601Duration(_)) => Ok(PrimKind::Duration),
            _ => self.err(SyntaxErrorCode::Sccog, "unrecognised interval value type"),
        }
    }

    /// Consume a trailing list-continuation marker (`, ...`) — the comma is
    /// already eaten by the caller, so this only tests for the `...`.
    ///
    /// Every ODIN list production ends with the alternative
    /// `',' SYM_LIST_CONTINUE` (`odin_values.g4` `string_list_value` and its
    /// siblings), and the cADL 1.4 grammar spells it out for strings as
    /// `c_string_spec : … | string_list_value ',' SYM_LIST_CONTINUE`
    /// (`ADL1.4/master05-cadl.adoc` §Syntax L1244-1249).
    ///
    /// NOTE: the marker adds no member and no openness flag — its only
    /// normative gloss is a LIST INDICATOR (`ADL1.4/master04-dadl` §Data of
    /// any primitive type L686; `LANG/docs/odin/master07-leaf_data` §Lists
    /// L208), no vendored text binds it to AOM 1.4's `list_open`, and the
    /// AOM2 `C_STRING` built here has no such property — so the constraint is
    /// exactly the listed values (inferred openness would silently turn a
    /// stated constraint into "any value").
    pub(crate) fn eat_list_continue(&mut self) -> bool {
        self.eat(|t| matches!(t, Token::SymListContinue))
    }

    /// Parse a comma-separated list of `Interval<V>` items (a value list, an
    /// interval, or a list of intervals — the AOM2 constraint is a flat
    /// `Vec<Interval<V>>` regardless).
    pub(crate) fn parse_value_list<V: CadlValue>(&mut self) -> PResult<Vec<Interval<V>>> {
        let mut out = Vec::new();
        loop {
            out.push(self.parse_value_item::<V>()?);
            if !self.eat(|t| matches!(t, Token::SymComma)) || self.eat_list_continue() {
                break;
            }
        }
        Ok(out)
    }

    /// One `Interval<V>` item: a bare value (point interval) or a `|…|`
    /// interval.
    pub(crate) fn parse_value_item<V: CadlValue>(&mut self) -> PResult<Interval<V>> {
        if matches!(self.peek(), Some(Token::SymIvlDelim)) {
            self.parse_bar_interval::<V>()
        } else {
            let v = V::parse_one(self)?;
            Ok(point_interval(v))
        }
    }

    /// A `|…|` interval (`odin_values.g4` interval forms): two-sided
    /// (`|a..b|`, `|>a..<b|`, `|a>..<b|`), single-relop (`|>a|`,`|<=a|`), point
    /// (`|a|`), or centre±delta (`|a+/-b|`). Either endpoint may be an unbounded
    /// marker (see [`Parser::parse_endpoint`]).
    ///
    /// The exclusive LOWER bound of a two-sided interval has two spellings and
    /// both are accepted: the prefix `>` of the normative grammar
    /// (`odin_values.g4` `integer_interval_value : '|' SYM_GT? integer_value
    /// '..' SYM_LT? integer_value '|'`) and the postfix `>` the ADL 1.4 chapters
    /// write — `ADL1.4/master04-dadl.adoc` §Intervals of Ordered Primitive Types
    /// L611-614 (`|N>..M|` "the two-sided range N > x <= M", `|N>..<M|`) and the
    /// cADL chapter's own worked example `length matches {|0>..<1000|}`
    /// (`ADL1.4/master05-cadl.adoc` §Interval of Integer L769). Accepting both is
    /// a superset that rejects nothing either source admits.
    fn parse_bar_interval<V: CadlValue>(&mut self) -> PResult<Interval<V>> {
        self.check_interval_timezone_symmetry()?;
        self.pos += 1; // opening '|'
        let lower_rel = self.eat_relop();
        let first = self.parse_endpoint::<V>()?;
        // Only a `>` that immediately precedes the `..` is the postfix
        // exclusive-lower marker; anywhere else it is not part of the form.
        let lower_excl_postfix = matches!(self.peek(), Some(Token::SymGt))
            && matches!(self.peek_at(1), Some(Token::SymIvlSep));
        if lower_excl_postfix {
            self.pos += 1;
        }
        let ivl = if self.eat(|t| matches!(t, Token::SymIvlSep)) {
            // Two-sided: [rel] first ['>'] '..' ['<'] upper.
            let upper_excl = self.eat(|t| matches!(t, Token::SymLt));
            let upper = self.parse_endpoint::<V>()?;
            let lower_included =
                first.is_some() && !lower_excl_postfix && !matches!(lower_rel, Some(Relop::Gt));
            let lower_unbounded = first.is_none();
            let upper_unbounded = upper.is_none();
            proper_interval(
                first,
                upper,
                lower_included,
                !upper_excl && !upper_unbounded,
                lower_unbounded,
                upper_unbounded,
            )
        } else if self.eat(|t| matches!(t, Token::SymPlusOrMinus)) {
            let delta = self.parse_endpoint::<V>()?;
            match (&first, &delta) {
                (Some(centre), Some(delta)) => match V::plus_minus(centre, delta) {
                    Some((lo, hi)) => proper_interval(Some(lo), Some(hi), true, true, false, false),
                    // NOTE: `±` on a non-numeric type is not reducible without
                    // RM type context; represented as a point at the centre for
                    // now (rare — not exercised by the primitive corpus).
                    None => point_interval(centre.clone()),
                },
                // An unbounded centre or half-width constrains nothing.
                _ => proper_interval(None, None, false, false, true, true),
            }
        } else {
            // Point `|a|` or single-relop `|>a|`,`|<=a|`.
            match (lower_rel, first) {
                (None, Some(v)) => point_interval(v),
                (Some(Relop::Gt), Some(v)) => {
                    proper_interval(Some(v), None, false, false, false, true)
                }
                (Some(Relop::Ge), Some(v)) => {
                    proper_interval(Some(v), None, true, false, false, true)
                }
                (Some(Relop::Lt), Some(v)) => {
                    proper_interval(None, Some(v), false, false, true, false)
                }
                (Some(Relop::Le), Some(v)) => {
                    proper_interval(None, Some(v), false, true, true, false)
                }
                // A one-sided form whose single endpoint is itself an unbounded
                // marker (`|>=-infinity|`, `|<*|`) constrains nothing at all.
                (_, None) => proper_interval(None, None, false, false, true, true),
            }
        };
        self.expect(
            |t| matches!(t, Token::SymIvlDelim),
            SyntaxErrorCode::Sccog,
            "expecting '|' closing the interval",
        )?;
        Ok(ivl)
    }

    /// The timezone-symmetry rule for a two-sided date/time interval, checked
    /// with the cursor still on the opening `|`.
    ///
    /// `ADL1.4/master05-cadl.adoc` §Intervals L932: "Within any interval
    /// containing two literal date/time values (i.e. not one-sided intervals),
    /// if a timezone is used on one, it must be used on both, to ensure
    /// comparability. The timezones need not be identical." So the rule fires
    /// exactly when the `|…|` window holds TWO literal time / date-time values
    /// whose timezone presence differs; a one-sided form (one value) and a
    /// date interval (dates carry no timezone) are outside its scope.
    ///
    /// NOTE: the openEHR `S*` catalogue
    /// (`ADL2/master04.6-cadl_validity_rules.adoc` §Syntax Validity Rules) has
    /// no dedicated code for this rule, and this crate never invents one — so
    /// the refusal reuses the catalogue code this parser already uses for a bad
    /// time / date-time value in a constraint position (`SCTAV` / `SCDTAV`) and
    /// names the rule in the message.
    fn check_interval_timezone_symmetry(&mut self) -> PResult<()> {
        let mut endpoints: Vec<(bool, SyntaxErrorCode)> = Vec::new();
        let mut i = self.pos + 1;
        while let Some(spanned) = self.toks.get(i) {
            match &spanned.token {
                Token::SymIvlDelim => break,
                Token::Iso8601Time(v) => {
                    endpoints.push((iso_has_timezone(v), SyntaxErrorCode::Sctav));
                }
                Token::Iso8601DateTime(v) => {
                    endpoints.push((iso_has_timezone(v), SyntaxErrorCode::Scdtav));
                }
                _ => {}
            }
            i += 1;
        }
        if let [(lower_tz, code), (upper_tz, _)] = endpoints[..]
            && lower_tz != upper_tz
        {
            let span = self.cur_span().start..self.span_at(i).end;
            self.push(
                code,
                "a two-sided date/time interval must use a timezone on both endpoints or on \
                 neither (the timezones need not be identical)",
                span,
            );
            return Err(());
        }
        Ok(())
    }

    /// One interval endpoint: a value, or an unbounded marker yielding `None`.
    ///
    /// The markers are `infinity`, `-infinity` and `*`. `infinity` is a cADL
    /// keyword in its own right (`ADL1.4/master05-cadl.adoc` §Keywords L50 and
    /// §Symbols L1349 `[Ii][Nn][Ff][Ii][Nn][Ii][Tt][Yy] -> SYM_INFINITY`) and
    /// the chapter's own §Interval of Integer example is
    /// `rate matches {|0..infinity|}  -- allow 0 - infinity, i.e. same as >= 0`
    /// (L771); L761 defers the interval syntax itself to dADL, whose §Intervals
    /// of Ordered Primitive Types lists `infinity` / `-infinity` / `*` as
    /// allowable endpoint values (`ADL1.4/master04-dadl.adoc` L628-643). The
    /// sign carried by `-infinity` is not recorded separately: the side of the
    /// interval the endpoint sits on already fixes the direction — the same
    /// representation `openehr_lang::v1_1::odin` uses for the identical markers.
    fn parse_endpoint<V: CadlValue>(&mut self) -> PResult<Option<V>> {
        if self.eat(|t| matches!(t, Token::SymStar)) {
            return Ok(None);
        }
        let save = self.pos;
        self.eat(|t| matches!(t, Token::SymMinus));
        if self.eat(|t| matches!(t, Token::SymInfinity)) {
            return Ok(None);
        }
        self.pos = save;
        V::parse_one(self).map(Some)
    }

    fn eat_relop(&mut self) -> Option<Relop> {
        let r = match self.peek() {
            Some(Token::SymGt) => Relop::Gt,
            Some(Token::SymGe) => Relop::Ge,
            Some(Token::SymLt) => Relop::Lt,
            Some(Token::SymLe) => Relop::Le,
            _ => return None,
        };
        self.pos += 1;
        Some(r)
    }

    pub(crate) fn parse_signed_int(&mut self, code: SyntaxErrorCode) -> PResult<i32> {
        let neg = self.eat(|t| matches!(t, Token::SymMinus));
        if !neg {
            self.eat(|t| matches!(t, Token::SymPlus));
        }
        match self.peek().cloned() {
            Some(Token::Integer(s)) => {
                self.pos += 1;
                // The lexeme and span are already in the pushed diagnostic; a
                // `ParseIntError`/`TryFromIntError` adds nothing to it.
                let Ok(v) = s.parse::<i64>() else {
                    self.push(
                        code,
                        format!("invalid integer {s:?}"),
                        self.span_at(self.pos - 1),
                    );
                    return Err(());
                };
                let v = if neg { -v } else { v };
                let Ok(narrowed) = i32::try_from(v) else {
                    self.push(
                        code,
                        format!("integer {v} out of range"),
                        self.span_at(self.pos - 1),
                    );
                    return Err(());
                };
                Ok(narrowed)
            }
            _ => self.err(code, "expecting an integer value"),
        }
    }

    pub(crate) fn parse_signed_real(&mut self, code: SyntaxErrorCode) -> PResult<f64> {
        let neg = self.eat(|t| matches!(t, Token::SymMinus));
        if !neg {
            self.eat(|t| matches!(t, Token::SymPlus));
        }
        match self.peek().cloned() {
            Some(Token::Real(s)) => {
                self.pos += 1;
                // The lexeme and span are already in the pushed diagnostic; a
                // `ParseFloatError` adds nothing to it.
                let v = s.parse::<f64>().ok().ok_or_else(|| {
                    self.push(
                        code,
                        format!("invalid real {s:?}"),
                        self.span_at(self.pos - 1),
                    );
                })?;
                Ok(if neg { -v } else { v })
            }
            _ => self.err(code, "expecting a real value (with a decimal point)"),
        }
    }
}

/// A relational operator prefixing an interval endpoint.
#[derive(Debug, Clone, Copy)]
enum Relop {
    Gt,
    Ge,
    Lt,
    Le,
}

/// The primitive kind an interval resolves to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PrimKind {
    Integer,
    Real,
    Date,
    Time,
    DateTime,
    Duration,
}

// ── value-kind trait: parse one endpoint value of type `V` ────────────────
/// A primitive value type that can be parsed from the token stream as an
/// interval endpoint / list element.
pub(crate) trait CadlValue: Clone + Sized {
    /// Parse a single value of this type from the current position.
    fn parse_one(p: &mut Parser<'_>) -> PResult<Self>;
    /// Combine a centre and half-width for the `±` interval form; `None` if not
    /// reducible without RM type context (non-numeric types).
    fn plus_minus(centre: &Self, delta: &Self) -> Option<(Self, Self)>;
}

impl CadlValue for i32 {
    fn parse_one(p: &mut Parser<'_>) -> PResult<Self> {
        p.parse_signed_int(SyntaxErrorCode::Sciav)
    }
    fn plus_minus(centre: &Self, delta: &Self) -> Option<(Self, Self)> {
        Some((centre - delta, centre + delta))
    }
}

impl CadlValue for f64 {
    fn parse_one(p: &mut Parser<'_>) -> PResult<Self> {
        p.parse_signed_real(SyntaxErrorCode::Scrav)
    }
    fn plus_minus(centre: &Self, delta: &Self) -> Option<(Self, Self)> {
        Some((centre - delta, centre + delta))
    }
}

impl CadlValue for Iso8601Date {
    fn parse_one(p: &mut Parser<'_>) -> PResult<Self> {
        match p.peek().cloned() {
            Some(Token::Iso8601Date(v)) => {
                p.pos += 1;
                Ok(Iso8601Date { value: v })
            }
            _ => p.err(SyntaxErrorCode::Scdav, "expecting an ISO8601 date value"),
        }
    }
    fn plus_minus(_: &Self, _: &Self) -> Option<(Self, Self)> {
        None
    }
}

impl CadlValue for Iso8601Time {
    fn parse_one(p: &mut Parser<'_>) -> PResult<Self> {
        match p.peek().cloned() {
            Some(Token::Iso8601Time(v)) => {
                p.pos += 1;
                Ok(Iso8601Time { value: v })
            }
            _ => p.err(SyntaxErrorCode::Sctav, "expecting an ISO8601 time value"),
        }
    }
    fn plus_minus(_: &Self, _: &Self) -> Option<(Self, Self)> {
        None
    }
}

impl CadlValue for Iso8601DateTime {
    fn parse_one(p: &mut Parser<'_>) -> PResult<Self> {
        match p.peek().cloned() {
            Some(Token::Iso8601DateTime(v)) => {
                p.pos += 1;
                Ok(Iso8601DateTime { value: v })
            }
            _ => p.err(
                SyntaxErrorCode::Scdtav,
                "expecting an ISO8601 date/time value",
            ),
        }
    }
    fn plus_minus(_: &Self, _: &Self) -> Option<(Self, Self)> {
        None
    }
}

impl CadlValue for Iso8601Duration {
    fn parse_one(p: &mut Parser<'_>) -> PResult<Self> {
        match p.peek().cloned() {
            Some(Token::Iso8601Duration(v)) => {
                p.pos += 1;
                Ok(Iso8601Duration { value: v })
            }
            _ => p.err(
                SyntaxErrorCode::Scduav,
                "expecting an ISO8601 duration value",
            ),
        }
    }
    fn plus_minus(_: &Self, _: &Self) -> Option<(Self, Self)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
        CComplexObject, CComplexObjectData,
    };
    use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

    use crate::error::SyntaxErrorCode;
    use crate::parse::{Dialect, parse_definition_body};

    fn parse(body: &str) -> CComplexObject {
        parse_definition_body(body, Dialect::Adl2).unwrap_or_else(|e| panic!("parse failed: {e:?}"))
    }

    fn data(cco: &CComplexObject) -> &CComplexObjectData {
        match cco {
            CComplexObject::CComplexObject(d) => d,
            CComplexObject::CArchetypeRoot(_) => panic!("expected plain complex object"),
        }
    }

    /// The `(lower, upper, lower_included, upper_included)` of the single
    /// interval an integer constraint carries; an unbounded endpoint is `None`.
    fn int_bounds(o: &CObject) -> (Option<f64>, Option<f64>, bool, bool) {
        match o {
            CObject::CInteger(ci) => crate::aom::interval::interval_bounds_f64(
                &ci.constraint.as_deref().unwrap_or_default()[0],
            ),
            other => panic!("expected CInteger, got {other:?}"),
        }
    }

    /// `ADL1.4/master05-cadl.adoc` §Interval of Integer L771 —
    /// `rate matches {|0..infinity|}` — plus the `-infinity` and `*` endpoint
    /// spellings of `master04-dadl` §Intervals of Ordered Primitive Types
    /// L625-630.
    #[test]
    fn infinity_endpoints_are_unbounded() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             a matches {|0..infinity|}\n\
             b matches {|-infinity..5|}\n\
             c matches {|0..*|}\n\
             d matches {|0..10|}\n\
             }",
        );
        let d = data(&cco);
        assert_eq!(
            int_bounds(
                &d.attributes.as_deref().unwrap_or_default()[0]
                    .children
                    .as_deref()
                    .unwrap_or_default()[0]
            ),
            (Some(0.0), None, true, false)
        );
        assert_eq!(
            int_bounds(
                &d.attributes.as_deref().unwrap_or_default()[1]
                    .children
                    .as_deref()
                    .unwrap_or_default()[0]
            ),
            (None, Some(5.0), false, true)
        );
        assert_eq!(
            int_bounds(
                &d.attributes.as_deref().unwrap_or_default()[2]
                    .children
                    .as_deref()
                    .unwrap_or_default()[0]
            ),
            (Some(0.0), None, true, false)
        );
        assert_eq!(
            int_bounds(
                &d.attributes.as_deref().unwrap_or_default()[3]
                    .children
                    .as_deref()
                    .unwrap_or_default()[0]
            ),
            (Some(0.0), Some(10.0), true, true)
        );
    }

    /// Both spellings of the exclusive lower bound denote the same interval:
    /// the prefix `>` of `odin_values.g4` and the postfix `>` of
    /// `ADL1.4/master04-dadl.adoc` §Intervals L611-614 (used by `master05` L769
    /// as `length matches {|0>..<1000|}`).
    #[test]
    fn both_exclusive_lower_spellings_agree() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             a matches {|>0..<1000|}\n\
             b matches {|0>..<1000|}\n\
             c matches {|0>..1000|}\n\
             }",
        );
        let d = data(&cco);
        assert_eq!(
            int_bounds(
                &d.attributes.as_deref().unwrap_or_default()[0]
                    .children
                    .as_deref()
                    .unwrap_or_default()[0]
            ),
            (Some(0.0), Some(1000.0), false, false)
        );
        assert_eq!(
            int_bounds(
                &d.attributes.as_deref().unwrap_or_default()[1]
                    .children
                    .as_deref()
                    .unwrap_or_default()[0]
            ),
            (Some(0.0), Some(1000.0), false, false)
        );
        assert_eq!(
            int_bounds(
                &d.attributes.as_deref().unwrap_or_default()[2]
                    .children
                    .as_deref()
                    .unwrap_or_default()[0]
            ),
            (Some(0.0), Some(1000.0), false, true)
        );
    }

    /// The `, ...` list-continuation marker (`ADL1.4/master05-cadl.adoc` §Syntax
    /// L1244-1249 for strings, `master04-dadl` §Syntax L985-1160 for every other
    /// primitive list) is a list INDICATOR, not a member and not an openness
    /// flag — see [`Parser::eat_list_continue`].
    #[test]
    fn list_continuation_adds_no_member() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             a matches {\"en\", ...}\n\
             b matches {1, 2, ...}\n\
             c matches {True, ...}\n\
             }",
        );
        let d = data(&cco);
        match &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => assert_eq!(cs.constraint, Some(vec!["en".to_owned()])),
            other => panic!("expected CString, got {other:?}"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[1]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CInteger(ci) => assert_eq!(ci.constraint.as_ref().map_or(0, Vec::len), 2),
            other => panic!("expected CInteger, got {other:?}"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[2]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CBoolean(cb) => assert_eq!(cb.constraint, Some(vec![true])),
            other => panic!("expected CBoolean, got {other:?}"),
        }
    }

    /// `ADL1.4/master05-cadl.adoc` §Intervals L932: a two-sided date/time
    /// interval must carry a timezone on both endpoints or on neither; a
    /// one-sided interval is outside the rule by its own wording.
    #[test]
    fn interval_timezone_symmetry_is_enforced() {
        let asymmetric = parse_definition_body(
            "WHOLE[id1] matches {\n d matches {|2004-05-20T00:00:00Z..2005-05-19T23:59:59|}\n}",
            Dialect::Adl2,
        )
        .expect_err("an asymmetric-timezone interval must be refused");
        assert!(
            asymmetric.iter().any(|e| e.code == SyntaxErrorCode::Scdtav),
            "expected SCDTAV, got {:?}",
            asymmetric.iter().map(|e| e.code).collect::<Vec<_>>()
        );
        let time_asymmetric = parse_definition_body(
            "WHOLE[id1] matches {\n t matches {|09:30:00+0200..10:30:00|}\n}",
            Dialect::Adl2,
        )
        .expect_err("an asymmetric-timezone time interval must be refused");
        assert!(
            time_asymmetric
                .iter()
                .any(|e| e.code == SyntaxErrorCode::Sctav),
            "expected SCTAV, got {:?}",
            time_asymmetric.iter().map(|e| e.code).collect::<Vec<_>>()
        );
        // Both endpoints with a timezone, both without, and the one-sided form
        // all parse.
        parse(
            "WHOLE[id1] matches {\n\
             a matches {|2004-05-20T00:00:00Z..2005-05-19T23:59:59Z|}\n\
             b matches {|2004-05-20T00:00:00..2005-05-19T23:59:59|}\n\
             c matches {|>09:30:00+0200|}\n\
             }",
        );
    }
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! ODIN parser — a `chumsky` parser over the shared
//! [`crate::v1_0::lexer`] token stream under its ODIN reading
//! ([`crate::v1_0::lexer::lex_odin`]), transcribed from `odin.g4` /
//! `odin_values.g4`. Produces an [`OdinValue`] tree.

use chumsky::DefaultExpected;
use chumsky::error::{Error, LabelError};
use chumsky::prelude::*;
use chumsky::util::MaybeRef;
use indexmap::IndexMap;

use super::{OdinInterval, OdinKey, OdinValue};
use crate::v1_0::lexer::{Spanned, Token};

// The chumsky extra-parameter alias. `chumsky::extra::Err` stays fully
// qualified deliberately: shortening it to `Err<Failure>` would make this
// alias refer to itself.
#[expect(
    unused_qualifications,
    reason = "the local alias shadows the name being qualified — dropping the path makes the definition self-referential"
)]
type Err<'a> = chumsky::extra::Err<Failure>;

/// A parse failure: a plain syntactic conflict, or the typed
/// duplicate-sibling-attribute violation.
#[derive(Debug, Clone)]
pub(super) enum Failure {
    /// An unexpected token at the given token-index span.
    Syntax(SimpleSpan),
    /// VDATU: two sibling attributes share a name
    /// (`LANG/docs/odin/master05-content` §General Structure).
    DuplicateAttribute {
        /// The repeated attribute name.
        name: String,
        /// Token-index span of the offending (second) occurrence.
        span: SimpleSpan,
    },
    /// VDOBU: two sibling container objects share a key
    /// (`LANG/docs/odin/master05-content` §Container Objects).
    DuplicateKey {
        /// The repeated key, in its path-predicate rendering.
        key: String,
        /// Token-index span of the offending (second) occurrence.
        span: SimpleSpan,
    },
}

impl Failure {
    /// The token-index span the failure points at.
    fn span(&self) -> SimpleSpan {
        match self {
            Self::Syntax(s)
            | Self::DuplicateAttribute { span: s, .. }
            | Self::DuplicateKey { span: s, .. } => *s,
        }
    }

    /// Keep whichever failure carries the most information: a typed
    /// uniqueness violation (VDATU/VDOBU) always outranks a bare syntax
    /// conflict (the enclosing `choice`s retry other alternatives after it,
    /// and each of those contributes a syntax error at the same position).
    fn preferred(self, other: Self) -> Self {
        match (&self, &other) {
            (Self::DuplicateAttribute { .. } | Self::DuplicateKey { .. }, _) => self,
            (_, Self::DuplicateAttribute { .. } | Self::DuplicateKey { .. }) => other,
            _ => self,
        }
    }
}

impl<'a> Error<'a, &'a [Token]> for Failure {
    fn merge(self, other: Self) -> Self {
        self.preferred(other)
    }
}

impl<'a> LabelError<'a, &'a [Token], DefaultExpected<'a, Token>> for Failure {
    fn expected_found<E: IntoIterator<Item = DefaultExpected<'a, Token>>>(
        _expected: E,
        _found: Option<MaybeRef<'a, Token>>,
        span: SimpleSpan,
    ) -> Self {
        Self::Syntax(span)
    }

    fn merge_expected_found<E: IntoIterator<Item = DefaultExpected<'a, Token>>>(
        self,
        _expected: E,
        _found: Option<MaybeRef<'a, Token>>,
        span: SimpleSpan,
    ) -> Self {
        self.preferred(Self::Syntax(span))
    }

    // A REPLACE keeps a typed uniqueness violation but replaces a plain syntax
    // conflict, so reports never pin the first backtracked branch failure.
    fn replace_expected_found<E: IntoIterator<Item = DefaultExpected<'a, Token>>>(
        self,
        _expected: E,
        _found: Option<MaybeRef<'a, Token>>,
        span: SimpleSpan,
    ) -> Self {
        match self {
            Self::Syntax(_) => Self::Syntax(span),
            typed => typed,
        }
    }
}

/// A parse failure resolved to a byte offset in the original source.
pub(super) struct Located {
    /// The failure.
    pub(super) failure: Failure,
    /// Byte offset of the failure in the original source.
    pub(super) offset: usize,
}

/// Parse a spanned ODIN token stream into an [`super::OdinDocument`].
///
/// # Errors
/// Returns the first failure, resolved to a byte offset in the original
/// source.
pub(super) fn parse_tokens(spanned: &[Spanned]) -> Result<super::OdinDocument, Located> {
    let tokens: Vec<Token> = spanned.iter().map(|s| s.token.clone()).collect();
    odin_text().parse(&tokens).into_result().map_err(|errs| {
        let failure = errs
            .into_iter()
            .reduce(Failure::preferred)
            .unwrap_or(Failure::Syntax(SimpleSpan::new(
                (),
                spanned.len()..spanned.len(),
            )));
        let idx = failure.span().start;
        let offset = spanned
            .get(idx)
            .map(|s| s.span.start)
            .or_else(|| spanned.last().map(|s| s.span.end))
            .unwrap_or(0);
        Located { failure, offset }
    })
}

/// `odin_text : ( schema_identifier )? main_text`, where the main text is
/// `attr_vals | keyed_objects | included_other_language |
/// object_value_block`.
///
/// The prefix is a docs-text production of
/// `LANG/docs/odin/master04-odin_artefacts` (the chapter intro defines
/// `odin_text ::= ( schema_identifier )? main_text`); the top-level
/// keyed-object and whole-document plug-in alternatives are the
/// Release-1.0.0 `odin.g4` start rule's own `keyed_object+` and
/// `included_other_language` branches, agreeing with §Identified Object
/// Document.
fn odin_text<'a>() -> impl Parser<'a, &'a [Token], super::OdinDocument, Err<'a>> {
    let block = object_block();
    let attrs = attr_vals(block.clone());
    let keyed = keyed_objects(block.clone());
    schema_identifier()
        .or_not()
        .then(choice((attrs, keyed, plug_in(), block)))
        .map(|(schema, root)| super::OdinDocument { schema, root })
        .then_ignore(end())
}

/// `schema_identifier ::= '@' schema '=' URI`
/// (`LANG/docs/odin/master04-odin_artefacts` intro: "used to indicate the
/// schema, including its version, on which the main ODIN text is based").
///
/// NOTE: two ambiguities in that production, adjudicated here (the chapter
/// defines neither and gives no example): its `schema` is an unquoted,
/// never-defined nonterminal — read as the identifier naming the schema
/// (commonly the literal word `schema`), in the same identifier class as an
/// ODIN attribute key (`ALPHA_LC_ID` in this generation); and its `URI`
/// ("a value of the URI primitive type") is taken in the embedded `<uri>`
/// spelling every other ODIN URI value uses.
fn schema_identifier<'a>() -> impl Parser<'a, &'a [Token], super::OdinSchemaId, Err<'a>> + Clone {
    let name = select! {
        Token::AlphaLcId(s) => s,
    };
    just(Token::SymAt)
        .ignore_then(name)
        .then_ignore(just(Token::SymEq))
        .then(select! { Token::EmbeddedUri(s) => s })
        .map(|(name, uri)| super::OdinSchemaId { name, uri })
}

/// `attr_vals : ( attr_val ';'? )+` → [`OdinValue::Object`].
///
/// Sibling attribute names must be unique (`LANG/docs/odin/master05-content`
/// §General Structure rule VDATU), so a repeat is a
/// [`Failure::DuplicateAttribute`].
///
/// NOTE: the attribute key is `ALPHA_LC_ID` alone, the Release-1.0.0
/// `base_patterns.g4` `attribute_id`, since the docs text names no lexical class.
fn attr_vals<'a>(
    block: impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone {
    let key = select! {
        Token::AlphaLcId(s) => s,
    };
    key.then_ignore(just(Token::SymEq))
        .then(block)
        .then_ignore(just(Token::SymSemiColon).or_not())
        .map_with(|pair, e| (pair, e.span()))
        .repeated()
        .at_least(1)
        .collect::<Vec<((String, OdinValue), SimpleSpan)>>()
        .try_map(|pairs, _| {
            let mut map: IndexMap<String, OdinValue> = IndexMap::with_capacity(pairs.len());
            for ((name, value), span) in pairs {
                if map.insert(name.clone(), value).is_some() {
                    return Err(Failure::DuplicateAttribute { name, span });
                }
            }
            Ok(OdinValue::Object(map))
        })
}

/// `( keyed_object ';'? )+` with `keyed_object : '[' primitive_value ']' '='
/// object_block` (Release-1.0.0 `odin.g4`) → [`OdinValue::KeyedList`] — the
/// container-item form, both inside a block and as a whole Identified Object
/// Document (`LANG/docs/odin/master04-odin_artefacts` §Identified Object
/// Document).
///
/// NOTE: the trailing optional `';'` is a docs-text widening over the
/// vendored `odin.g4`, which writes the separator only on `attr_vals`:
/// "Semi-colons can be used to separate ODIN blocks … Semi-colons make no
/// semantic difference at all" (`LANG/docs/odin/master03-basics`
/// §Semi-colons), and a keyed object ends in an ODIN block, so the separator
/// is accepted (and ignored) here exactly as between attribute pairs.
fn keyed_objects<'a>(
    block: impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone {
    let key_id = key_id();
    // Sibling keys are unique as typed values, so `[01]` duplicates `[1]` (rule
    // VDOBU, `LANG/docs/odin/master05-content` §Container Objects).
    choice((
        just(Token::LBracket)
            .ignore_then(key_id)
            .then_ignore(just(Token::RBracket)),
        local_key(),
    ))
    .then_ignore(just(Token::SymEq))
    .then(block)
    .then_ignore(just(Token::SymSemiColon).or_not())
    .map_with(|pair, e| (pair, e.span()))
    .repeated()
    .at_least(1)
    .collect::<Vec<((OdinKey, OdinValue), SimpleSpan)>>()
    .try_map(|entries, _| {
        let mut seen: std::collections::HashSet<OdinKey> =
            std::collections::HashSet::with_capacity(entries.len());
        let mut list = Vec::with_capacity(entries.len());
        for ((key, value), span) in entries {
            if !seen.insert(key.clone()) {
                return Err(Failure::DuplicateKey {
                    key: key.path_text(),
                    span,
                });
            }
            list.push((key, value));
        }
        Ok(OdinValue::KeyedList(list))
    })
}

/// `'[' … ']'`'s inner key.
///
/// NOTE: the key is any `primitive_value` — the Release-1.0.0 `odin.g4`
/// `keyed_object : '[' primitive_value ']' '=' object_block`, agreeing with
/// §Container Objects ("any primitive comparable value is allowed as the
/// key"); the five-type `key_id` narrowing postdates this generation. The
/// signed numeric forms follow `odin_values.g4`
/// (`integer_value : ('+'|'-')? INTEGER`, likewise `real_value`).
fn key_id<'a>() -> impl Parser<'a, &'a [Token], OdinKey, Err<'a>> + Clone {
    // A `custom` primitive with ONE failure point: a composed
    // sign-prefix parser would emit a stray branch failure on every
    // unsigned key, and chumsky's furthest-position error tracking would
    // then outrank the typed VDOBU refusal raised behind it.
    custom(|inp| {
        let before = inp.cursor();
        let sign = match inp.peek() {
            Some(Token::SymPlus) => {
                inp.next();
                Some(1i64)
            }
            Some(Token::SymMinus) => {
                inp.next();
                Some(-1i64)
            }
            _ => None,
        };
        let token = inp.next();
        let span = inp.span_since(&before);
        match (sign, token) {
            (sign, Some(Token::Integer(s))) => {
                let mag = integer_lexeme(&s).ok_or(Failure::Syntax(span))?;
                sign.unwrap_or(1)
                    .checked_mul(mag)
                    .map(OdinKey::Integer)
                    .ok_or(Failure::Syntax(span))
            }
            (sign, Some(Token::Real(s))) => {
                let Ok(mag) = s.parse::<f64>() else {
                    return Err(Failure::Syntax(span));
                };
                let value = if sign == Some(-1) { -mag } else { mag };
                Ok(OdinKey::Real(super::OdinRealKey::new(value)))
            }
            (None, Some(Token::String(s))) => Ok(OdinKey::String(decode_string(&s))),
            (None, Some(Token::SymTrue)) => Ok(OdinKey::Boolean(true)),
            (None, Some(Token::SymFalse)) => Ok(OdinKey::Boolean(false)),
            (None, Some(Token::Character(s))) => Ok(OdinKey::Character(decode_char(&s))),
            (None, Some(Token::TermCodeRef(s) | Token::LocalTermCodeRef(s))) => {
                Ok(OdinKey::TermCode(s))
            }
            (None, Some(Token::Iso8601Date(s))) => Ok(OdinKey::Date(s)),
            (None, Some(Token::Iso8601Time(s))) => Ok(OdinKey::Time(s)),
            (None, Some(Token::Iso8601DateTime(s))) => Ok(OdinKey::DateTime(s)),
            (None, Some(Token::Iso8601Duration(s))) => Ok(OdinKey::Duration(s)),
            _ => Err(Failure::Syntax(span)),
        }
    })
}

/// A `[True]` / `[P1Y]`-shaped container key that arrives from the lexer as
/// ONE `LocalTermCodeRef` token.
///
/// The local-code token is the ADL 1.4 leaf widening the shared lexer
/// carries, and it swallows any `[alpha…]` bracket construct whole — but in
/// key position the Release-1.0.0 `keyed_object : '[' primitive_value ']'`
/// production is the specific authority, so the token's body is re-read and
/// admitted exactly when it spells a primitive key (a case-insensitive
/// boolean per `master07-leaf_data` §Boolean Data, or an ISO 8601 duration).
/// Any other body (`[at0200]`, …) is not a `primitive_value` and stays a
/// refusal in key position, exactly as in the v1_1 reading.
fn local_key<'a>() -> impl Parser<'a, &'a [Token], OdinKey, Err<'a>> + Clone {
    custom(|inp| {
        let before = inp.cursor();
        let token = inp.next();
        let span = inp.span_since(&before);
        let Some(Token::LocalTermCodeRef(text)) = token else {
            return Err(Failure::Syntax(span));
        };
        let body = text
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or(&text);
        let Ok(tokens) = crate::v1_0::lexer::lex_odin(body) else {
            return Err(Failure::Syntax(span));
        };
        match tokens.as_slice() {
            [one] => match &one.token {
                Token::SymTrue => Ok(OdinKey::Boolean(true)),
                Token::SymFalse => Ok(OdinKey::Boolean(false)),
                Token::Iso8601Duration(d) => Ok(OdinKey::Duration(d.clone())),
                _ => Err(Failure::Syntax(span)),
            },
            _ => Err(Failure::Syntax(span)),
        }
    })
}

/// `object_block : object_value_block | object_reference_block` (+ the
/// `EMBEDDED_URI` and typed-cast forms).
fn object_block<'a>() -> impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone {
    recursive(|block| {
        let keyed_list = keyed_objects(block.clone());

        // object_reference_block : odin_path ( (',' odin_path)+ | '...' )?
        //
        // A path may be rooted at an object identifier
        // (`LANG/docs/odin/master06-references` §Across Objects; the docs text
        // wins over `odin.g4`).
        let bare_path =
            select! { Token::AdlPath(s) => s }.or(just(Token::SymSlash).to("/".to_owned()));
        let path = key_id()
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .or_not()
            .then(bare_path)
            .map(|(root, p)| match root {
                Some(key) => format!("[{}]{p}", key.path_text()),
                None => p,
            });
        let ref_list = path
            .clone()
            .then(
                choice((
                    just(Token::SymComma)
                        .ignore_then(path)
                        .repeated()
                        .at_least(1)
                        .collect::<Vec<String>>(),
                    just(Token::SymListContinue).to(Vec::new()),
                ))
                .or_not(),
            )
            .map(|(first, rest)| {
                let mut v = vec![first];
                if let Some(mut more) = rest {
                    v.append(&mut more);
                }
                OdinValue::PathList(v)
            });

        // `<...>` is an empty section or void object, legal at any level
        // (`AM/docs/ADL1.4/master04-dadl` §Empty Sections,
        // `LANG/docs/odin/master05-content` §Void Objects).
        //
        // NOTE: both passages define it as an attribute with no data, so it is
        // [`OdinValue::Empty`] like `<>`.
        let void = just(Token::SymListContinue).to(OdinValue::Empty);

        let inner = choice((
            void,
            keyed_list,
            ref_list,
            attr_vals(block.clone()),
            primitive_object(),
        ));

        let value_block = rm_type_id()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .or_not()
            .then(
                inner
                    .or_not()
                    .delimited_by(just(Token::SymLt), just(Token::SymGt)),
            )
            .map(|(cast, inner_opt)| {
                let val = inner_opt.unwrap_or(OdinValue::Empty);
                match cast {
                    Some(rm_type) => OdinValue::Typed {
                        rm_type,
                        value: Box::new(val),
                    },
                    None => val,
                }
            });

        let uri = select! { Token::EmbeddedUri(s) => OdinValue::Uri(s) };

        choice((uri, plug_in(), value_block))
    })
}

/// `(syntax) <# … #>` — a plug-in-syntax object block
/// (`LANG/docs/odin/master09-plug_in_syntaxes`: "Plug-in syntaxes are
/// indicated in ODIN … by the use of the syntax type in parentheses
/// preceding the `<>` block. For a plug-in section, the `<>` delimiters are
/// modified to `<# #>`").
///
/// The general form makes the `(syntax)` tag part of the construct, so a
/// bare `<# … #>` with no tag stays a parse error. The tag is an ordinary
/// identifier and the body is handed over verbatim. The same form is legal
/// as a whole document — the Release-1.0.0 `odin.g4` start rule's
/// `included_other_language` alternative — so [`odin_text`] offers it
/// beside the block position.
fn plug_in<'a>() -> impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone {
    let plug_in_tag = select! {
        Token::AlphaLcId(s) => s,
        Token::AlphaUcId(s) => s,
    };
    plug_in_tag
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .then(select! { Token::PlugInBlock(s) => s })
        .map(|(syntax, raw)| OdinValue::PlugIn {
            syntax,
            text: raw
                .strip_prefix("<#")
                .and_then(|t| t.strip_suffix("#>"))
                .unwrap_or(&raw)
                .to_owned(),
        })
}

/// `type_id : ( package_id '.' )* ALPHA_UC_ID ( '<' type_id ( ','
/// type_id )* '>' )?`, reconstructed as a flat string as authored (e.g.
/// `Interval<Quantity>`, `org.openehr.rm.ehr.content.ENTRY`).
///
/// NOTE: `base_patterns.g4` has no namespace form; the docs text admits one on
/// any type identifier (`LANG/docs/odin/master05-content` §Adding Type Information).
///
/// Package segments take either case, as the chapter's examples do; the type
/// stays `ALPHA_UC_ID`, and a template parameter may be qualified too.
fn rm_type_id<'a>() -> impl Parser<'a, &'a [Token], String, Err<'a>> + Clone {
    recursive(|t| {
        let package_segment = select! {
            Token::AlphaUcId(s) => s,
            Token::AlphaLcId(s) => s,
        };
        package_segment
            .then_ignore(just(Token::SymDot))
            .repeated()
            .collect::<Vec<String>>()
            .then(select! { Token::AlphaUcId(s) => s })
            .map(|(namespace, name)| {
                if namespace.is_empty() {
                    name
                } else {
                    format!("{}.{name}", namespace.join("."))
                }
            })
            .then(
                t.separated_by(just(Token::SymComma))
                    .at_least(1)
                    .collect::<Vec<String>>()
                    .delimited_by(just(Token::SymLt), just(Token::SymGt))
                    .or_not(),
            )
            .map(|(name, generics)| match generics {
                Some(gs) => format!("{name}<{}>", gs.join(",")),
                None => name,
            })
    })
}

/// `primitive_object : primitive_value | primitive_list_value |
/// primitive_interval_value` (+ the `*_interval_list_value` productions).
fn primitive_object<'a>() -> impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone {
    let leaf = leaf_value();

    // A list item is a leaf value or an interval — `odin_values.g4` (App.B)
    // defines both the per-type `*_list_value` productions and the per-type
    // `*_interval_list_value` productions (`|0..5|, |8..9|` and the open
    // `|0..5|, ...`).
    let item = choice((interval_value(leaf.clone()), leaf));

    // A scalar, or a homogeneous comma-separated list optionally left open with
    // `, ...` (`master07` §Lists of Built-in Types), a superset of
    // `odin_values.g4`; per-type interval productions pin interval endpoints.
    item.clone()
        .map_with(|value, e| (value, e.span()))
        .then(
            just(Token::SymComma)
                .ignore_then(item.map_with(|value, e| (value, e.span())))
                .repeated()
                .collect::<Vec<(OdinValue, SimpleSpan)>>(),
        )
        .then(
            just(Token::SymComma)
                .ignore_then(just(Token::SymListContinue))
                .or_not(),
        )
        .try_map(|(((first, _), more), open), _| {
            if more.is_empty() && open.is_none() {
                return Ok(first);
            }
            let kind = std::mem::discriminant(&first);
            let mut values = Vec::with_capacity(more.len() + 2);
            values.push(first);
            for (value, span) in more {
                if std::mem::discriminant(&value) != kind {
                    return Err(Failure::Syntax(span));
                }
                values.push(value);
            }
            if open.is_some() {
                values.push(OdinValue::ListContinue);
            }
            Ok(OdinValue::List(values))
        })
}

/// One interval `| … |` (`odin_values.g4` `*_interval_value`).
fn interval_value<'a>(
    leaf: impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone {
    // An interval endpoint: a leaf value, or the unbounded `infinity`,
    // `-infinity` or `*` read as `None` (`AM/docs/ADL1.4/master04-dadl`
    // §Intervals of Ordered Primitive Types).
    let unbounded = choice((
        just(Token::SymMinus)
            .or_not()
            .ignore_then(just(Token::SymInfinity))
            .ignored(),
        just(Token::SymStar).ignored(),
    ))
    .to(None);
    let bound = choice((unbounded, leaf.clone().map(Some)));

    // `| '>'? a '..' '<'? b |`
    let range = just(Token::SymGt)
        .or_not()
        .then(bound.clone())
        .then_ignore(just(Token::SymIvlSep))
        .then(just(Token::SymLt).or_not())
        .then(bound.clone())
        .map(|(((gt, lo), lt), hi)| OdinInterval::Range {
            lower_included: gt.is_none() && lo.is_some(),
            upper_included: lt.is_none() && hi.is_some(),
            lower: lo.map(Box::new),
            upper: hi.map(Box::new),
        });

    // `| a '+/-' b |`
    //
    // NOTE: `master07-leaf_data` §Intervals (generation-identical text)
    // defines `|N +/-M|` / `|N±M|` with worked examples — the docs text
    // wins over the Release-1.0.0 `odin_values.g4`'s missing ± production.
    let plus_minus = leaf
        .clone()
        .then_ignore(just(Token::SymPlusOrMinus))
        .then(leaf.clone())
        .map(|(centre, delta)| OdinInterval::PlusMinus {
            centre: Box::new(centre),
            delta: Box::new(delta),
        });

    // `| relop? a |`  (relop absent ⇒ a point interval `[a, a]`)
    let relop = choice((
        just(Token::SymGe).to(RelBound::Lower(true)),
        just(Token::SymGt).to(RelBound::Lower(false)),
        just(Token::SymLe).to(RelBound::Upper(true)),
        just(Token::SymLt).to(RelBound::Upper(false)),
    ));
    let single = relop.or_not().then(bound).map(|(op, v)| match (op, v) {
        (None, Some(v)) => OdinInterval::Range {
            lower: Some(Box::new(v.clone())),
            lower_included: true,
            upper: Some(Box::new(v)),
            upper_included: true,
        },
        (Some(RelBound::Lower(incl)), Some(v)) => OdinInterval::Range {
            lower: Some(Box::new(v)),
            lower_included: incl,
            upper: None,
            upper_included: false,
        },
        (Some(RelBound::Upper(incl)), Some(v)) => OdinInterval::Range {
            lower: None,
            lower_included: false,
            upper: Some(Box::new(v)),
            upper_included: incl,
        },
        // A one-sided form whose single endpoint is itself an unbounded marker
        // (`|>=-infinity|`, `|<*|`) constrains nothing at all.
        (_, None) => OdinInterval::Range {
            lower: None,
            lower_included: false,
            upper: None,
            upper_included: false,
        },
    });

    just(Token::SymIvlDelim)
        .ignore_then(choice((range, plus_minus, single)))
        .then_ignore(just(Token::SymIvlDelim))
        .map(OdinValue::Interval)
}

/// A single-bound relational operator's target side + inclusivity.
#[derive(Clone, Copy)]
enum RelBound {
    Lower(bool),
    Upper(bool),
}

/// A single primitive leaf value (`primitive_value` in `odin_values.g4`).
fn leaf_value<'a>() -> impl Parser<'a, &'a [Token], OdinValue, Err<'a>> + Clone {
    let int_sign = choice((
        just(Token::SymPlus).to(1i64),
        just(Token::SymMinus).to(-1i64),
    ))
    .or_not();
    let integer = int_sign
        .then(select! { Token::Integer(s) => s })
        .try_map(|(sign, s), span| {
            let mag = integer_lexeme(&s).ok_or(Failure::Syntax(span))?;
            sign.unwrap_or(1)
                .checked_mul(mag)
                .map(OdinValue::Integer)
                .ok_or(Failure::Syntax(span))
        });

    let real_sign = choice((
        just(Token::SymPlus).to(1f64),
        just(Token::SymMinus).to(-1f64),
    ))
    .or_not();
    let real = real_sign
        .then(select! { Token::Real(s) => s })
        .try_map(|(sign, s), span| {
            // The lexeme is already pinned by the token and the span; a
            // `ParseFloatError` adds nothing a `Syntax(span)` does not carry.
            let Ok(mag) = s.parse::<f64>() else {
                return Err(Failure::Syntax(span));
            };
            Ok(OdinValue::Real(sign.unwrap_or(1.0) * mag))
        });

    let string = select! { Token::String(s) => OdinValue::String(decode_string(&s)) };
    let boolean = select! {
        Token::SymTrue => OdinValue::Boolean(true),
        Token::SymFalse => OdinValue::Boolean(false),
    };
    let character =
        select! { Token::Character(s) => s }.map(|s| OdinValue::Character(decode_char(&s)));
    let term_code = select! {
        Token::TermCodeRef(s) => OdinValue::TermCode(s),
        Token::LocalTermCodeRef(s) => OdinValue::TermCode(s),
    };
    let date = select! { Token::Iso8601Date(s) => OdinValue::Date(s) };
    let time = select! { Token::Iso8601Time(s) => OdinValue::Time(s) };
    let date_time = select! { Token::Iso8601DateTime(s) => OdinValue::DateTime(s) };
    let duration = select! { Token::Iso8601Duration(s) => OdinValue::Duration(s) };

    choice((
        real, integer, string, boolean, character, term_code, date_time, date, time, duration,
    ))
}

/// The value of an `INTEGER` lexeme, evaluating the optional exponent suffix.
///
/// `AM/docs/ADL1.4/master04-dadl` §Integer Data lists `29e6` beside `25` and
/// `300000` as integer data, and both the chapter's lex rule
/// (`[0-9]+[eE][+-]?[0-9]+`) and the vendored `base_lexer.g4`
/// (`INTEGER : DIGIT+ E_SUFFIX?`) admit the suffix — so the lexeme is
/// *evaluated*, not parsed as a bare decimal literal.
///
/// NOTE: neither chapter says what a negative exponent means for an integer
/// (`29e-2` is not an integer). It is accepted only when the scaling is exact
/// (`2900e-2` = 29); an inexact one has no integer value and is rejected as a
/// malformed leaf rather than silently truncated. No openEHR spec governs that
/// case — our own design/extension.
#[expect(
    clippy::integer_division,
    reason = "the `mantissa % scale == 0` guard proves the division is exact — a negative exponent that does not divide evenly returns None instead"
)]
fn integer_lexeme(s: &str) -> Option<i64> {
    let Some(e) = s.find(['e', 'E']) else {
        return s.parse::<i64>().ok();
    };
    let mantissa = s.get(..e)?.parse::<i64>().ok()?;
    let exponent = s.get(e + 1..)?.parse::<i32>().ok()?;
    let scale = 10i64.checked_pow(exponent.unsigned_abs())?;
    if exponent >= 0 {
        mantissa.checked_mul(scale)
    } else if mantissa % scale == 0 {
        Some(mantissa / scale)
    } else {
        None
    }
}

/// Strip the surrounding double quotes and decode `master03` escapes.
#[expect(
    clippy::expect_used,
    reason = "`Token::String` only exists when the lexer's validate_string ran crate::v1_0::escape::validate over the same body and it succeeded, so this decode of that body cannot fail"
)]
fn decode_string(raw: &str) -> String {
    crate::v1_0::escape::decode_string_literal(raw)
        .expect("a lexer-validated string literal should decode")
}

/// Decode a single-quoted `CHARACTER` literal to its `char`.
#[expect(
    clippy::expect_used,
    reason = "`Token::Character` only exists when the lexer's validate_char admitted the body, which restricts an escape to the six quoted forms none of which can fail to decode; the same token regex admits one body character or one two-character escape, each decoding to exactly one char, so the literal is never empty"
)]
fn decode_char(raw: &str) -> char {
    crate::v1_0::escape::decode_character_literal(raw)
        .expect("a lexer-validated character literal should decode")
        .chars()
        .next()
        .expect("a lexer-validated character literal should decode to one character")
}

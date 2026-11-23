// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The shared token superset: one `logos` DFA, read in this generation under
//! the single Release-1.0.0 ODIN lexical layer.

use logos::Logos;

use crate::v1_1::lexer::token::scan;
use crate::v1_1::lexer::token::scan::{
    contained_regexp_rest, embedded_uri_rest, opening_or, plug_in_rest,
};

/// A token of the shared openEHR lexical superset.
///
/// Text-bearing variants hold the owned source slice verbatim (delimiters
/// included; the parsers strip and decode them). The one exception is
/// [`Token::String`] under the ODIN reading, whose payload has the multi-line
/// white-space leaders removed (`LANG/docs/odin/master07-leaf_data` §String
/// Data) — that transform is applied by the ODIN reclassification pass.
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")] // WS / LINE (base_lexer.g4)
#[logos(skip("--", scan::comment))] // CMT_LINE `-- … EOL` incl. the `----…` overlay separator (base_lexer.g4)
pub enum Token {
    /// A UTF-8 byte-order mark.
    ///
    /// `ADL2/master03-file_encoding.adoc` §File Encoding forbids it, but
    /// real-world sources carry one, so the ODIN reading tolerates (and
    /// drops) it. It is a token rather than a `logos` skip purely so the
    /// reclassification pass can make that choice; no returned token stream
    /// ever contains it.
    #[token("\u{feff}")]
    Bom,

    // ── foreign keywords of the shared superset: exact literals outrank the
    // identifier regexes by logos priority, and the ODIN pass demotes each ──
    /// `matches` / `is_in` / `∈` (`SYM_MATCHES`).
    #[token("matches", ignore(case))]
    #[token("is_in", ignore(case))]
    #[token("\u{2208}")]
    SymMatches,
    /// `∉` — "not in" (`~matches` is lexed as `SymNot` `SymMatches`).
    #[token("\u{2209}")]
    SymNotMatches,
    /// `and` / `∧` (`SYM_AND`).
    #[token("and", ignore(case))]
    #[token("\u{2227}")]
    SymAnd,
    /// `or` / `∨` (`SYM_OR`).
    #[token("or", ignore(case))]
    #[token("\u{2228}")]
    SymOr,
    /// `xor` / `⊻` (`SYM_XOR`).
    ///
    /// The symbol form is the EL Logical Operators table's
    /// (`LANG/docs/EL/master05-expressions.adoc` §Primitive Operators); the
    /// vendored `ElLexer.g4` lists only the two word spellings, and the other
    /// readings have no symbolic `xor` at all.
    #[token("xor", ignore(case))]
    #[token("\u{22BB}")]
    SymXor,
    /// `not` / `~` / `∼` / `¬` / `!` (`SYM_NOT`).
    #[token("not", ignore(case))]
    #[token("~")]
    #[token("\u{223C}")]
    #[token("\u{00AC}")]
    #[token("!")]
    SymNot,
    /// `implies` / `®` / `->` / `⇒` / `→` (`SYM_IMPLIES`).
    ///
    /// The two arrow forms are EL's (`ElLexer.g4` `SYM_IMPLIES : 'implies' |
    /// '⇒' | '→'`; the `⇒` spelling is also the Logical Operators table's in
    /// `LANG/docs/EL/master05-expressions.adoc` §Primitive Operators). No
    /// other reading has an arrow spelling.
    #[token("implies", ignore(case))]
    #[token("\u{00AE}")]
    #[token("->")]
    #[token("\u{21D2}")]
    #[token("\u{2192}")]
    SymImplies,
    /// `⇔` / `↔` — material equivalence (`ElLexer.g4` `SYM_IFF`), an
    /// EL-only operator (`ElParser.g4` `elBooleanExpr`).
    #[token("\u{21D4}")]
    #[token("\u{2194}")]
    SymIff,
    /// `for_all` / `∀` (`SYM_FOR_ALL`).
    #[token("for_all", ignore(case))]
    #[token("\u{2200}")]
    SymForAll,
    /// `exists` / `□` (`SYM_EXISTS`).
    #[token("exists", ignore(case))]
    #[token("\u{25A1}")]
    SymExists,
    /// `there_exists` / `∃` (`SYM_THERE_EXISTS`).
    ///
    /// The ONE keyword that is NOT case-folded: `adl_keywords.g4` spells it as
    /// the plain literal `SYM_THERE_EXISTS: 'there_exists' | '∃' ;` rather than
    /// in the `[Tt][Hh]…` form every other keyword on that list uses, and the
    /// ADL 1.4 chapter's own §Symbols lexer has no `there_exists` rule at all
    /// (`ADL1.4/master05-cadl.adoc` L1329-1340 stops at `SYM_EXISTS`). Folding
    /// it anyway would swallow `There_Exists`, a well-formed `ALPHA_UC_ID`
    /// under both lexers.
    #[token("there_exists")]
    #[token("\u{2203}")]
    SymThereExists,
    /// `in` — the 1.1.0-line quantifier binding keyword
    /// (`base_expressions.g4` `for_all_expr`); no 1.0.0 production, so the
    /// ODIN pass demotes it.
    #[token("in")]
    SymIn,
    /// `occurrences` (`SYM_OCCURRENCES`).
    #[token("occurrences", ignore(case))]
    SymOccurrences,
    /// `existence` (`SYM_EXISTENCE`).
    #[token("existence", ignore(case))]
    SymExistence,
    /// `cardinality` (`SYM_CARDINALITY`).
    #[token("cardinality", ignore(case))]
    SymCardinality,
    /// `ordered` (`SYM_ORDERED`).
    #[token("ordered", ignore(case))]
    SymOrdered,
    /// `unordered` (`SYM_UNORDERED`).
    #[token("unordered", ignore(case))]
    SymUnordered,
    /// `unique` (`SYM_UNIQUE`).
    #[token("unique", ignore(case))]
    SymUnique,
    /// `infinity` (`SYM_INFINITY`) — the unbounded interval endpoint of
    /// `ADL1.4/master05-cadl.adoc` §Keywords L50 + §Symbols L1349, whose own
    /// worked example is `rate matches {|0..infinity|}` (L771), and of
    /// `AM/docs/ADL1.4/master04-dadl` §Intervals of Ordered Primitive Types
    /// for the ODIN reading.
    #[token("infinity", ignore(case))]
    SymInfinity,
    /// `use_node` (`SYM_USE_NODE`).
    #[token("use_node", ignore(case))]
    SymUseNode,
    /// `use_archetype` (`SYM_USE_ARCHETYPE`).
    #[token("use_archetype", ignore(case))]
    SymUseArchetype,
    /// `allow_archetype` (`SYM_ALLOW_ARCHETYPE`).
    #[token("allow_archetype", ignore(case))]
    SymAllowArchetype,
    /// `include` (`SYM_INCLUDE`).
    #[token("include", ignore(case))]
    SymInclude,
    /// `exclude` (`SYM_EXCLUDE`).
    #[token("exclude", ignore(case))]
    SymExclude,
    /// `after` (`SYM_AFTER`).
    #[token("after", ignore(case))]
    SymAfter,
    /// `before` (`SYM_BEFORE`).
    #[token("before", ignore(case))]
    SymBefore,
    /// `closed` (`SYM_CLOSED`).
    #[token("closed", ignore(case))]
    SymClosed,
    /// `then` (`SYM_THEN`).
    #[token("then", ignore(case))]
    SymThen,

    // ── EL-only word keywords (`ElLexer.g4`). No `#[token]`: the shared
    //    DFA reads each as a plain identifier, and no 1.0.0 reading ever
    //    produces these re-tag-only variants. ──
    /// `Self` (`SYM_SELF`) — the current-object reference.
    SymSelf,
    /// `Result` (`SYM_RESULT`) — a function's automatic result variable.
    SymResult,
    /// `case` (`SYM_CASE`) — a decision-table case head.
    SymCase,
    /// `choice` (`SYM_CHOICE`) — a decision-table condition-chain head.
    SymChoice,
    /// `assert` (`SYM_ASSERT`).
    SymAssert,

    // ── boolean word symbols (base_lexer.g4 SYM_TRUE / SYM_FALSE) ──
    /// `True` / `true` (case-insensitive per `base_lexer.g4` `SYM_TRUE`,
    /// `LANG/docs/odin/master07-leaf_data` §Boolean Data).
    #[token("true", ignore(case))]
    SymTrue,
    /// `False` / `false` (case-insensitive; same citations as [`Token::SymTrue`]).
    #[token("false", ignore(case))]
    SymFalse,

    // ── codes (base_lexer.g4). Higher priority than ALPHA_*_ID so `id1`/`at3`
    //    classify as codes, not identifiers. ──
    /// `id1` `.1`* — the AOM root id-code (`ROOT_ID_CODE`).
    #[regex(r"id1(\.1)*", |lex| lex.slice().to_owned(), priority = 22)]
    RootIdCode(String),
    /// `id` `CODE_STR` — an id-code (`ID_CODE`).
    #[regex(r"id(0|[1-9][0-9]*)(\.(0|[1-9][0-9]*))*", |lex| lex.slice().to_owned(), priority = 20)]
    IdCode(String),
    /// `at` `CODE_STR` — an at-code / term code (`AT_CODE`). Permissive on
    /// leading zeros (`at0000`, `at0.1`) to admit legacy corpus at-codes.
    #[regex(r"at[0-9]+(\.[0-9]+)*", |lex| lex.slice().to_owned(), priority = 20)]
    AtCode(String),
    /// `ac` `CODE_STR` — a value-set (constraint) code (`AC_CODE`).
    #[regex(r"ac[0-9]+(\.[0-9]+)*", |lex| lex.slice().to_owned(), priority = 20)]
    AcCode(String),

    // ── archetype identifiers (base_lexer.g4 ARCHETYPE_HRID / ARCHETYPE_REF) ──
    /// An archetype HRID or version-partial reference — one token covering both
    /// `ARCHETYPE_HRID` (full `vN.M.P`) and `ARCHETYPE_REF` (partial `vN`); the
    /// parser resolves which. e.g. `openEHR-EHR-OBSERVATION.blood_pressure.v1`.
    #[regex(
        r"([a-zA-Z][a-zA-Z0-9_.\-]*::)?[a-zA-Z][a-zA-Z0-9_]*-[a-zA-Z][a-zA-Z0-9_]*-[a-zA-Z][a-zA-Z0-9_]*\.[a-zA-Z][a-zA-Z0-9_-]*\.v[0-9]+(\.[0-9]+)*((-rc|-alpha|-beta)(\.[0-9]+)?)?",
        |lex| lex.slice().to_owned()
    )]
    ArchetypeId(String),
    /// `DIGIT+ '.' DIGIT+ '.' DIGIT+ [pre-release]` — a 3-part version
    /// (`VERSION_ID`); e.g. an `adl_version`/`rm_release` meta value. The
    /// 3+-part form also carries a dotted-numeric OID `uid` (e.g.
    /// `2.4.34.666.7.2`) — `master07.05` admits an OID or GUID `uid`, and both
    /// arrive here as a single string token the identification parser records
    /// verbatim.
    #[regex(r"[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)*((-rc|-alpha|-beta)(\.[0-9]+)?)?", |lex| lex.slice().to_owned())]
    VersionId(String),
    /// A GUID (`base_lexer.g4 GUID`).
    #[regex(r"[0-9a-fA-F]+-[0-9a-fA-F]+-[0-9a-fA-F]+-[0-9a-fA-F]+-[0-9a-fA-F]+", |lex| lex.slice().to_owned())]
    Guid(String),

    // ── ISO8601 VALUES (base_lexer.g4): the high priority makes a value win
    // every tie against an all-literal constraint pattern
    // (`ADL1.4/master05-cadl.adoc` §Patterns) ──
    /// `ISO8601_DATE_TIME` (with optional partial `??` fields / timezone),
    /// fractional seconds `,`-separated (see [`Token::Iso8601Time`]).
    ///
    /// The `??`-partial family covers the whole set of
    /// `AM/docs/ADL1.4/master04-dadl` §Partial Date/Times, including the
    /// `yyyy-MM-ddT??:??:??`, `yyyy-MM-??T??:??:??` and `yyyy-??-??T??:??:??`
    /// forms.
    #[regex(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T([0-9]{2}(:[0-9]{2}(:([0-9]{2}(,[0-9]+)?|\?\?))?|:\?\?:\?\?)?|\?\?:\?\?:\?\?)(Z|[+\-][0-9]{4})?",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    #[regex(
        r"[0-9]{4}-([0-9]{2}-\?\?|\?\?-\?\?)T\?\?:\?\?:\?\?(Z|[+\-][0-9]{4})?",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    // The space form of the `AM/docs/ADL1.4/master08-adl` §Revision History
    // example, an ODIN-only widening the ODIN pass normalises to the `T` form.
    #[regex(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}(:[0-9]{2}(,[0-9]+)?)?(Z|[+\-][0-9]{4})?",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    Iso8601DateTime(String),
    /// `ISO8601_DATE` (with optional partial `??` fields).
    #[regex(
        r"[0-9]{4}-([0-9]{2}(-([0-9]{2}|\?\?))?|\?\?-\?\?)",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    Iso8601Date(String),
    /// `ISO8601_TIME` (with optional partial `??` fields / timezone).
    #[regex(
        r"[0-9]{2}:([0-9]{2}(:([0-9]{2}(,[0-9]+)?|\?\?))?|\?\?:\?\?)(Z|[+\-][0-9]{4})?",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    Iso8601Time(String),
    /// `ISO8601_DURATION`, requiring at least one component (never bare `P`).
    #[regex(
        r"-?P([0-9]+[YyMmWwDd])+(T([0-9]+[HhMm])*[0-9]+(\.[0-9]+)?[Ss]|T([0-9]+[Hh])?([0-9]+[Mm])?([0-9]+(\.[0-9]+)?[Ss])?)?|-?PT([0-9]+[HhMm])*[0-9]+(\.[0-9]+)?[Ss]|-?PT([0-9]+[Hh])([0-9]+[Mm])?|-?PT[0-9]+[Mm]",
        |lex| lex.slice().to_owned()
    )]
    Iso8601Duration(String),

    // ── constraint PATTERNS (base_lexer.g4, `ADL1.4/master05-cadl.adoc`
    // §Symbols), widened to literal date/time numbers, ASCII `+`/`-` zones and
    // a `[T ]` separator ──
    //
    // NOTE: the low priority lets the VALUE tokens above win every equal-length
    // tie.
    /// `DATE_TIME_CONSTRAINT_PATTERN` — e.g. `yyyy-mm-ddThh:mm:ss`.
    #[regex(
        r"(yyyy|YYYY|yyy|YYY|[0-9]{4})-(mm|MM|\?\?|XX|xx|[0-9]{2})-(dd|DD|\?\?|XX|xx|[0-9]{2})[T ](hh|HH|\?\?|XX|xx|[0-9]{2}):(mm|MM|\?\?|XX|xx|[0-9]{2}):(ss|SS|\?\?|XX|xx|[0-9]{2})([+\-\u{00B1}](hh|HH)(:?(mm|MM))?|Z)?",
        |lex| lex.slice().to_owned(),
        priority = 3
    )]
    DateTimeConstraintPattern(String),
    /// `DATE_CONSTRAINT_PATTERN` — e.g. `yyyy-mm-??`, `1995-??-XX`.
    #[regex(
        r"(yyyy|YYYY|yyy|YYY|[0-9]{4})-(mm|MM|\?\?|XX|xx|[0-9]{2})-(dd|DD|\?\?|XX|xx|[0-9]{2})",
        |lex| lex.slice().to_owned(),
        priority = 3
    )]
    DateConstraintPattern(String),
    /// `TIME_CONSTRAINT_PATTERN` — e.g. `hh:??:XX`, `hh:mm:ss+hh:mm`.
    #[regex(
        r"(hh|HH|\?\?|XX|xx|[0-9]{2}):(mm|MM|\?\?|XX|xx|[0-9]{2}):(ss|SS|\?\?|XX|xx|[0-9]{2})([+\-\u{00B1}](hh|HH)(:?(mm|MM))?|Z)?",
        |lex| lex.slice().to_owned(),
        priority = 3
    )]
    TimeConstraintPattern(String),
    /// `DURATION_CONSTRAINT_PATTERN` — letters only, e.g. `PYMD`, `PWD`, `PT`.
    #[regex(
        r"P([Yy][Mm]?[Ww]?[Dd]?|[Mm][Ww]?[Dd]?|[Ww][Dd]?|[Dd])(T([Hh][Mm]?[Ss]?|[Mm][Ss]?|[Ss]))?|PT([Hh][Mm]?[Ss]?|[Mm][Ss]?|[Ss])",
        |lex| lex.slice().to_owned(),
        priority = 20
    )]
    DurationConstraintPattern(String),

    // ── composed primitives (base_lexer.g4) ──
    /// A delimited contained regexp `{ /re/ }` / `{ ^re^ }` with optional
    /// `;"assumed"` (`CONTAINED_REGEXP`).
    ContainedRegexp(String),
    /// A term-code reference `[terminology(ver)?::code]` (`TERM_CODE_REF`).
    #[regex(r"\[[a-zA-Z0-9._\-]+(\([a-zA-Z0-9._\-]+\))?::[a-zA-Z0-9._\-]+\]", |lex| lex.slice().to_owned())]
    TermCodeRef(String),
    /// A *local* term-code reference `[at0200]` — a bracketed code with no
    /// `terminology::` qualifier (`AM/docs/ADL1.4/master04-dadl` §Symbols,
    /// `V_LOCAL_TERM_CODE_REF : \[{ALPHANUM}{NAMECHAR}*\]`), an ODIN leaf
    /// value.
    ///
    /// NOTE (spec-internal conflict, resolved in favour of the lex rules + the
    /// worked examples): the same chapter's yacc grammar admits only
    /// `V_QUALIFIED_TERM_CODE_REF` under `primitive_object_value`, so by the
    /// production rules alone `[at0200]` would not be a leaf value — yet the
    /// chapter's own §Lists of Built-in Types example lists `[at0200], ...`
    /// beside `"en", ...`, and `LANG/docs/odin/master07-leaf_data` §Lists of
    /// Built-in Types repeats that example verbatim. Two normative passages
    /// against one production rule: the local code is read as a leaf value.
    #[regex(r"\[[a-zA-Z][a-zA-Z0-9._\-]*\]", |lex| lex.slice().to_owned())]
    LocalTermCodeRef(String),
    /// A plug-in-syntax object block `<# … #>`, captured verbatim including
    /// the delimiters (`LANG/docs/odin/master09-plug_in_syntaxes`: "the `<>`
    /// delimiters are modified to `<# #>`, to allow for easier parser design";
    /// the delimiters are reserved by `master03-basics` §Reserved
    /// Characters). The body is raw foreign text ("expressed in some other
    /// syntax"), never lexed.
    PlugInBlock(String),
    /// An embedded URI `<scheme:…>` (`EMBEDDED_URI`).
    ///
    /// NOTE: captured VERBATIM — the token pins only the `scheme:` shape.
    /// `LANG/docs/odin/master07-leaf_data` §URIs says ODIN URIs "follow the
    /// standard syntax from IETF RFC 3986" (percent-encoding per `master03`
    /// §File Encoding), but RFC 3986 validity is deliberately NOT policed at
    /// the lexical layer: refusing here would punish real-world authored
    /// data, and URI validity is the consuming model's typed concern
    /// (adjudicated against §7.3).
    EmbeddedUri(String),
    /// An ADL path (`base_lexer.g4 ADL_PATH`), each segment
    /// `ALPHA_LC_ID ('[' predicate ']')?`.
    #[regex(
        r"(/[a-zA-Z][a-zA-Z0-9_]*(\[[^\]\r\n]*\])?)+|[a-zA-Z][a-zA-Z0-9_]*(\[[^\]\r\n]*\])?(/[a-zA-Z][a-zA-Z0-9_]*(\[[^\]\r\n]*\])?)+",
        |lex| lex.slice().to_owned()
    )]
    #[regex(
        r"//[a-z][a-zA-Z0-9_]*(\[[^\]\r\n]*\])?(/[a-z][a-zA-Z0-9_]*(\[[^\]\r\n]*\])?)*|[a-z][a-zA-Z0-9_]*\[[^\]\r\n]*\]",
        |lex| lex.slice().to_owned()
    )]
    AdlPath(String),

    // ── atomic primitives (base_lexer.g4) ──
    /// `REAL` — `DIGIT+ '.' DIGIT+` with optional `E` suffix.
    #[regex(r"[0-9]+\.[0-9]+([eE][+\-]?[0-9]+)?", |lex| lex.slice().to_owned())]
    Real(String),
    /// `INTEGER` — `DIGIT+` with optional `E` suffix
    /// (`AM/docs/ADL1.4/master04-dadl` §Integer Data lists `29e6` as integer
    /// data).
    #[regex(r"[0-9]+([eE][+\-]?[0-9]+)?", |lex| lex.slice().to_owned())]
    Integer(String),
    /// A double-quoted `STRING` (may span lines); escapes validated per
    /// `master03` (`\r \n \t \\ \" \'` + `\uHHHH`/`\uHHHHHHHH`).
    #[token("\"", |lex| scan::string(lex).and_then(|()| validate_string(lex)))]
    String(String),
    /// A single-quoted `CHARACTER`. An escaped character must be one of the
    /// six legal quoted forms (see `validate_char`).
    #[regex(r"'([^'\\\r\n]|\\.)'", validate_char)]
    Character(String),

    // ── rule/assertion variable ids (base_lexer.g4) ──
    /// `$name/path` (`VARIABLE_WITH_PATH`).
    #[regex(r"\$[a-z][a-zA-Z0-9_]*(/[a-zA-Z][a-zA-Z0-9_]*(\[[^\]\r\n]*\])?)+", |lex| lex.slice().to_owned())]
    VariableWithPath(String),
    /// `$name` (`VARIABLE_ID`).
    #[regex(r"\$[a-z][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    VariableId(String),

    // ── identifiers (base_lexer.g4) ──
    /// `ALPHA_UC_ID` — upper-initial identifier (RM type ids, ODIN keys).
    #[regex(r"[A-Z][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    AlphaUcId(String),
    /// `ALPHA_LC_ID` — lower-initial identifier (attribute ids, ODIN keys,
    /// section/artefact keywords).
    #[regex(r"[a-z][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    AlphaLcId(String),
    /// `ALPHA_UNDERSCORE_ID` — `_`-initial identifier (`_default`, meta ids).
    #[regex(r"_[a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    AlphaUnderscoreId(String),

    // ── symbols (base_lexer.g4) ──
    /// `(`
    #[token("(")]
    LParen,
    /// `)`
    #[token(")")]
    RParen,
    /// `[`
    #[token("[")]
    LBracket,
    /// `]`
    #[token("]")]
    RBracket,
    /// `{`
    #[token("{", |lex| opening_or(lex, &[(contained_regexp_rest, Token::ContainedRegexp)], Token::LCurly))]
    LCurly,
    /// `}`
    #[token("}")]
    RCurly,
    /// `,` (`SYM_COMMA`).
    #[token(",")]
    SymComma,
    /// `;` (`SYM_SEMI_COLON`).
    #[token(";")]
    SymSemiColon,
    /// `:=` / `::=` (`SYM_ASSIGNMENT`).
    #[token(":=")]
    #[token("::=")]
    SymAssignment,
    /// `:` — colon (rule declarations, quantifier binding).
    #[token(":")]
    SymColon,
    /// `/=` / `!=` / `≠` (`SYM_NE`), plus the ADL 1.4 spelling `<>` (ADL1.4
    /// `master06-assertions.adoc` §Equality Operators and its yacc `SYM_NE`).
    /// No 1.0.0 production admits any spelling — the ODIN pass refuses it.
    #[token("/=")]
    #[token("!=")]
    #[token("<>")]
    #[token("\u{2260}")]
    SymNe,
    /// `=` (`SYM_EQ`).
    #[token("=")]
    SymEq,
    /// `<=` / `≤` (`SYM_LE`).
    #[token("<=")]
    #[token("\u{2264}")]
    SymLe,
    /// `>=` / `≥` (`SYM_GE`).
    #[token(">=")]
    #[token("\u{2265}")]
    SymGe,
    /// `>` (`SYM_GT`).
    #[token(">")]
    SymGt,
    /// `<` (`SYM_LT`).
    #[token("<", |lex| opening_or(
        lex,
        &[(plug_in_rest, Token::PlugInBlock), (embedded_uri_rest, Token::EmbeddedUri)],
        Token::SymLt,
    ))]
    SymLt,
    /// `...` (`SYM_LIST_CONTINUE`).
    #[token("...")]
    SymListContinue,
    /// `..` (`SYM_IVL_SEP`).
    #[token("..")]
    SymIvlSep,
    /// `.` — the namespace separator of a qualified ODIN type identifier.
    ///
    /// NOTE: the vendored `odin.g4` `rm_type_id` admits only `ALPHA_UC_ID`,
    /// but the docs text is the oracle and allows the qualified form
    /// ("Namespaces are included by prepending package names, separated by
    /// the '.' character" — `LANG/docs/odin/master05-content` §Adding Type
    /// Information, verbatim in `AM/docs/ADL1.4/master04-dadl`); every dotted
    /// composite is a longer match, so logos still prefers it.
    #[token(".")]
    SymDot,
    /// `|` (`SYM_IVL_DELIM`).
    #[token("|")]
    SymIvlDelim,
    /// `¦` (`SYM_BROKEN_BAR`) — the EL quantifier body separator
    /// (`ElParser.g4` `elForAllExpr`/`elThereExistsExpr`). EL-only.
    #[token("\u{00A6}")]
    SymBrokenBar,
    /// `+/-` / `±` (`SYM_PLUS_OR_MINUS`).
    #[token("+/-")]
    #[token("\u{00B1}")]
    SymPlusOrMinus,
    /// `+` (`SYM_PLUS`).
    #[token("+")]
    SymPlus,
    /// `-` (`SYM_MINUS`).
    #[token("-")]
    SymMinus,
    /// `*` / `∗` (`SYM_STAR` — the "any" wildcard / multiplication).
    #[token("*")]
    #[token("\u{2217}")]
    SymStar,
    /// `/` (`SYM_SLASH` — division / bare path root).
    #[token("/")]
    SymSlash,
    /// `%` (`SYM_PERCENT`).
    #[token("%")]
    SymPercent,
    /// `^` (`SYM_CARAT` — exponent).
    #[token("^")]
    SymCarat,
    /// `@` — terminology binding separator in the OPT `[acN@terminology]` form.
    #[token("@")]
    SymAt,
}

/// Validate a `CHARACTER` token's escape and return the raw slice
/// (delimiters retained; the parser decodes).
fn validate_char(lex: &logos::Lexer<Token>) -> Result<String, ()> {
    let raw = lex.slice();
    crate::v1_0::escape::validate(raw).map_err(|_defect| ())?;
    Ok(raw.to_owned())
}

/// Validate a `STRING` token's `master03` escapes and return the raw slice
/// (delimiters retained; the parser decodes).
///
/// As [`validate_char`], the rules come from [`crate::v1_0::escape`]: the six
/// customary quoted forms plus `\uHHHH`/`\uHHHHHHHH`, and nothing else — "Any
/// other character combination starting with a backslash is illegal"
/// (`ADL2/master03-file_encoding.adoc` §Special Character Sequences). Refusing
/// at the lex keeps the token stream free of escapes the readers cannot
/// decode, which is what lets their parsers decode infallibly.
fn validate_string(lex: &logos::Lexer<Token>) -> Result<String, ()> {
    let raw = lex.slice();
    crate::v1_0::escape::validate(raw).map_err(|_defect| ())?;
    Ok(raw.to_owned())
}

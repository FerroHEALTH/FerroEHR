// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The shared token superset: one `logos` DFA covering the ADL-outer, cADL,
//! ODIN and BEL lexical layers.
//!
//! The patterns are transcribed from the authoritative vendored grammars
//! (`vendor/grammar/v1_1/base_lexer.g4`, `adl_keywords.g4`, `odin.g4`,
//! `odin_values.g4`, `base_expressions.g4`) and the normative chapters they
//! implement. Which of these tokens a given language actually produces is
//! decided by [`super::reclassify()`], not here — see the module docs of
//! [`super`] for the two-stage contract.
//!
//! NOTE: The Expression Language (`LANG/docs/EL/`) is deliberately NOT
//! part of the union: it uses `#`-prefixed codes, a different bracket algebra
//! and `|`-delimited comments, all of which would change the reading of text
//! in the four languages above. EL is DEVELOPMENT status with no vendored
//! grammar, so there is nothing normative to fold in.

use logos::Logos;

/// A token of the shared openEHR lexical superset.
///
/// Text-bearing variants hold the owned source slice verbatim (delimiters
/// included; the parsers strip and decode them). The one exception is
/// [`Token::String`] under the ODIN reading, whose payload has the multi-line
/// white-space leaders removed (`LANG/docs/odin/master07-leaf_data` §String
/// Data) — that transform is applied by the ODIN reclassification pass.
///
/// Keyword variants are UNIT variants. A language that does not reserve a
/// keyword does not get a differently-shaped token: the reclassification pass
/// re-tags the token to the identifier variant its own lexer would have
/// produced, reading the spelling back out of the source at the token's span.
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")] // WS / LINE (base_lexer.g4)
#[logos(skip(r"--[^\n]*", allow_greedy = true))] // CMT_LINE `-- … EOL` incl. the `----…` overlay separator (base_lexer.g4)
pub enum Token {
    /// A UTF-8 byte-order mark.
    ///
    /// `ADL2/master03-file_encoding.adoc` §File Encoding forbids it, but 18
    /// vendored ADL2 corpus sources carry one, so the ADL and ODIN readings
    /// tolerate it (a rejection would be a lexer-level FAIL the corpus does
    /// not intend) while the BEL reading refuses it, exactly as each
    /// language's lexer did before the union. It is a token rather than a
    /// `logos` skip purely so the per-language passes can make that choice;
    /// no returned token stream ever contains it.
    #[token("\u{feff}")]
    Bom,

    // ── ADL / cADL / BEL keywords (adl_keywords.g4, base_expressions.g4);
    //    text + unicode symbol forms fold to one variant. Exact-literal
    //    `#[token]`s outrank the ALPHA_*_ID regexes by logos priority, so
    //    keywords beat identifiers. ──
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
    ///
    /// EL admits the modal-logic box as the non-null assertion operator
    /// (`ElLexer.g4` `SYM_EXISTS : 'exists' | '□'`); no other reading has it.
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
    /// `in` — the BEL quantifier binding keyword (`for_all v in coll`,
    /// `base_expressions.g4` `for_all_expr`).
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

    // ── EL-only word keywords (`ElLexer.g4`). No `#[token]`: the shared DFA
    //    reads each as the identifier every other layer sees, and only the EL
    //    reclassification re-tags it, so no other reading changes. ──
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

    // ── ISO8601 date/time/duration VALUES (base_lexer.g4) ──
    //
    // The three date/time VALUE tokens carry an explicit high priority so they
    // win every tie against the constraint-PATTERN tokens below, whose fields
    // admit literal date/time numbers (`ADL1.4/master05-cadl.adoc` §Patterns).
    // Where a text is BOTH a legal value and a legal all-literal pattern
    // (`1995-??-??`), the value reading wins; the pattern tokens keep the shapes
    // only they match (`1995-??-XX`, `1995-mm-dd`).
    /// `ISO8601_DATE_TIME` (with optional partial `??` fields / timezone).
    ///
    /// The `??`-partial family covers the whole set of
    /// `AM/docs/ADL1.4/master04-dadl` §Partial Date/Times, including the
    /// `yyyy-MM-ddT??:??:??`, `yyyy-MM-??T??:??:??` and `yyyy-??-??T??:??:??`
    /// forms.
    #[regex(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T([0-9]{2}(:[0-9]{2}(:([0-9]{2}([.,][0-9]+)?|\?\?))?|:\?\?:\?\?)?|\?\?:\?\?:\?\?)(Z|[+\-][0-9]{4})?",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    #[regex(
        r"[0-9]{4}-([0-9]{2}-\?\?|\?\?-\?\?)T\?\?:\?\?:\?\?(Z|[+\-][0-9]{4})?",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    // The space-separated date/time form of the `AM/docs/ADL1.4/master08-adl`
    // §Revision History Section example (`time_committed = <2004-11-02
    // 09:31:04+1000>`). That example contradicts its own chapter set —
    // `master04-dadl` §Complete Date/Times mandates the ISO 8601 extended form
    // with the `T` designator, and neither the dADL lex rules nor
    // `base_lexer.g4` `ISO8601_DATE_TIME` admit a space — so the form is a
    // deliberate ODIN-only widening: the ODIN pass normalises it to the `T`
    // form on read, and the ADL/BEL passes re-tag or split it back.
    #[regex(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}(:[0-9]{2}([.,][0-9]+)?)?(Z|[+\-][0-9]{4})?",
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
        r"[0-9]{2}:([0-9]{2}(:([0-9]{2}([.,][0-9]+)?|\?\?))?|\?\?:\?\?)(Z|[+\-][0-9]{4})?",
        |lex| lex.slice().to_owned(),
        priority = 30
    )]
    Iso8601Time(String),
    /// `ISO8601_DURATION`, requiring at least one component (never bare `P`).
    #[regex(
        r"-?P([0-9]+[YyMmWwDd])+(T([0-9]+[HhMm])*[0-9]+([.,][0-9]+)?[Ss]|T([0-9]+[Hh])?([0-9]+[Mm])?([0-9]+([.,][0-9]+)?[Ss])?)?|-?PT([0-9]+[HhMm])*[0-9]+([.,][0-9]+)?[Ss]|-?PT([0-9]+[Hh])([0-9]+[Mm])?|-?PT[0-9]+[Mm]",
        |lex| lex.slice().to_owned()
    )]
    Iso8601Duration(String),

    // ── constraint PATTERNS (base_lexer.g4 + `ADL1.4/master05-cadl.adoc`
    //    §Symbols L1415-1426) ──
    //
    // Three spec-grounded widenings over `base_lexer.g4`'s transcription, all
    // supersets: every field also admits a LITERAL date/time number (master05
    // L894); the timezone modifier admits the ASCII `+`/`-` forms, not only `±`
    // (§Patterns L852 + the timezone table L900-906); and the date/time
    // separator is `[T ]` per `V_ISO8601_DATE_TIME_CONSTRAINT_PATTERN` (L1422).
    // NOTE: the explicit low priority keeps the date/time VALUE tokens above
    // winning every equal-length tie.
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
    ///
    /// Can be identical to an uppercase identifier (`PWD`); the higher token
    /// priority classifies the real pattern while longer type names
    /// (`POINT_EVENT`) win by length. Date/time constraint patterns contain
    /// `-`/`:` and never collide.
    #[regex(
        r"P([Yy][Mm]?[Ww]?[Dd]?|[Mm][Ww]?[Dd]?|[Ww][Dd]?|[Dd])(T([Hh][Mm]?[Ss]?|[Mm][Ss]?|[Ss]))?|PT([Hh][Mm]?[Ss]?|[Mm][Ss]?|[Ss])",
        |lex| lex.slice().to_owned(),
        priority = 20
    )]
    DurationConstraintPattern(String),

    // ── composed primitives (base_lexer.g4) ──
    /// A delimited contained regexp `{ /re/ }` / `{ ^re^ }` with optional
    /// `;"assumed"` (`CONTAINED_REGEXP`).
    #[regex(
        r#"\{[ \t\r]*(/([^/\r\n\\]|\\.)+/|\^([^\^\r\n\\]|\\.)+\^)[ \t\r]*(;[ \t\r]*"([^"\\]|\\.)*")?[ \t\r]*\}"#,
        |lex| lex.slice().to_owned()
    )]
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
    ///
    /// The pattern deliberately requires an ALPHA first character (narrower
    /// than the 1.4 `ALPHANUM`) so that the container keys `[1]`, `[01234]`
    /// and `[2004-06-11]` keep lexing as `'[' key ']'`, which the 1.4 lex rule
    /// would otherwise swallow whole. cADL and BEL have no such production, so
    /// their passes split the token back into `'['`, the inner code and `']'`.
    #[regex(r"\[[a-zA-Z][a-zA-Z0-9._\-]*\]", |lex| lex.slice().to_owned())]
    LocalTermCodeRef(String),
    /// A plug-in-syntax object block `<# … #>`, captured verbatim including
    /// the delimiters (`LANG/docs/odin/master09-plug_in_syntaxes`: "the `<>`
    /// delimiters are modified to `<# #>`, to allow for easier parser design";
    /// the delimiters are reserved by `master03-basics` §Reserved
    /// Characters). ODIN-only — the vendored ADL/BEL grammars have no `#`
    /// production at all, so the other readings refuse the token. The body is
    /// raw foreign text ("expressed in some other syntax"), never lexed.
    #[regex(r"<#([^#]|#[^>])*#*#>", |lex| lex.slice().to_owned())]
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
    #[regex(r"<[ \t\r\n]*[a-zA-Z][a-zA-Z0-9+.\-]*:[^>]*>", |lex| lex.slice().to_owned())]
    EmbeddedUri(String),
    /// An ADL path (`base_lexer.g4 ADL_PATH`), each segment
    /// `ALPHA_*_ID ('[' predicate ']')?`.
    ///
    /// The union of the three languages' path productions: cADL's absolute
    /// `(/seg)+` and relative `seg(/seg)+` with **lower-case-initial** segment
    /// heads; ODIN's object-reference path, whose `odin.g4` segment head takes
    /// either case; and BEL's additional movable-path leader `//seg…` (ADL1.4
    /// `master07-paths.adoc` §Grammar `movable_path: SYM_MOVABLE_LEADER
    /// relative_path`) and single-segment-with-predicate form `seg[pred]`.
    /// Each pass keeps only the shapes its own production admits.
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
    /// data). BEL's `base_expressions.g4` has no exponent form, so its pass
    /// splits the suffix off.
    #[regex(r"[0-9]+([eE][+\-]?[0-9]+)?", |lex| lex.slice().to_owned())]
    Integer(String),
    /// A double-quoted `STRING` (may span lines); escapes validated per
    /// `master03` (`\r \n \t \\ \" \'` + `\uHHHH`/`\uHHHHHHHH`).
    #[regex(r#""([^"\\]|\\.)*""#, validate_string)]
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
    #[token("{")]
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
    /// `master06-assertions.adoc` §Equality Operators and its yacc `SYM_NE`),
    /// which only the BEL reading admits.
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
    #[token("<")]
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
///
/// The escape rules are [`crate::v1_1::escape`]'s — the ONE `master03`
/// implementation the whole workspace shares — so an illegal escape fails the
/// lex here rather than being judged a second time against a drifting local
/// copy of the same rules. A `\uHHHH` form cannot fit the single-character
/// token, and the shared decoder refuses it as a malformed unicode escape.
fn validate_char(lex: &logos::Lexer<Token>) -> Result<String, ()> {
    let raw = lex.slice();
    crate::v1_1::escape::validate(raw).map_err(|_defect| ())?;
    Ok(raw.to_owned())
}

/// Validate a `STRING` token's `master03` escapes and return the raw slice
/// (delimiters retained; the parser decodes).
///
/// As [`validate_char`], the rules come from [`crate::v1_1::escape`]: the six
/// customary quoted forms plus `\uHHHH`/`\uHHHHHHHH`, and nothing else — "Any
/// other character combination starting with a backslash is illegal"
/// (`ADL2/master03-file_encoding.adoc` §Special Character Sequences). Refusing
/// at the lex keeps the token stream free of escapes the readers cannot
/// decode, which is what lets their parsers decode infallibly.
fn validate_string(lex: &logos::Lexer<Token>) -> Result<String, ()> {
    let raw = lex.slice();
    crate::v1_1::escape::validate(raw).map_err(|_defect| ())?;
    Ok(raw.to_owned())
}

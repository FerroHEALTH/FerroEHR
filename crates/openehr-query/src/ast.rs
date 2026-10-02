// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL abstract syntax tree, transcribed from `AqlParser.g4` (vendored at
//! `vendor/grammar/`).
//!
//! Each grammar rule maps to a type here; [`crate::parser`] builds these from
//! the [`crate::lexer`] token stream. This is the SYNTACTIC tree only —
//! resolving paths and typing quoted temporal literals are semantic concerns.
//!
//! Every [`IdentifiedPath`] and every WHERE condition carries a [`Span`]: where
//! it was written in the source, so a diagnostic can name a position instead of
//! echoing the text there. Spans never take part in equality.

use std::ops::Range;

use crate::lexer::CompOp;

/// Where a node was written: a half-open byte range into the AQL source.
///
/// A span is position metadata, not syntax, so equality ignores it: two nodes
/// are equal when their syntax is, wherever each was written. That keeps the
/// printer's round-trip invariant (`parse(to_aql(ast)) == ast`) a statement
/// about syntax alone. A node built in code, or parsed from a bare token slice
/// ([`crate::parser::parse`]), has no source position and carries the
/// [`Span::default`] unknown span.
///
/// Offsets are held as `u32`, which keeps a span small enough to ride on every
/// path; a position past 4 GiB of source is not representable and reads as
/// unknown.
#[derive(Debug, Clone, Copy, Default)]
pub struct Span {
    bytes: Option<(u32, u32)>,
}

impl Span {
    /// A span over `bytes` of the source.
    #[must_use]
    pub fn new(bytes: Range<usize>) -> Self {
        let start = u32::try_from(bytes.start).ok();
        let end = u32::try_from(bytes.end).ok();
        Self {
            bytes: start.zip(end),
        }
    }

    /// The half-open byte range of the source this node was written at, or
    /// `None` when the node has no source position.
    #[must_use]
    pub fn bytes(&self) -> Option<Range<usize>> {
        let (start, end) = self.bytes?;
        let start = usize::try_from(start).ok()?;
        let end = usize::try_from(end).ok()?;
        Some(start..end)
    }
}

impl PartialEq for Span {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for Span {}

/// `selectQuery : selectClause fromClause whereClause? orderByClause? limitClause?`
#[derive(Debug, Clone, PartialEq)]
pub struct SelectQuery {
    /// `SELECT [DISTINCT] [TOP n] col, …`
    pub select: SelectClause,
    /// `FROM <containsExpr>`
    pub from: ContainsExpr,
    /// `WHERE <whereExpr>`
    pub where_: Option<WhereExpr>,
    /// `ORDER BY …`
    pub order_by: Vec<OrderByExpr>,
    /// `LIMIT n [OFFSET m]`
    pub limit: Option<Limit>,
}

/// `selectClause : SELECT DISTINCT? top? selectExpr (',' selectExpr)*`
#[derive(Debug, Clone, PartialEq)]
pub struct SelectClause {
    /// `DISTINCT` present.
    pub distinct: bool,
    /// The deprecated `TOP n [FORWARD|BACKWARD]`.
    pub top: Option<Top>,
    /// One or more selected columns.
    pub columns: Vec<SelectExpr>,
}

/// The deprecated `top : TOP INTEGER (FORWARD|BACKWARD)?`.
#[derive(Debug, Clone, PartialEq)]
pub struct Top {
    /// The row count.
    pub count: i64,
    /// Optional direction.
    pub direction: Option<TopDirection>,
}

/// `TOP` direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopDirection {
    /// `FORWARD`
    Forward,
    /// `BACKWARD`
    Backward,
}

/// `selectExpr : columnExpr (AS aliasName)?`
#[derive(Debug, Clone, PartialEq)]
pub struct SelectExpr {
    /// The selected expression.
    pub column: ColumnExpr,
    /// Optional `AS alias`.
    pub alias: Option<String>,
}

/// `columnExpr : identifiedPath | primitive | aggregateFunctionCall | functionCall`
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnExpr {
    /// A data path.
    Path(IdentifiedPath),
    /// A literal.
    Primitive(Primitive),
    /// `COUNT`/`MIN`/`MAX`/`SUM`/`AVG`.
    Aggregate(AggregateCall),
    /// A named function call.
    Function(FunctionCall),
}

/// `containsExpr` — a boolean tree whose leaves are class expressions each
/// optionally constrained by a nested `CONTAINS`.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainsExpr {
    /// `classExprOperand (NOT? CONTAINS containsExpr)?`
    Contained {
        /// The class/version operand.
        operand: ClassExprOperand,
        /// Optional `[NOT] CONTAINS <sub>`.
        contains: Option<Box<ContainsConstraint>>,
    },
    /// `containsExpr AND containsExpr`
    And(Box<ContainsExpr>, Box<ContainsExpr>),
    /// `containsExpr OR containsExpr`
    Or(Box<ContainsExpr>, Box<ContainsExpr>),
}

/// The `[NOT] CONTAINS <containsExpr>` tail of a [`ContainsExpr::Contained`].
#[derive(Debug, Clone, PartialEq)]
pub struct ContainsConstraint {
    /// `NOT CONTAINS`.
    pub negated: bool,
    /// The contained sub-expression.
    pub expr: ContainsExpr,
}

/// `classExprOperand : #classExpression | #versionClassExpr`
#[derive(Debug, Clone, PartialEq)]
pub enum ClassExprOperand {
    /// `IDENTIFIER variable? pathPredicate?` (e.g. `COMPOSITION c[openEHR-…]`).
    Class {
        /// The RM class name.
        rm_type: String,
        /// Optional bound variable.
        variable: Option<String>,
        /// Optional `[predicate]`.
        predicate: Option<PathPredicate>,
    },
    /// `VERSION variable? [versionPredicate]?`
    Version {
        /// Optional bound variable.
        variable: Option<String>,
        /// Optional version predicate.
        predicate: Option<VersionPredicate>,
    },
}

/// `whereExpr` — a boolean tree of [`IdentifiedExpr`] leaves.
#[derive(Debug, Clone, PartialEq)]
pub enum WhereExpr {
    /// A leaf condition, and where it was written.
    Identified(IdentifiedExpr, Span),
    /// `NOT whereExpr`
    Not(Box<WhereExpr>),
    /// `whereExpr AND whereExpr`
    And(Box<WhereExpr>, Box<WhereExpr>),
    /// `whereExpr OR whereExpr`
    Or(Box<WhereExpr>, Box<WhereExpr>),
}

impl WhereExpr {
    /// A leaf condition with no source position, for a tree built in code.
    #[must_use]
    pub fn identified(expr: IdentifiedExpr) -> Self {
        Self::Identified(expr, Span::default())
    }
}

/// `identifiedExpr` — a single WHERE condition.
#[derive(Debug, Clone, PartialEq)]
pub enum IdentifiedExpr {
    /// `EXISTS identifiedPath`
    Exists(IdentifiedPath),
    /// `identifiedPath|functionCall COMPARISON_OPERATOR terminal`
    Compare {
        /// Left operand.
        lhs: CompareOperand,
        /// The operator.
        op: CompOp,
        /// Right operand.
        rhs: Terminal,
    },
    /// `identifiedPath LIKE likeOperand`
    Like {
        /// The path.
        path: IdentifiedPath,
        /// String or parameter.
        operand: LikeOperand,
    },
    /// `identifiedPath MATCHES matchesOperand`
    Matches {
        /// The path.
        path: IdentifiedPath,
        /// The match set.
        operand: MatchesOperand,
    },
    /// A condition already resolved to a constant during semantic analysis —
    /// the product of evaluating a `TERMINOLOGY()` Boolean value expression
    /// (QUERY master03 §TERMINOLOGY: "as a Boolean value expression"). Never
    /// produced by the parser.
    Resolved(bool),
}

/// Left side of a comparison: a path or a function call.
#[derive(Debug, Clone, PartialEq)]
pub enum CompareOperand {
    /// `identifiedPath`
    Path(IdentifiedPath),
    /// `functionCall`
    Function(FunctionCall),
}

/// `terminal : primitive | PARAMETER | identifiedPath | functionCall`
#[derive(Debug, Clone, PartialEq)]
pub enum Terminal {
    /// A literal.
    Primitive(Primitive),
    /// `$param`.
    Parameter(String),
    /// A path.
    Path(IdentifiedPath),
    /// A function call.
    Function(FunctionCall),
}

/// `orderByExpr : identifiedPath (DESCENDING|DESC|ASCENDING|ASC)?`
#[derive(Debug, Clone, PartialEq)]
pub struct OrderByExpr {
    /// The path to order by.
    pub path: IdentifiedPath,
    /// Sort direction (defaults to ascending when absent).
    pub order: Option<SortOrder>,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// `ASC` / `ASCENDING`
    Ascending,
    /// `DESC` / `DESCENDING`
    Descending,
}

/// `limitClause : LIMIT INTEGER (OFFSET INTEGER)?`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limit {
    /// Row limit.
    pub limit: i64,
    /// Optional offset.
    pub offset: Option<i64>,
}

/// `identifiedPath : IDENTIFIER pathPredicate? ('/' objectPath)?`
#[derive(Debug, Clone, PartialEq)]
pub struct IdentifiedPath {
    /// The root variable/identifier.
    pub root: String,
    /// Optional predicate on the root.
    pub predicate: Option<PathPredicate>,
    /// Optional trailing `/a/b/c` object path.
    pub path: Option<ObjectPath>,
    /// Where the path was written, root to last part.
    pub span: Span,
}

impl IdentifiedPath {
    /// A path with no source position, for a tree built in code.
    #[must_use]
    pub fn new(root: String, predicate: Option<PathPredicate>, path: Option<ObjectPath>) -> Self {
        Self {
            root,
            predicate,
            path,
            span: Span::default(),
        }
    }
}

/// `objectPath : pathPart ('/' pathPart)*`
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectPath {
    /// The `/`-separated parts.
    pub parts: Vec<PathPart>,
}

/// `pathPart : IDENTIFIER pathPredicate?`
#[derive(Debug, Clone, PartialEq)]
pub struct PathPart {
    /// The attribute name.
    pub name: String,
    /// Optional `[predicate]`.
    pub predicate: Option<PathPredicate>,
}

/// `pathPredicate : '[' (standardPredicate | archetypePredicate | nodePredicate) ']'`
#[derive(Debug, Clone, PartialEq)]
pub enum PathPredicate {
    /// `objectPath COMPARISON_OPERATOR operand`
    Standard(Box<StandardPredicate>),
    /// `ARCHETYPE_HRID | PARAMETER`
    Archetype(ArchetypePredicate),
    /// A node predicate (id/at code, archetype id, boolean tree, …).
    Node(Box<NodePredicate>),
}

/// `standardPredicate : objectPath COMPARISON_OPERATOR pathPredicateOperand`
#[derive(Debug, Clone, PartialEq)]
pub struct StandardPredicate {
    /// The constrained path.
    pub path: ObjectPath,
    /// The operator.
    pub op: CompOp,
    /// The operand.
    pub operand: PathPredicateOperand,
}

/// `archetypePredicate : ARCHETYPE_HRID | PARAMETER`
#[derive(Debug, Clone, PartialEq)]
pub enum ArchetypePredicate {
    /// An archetype HRID.
    Hrid(String),
    /// `$param`.
    Parameter(String),
}

/// `nodePredicate` — the (partial) common forms: a node code with optional
/// name/term, an archetype id with optional name/term, a parameter, a standard
/// comparison, or a boolean combination.
#[derive(Debug, Clone, PartialEq)]
pub enum NodePredicate {
    /// `(ID_CODE|AT_CODE) (',' (STRING|PARAMETER|TERM_CODE|AT_CODE|ID_CODE))?`
    Code {
        /// The `id`/`at` code.
        code: String,
        /// Optional name/term operand.
        name: Option<NodeNameConstraint>,
    },
    /// `ARCHETYPE_HRID (',' …)?`
    Archetype {
        /// The archetype HRID.
        hrid: String,
        /// Optional name/term operand.
        name: Option<NodeNameConstraint>,
    },
    /// `$param`
    Parameter(String),
    /// `objectPath COMPARISON_OPERATOR operand`
    Standard(Box<StandardPredicate>),
    /// `objectPath MATCHES CONTAINED_REGEX`
    MatchesRegex {
        /// The constrained path.
        path: ObjectPath,
        /// The raw `{/regex/}` token text.
        regex: String,
    },
    /// `nodePredicate AND nodePredicate`
    And(Box<NodePredicate>, Box<NodePredicate>),
    /// `nodePredicate OR nodePredicate`
    Or(Box<NodePredicate>, Box<NodePredicate>),
}

/// The optional name/term after a node code in a [`NodePredicate`].
#[derive(Debug, Clone, PartialEq)]
pub enum NodeNameConstraint {
    /// A quoted name.
    String(String),
    /// `$param`.
    Parameter(String),
    /// A term code.
    TermCode(String),
    /// An `at`/`id` code.
    Code(String),
}

/// `versionPredicate : LATEST_VERSION | ALL_VERSIONS | standardPredicate`
#[derive(Debug, Clone, PartialEq)]
pub enum VersionPredicate {
    /// `LATEST_VERSION`
    Latest,
    /// `ALL_VERSIONS`
    All,
    /// A standard comparison predicate.
    Standard(Box<StandardPredicate>),
}

/// `pathPredicateOperand : primitive | objectPath | PARAMETER | ID_CODE | AT_CODE`
#[derive(Debug, Clone, PartialEq)]
pub enum PathPredicateOperand {
    /// A literal.
    Primitive(Primitive),
    /// A path.
    Path(ObjectPath),
    /// `$param`.
    Parameter(String),
    /// An `id`/`at` code.
    Code(String),
}

/// `likeOperand : STRING | PARAMETER`
#[derive(Debug, Clone, PartialEq)]
pub enum LikeOperand {
    /// A quoted pattern.
    String(String),
    /// `$param`.
    Parameter(String),
}

/// `matchesOperand : '{' valueListItem (',' valueListItem)* '}' | terminologyFunction | '{' URI '}'`
#[derive(Debug, Clone, PartialEq)]
pub enum MatchesOperand {
    /// A `{ … }` value list.
    ValueList(Vec<ValueListItem>),
    /// `terminology(...)`.
    Terminology(TerminologyFunction),
    /// `{ uri }`.
    Uri(String),
}

/// `valueListItem : primitive | PARAMETER | terminologyFunction`
#[derive(Debug, Clone, PartialEq)]
pub enum ValueListItem {
    /// A literal.
    Primitive(Primitive),
    /// `$param`.
    Parameter(String),
    /// `terminology(...)`.
    Terminology(TerminologyFunction),
}

/// `functionCall` — a named function with terminal arguments (also covers
/// `terminologyFunction`, kept as [`FunctionCall::Terminology`]).
///
/// A name is classified against the built-in functions QUERY master03
/// §Functions defines (the grammar's `STRING_FUNCTION_ID`,
/// `NUMERIC_FUNCTION_ID` and `DATE_TIME_FUNCTION_ID` groups); a name outside
/// them is [`FunctionCall::Other`]. Both keep the name as spelled, so the
/// printer round-trips it.
#[derive(Debug, Clone, PartialEq)]
pub enum FunctionCall {
    /// `name ( terminal, … )` naming an AQL built-in function.
    Builtin {
        /// The function the name denotes.
        function: BuiltinFunction,
        /// The name as written (AQL names are case-insensitive).
        name: String,
        /// The arguments.
        args: Vec<Terminal>,
    },
    /// `IDENTIFIER ( terminal, … )` naming no AQL built-in function: the
    /// spec's "various other functions may exist however in various AQL
    /// implementations" (QUERY master03 §Functions).
    Other {
        /// The name as written.
        name: String,
        /// The arguments.
        args: Vec<Terminal>,
    },
    /// `terminology(str, str, str)`.
    Terminology(TerminologyFunction),
}

impl FunctionCall {
    /// The call `name(args)`, classified: a built-in function when the name
    /// denotes one, [`FunctionCall::Other`] otherwise.
    #[must_use]
    pub fn named(name: String, args: Vec<Terminal>) -> Self {
        match BuiltinFunction::from_name(&name) {
            Some(function) => Self::Builtin {
                function,
                name,
                args,
            },
            None => Self::Other { name, args },
        }
    }
}

/// An AQL built-in single-row function (QUERY master03 §Functions), by group.
///
/// The aggregates and `TERMINOLOGY` are not here: their argument grammar
/// differs, so the AST carries them as [`AggregateCall`] and
/// [`TerminologyFunction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinFunction {
    /// A string function (`STRING_FUNCTION_ID`).
    String(StringFunction),
    /// A numeric function (`NUMERIC_FUNCTION_ID`).
    Numeric(NumericFunction),
    /// A date and time function (`DATE_TIME_FUNCTION_ID`).
    DateTime(DateTimeFunction),
}

impl BuiltinFunction {
    /// The built-in function `name` denotes, compared case-insensitively, or
    /// `None` when AQL defines no function of that name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        StringFunction::from_name(name)
            .map(Self::String)
            .or_else(|| NumericFunction::from_name(name).map(Self::Numeric))
            .or_else(|| DateTimeFunction::from_name(name).map(Self::DateTime))
    }

    /// The function's name as the specification spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String(f) => f.as_str(),
            Self::Numeric(f) => f.as_str(),
            Self::DateTime(f) => f.as_str(),
        }
    }
}

/// Declares one function-id group: the enum, its spelled names, and the
/// case-insensitive lookup.
macro_rules! function_group {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $spelling:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        impl $name {
            /// Every function of the group, in specification order.
            pub const ALL: &[Self] = &[$(Self::$variant,)+];

            /// The function `name` denotes, compared case-insensitively.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|f| f.as_str().eq_ignore_ascii_case(name))
            }

            /// The function's name as the specification spells it.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)+
                }
            }
        }
    };
}

function_group! {
    /// A string function (QUERY master03 §Functions/String functions).
    StringFunction {
        /// `LENGTH(expression)`.
        Length => "LENGTH",
        /// `CONTAINS(expression, substring)`, the string function, not the
        /// containment operator.
        Contains => "CONTAINS",
        /// `POSITION(substring, expression)`.
        Position => "POSITION",
        /// `SUBSTRING(expression, position[, length])`.
        Substring => "SUBSTRING",
        /// `CONCAT(expr1, expr2, …)`.
        Concat => "CONCAT",
        /// `CONCAT_WS(separator, expr1, expr2, …)`.
        ConcatWs => "CONCAT_WS",
    }
}

function_group! {
    /// A numeric function (QUERY master03 §Functions/Numeric functions).
    NumericFunction {
        /// `ABS(expression)`.
        Abs => "ABS",
        /// `MOD(x, y)`.
        Mod => "MOD",
        /// `CEIL(expression)`.
        Ceil => "CEIL",
        /// `FLOOR(expression)`.
        Floor => "FLOOR",
        /// `ROUND(expression[, decimal])`.
        Round => "ROUND",
    }
}

function_group! {
    /// A date and time function (QUERY master03 §Functions/Date and time
    /// functions).
    DateTimeFunction {
        /// `CURRENT_DATE()`.
        CurrentDate => "CURRENT_DATE",
        /// `CURRENT_TIME()`.
        CurrentTime => "CURRENT_TIME",
        /// `CURRENT_DATE_TIME()`.
        CurrentDateTime => "CURRENT_DATE_TIME",
        /// `NOW()`, the specification's alias for `CURRENT_DATE_TIME()`, kept
        /// apart so the query prints as written.
        Now => "NOW",
        /// `CURRENT_TIMEZONE()`.
        CurrentTimezone => "CURRENT_TIMEZONE",
    }
}

/// `terminologyFunction : TERMINOLOGY '(' STRING ',' STRING ',' STRING ')'`
#[derive(Debug, Clone, PartialEq)]
pub struct TerminologyFunction {
    /// First argument (operation).
    pub operation: String,
    /// Second argument.
    pub arg2: String,
    /// Third argument.
    pub arg3: String,
}

/// `aggregateFunctionCall`
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateCall {
    /// `COUNT ( DISTINCT? identifiedPath | '*' )`
    Count {
        /// `DISTINCT` present.
        distinct: bool,
        /// The path, or `None` for `COUNT(*)`.
        path: Option<IdentifiedPath>,
    },
    /// `MIN|MAX|SUM|AVG ( identifiedPath )`
    Stat {
        /// Which aggregate.
        func: StatFunc,
        /// The path.
        path: IdentifiedPath,
    },
}

/// The non-count aggregate functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatFunc {
    /// `MIN`
    Min,
    /// `MAX`
    Max,
    /// `SUM`
    Sum,
    /// `AVG`
    Avg,
}

/// `primitive` — an AQL literal.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    /// A quoted string (unescaping/temporal-typing deferred; raw slice sans
    /// surrounding quotes).
    String(String),
    /// An integer literal.
    Integer(i64),
    /// A real literal.
    Real(f64),
    /// A boolean literal.
    Boolean(bool),
    /// `NULL`.
    Null,
}

// ── path text rendering (RESULT_SET column `path`) ─────────────────────────────
//
// ITS-REST 1.1.0 RESULT_SET columns carry the SELECT expression's path
// (`{"name": "#0", "path": "/ehr_id/value"}`) exactly as written in the query,
// minus the root variable, rather than a normalized reconstruction.

use std::fmt;

impl IdentifiedPath {
    /// The `RESULT_SET` column-path text of this select expression: the trailing
    /// object path (with predicates) as written, `/`-prefixed; a bare variable
    /// (whole-object select) renders as `"/"`.
    #[must_use]
    pub fn column_path_text(&self) -> String {
        match &self.path {
            None => "/".to_owned(),
            Some(p) => format!("/{p}"),
        }
    }
}

impl fmt::Display for ObjectPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            write!(f, "{part}")?;
        }
        Ok(())
    }
}

impl fmt::Display for PathPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if let Some(p) = &self.predicate {
            write!(f, "[{p}]")?;
        }
        Ok(())
    }
}

impl fmt::Display for PathPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathPredicate::Standard(s) => write!(f, "{s}"),
            PathPredicate::Archetype(ArchetypePredicate::Hrid(h)) => f.write_str(h),
            PathPredicate::Archetype(ArchetypePredicate::Parameter(p)) => f.write_str(p),
            PathPredicate::Node(n) => write!(f, "{n}"),
        }
    }
}

impl fmt::Display for StandardPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{}", self.path, comp_op_text(self.op), self.operand)
    }
}

/// The AQL surface symbol of a comparison operator.
#[must_use]
pub fn comp_op_text(op: CompOp) -> &'static str {
    match op {
        CompOp::Eq => "=",
        CompOp::Ne => "!=",
        CompOp::Gt => ">",
        CompOp::Ge => ">=",
        CompOp::Lt => "<",
        CompOp::Le => "<=",
    }
}

impl fmt::Display for NodePredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodePredicate::Code { code, name } | NodePredicate::Archetype { hrid: code, name } => {
                f.write_str(code)?;
                if let Some(n) = name {
                    write!(f, ",{n}")?;
                }
                Ok(())
            }
            NodePredicate::Parameter(p) => f.write_str(p),
            NodePredicate::Standard(s) => write!(f, "{s}"),
            NodePredicate::MatchesRegex { path, regex } => {
                write!(f, "{path} matches {regex}")
            }
            NodePredicate::And(a, b) => write!(f, "{a} and {b}"),
            NodePredicate::Or(a, b) => write!(f, "{a} or {b}"),
        }
    }
}

impl fmt::Display for NodeNameConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeNameConstraint::String(s) => {
                write!(f, "'{}'", crate::printer::escape_string(s))
            }
            NodeNameConstraint::Parameter(p) | NodeNameConstraint::Code(p) => f.write_str(p),
            NodeNameConstraint::TermCode(t) => f.write_str(t),
        }
    }
}

impl fmt::Display for PathPredicateOperand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathPredicateOperand::Primitive(p) => write!(f, "{p}"),
            PathPredicateOperand::Path(p) => write!(f, "{p}"),
            PathPredicateOperand::Parameter(p) | PathPredicateOperand::Code(p) => f.write_str(p),
        }
    }
}

impl fmt::Display for Primitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Primitive::String(s) => write!(f, "'{}'", crate::printer::escape_string(s)),
            Primitive::Integer(i) => write!(f, "{i}"),
            Primitive::Real(r) => write!(f, "{r}"),
            Primitive::Boolean(b) => write!(f, "{b}"),
            Primitive::Null => f.write_str("NULL"),
        }
    }
}

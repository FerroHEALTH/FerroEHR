// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Binding ITS-REST `query_parameters` into a parsed query.
//!
//! ITS-REST carries the parameter values beside the AQL text, keyed by name
//! without the `$` (`ITS-REST/specifications/docs/query/Request.md` §Query
//! parameters), and QUERY substitutes each "following the same rules as each
//! type when the value is specified as a literal" (`QUERY/docs/AQL/
//! master03-syntax.adoc` §Parameters). [`bind`] does that substitution on the
//! AST, never on the text: every `$name` becomes a typed literal the printer
//! escapes, so no value is ever spliced into AQL source.
//!
//! A parameter that cannot be bound is reported by NAME and by the position it
//! stands in, never by its value: a value may be a patient identifier, and the
//! error travels into logs and HTTP responses.
//!
//! # Examples
//!
//! ```
//! use openehr_query::ast::Primitive;
//! use openehr_query::bind::{Parameters, bind};
//! use openehr_query::parser::parse_str;
//! use openehr_query::printer::to_aql;
//!
//! let mut query = parse_str("SELECT c/uid/value FROM COMPOSITION c WHERE c/uid/value = $uid")?;
//! let mut parameters = Parameters::new();
//! parameters.insert("uid", Primitive::String("8849182c-82ad-4088-a07f-48ead4180515::node::1".into()));
//! bind(&mut query, &parameters)?;
//! assert!(to_aql(&query).ends_with("='8849182c-82ad-4088-a07f-48ead4180515::node::1'"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;

use crate::ast::{
    ArchetypePredicate, LikeOperand, NodeNameConstraint, NodePredicate, PathPredicateOperand,
    Primitive, SelectQuery, Terminal, ValueListItem,
};
use crate::lexer::Token;
use crate::visit::{
    VisitMut, walk_node_predicate_mut, walk_path_predicate_operand_mut, walk_terminal_mut,
    walk_value_list_item_mut,
};

/// The parameter values supplied beside a query, keyed by name.
///
/// A name is stored without its `$`: ITS-REST says supplied names "SHOULD NOT
/// be prefixed with `$` sign" and leaves the server to add it, so a supplied
/// `$uid` and `uid` name the same parameter, and supplying both is a
/// [`FaultKind::Duplicate`]. `Debug` lists the names only, never a value.
#[derive(Clone, Default)]
pub struct Parameters {
    supplied: BTreeMap<String, Supplied>,
}

/// One supplied entry: a value, or the reason it can never be bound.
#[derive(Clone)]
enum Supplied {
    Value(Primitive),
    Refused(FaultKind),
}

impl Parameters {
    /// An empty parameter set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `value` under `name`, with or without its leading `$`.
    ///
    /// A name supplied twice is kept as a [`FaultKind::Duplicate`], so the
    /// conflict is reported at [`bind`] rather than one value silently winning.
    pub fn insert(&mut self, name: &str, value: Primitive) {
        self.supply(name, Supplied::Value(value));
    }

    /// Records that `name` was supplied with a value that is no AQL literal,
    /// so [`bind`] reports it as [`FaultKind::NotALiteral`] by name.
    ///
    /// For an ITS-REST `query_parameters` member that is a JSON object or
    /// array, or a number no [`Primitive`] holds without losing its value: the
    /// caller decodes the JSON, and only it can see such a member.
    pub fn insert_not_a_literal(&mut self, name: &str) {
        self.supply(name, Supplied::Refused(FaultKind::NotALiteral));
    }

    /// The supplied names, without their `$`, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.supplied.keys().map(String::as_str)
    }

    /// Whether no parameter was supplied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.supplied.is_empty()
    }

    fn supply(&mut self, name: &str, entry: Supplied) {
        let name = name.strip_prefix('$').unwrap_or(name).to_owned();
        match self.supplied.entry(name) {
            Entry::Occupied(mut held) => {
                held.insert(Supplied::Refused(FaultKind::Duplicate));
            }
            Entry::Vacant(free) => {
                free.insert(entry);
            }
        }
    }

    fn get(&self, name: &str) -> Option<&Supplied> {
        self.supplied.get(name)
    }
}

impl fmt::Debug for Parameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.names()).finish()
    }
}

/// Where in the grammar a parameter stands, which decides what may replace it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// A comparison operand, a function argument, a `MATCHES` list item or a
    /// predicate operand: any literal.
    Value,
    /// The operand of `LIKE`: a string (`AqlParser.g4 likeOperand`).
    Pattern,
    /// The name after a node code (`[at0003, $name]`): a string.
    NodeName,
    /// An archetype predicate (`[$archetypeId]`): a string that is one
    /// archetype HRID.
    ArchetypeId,
    /// A node predicate: a string that is one `id`/`at` code or one archetype
    /// HRID.
    NodeId,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Value => "a literal value",
            Self::Pattern => "a LIKE pattern string",
            Self::NodeName => "a node name string",
            Self::ArchetypeId => "an archetype identifier",
            Self::NodeId => "a node code or archetype identifier",
        })
    }
}

/// Why one parameter was not bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// The query uses the parameter and no value was supplied.
    Unbound,
    /// A value was supplied for a parameter the query does not use.
    Unknown,
    /// The parameter was supplied more than once.
    Duplicate,
    /// The supplied value is no AQL literal (a JSON object or array, or an
    /// integer beyond `i64`).
    NotALiteral,
    /// The value cannot stand where the query uses the parameter.
    Unbindable(Position),
}

/// One parameter that was not bound: its name, without the `$`, and why.
///
/// It carries no value by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// The parameter name, without the `$`.
    pub name: String,
    /// Why it was not bound.
    pub kind: FaultKind,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.name;
        match self.kind {
            FaultKind::Unbound => write!(f, "parameter `${name}` has no value"),
            FaultKind::Unknown => write!(f, "parameter `{name}` is not used by the query"),
            FaultKind::Duplicate => write!(f, "parameter `{name}` is supplied more than once"),
            FaultKind::NotALiteral => {
                write!(
                    f,
                    "parameter `{name}` is not a string, number, boolean or null"
                )
            }
            FaultKind::Unbindable(position) => {
                write!(f, "parameter `${name}` stands where {position} is required")
            }
        }
    }
}

/// The parameters [`bind`] could not bind, each reported once.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", faults.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
pub struct BindError {
    /// The faults: those of the query's parameters in order of first use,
    /// then the unknown supplied names in name order.
    pub faults: Vec<Fault>,
}

/// Replaces every `$name` in `query` with the literal `parameters` supplies.
///
/// Every parameter position the grammar admits is bound: comparison operands,
/// function arguments, `MATCHES` lists, `LIKE` patterns, and the standard,
/// archetype and node predicates of paths and the `FROM` clause. An archetype
/// or node predicate takes the value only when it is exactly one archetype
/// HRID or node code token, so the printer emits it verbatim and it re-parses
/// as itself. The binding is all or nothing: on any fault `query` is left as
/// it was.
///
/// # Errors
/// [`BindError`] naming every parameter that is used without a value
/// ([`FaultKind::Unbound`]), supplied without being used
/// ([`FaultKind::Unknown`]), supplied twice ([`FaultKind::Duplicate`]), not a
/// literal ([`FaultKind::NotALiteral`]), or of a form its position cannot hold
/// ([`FaultKind::Unbindable`]).
pub fn bind(query: &mut SelectQuery, parameters: &Parameters) -> Result<(), BindError> {
    let mut bound = query.clone();
    let mut binder = Binder {
        parameters,
        used: Vec::new(),
        faults: Vec::new(),
    };
    binder.visit_select_query_mut(&mut bound);
    let Binder {
        used, mut faults, ..
    } = binder;
    for name in parameters.names() {
        if !used.iter().any(|u| u == name) {
            push_fault(&mut faults, name, FaultKind::Unknown);
        }
    }
    if faults.is_empty() {
        *query = bound;
        Ok(())
    } else {
        Err(BindError { faults })
    }
}

/// The substitution pass: binds each parameter it meets and records what it
/// could not bind.
struct Binder<'p> {
    parameters: &'p Parameters,
    used: Vec<String>,
    faults: Vec<Fault>,
}

impl Binder<'_> {
    /// The literal for the parameter token `raw` (`$name`) at `position`, or
    /// `None` after recording why there is none.
    fn literal(&mut self, raw: &str, position: Position) -> Option<Primitive> {
        let name = raw.strip_prefix('$').unwrap_or(raw);
        if !self.used.iter().any(|u| u == name) {
            self.used.push(name.to_owned());
        }
        let value = match self.parameters.get(name) {
            None => Err(FaultKind::Unbound),
            Some(Supplied::Refused(kind)) => Err(*kind),
            Some(Supplied::Value(value)) => fits(value, position)
                .then(|| value.clone())
                .ok_or(FaultKind::Unbindable(position)),
        };
        value
            .map_err(|kind| push_fault(&mut self.faults, name, kind))
            .ok()
    }

    /// The text of the string literal for `raw` at `position`.
    fn text(&mut self, raw: &str, position: Position) -> Option<String> {
        match self.literal(raw, position)? {
            Primitive::String(text) => Some(text),
            _ => None,
        }
    }
}

/// Whether `value` can stand at `position` and print back as itself.
fn fits(value: &Primitive, position: Position) -> bool {
    match (position, value) {
        (Position::Value, Primitive::Real(real)) => real.is_finite(),
        (Position::Value, _) | (Position::Pattern | Position::NodeName, Primitive::String(_)) => {
            true
        }
        (Position::ArchetypeId, Primitive::String(text)) => {
            matches!(sole_token(text), Some(Token::ArchetypeHrid(_)))
        }
        (Position::NodeId, Primitive::String(text)) => matches!(
            sole_token(text),
            Some(Token::ArchetypeHrid(_) | Token::IdCode(_) | Token::AtCode(_))
        ),
        _ => false,
    }
}

/// The one token `text` lexes to, when it is exactly one token and nothing
/// else (no surrounding whitespace, nothing after it).
fn sole_token(text: &str) -> Option<Token> {
    let stream = crate::lexer::lex_spanned(text).ok()?;
    match (stream.tokens(), stream.spans()) {
        ([token], [span]) if *span == (0..text.len()) => Some(token.clone()),
        _ => None,
    }
}

/// Records `kind` for `name` unless that exact fault is already recorded.
fn push_fault(faults: &mut Vec<Fault>, name: &str, kind: FaultKind) {
    if !faults.iter().any(|f| f.name == name && f.kind == kind) {
        faults.push(Fault {
            name: name.to_owned(),
            kind,
        });
    }
}

impl VisitMut for Binder<'_> {
    fn visit_terminal_mut(&mut self, node: &mut Terminal) {
        if let Terminal::Parameter(raw) = node
            && let Some(value) = self.literal(raw, Position::Value)
        {
            *node = Terminal::Primitive(value);
        }
        walk_terminal_mut(self, node);
    }

    fn visit_path_predicate_operand_mut(&mut self, node: &mut PathPredicateOperand) {
        if let PathPredicateOperand::Parameter(raw) = node
            && let Some(value) = self.literal(raw, Position::Value)
        {
            *node = PathPredicateOperand::Primitive(value);
        }
        walk_path_predicate_operand_mut(self, node);
    }

    fn visit_value_list_item_mut(&mut self, node: &mut ValueListItem) {
        if let ValueListItem::Parameter(raw) = node
            && let Some(value) = self.literal(raw, Position::Value)
        {
            *node = ValueListItem::Primitive(value);
        }
        walk_value_list_item_mut(self, node);
    }

    fn visit_like_operand_mut(&mut self, node: &mut LikeOperand) {
        if let LikeOperand::Parameter(raw) = node
            && let Some(text) = self.text(raw, Position::Pattern)
        {
            *node = LikeOperand::String(text);
        }
    }

    fn visit_node_name_constraint_mut(&mut self, node: &mut NodeNameConstraint) {
        if let NodeNameConstraint::Parameter(raw) = node
            && let Some(text) = self.text(raw, Position::NodeName)
        {
            *node = NodeNameConstraint::String(text);
        }
    }

    fn visit_archetype_predicate_mut(&mut self, node: &mut ArchetypePredicate) {
        if let ArchetypePredicate::Parameter(raw) = node
            && let Some(hrid) = self.text(raw, Position::ArchetypeId)
        {
            *node = ArchetypePredicate::Hrid(hrid);
        }
    }

    fn visit_node_predicate_mut(&mut self, node: &mut NodePredicate) {
        if let NodePredicate::Parameter(raw) = node
            && let Some(text) = self.text(raw, Position::NodeId)
        {
            *node = match sole_token(&text) {
                Some(Token::ArchetypeHrid(hrid)) => NodePredicate::Archetype { hrid, name: None },
                _ => NodePredicate::Code {
                    code: text,
                    name: None,
                },
            };
        }
        walk_node_predicate_mut(self, node);
    }
}

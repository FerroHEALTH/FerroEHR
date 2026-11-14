// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The paths of an AQL query over `ACCESS_LOG_ENTRY` (#5131), typed against
//! the draft's class table and the RM's, and read out of an entry's view.
//!
//! An attribute path names RM attributes from the entry down, as AQL paths do
//! (QUERY `master03-syntax.adoc` §Identified paths); each step resolves on the
//! class the previous step reached, or on one of its concrete descendants, and
//! a formal generic parameter resolves to the argument its attribute binds
//! (`occurred: DV_INTERVAL<DV_DATE_TIME>`). A path that crosses a container
//! reaches every member, and reads as the list of what it reaches: no openEHR
//! spec governs AQL over the access log, so that is our own design.

#![expect(
    clippy::disallowed_types,
    reason = "an entry's view is its ITS-JSON form, the subject view among them, which no \
              generated type carries"
)]

use openehr_query::ast::IdentifiedPath;
use openehr_rm::draft::access_log::model::CLASSES;
use openehr_rm::v1_2::model::{self, Container, RmAttribute, RmClass, RmTypeRef};
use serde_json::Value;

use super::LogQueryError;

/// The class the access log is queried as.
pub const ACCESS_LOG_ENTRY: &str = "ACCESS_LOG_ENTRY";

/// What a path's last step reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaf {
    /// A `String`, `Uri` or `Character`.
    Text,
    /// An `Integer`, `Integer64`, `Real` or `Double`.
    Number,
    /// A `Boolean`.
    Boolean,
    /// An object, or the entry itself.
    Object,
}

/// A typed attribute path from the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPath {
    /// The attribute names, the entry's first; empty for the entry itself.
    pub steps: Vec<String>,
    /// Whether a step crosses a container, so the path reaches a list.
    pub multiple: bool,
    /// What the last step reaches.
    pub leaf: Leaf,
    /// The path as the query wrote it, its variable kept.
    pub text: String,
}

impl LogPath {
    /// The steps as string slices.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.steps.iter().map(String::as_str).collect()
    }

    /// What this path reaches in `view`: the value, `null` where it reaches
    /// nothing, and for a path that crosses a container the list of every
    /// value it reaches.
    #[must_use]
    pub fn read(&self, view: &Value) -> Value {
        let mut reached = vec![view];
        for step in &self.steps {
            reached = reached
                .into_iter()
                .filter_map(|value| value.get(step))
                .flat_map(|value| match value {
                    Value::Array(members) => members.iter().collect(),
                    other => vec![other],
                })
                .collect();
        }
        if self.multiple {
            Value::Array(reached.into_iter().cloned().collect())
        } else {
            reached
                .first()
                .map_or(Value::Null, |value| (*value).clone())
        }
    }
}

/// `path`, written over `variable`, typed against the entry's classes.
///
/// # Errors
/// [`LogQueryError::UnknownVariable`] for a path over another variable,
/// [`LogQueryError::PathPredicate`] for a predicate on any step, and
/// [`LogQueryError::UnknownAttribute`] for a step no class it can reach
/// declares.
pub(super) fn resolve(path: &IdentifiedPath, variable: &str) -> Result<LogPath, LogQueryError> {
    let text = path.column_path_with_variable();
    if path.root != variable {
        return Err(LogQueryError::UnknownVariable(path.root.clone()));
    }
    let parts = path.path.as_ref().map_or(&[][..], |p| p.parts.as_slice());
    if path.predicate.is_some() || parts.iter().any(|part| part.predicate.is_some()) {
        return Err(LogQueryError::PathPredicate(text));
    }
    let mut on = Reached::Class {
        class: class(ACCESS_LOG_ENTRY).ok_or(LogQueryError::NoModel)?,
        arguments: &[],
    };
    let mut multiple = false;
    for part in parts {
        let Reached::Class { class, arguments } = on else {
            return Err(LogQueryError::UnknownAttribute {
                attribute: part.name.clone(),
                class: on.name().to_owned(),
            });
        };
        let found =
            attribute(class, &part.name).ok_or_else(|| LogQueryError::UnknownAttribute {
                attribute: part.name.clone(),
                class: class.name.to_owned(),
            })?;
        multiple |= found.container != Container::None;
        on = reached(class, arguments, found)?;
    }
    Ok(LogPath {
        steps: parts.iter().map(|part| part.name.clone()).collect(),
        multiple,
        leaf: on.leaf(),
        text,
    })
}

/// What a step reached: a class with its generic arguments, or a primitive.
#[derive(Clone, Copy)]
enum Reached {
    /// A class of the draft or the RM.
    Class {
        /// The class.
        class: &'static RmClass,
        /// The arguments its formal generic parameters are bound to.
        arguments: &'static [RmTypeRef],
    },
    /// A primitive type, by name.
    Primitive(&'static str),
}

impl Reached {
    /// The name of what was reached.
    fn name(self) -> &'static str {
        match self {
            Self::Class { class, .. } => class.name,
            Self::Primitive(name) => name,
        }
    }

    /// What a path ending here reaches.
    fn leaf(self) -> Leaf {
        match self.name() {
            "Integer" | "Integer64" | "Real" | "Double" => Leaf::Number,
            "Boolean" => Leaf::Boolean,
            _ if matches!(self, Self::Primitive(_)) => Leaf::Text,
            _ => Leaf::Object,
        }
    }
}

/// What `found`, an attribute of `class` whose generic parameters `arguments`
/// bind, reaches.
fn reached(
    class: &'static RmClass,
    arguments: &'static [RmTypeRef],
    found: &'static RmAttribute,
) -> Result<Reached, LogQueryError> {
    let bound = found.formal.and_then(|formal| {
        let position = class.generic_params.iter().position(|p| p.name == formal)?;
        arguments.get(position)
    });
    let (name, arguments) = bound.map_or((found.declared_type, found.type_params), |argument| {
        (argument.name, argument.params)
    });
    if model::is_primitive(name) {
        return Ok(Reached::Primitive(name));
    }
    let class = class_named(name)?;
    Ok(Reached::Class { class, arguments })
}

/// The class `name` names, of the draft or the RM.
fn class(name: &str) -> Option<&'static RmClass> {
    CLASSES
        .iter()
        .find(|class| class.name == name)
        .or_else(|| model::class(name))
}

/// The class `name` names.
///
/// # Errors
/// [`LogQueryError::NoModel`] when neither table holds it.
fn class_named(name: &str) -> Result<&'static RmClass, LogQueryError> {
    class(name).ok_or(LogQueryError::NoModel)
}

/// The attribute `name` of `on`, or of the first of its concrete descendants
/// that declares it.
fn attribute(on: &'static RmClass, name: &str) -> Option<&'static RmAttribute> {
    let own = |class: &'static RmClass| class.attributes.iter().find(|a| a.name == name);
    own(on).or_else(|| {
        on.descendants
            .iter()
            .filter_map(|descendant| class(descendant))
            .find_map(own)
    })
}

#[cfg(test)]
mod tests {
    use openehr_query::parser::parse_str;

    use super::*;

    /// The first column of the query `aql`, typed.
    fn path(aql: &str) -> Result<LogPath, LogQueryError> {
        let query = parse_str(aql).expect("parses");
        let path = query
            .select
            .columns
            .first()
            .and_then(|column| match &column.column {
                openehr_query::ast::ColumnExpr::Path(path) => Some(path.clone()),
                _ => None,
            })
            .expect("a path column");
        resolve(&path, "l")
    }

    #[test]
    fn the_paths_the_design_names_resolve() {
        let party = path("SELECT l/accessor/party/external_ref/id/value FROM ACCESS_LOG_ENTRY l")
            .expect("resolves");
        assert_eq!(party.leaf, Leaf::Text);
        assert!(!party.multiple);
        let version =
            path("SELECT l/items/version_id/value FROM ACCESS_LOG_ENTRY l").expect("resolves");
        assert!(version.multiple, "items is a list");
        assert_eq!(version.leaf, Leaf::Text);
        assert_eq!(
            path("SELECT l/link/sequence FROM ACCESS_LOG_ENTRY l")
                .expect("resolves")
                .leaf,
            Leaf::Number
        );
        assert_eq!(
            path("SELECT l/is_emergency FROM ACCESS_LOG_ENTRY l")
                .expect("resolves")
                .leaf,
            Leaf::Boolean
        );
        assert_eq!(
            path("SELECT l FROM ACCESS_LOG_ENTRY l")
                .expect("resolves")
                .leaf,
            Leaf::Object
        );
    }

    #[test]
    fn a_generic_parameter_resolves_to_its_argument() {
        let lower = path("SELECT l/occurred/lower/value FROM ACCESS_LOG_ENTRY l")
            .expect("DV_INTERVAL<DV_DATE_TIME>");
        assert_eq!(lower.leaf, Leaf::Text);
    }

    #[test]
    fn an_unknown_step_is_refused() {
        assert!(matches!(
            path("SELECT l/accessor/nickname FROM ACCESS_LOG_ENTRY l"),
            Err(LogQueryError::UnknownAttribute { .. })
        ));
        assert!(matches!(
            path("SELECT x/uid/value FROM ACCESS_LOG_ENTRY l"),
            Err(LogQueryError::UnknownVariable(_))
        ));
        assert!(matches!(
            path("SELECT l/items[rm_type='COMPOSITION']/version_id FROM ACCESS_LOG_ENTRY l"),
            Err(LogQueryError::PathPredicate(_))
        ));
    }

    #[test]
    fn a_path_reads_through_lists() {
        let view = serde_json::json!({
            "items": [{"version_id": {"value": "a::s::1"}}, {"version_id": {"value": "b::s::2"}}],
            "ehr_id": {"value": "e"}
        });
        let version =
            path("SELECT l/items/version_id/value FROM ACCESS_LOG_ENTRY l").expect("resolves");
        assert_eq!(
            version.read(&view),
            serde_json::json!(["a::s::1", "b::s::2"])
        );
        let ehr = path("SELECT l/ehr_id/value FROM ACCESS_LOG_ENTRY l").expect("resolves");
        assert_eq!(ehr.read(&view), serde_json::json!("e"));
        let absent = path("SELECT l/query/language FROM ACCESS_LOG_ENTRY l").expect("resolves");
        assert_eq!(absent.read(&view), Value::Null);
    }
}

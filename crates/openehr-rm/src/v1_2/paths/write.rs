// @generated-from-template templates/openehr-rm/paths/write.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Writing at a path into a canonical-JSON instance.
//!
//! [`item_at_path_mut`] is the mutable twin of
//! [`item_at_path`](super::item_at_path): the first item the path selects, in
//! document order, by the same selection rules (BASE
//! `docs/specs/openehr/BASE/docs/architecture_overview/master11-paths.adoc`
//! §Predicate Expressions, and §Using Positional Parameters for `[n]`, the
//! nth member of a container).
//!
//! [`set_at_path`] writes one value at a path, creating the members the path
//! names that the instance lacks. The RM defines no write at a path (RM
//! `common.pathable.adoc` gives `PATHABLE` read functions only), so the rules
//! are our own design: each step must select exactly one member or name one
//! to create; a member is created only where nothing in the path asks for more
//! than it can carry (an archetype node id, name or uid predicate is refused
//! on a member that does not exist yet, and `[n]` appends only at the end of a
//! container); the written value must satisfy the last segment's predicate; and
//! nothing is changed unless the whole write succeeds. Each step is typed with
//! [`step_type`] from the type of the node it leaves: the root's RM type, or
//! the step before's item type, narrowed to the `_type` an existing node
//! carries. The type decides whether a missing member is a container and
//! whether a created node needs its `_type`: the
//! ITS-JSON schema requires `_type` where the attribute's declared type is
//! abstract (`model::declared_concrete_type`), so a node created there is
//! tagged with the one concrete class the path admits.

use serde_json::{Map, Value};

use crate::v1_2::model::{self, Container};
use crate::v1_2::paths::{PathSegment, Predicate, RmPath};
use crate::v1_2::typing::{PathStep, RmType, TypingError, narrowed, step_type};

/// One hop of a route from the root to a node: the attribute, and the index
/// within it when it holds an array.
#[derive(Debug, Clone)]
struct Hop {
    attribute: String,
    index: Option<usize>,
}

/// Pushes the route of every child of `node` that `segment` selects, after
/// `base`, mirroring `super::select_children`.
fn child_routes(node: &Value, segment: &PathSegment, base: &[Hop], out: &mut Vec<Vec<Hop>>) {
    let Some(child) = node.get(&segment.attribute) else {
        return;
    };
    let predicate = &segment.predicate;
    let mut push = |index: Option<usize>| {
        let mut route = base.to_vec();
        route.push(Hop {
            attribute: segment.attribute.clone(),
            index,
        });
        out.push(route);
    };
    match child {
        Value::Array(items) => match predicate.position {
            Some(position) => {
                let index = position.saturating_sub(1);
                if items.get(index).is_some_and(|v| predicate.matches(v)) {
                    push(Some(index));
                }
            }
            None => {
                for (index, item) in items.iter().enumerate() {
                    if predicate.matches(item) {
                        push(Some(index));
                    }
                }
            }
        },
        single => {
            if predicate.position.is_none_or(|p| p == 1) && predicate.matches(single) {
                push(None);
            }
        }
    }
}

/// Pushes the routes a `//` segment selects at `node` and at every object
/// below it, in the order `super::items_at_path` visits them.
fn descendant_routes(node: &Value, segment: &PathSegment, base: &[Hop], out: &mut Vec<Vec<Hop>>) {
    child_routes(node, segment, base, out);
    let Value::Object(map) = node else {
        return;
    };
    for (attribute, value) in map {
        match value {
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    if item.is_object() {
                        let mut route = base.to_vec();
                        route.push(Hop {
                            attribute: attribute.clone(),
                            index: Some(index),
                        });
                        descendant_routes(item, segment, &route, out);
                    }
                }
            }
            Value::Object(_) => {
                let mut route = base.to_vec();
                route.push(Hop {
                    attribute: attribute.clone(),
                    index: None,
                });
                descendant_routes(value, segment, &route, out);
            }
            _ => {}
        }
    }
}

/// The node at the end of `route` below `root`.
fn follow<'a>(root: &'a Value, route: &[Hop]) -> Option<&'a Value> {
    route.iter().try_fold(root, |node, hop| {
        let child = node.get(&hop.attribute)?;
        match hop.index {
            Some(index) => child.get(index),
            None => Some(child),
        }
    })
}

/// The route of the first item `path` selects below `root`, in document order.
fn first_route(root: &Value, path: &RmPath) -> Option<Vec<Hop>> {
    let mut routes: Vec<Vec<Hop>> = vec![Vec::new()];
    for segment in &path.segments {
        let mut next = Vec::new();
        for route in &routes {
            let Some(node) = follow(root, route) else {
                continue;
            };
            if segment.descendant {
                descendant_routes(node, segment, route, &mut next);
            } else {
                child_routes(node, segment, route, &mut next);
            }
        }
        routes = next;
    }
    routes.into_iter().next()
}

/// The first item `path` selects below `root`, in document order, for
/// writing: the item [`item_at_path`](super::item_at_path) returns.
#[must_use]
pub fn item_at_path_mut<'a>(root: &'a mut Value, path: &RmPath) -> Option<&'a mut Value> {
    let route = first_route(root, path)?;
    route.iter().try_fold(root, |node, hop| {
        let child = node.get_mut(&hop.attribute)?;
        match hop.index {
            Some(index) => child.get_mut(index),
            None => Some(child),
        }
    })
}

/// Why [`set_at_path`] wrote nothing. Each `segment` is a 0-based index into
/// the path's segments.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetPathError {
    /// The path could not be typed below the root's RM type.
    #[error(transparent)]
    Typing(#[from] TypingError),
    /// The node a segment steps from is no JSON object.
    #[error("the node segment {segment} steps from is no object")]
    NotAnObject {
        /// The segment.
        segment: usize,
    },
    /// A container attribute holds something other than a JSON array.
    #[error("the container segment {segment} names holds no array")]
    NotAnArray {
        /// The segment.
        segment: usize,
    },
    /// No member satisfies the segment's predicate, and none can be created
    /// to satisfy it; or the written value does not satisfy it.
    #[error("no member satisfies the predicate of segment {segment}")]
    Unmatched {
        /// The segment.
        segment: usize,
    },
    /// More than one member satisfies the segment's predicate.
    #[error("more than one member satisfies segment {segment}")]
    Ambiguous {
        /// The segment.
        segment: usize,
    },
    /// A positional predicate names a member past the end of the container
    /// plus one.
    #[error("segment {segment} names member {position} of a container of {len}")]
    Gap {
        /// The segment.
        segment: usize,
        /// The 1-based position named.
        position: usize,
        /// The container's length.
        len: usize,
    },
    /// The segment steps into a `Hash` container, which a path does not index.
    #[error("segment {segment} steps into a Hash container")]
    Hash {
        /// The segment.
        segment: usize,
    },
    /// A node to create sits under an abstract declared type, and the path
    /// admits more than one concrete class for it.
    #[error("segment {segment} would create a `{class}` node, which is abstract")]
    Abstract {
        /// The segment.
        segment: usize,
        /// The type the node would have.
        class: String,
    },
}

/// What one step of a write does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Step into the existing member (its array index, if any).
    Existing(Option<usize>),
    /// Create the member (appended to an array, if `true`).
    Create { append: bool },
}

/// Whether the predicate asks for anything a newly created node lacks.
fn constrains(predicate: &Predicate) -> bool {
    predicate.archetype_node_id.is_some()
        || predicate.name_value.is_some()
        || predicate.uid.is_some()
        || !predicate.comparisons.is_empty()
}

/// Decides the step `segment` takes on `child`, the attribute's current
/// value (`None` when the attribute is absent), writing `value` there when the
/// step is the last.
///
/// An existing member is selected as [`item_at_path`](super::item_at_path)
/// selects it. A written value must satisfy the predicate too, and a member
/// created on the way must not be asked for anything it lacks.
fn decide(
    index: usize,
    segment: &PathSegment,
    step: &PathStep,
    child: Option<&Value>,
    value: Option<&Value>,
) -> Result<Action, SetPathError> {
    let predicate = &segment.predicate;
    let fits = value.is_none_or(|v| predicate.matches(v));
    let creatable = match value {
        Some(v) => predicate.matches(v),
        None => !constrains(predicate),
    };
    let unmatched = SetPathError::Unmatched { segment: index };
    let found = match step.container() {
        Container::Hash => return Err(SetPathError::Hash { segment: index }),
        Container::None => {
            if predicate.position.is_some_and(|p| p != 1) {
                return Err(unmatched);
            }
            match child {
                Some(existing) if predicate.matches(existing) => Some(None),
                Some(_) => return Err(unmatched),
                None => None,
            }
        }
        Container::List | Container::Set => {
            let items: &[Value] = match child {
                Some(Value::Array(items)) => items,
                Some(_) => return Err(SetPathError::NotAnArray { segment: index }),
                None => &[],
            };
            selected(index, predicate, items)?.map(Some)
        }
    };
    let append = step.container() != Container::None;
    match found {
        Some(at) if fits => Ok(Action::Existing(at)),
        None if creatable => Ok(Action::Create { append }),
        _ => Err(unmatched),
    }
}

/// The index of the one member of `items` the predicate selects, `None` when
/// it names a member to append.
fn selected(
    index: usize,
    predicate: &Predicate,
    items: &[Value],
) -> Result<Option<usize>, SetPathError> {
    match predicate.position {
        Some(position) if position <= items.len() => {
            let at = position.saturating_sub(1);
            match items.get(at) {
                Some(item) if predicate.matches(item) => Ok(Some(at)),
                _ => Err(SetPathError::Unmatched { segment: index }),
            }
        }
        Some(position) if position == items.len().saturating_add(1) => Ok(None),
        Some(position) => Err(SetPathError::Gap {
            segment: index,
            position,
            len: items.len(),
        }),
        None => {
            let mut matching = items
                .iter()
                .enumerate()
                .filter(|(_, item)| predicate.matches(item))
                .map(|(at, _)| at);
            let first = matching.next();
            if matching.next().is_some() {
                return Err(SetPathError::Ambiguous { segment: index });
            }
            Ok(first)
        }
    }
}

/// The empty node a write creates for `step`, which `next` steps out of:
/// tagged with its concrete class where the attribute's declared type is
/// abstract, which must then be the one concrete class `next` admits.
fn new_node(index: usize, step: &PathStep, next: &PathStep) -> Result<Value, SetPathError> {
    let mut node = Map::new();
    let declared_abstract =
        model::class(step.ty.attribute.declared_type).is_some_and(|class| class.is_abstract);
    if declared_abstract {
        let mut admitted: Vec<&'static str> = Vec::new();
        for owner in &next.owners {
            for &form in model::descendants(owner) {
                if !admitted.contains(&form) {
                    admitted.push(form);
                }
            }
        }
        let [class] = admitted.as_slice() else {
            return Err(SetPathError::Abstract {
                segment: index,
                class: step.item().to_string(),
            });
        };
        node.insert("_type".to_owned(), Value::String((*class).to_owned()));
    }
    Ok(Value::Object(node))
}

/// Writes `value` at `path` below `root`, an instance of the RM type
/// `root_type`, and returns the value it replaces.
///
/// Every segment but the last steps into one member, creating it where the
/// instance lacks it; the last is set to `value`, appended to a container
/// where it names a new member. See the module documentation for the rules.
/// `value` is written as given: its own conformance to the step's type is the
/// caller's (the RM-instance validator checks a finished instance).
///
/// # Errors
///
/// [`SetPathError::Typing`] when the path does not type below `root_type`
/// (a `//` segment included), and the other [`SetPathError`] variants for the
/// segment that selects no single member or cannot be created. On an error the
/// instance is unchanged.
pub fn set_at_path(
    root: &mut Value,
    root_type: &RmType,
    path: &RmPath,
    value: Value,
) -> Result<Option<Value>, SetPathError> {
    let Some(last) = path.segments.len().checked_sub(1) else {
        return Ok(Some(core::mem::replace(root, value)));
    };
    let mut planned: Vec<(Action, PathStep)> = Vec::with_capacity(path.segments.len());
    let mut holder = root_type.clone();
    let mut node: Option<&Value> = Some(&*root);
    for (index, segment) in path.segments.iter().enumerate() {
        if segment.descendant {
            return Err(TypingError::Pattern(index).into());
        }
        let child = match node {
            Some(Value::Object(map)) => {
                if let Some(tag) = map.get("_type").and_then(Value::as_str) {
                    holder = narrowed(&holder, tag)?;
                }
                map.get(&segment.attribute)
            }
            Some(_) => return Err(SetPathError::NotAnObject { segment: index }),
            None => None,
        };
        let step = step_type(&holder, &segment.attribute)?;
        let written = (index == last).then_some(&value);
        let action = decide(index, segment, &step, child, written)?;
        node = match (action, child) {
            (Action::Existing(Some(at)), Some(items)) => items.get(at),
            (Action::Existing(None), found) => found,
            _ => None,
        };
        holder = step.item().clone();
        planned.push((action, step));
    }
    let mut actions = Vec::with_capacity(planned.len());
    for (index, (action, step)) in planned.iter().enumerate() {
        let created = match (action, planned.get(index.saturating_add(1))) {
            (Action::Create { .. }, Some((_, next))) => Some(new_node(index, step, next)?),
            _ => None,
        };
        actions.push((*action, created));
    }
    apply(root, path, actions, value)
}

/// Carries out the actions [`set_at_path`] decided, all of which it checked.
fn apply(
    root: &mut Value,
    path: &RmPath,
    actions: Vec<(Action, Option<Value>)>,
    value: Value,
) -> Result<Option<Value>, SetPathError> {
    let mut node = root;
    let mut value = Some(value);
    let count = actions.len();
    for (index, ((action, created), segment)) in actions.into_iter().zip(&path.segments).enumerate()
    {
        let last = index.saturating_add(1) == count;
        let placed = if last { value.take() } else { created };
        let Value::Object(map) = node else {
            return Err(SetPathError::NotAnObject { segment: index });
        };
        let attribute = segment.attribute.clone();
        let slot = match (action, placed) {
            (Action::Existing(None), Some(placed)) if last => {
                return Ok(map.insert(attribute, placed));
            }
            (Action::Existing(None), _) => map.get_mut(&segment.attribute),
            (Action::Existing(Some(at)), Some(placed)) if last => {
                let item = map
                    .get_mut(&segment.attribute)
                    .and_then(|items| items.get_mut(at))
                    .ok_or(SetPathError::Unmatched { segment: index })?;
                return Ok(Some(core::mem::replace(item, placed)));
            }
            (Action::Existing(Some(at)), _) => map
                .get_mut(&segment.attribute)
                .and_then(|items| items.get_mut(at)),
            (Action::Create { append: false }, Some(placed)) => {
                map.insert(attribute, placed);
                if last {
                    return Ok(None);
                }
                map.get_mut(&segment.attribute)
            }
            (Action::Create { append: true }, Some(placed)) => {
                let items = map
                    .entry(attribute)
                    .or_insert_with(|| Value::Array(Vec::new()));
                let Value::Array(items) = items else {
                    return Err(SetPathError::NotAnArray { segment: index });
                };
                items.push(placed);
                if last {
                    return Ok(None);
                }
                items.last_mut()
            }
            (Action::Create { .. }, None) => None,
        };
        node = slot.ok_or(SetPathError::Unmatched { segment: index })?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn path(text: &str) -> RmPath {
        text.parse().unwrap()
    }

    fn ty(text: &str) -> RmType {
        text.parse().unwrap()
    }

    fn tree() -> Value {
        json!({
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "items": [
                {"_type": "ELEMENT", "archetype_node_id": "at0004",
                 "value": {"_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg"}},
                {"_type": "ELEMENT", "archetype_node_id": "at0005",
                 "value": {"_type": "DV_TEXT", "value": "a"}},
                {"_type": "ELEMENT", "archetype_node_id": "at0005",
                 "value": {"_type": "DV_TEXT", "value": "b"}}
            ]
        })
    }

    #[test]
    fn the_mutable_twin_reaches_the_item_the_reader_returns() {
        let mut root = tree();
        let p = path("items[at0005]/value/value");
        assert_eq!(super::super::item_at_path(&root, &p), Some(&json!("a")));
        *item_at_path_mut(&mut root, &p).unwrap() = json!("c");
        assert_eq!(root["items"][1]["value"]["value"], json!("c"));
        let third = path("items[3]/value/value");
        *item_at_path_mut(&mut root, &third).unwrap() = json!("d");
        assert_eq!(root["items"][2]["value"]["value"], json!("d"));
        let deep = path("//value/units");
        assert_eq!(item_at_path_mut(&mut root, &deep), Some(&mut json!("kg")));
        assert!(item_at_path_mut(&mut root, &path("items[9]")).is_none());
    }

    #[test]
    fn a_value_replaces_the_one_member_its_path_selects() {
        let mut root = tree();
        let old = set_at_path(
            &mut root,
            &ty("ITEM_TREE"),
            &path("items[at0004]/value/magnitude"),
            json!(2.5),
        );
        assert_eq!(old, Ok(Some(json!(1.0))));
        assert_eq!(root["items"][0]["value"]["magnitude"], json!(2.5));
    }

    #[test]
    fn a_position_one_past_the_end_appends() {
        let mut root = tree();
        let element = json!({"_type": "ELEMENT", "archetype_node_id": "at0006"});
        let old = set_at_path(
            &mut root,
            &ty("ITEM_TREE"),
            &path("items[4]"),
            element.clone(),
        );
        assert_eq!(old, Ok(None));
        assert_eq!(root["items"][3], element);
        assert_eq!(
            set_at_path(&mut root, &ty("ITEM_TREE"), &path("items[9]"), json!({})),
            Err(SetPathError::Gap {
                segment: 0,
                position: 9,
                len: 4
            })
        );
    }

    #[test]
    fn missing_members_are_created_and_tagged_where_the_declared_type_is_abstract() {
        // RM `reference_range.adoc`: the range of a quantity's reference range
        // holds quantities, and `DV_INTERVAL.lower` is declared `T`, bound
        // `DV_ORDERED`, so the created limit carries its `_type`.
        let mut root = json!({"_type": "DV_QUANTITY", "magnitude": 5.0, "units": "kg"});
        let old = set_at_path(
            &mut root,
            &ty("DV_QUANTITY"),
            &path("other_reference_ranges[1]/range/lower/magnitude"),
            json!(1.0),
        );
        assert_eq!(old, Ok(None));
        assert_eq!(
            root["other_reference_ranges"],
            json!([{"range": {"lower": {"_type": "DV_QUANTITY", "magnitude": 1.0}}}])
        );
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let mut root = json!({"_type": "DV_QUANTITY", "magnitude": 5.0, "units": "kg"});
        let before = root.clone();
        assert_eq!(
            set_at_path(
                &mut root,
                &ty("DV_QUANTITY"),
                &path("other_reference_ranges[2]/meaning/value"),
                json!("normal"),
            ),
            Err(SetPathError::Gap {
                segment: 0,
                position: 2,
                len: 0
            })
        );
        assert_eq!(root, before);
    }

    #[test]
    fn a_member_the_predicate_asks_for_is_never_invented() {
        let mut root = tree();
        let before = root.clone();
        assert_eq!(
            set_at_path(
                &mut root,
                &ty("ITEM_TREE"),
                &path("items[at0009]/value/value"),
                json!("x"),
            ),
            Err(SetPathError::Unmatched { segment: 0 })
        );
        assert_eq!(
            set_at_path(
                &mut root,
                &ty("ITEM_TREE"),
                &path("items[at0005]/value/value"),
                json!("x"),
            ),
            Err(SetPathError::Ambiguous { segment: 0 })
        );
        assert_eq!(root, before);
    }

    #[test]
    fn an_abstract_node_with_no_single_concrete_class_is_refused() {
        let mut root = json!({"_type": "ELEMENT", "archetype_node_id": "at0004"});
        assert_eq!(
            set_at_path(&mut root, &ty("ELEMENT"), &path("value/units"), json!("kg")),
            Ok(None)
        );
        assert_eq!(
            root["value"],
            json!({"_type": "DV_QUANTITY", "units": "kg"})
        );
        let mut bare = json!({"_type": "ELEMENT", "archetype_node_id": "at0004"});
        assert!(matches!(
            set_at_path(
                &mut bare,
                &ty("ELEMENT"),
                &path("value/null_flavour"),
                json!({})
            ),
            Err(SetPathError::Typing(_))
        ));
        let mut cluster = json!({"_type": "CLUSTER", "archetype_node_id": "at0001"});
        assert_eq!(
            set_at_path(
                &mut cluster,
                &ty("CLUSTER"),
                &path("items[1]/name/value"),
                json!("x")
            ),
            Err(SetPathError::Abstract {
                segment: 0,
                class: "ITEM".to_owned()
            })
        );
    }
}

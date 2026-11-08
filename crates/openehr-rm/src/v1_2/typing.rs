// @generated-from-template templates/openehr-rm/typing.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Generic RM type names, and the type of an attribute or a path below one.
//!
//! An RM type name is a class name with its actual generic parameters,
//! `DV_INTERVAL<DV_QUANTITY>` or `List<EVENT<ITEM_TREE>>`: BASE
//! `docs/specs/openehr/BASE/docs/foundation_types/master04-structure_types.adoc`
//! defines the container generics, and the RM class tables the RM ones
//! (`DV_INTERVAL<T>`, `REFERENCE_RANGE<T>`, `EVENT<T>`, `HISTORY<T>`,
//! `VERSION<T>`). [`RmType`] parses and prints that spelling.
//!
//! [`attribute_type`] types one attribute of an instantiated class: an attribute
//! the class table writes as its formal parameter (`DV_INTERVAL.lower: T`) takes
//! the actual argument, so `lower` of a `DV_INTERVAL<DV_QUANTITY>` is a
//! `DV_QUANTITY`. [`path_type`] types every step of an [`RmPath`], a sequence of
//! attribute names (BASE `docs/specs/openehr/BASE/docs/architecture_overview/master11-paths.adoc`
//! §Basic Syntax), and reports each step's container.
//!
//! A path may name an attribute that only a descendant of the step's declared
//! class carries (`ELEMENT.value` is a `DATA_VALUE`, and `magnitude` belongs to
//! `DV_QUANTITY`), since a node holds an instance of a descendant of the
//! declared type. Such a step takes the type every concrete descendant that
//! declares the attribute agrees on: one that each of them gives or conforms
//! to (`items` below an `ITEM_STRUCTURE` is a `List<ITEM>`, an `ITEM_LIST`
//! holding `ELEMENT`s and an `ITEM_TREE` `ITEM`s), and the step is refused as
//! ambiguous when there is none. No openEHR spec governs that narrowing: our
//! own design.
//!
//! Only stored attributes are typed: a computed attribute (a BMM function) is
//! never a member of an instance, so no instance path steps through one.

use core::fmt;
use core::str::FromStr;

use crate::v1_2::model::{self, Container, RmAttribute, RmClass, RmTypeRef};
use crate::v1_2::paths::RmPath;

/// An RM type name with its generic arguments: `DV_INTERVAL<DV_QUANTITY>`.
///
/// Printed with the arguments separated by `", "`, as the RM class tables
/// write them (`Hash<String, TRANSLATION_DETAILS>`); parsed with any spacing
/// around the brackets and commas.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RmType {
    name: String,
    params: Vec<RmType>,
}

impl RmType {
    /// A type named `name` with the generic arguments `params`.
    #[must_use]
    pub fn new(name: impl Into<String>, params: Vec<RmType>) -> Self {
        Self {
            name: name.into(),
            params,
        }
    }

    /// A type named `name` with no generic arguments.
    #[must_use]
    pub fn simple(name: impl Into<String>) -> Self {
        Self::new(name, Vec::new())
    }

    /// The class name, without its arguments (`DV_INTERVAL`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The generic arguments, empty for a type written without any.
    #[must_use]
    pub fn params(&self) -> &[RmType] {
        &self.params
    }

    /// The type a model reference names, each formal parameter at its bound.
    #[must_use]
    pub fn from_ref(reference: &RmTypeRef) -> Self {
        Self::new(
            reference.name,
            reference.params.iter().map(Self::from_ref).collect(),
        )
    }
}

impl fmt::Display for RmType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if let Some((first, rest)) = self.params.split_first() {
            write!(f, "<{first}")?;
            for param in rest {
                write!(f, ", {param}")?;
            }
            f.write_str(">")?;
        }
        Ok(())
    }
}

/// Why a text is no RM type name. Each position is a byte offset.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RmTypeParseError {
    /// A class name was expected (a letter or `_`, then letters, digits or `_`).
    #[error("a type name is expected at {0}")]
    MissingName(usize),
    /// A `<` argument list was not closed by `>`, or an argument was not
    /// followed by `,` or `>`.
    #[error("`,` or `>` is expected at {0}")]
    Unclosed(usize),
    /// Text follows the complete type name.
    #[error("unexpected text at {0}")]
    Trailing(usize),
}

impl FromStr for RmType {
    type Err = RmTypeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parser = TypeParser { text: s, at: 0 };
        let parsed = parser.parse_type()?;
        parser.skip_space();
        if parser.at == s.len() {
            Ok(parsed)
        } else {
            Err(RmTypeParseError::Trailing(parser.at))
        }
    }
}

/// A recursive-descent reader of one type name.
struct TypeParser<'a> {
    text: &'a str,
    at: usize,
}

impl TypeParser<'_> {
    fn rest(&self) -> &str {
        self.text.get(self.at..).unwrap_or_default()
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.at += rest.len() - rest.trim_start().len();
    }

    fn eat(&mut self, token: char) -> bool {
        self.skip_space();
        if self.rest().starts_with(token) {
            self.at += token.len_utf8();
            true
        } else {
            false
        }
    }

    fn parse_type(&mut self) -> Result<RmType, RmTypeParseError> {
        self.skip_space();
        let rest = self.rest();
        let starts = rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        let len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if !starts {
            return Err(RmTypeParseError::MissingName(self.at));
        }
        let name = rest.get(..len).unwrap_or_default().to_owned();
        self.at += len;
        let mut params = Vec::new();
        if self.eat('<') {
            loop {
                params.push(self.parse_type()?);
                if self.eat(',') {
                    continue;
                }
                if self.eat('>') {
                    break;
                }
                return Err(RmTypeParseError::Unclosed(self.at));
            }
        }
        Ok(RmType::new(name, params))
    }
}

/// Why an attribute or a path could not be typed against the RM model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypingError {
    /// The model carries no class of that name.
    #[error("`{0}` is no class of the reference model")]
    UnknownClass(String),
    /// Neither the class nor any concrete descendant of it declares the
    /// attribute.
    #[error("`{attribute}` is no attribute of `{class}` or of a class below it")]
    UnknownAttribute {
        /// The class the attribute was looked up on.
        class: String,
        /// The attribute name.
        attribute: String,
    },
    /// A type names a number of generic arguments its class does not declare.
    #[error("`{class}` takes {expected} generic argument(s), {found} given")]
    Arity {
        /// The class.
        class: String,
        /// The number of formal parameters the class declares.
        expected: usize,
        /// The number of arguments the type names.
        found: usize,
    },
    /// An argument does not conform to the bound of its parameter.
    #[error("`{argument}` does not conform to `{bound}`, the bound of `{class}<{parameter}>`")]
    Nonconforming {
        /// The class.
        class: String,
        /// The formal parameter.
        parameter: String,
        /// The argument given for it.
        argument: String,
        /// The parameter's bound.
        bound: String,
    },
    /// The concrete descendants of the step's class that declare the attribute
    /// give it different types.
    #[error("`{attribute}` below `{class}` has a different type on each of `{owners:?}`")]
    Ambiguous {
        /// The step's declared class.
        class: String,
        /// The attribute name.
        attribute: String,
        /// The descendants that declare it.
        owners: Vec<&'static str>,
    },
    /// A `//` segment matches at any depth, so it names no single attribute.
    #[error("segment {0} is a `//` pattern, which no single type answers")]
    Pattern(usize),
    /// A class a value was narrowed to is no descendant of its declared type.
    #[error("`{descendant}` does not conform to `{class}`")]
    NotADescendant {
        /// The declared type's class.
        class: String,
        /// The class the value names.
        descendant: String,
    },
}

/// The type of one attribute of an instantiated class.
#[derive(Debug, Clone)]
pub struct AttributeType {
    /// The model's row for the attribute, on the class that holds it.
    pub attribute: &'static RmAttribute,
    /// The type of one value of the attribute (one item of a container), with
    /// the class's actual arguments substituted for its formal parameters.
    pub item: RmType,
}

impl AttributeType {
    /// The attribute's container shape.
    #[must_use]
    pub fn container(&self) -> Container {
        self.attribute.container
    }
}

/// The class of `instantiated`, with its arguments checked against the
/// class's formal parameters.
fn checked_class(instantiated: &RmType) -> Result<&'static RmClass, TypingError> {
    let class = model::class(instantiated.name())
        .ok_or_else(|| TypingError::UnknownClass(instantiated.name().to_owned()))?;
    let args = instantiated.params();
    if args.is_empty() {
        return Ok(class);
    }
    if args.len() != class.generic_params.len() {
        return Err(TypingError::Arity {
            class: class.name.to_owned(),
            expected: class.generic_params.len(),
            found: args.len(),
        });
    }
    for (param, arg) in class.generic_params.iter().zip(args) {
        if let Some(bound) = param.conforms_to
            && !model::is_a(arg.name(), bound)
        {
            return Err(TypingError::Nonconforming {
                class: class.name.to_owned(),
                parameter: param.name.to_owned(),
                argument: arg.to_string(),
                bound: bound.to_owned(),
            });
        }
    }
    Ok(class)
}

/// The argument `instantiated` gives the formal parameter `formal` of `class`,
/// `None` when it gives none (a type written without arguments, or a
/// parameter only an ancestor declares).
fn argument<'t>(class: &RmClass, instantiated: &'t RmType, formal: &str) -> Option<&'t RmType> {
    let index = class.generic_params.iter().position(|p| p.name == formal)?;
    instantiated.params().get(index)
}

/// The type a model reference names below `instantiated`.
fn substituted(reference: &RmTypeRef, class: &RmClass, instantiated: &RmType) -> RmType {
    if let Some(arg) = reference
        .formal
        .and_then(|formal| argument(class, instantiated, formal))
    {
        return arg.clone();
    }
    RmType::new(
        reference.name,
        reference
            .params
            .iter()
            .map(|p| substituted(p, class, instantiated))
            .collect(),
    )
}

/// The type of the attribute `attribute` of `instantiated`, with the class's
/// actual arguments substituted (`lower` of `DV_INTERVAL<DV_QUANTITY>` is a
/// `DV_QUANTITY`; of a bare `DV_INTERVAL`, the bound `DV_ORDERED`).
///
/// # Errors
///
/// [`TypingError::UnknownClass`] for a class the model lacks,
/// [`TypingError::Arity`] or [`TypingError::Nonconforming`] for arguments the
/// class does not take, and [`TypingError::UnknownAttribute`] when the class
/// itself declares no such attribute.
pub fn attribute_type(
    instantiated: &RmType,
    attribute: &str,
) -> Result<AttributeType, TypingError> {
    let class = checked_class(instantiated)?;
    let found =
        model::attribute(class.name, attribute).ok_or_else(|| TypingError::UnknownAttribute {
            class: class.name.to_owned(),
            attribute: attribute.to_owned(),
        })?;
    let item = match found.formal.and_then(|f| argument(class, instantiated, f)) {
        Some(arg) => arg.clone(),
        None => RmType::new(
            found.declared_type,
            found
                .type_params
                .iter()
                .map(|p| substituted(p, class, instantiated))
                .collect(),
        ),
    };
    Ok(AttributeType {
        attribute: found,
        item,
    })
}

/// One typed step of a path.
#[derive(Debug, Clone)]
pub struct PathStep {
    /// The type the step starts from: the root class, or the item type of the
    /// step before.
    pub holder: RmType,
    /// The classes the attribute is read on: `[holder]` when the holder's class
    /// declares it, else every concrete descendant that does.
    pub owners: Vec<&'static str>,
    /// The attribute's type: on the holder, or the type every owner gives or
    /// conforms to, with the row of the owner that gives it.
    pub ty: AttributeType,
}

impl PathStep {
    /// The step's container shape.
    #[must_use]
    pub fn container(&self) -> Container {
        self.ty.container()
    }

    /// The type of one value the step reaches.
    #[must_use]
    pub fn item(&self) -> &RmType {
        &self.ty.item
    }
}

/// The arguments `holder` gives the parameters of its descendant `descendant`,
/// matched by name as the RM reuses a parameter's name down a family
/// (`EVENT<T>`, `POINT_EVENT<T>`).
fn carried(holder: &RmClass, args: &[RmType], descendant: &RmClass) -> Vec<RmType> {
    if args.is_empty() {
        return Vec::new();
    }
    descendant
        .generic_params
        .iter()
        .map(|param| {
            holder
                .generic_params
                .iter()
                .position(|p| p.name == param.name)
                .and_then(|i| args.get(i))
                .cloned()
                .unwrap_or_else(|| RmType::simple(param.conforms_to.unwrap_or("Any")))
        })
        .collect()
}

/// Whether every value of type `sub` is a value of type `sup`: the class
/// conforms, and `sup` names no arguments or the same ones.
fn conforms(sub: &AttributeType, sup: &AttributeType) -> bool {
    sub.container() == sup.container()
        && model::is_a(sub.item.name(), sup.item.name())
        && (sup.item.params().is_empty() || sub.item.params() == sup.item.params())
}

/// The type `holder` takes as an instance of its descendant `class`.
///
/// The arguments are carried by parameter name, so a value declared
/// `EVENT<ITEM_TREE>` that is a `POINT_EVENT` is a `POINT_EVENT<ITEM_TREE>`;
/// `holder` itself is returned when `class` is its own class.
///
/// # Errors
///
/// [`TypingError::UnknownClass`] for a class the model lacks, and
/// [`TypingError::NotADescendant`] when `class` does not conform to `holder`.
pub fn narrowed(holder: &RmType, class: &str) -> Result<RmType, TypingError> {
    if holder.name() == class {
        return Ok(holder.clone());
    }
    let declared = checked_class(holder)?;
    let target = model::class(class).ok_or_else(|| TypingError::UnknownClass(class.to_owned()))?;
    if !model::is_a(target.name, declared.name) {
        return Err(TypingError::NotADescendant {
            class: declared.name.to_owned(),
            descendant: target.name.to_owned(),
        });
    }
    Ok(RmType::new(
        target.name,
        carried(declared, holder.params(), target),
    ))
}

/// Types the attribute `attribute` of a value of type `holder`: on the
/// holder's class where it declares the attribute, else on the concrete
/// descendants that do (see the module documentation).
///
/// # Errors
///
/// Every error of [`attribute_type`], with [`TypingError::UnknownAttribute`]
/// when no descendant declares the attribute either and
/// [`TypingError::Ambiguous`] when the descendants type it differently.
pub fn step_type(holder: &RmType, attribute: &str) -> Result<PathStep, TypingError> {
    let class = checked_class(holder)?;
    if model::attribute(class.name, attribute).is_some() {
        return Ok(PathStep {
            holder: holder.clone(),
            owners: vec![class.name],
            ty: attribute_type(holder, attribute)?,
        });
    }
    let mut owners = Vec::new();
    let mut typed = Vec::new();
    for &name in class.descendants {
        if model::attribute(name, attribute).is_none() {
            continue;
        }
        typed.push(attribute_type(&narrowed(holder, name)?, attribute)?);
        owners.push(name);
    }
    if typed.is_empty() {
        return Err(TypingError::UnknownAttribute {
            class: class.name.to_owned(),
            attribute: attribute.to_owned(),
        });
    }
    let agreed = typed
        .iter()
        .find(|sup| typed.iter().all(|sub| conforms(sub, sup)))
        .cloned()
        .ok_or_else(|| TypingError::Ambiguous {
            class: class.name.to_owned(),
            attribute: attribute.to_owned(),
            owners: owners.clone(),
        })?;
    Ok(PathStep {
        holder: holder.clone(),
        owners,
        ty: agreed,
    })
}

/// Types every step of `path` below a value of type `class`, outermost first.
///
/// Each step starts from the item type of the step before, so a container
/// step (`items`) is followed by a step on one of its items. Predicates select
/// instances and do not change a step's type.
///
/// # Errors
///
/// [`TypingError::Pattern`] for a `//` segment, and every error of
/// [`attribute_type`] for the step it stops at, with
/// [`TypingError::Ambiguous`] when the descendants that declare an attribute
/// type it differently.
pub fn path_type(class: &RmType, path: &RmPath) -> Result<Vec<PathStep>, TypingError> {
    let mut steps: Vec<PathStep> = Vec::with_capacity(path.segments.len());
    let mut holder = class.clone();
    for (index, segment) in path.segments.iter().enumerate() {
        if segment.descendant {
            return Err(TypingError::Pattern(index));
        }
        let step = step_type(&holder, &segment.attribute)?;
        holder = step.item().clone();
        steps.push(step);
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(text: &str) -> RmType {
        text.parse().unwrap()
    }

    fn path(text: &str) -> RmPath {
        text.parse().unwrap()
    }

    #[test]
    fn a_generic_name_round_trips() {
        for text in [
            "DV_TEXT",
            "DV_INTERVAL<DV_QUANTITY>",
            "List<EVENT<ITEM_TREE>>",
            "Hash<String, TRANSLATION_DETAILS>",
        ] {
            assert_eq!(ty(text).to_string(), text);
        }
        assert_eq!(
            ty(" Hash< String ,List<DV_TEXT> > "),
            RmType::new(
                "Hash",
                vec![
                    RmType::simple("String"),
                    RmType::new("List", vec![RmType::simple("DV_TEXT")]),
                ],
            )
        );
    }

    #[test]
    fn a_malformed_name_is_refused_at_its_position() {
        assert_eq!("".parse::<RmType>(), Err(RmTypeParseError::MissingName(0)));
        assert_eq!(
            "DV_INTERVAL<".parse::<RmType>(),
            Err(RmTypeParseError::MissingName(12))
        );
        assert_eq!(
            "DV_INTERVAL<DV_QUANTITY".parse::<RmType>(),
            Err(RmTypeParseError::Unclosed(23))
        );
        assert_eq!(
            "DV_TEXT>".parse::<RmType>(),
            Err(RmTypeParseError::Trailing(7))
        );
        assert_eq!(
            "1X".parse::<RmType>(),
            Err(RmTypeParseError::MissingName(0))
        );
    }

    #[test]
    fn a_formal_parameter_takes_the_actual_argument() {
        let lower = attribute_type(&ty("DV_INTERVAL<DV_QUANTITY>"), "lower").unwrap();
        assert_eq!(lower.item, ty("DV_QUANTITY"));
        assert_eq!(lower.attribute.formal, Some("T"));
        assert_eq!(lower.attribute.declared_type, "DV_ORDERED");
        let bare = attribute_type(&ty("DV_INTERVAL"), "upper").unwrap();
        assert_eq!(bare.item, ty("DV_ORDERED"));
    }

    #[test]
    fn a_generic_attribute_carries_the_argument_inward() {
        // RM `reference_range.adoc`: REFERENCE_RANGE<T>.range is a DV_INTERVAL<T>.
        let range = attribute_type(&ty("REFERENCE_RANGE<DV_COUNT>"), "range").unwrap();
        assert_eq!(range.item, ty("DV_INTERVAL<DV_COUNT>"));
        // HISTORY<T>.events is a List<EVENT<T>>.
        let events = attribute_type(&ty("HISTORY<ITEM_TREE>"), "events").unwrap();
        assert_eq!(events.container(), Container::List);
        assert_eq!(events.item, ty("EVENT<ITEM_TREE>"));
    }

    #[test]
    fn arguments_the_class_does_not_take_are_refused() {
        assert!(matches!(
            attribute_type(&ty("DV_INTERVAL<DV_QUANTITY, DV_COUNT>"), "lower"),
            Err(TypingError::Arity {
                expected: 1,
                found: 2,
                ..
            })
        ));
        assert!(matches!(
            attribute_type(&ty("DV_INTERVAL<DV_TEXT>"), "lower"),
            Err(TypingError::Nonconforming { .. })
        ));
        assert!(matches!(
            attribute_type(&ty("NO_SUCH_CLASS"), "x"),
            Err(TypingError::UnknownClass(_))
        ));
    }

    #[test]
    fn a_path_is_typed_step_by_step_with_its_containers() {
        let steps = path_type(
            &ty("OBSERVATION"),
            &path("data/events[at0002]/data/items[at0004]/value/units"),
        )
        .unwrap();
        let shapes: Vec<(String, Container)> = steps
            .iter()
            .map(|s| (s.item().to_string(), s.container()))
            .collect();
        assert_eq!(
            shapes,
            [
                ("HISTORY<ITEM_STRUCTURE>".to_owned(), Container::None),
                ("EVENT<ITEM_STRUCTURE>".to_owned(), Container::List),
                ("ITEM_STRUCTURE".to_owned(), Container::None),
                ("ITEM".to_owned(), Container::List),
                ("DATA_VALUE".to_owned(), Container::None),
                ("String".to_owned(), Container::None),
            ]
        );
        assert_eq!(steps[1].owners, ["HISTORY"]);
        assert_eq!(steps[3].owners, ["ITEM_LIST", "ITEM_TREE"]);
        assert_eq!(steps[5].owners, ["DV_QUANTITY"]);
    }

    #[test]
    fn a_reference_range_below_a_quantity_holds_quantities() {
        let steps = path_type(
            &ty("DV_QUANTITY"),
            &path("other_reference_ranges[1]/range/lower/magnitude"),
        )
        .unwrap();
        let items: Vec<String> = steps.iter().map(|s| s.item().to_string()).collect();
        assert_eq!(
            items,
            [
                "REFERENCE_RANGE<DV_QUANTITY>",
                "DV_INTERVAL<DV_QUANTITY>",
                "DV_QUANTITY",
                "Real"
            ]
        );
        assert_eq!(steps[0].container(), Container::List);
    }

    #[test]
    fn descendants_that_disagree_make_a_step_ambiguous() {
        assert!(matches!(
            path_type(&ty("ELEMENT"), &path("value/value")),
            Err(TypingError::Ambiguous { .. })
        ));
        // DV_QUANTITY stores a Real magnitude, DV_COUNT an Integer64.
        assert!(matches!(
            path_type(&ty("ELEMENT"), &path("value/magnitude")),
            Err(TypingError::Ambiguous { .. })
        ));
        assert!(matches!(
            path_type(&ty("ELEMENT"), &path("value/no_such")),
            Err(TypingError::UnknownAttribute { .. })
        ));
        assert!(matches!(
            path_type(&ty("ELEMENT"), &path("//value")),
            Err(TypingError::Pattern(0))
        ));
    }
}

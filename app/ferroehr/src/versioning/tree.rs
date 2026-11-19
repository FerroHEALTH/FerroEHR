// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The JSON shape the passes that read a commit body as JSON read it through.
//!
//! A body is a canonical value, the index of its one rendering
//! ([`crate::storage::render::index`]) or the typed RM itself (`Typed`), so
//! a typed commit is read with no canonical value built (#6003) and, before
//! it is rendered, with nothing rendered (#6038). The data-minimisation pass, the party
//! checks and the binding facts read through it. No
//! openEHR spec governs this: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the canonical value is one of the two shapes \
              a body is read through"
)]

use std::borrow::Cow;

use openehr_base::serde_support::OpenSubtype;
use openehr_rm::prelude::EhrAccess;
use openehr_rm::v1_2::access::{RmObject, RmRef};
use serde_json::Value;

use crate::storage::render::index::{Kind, NodeId, Rendered};

/// The member that names an object's class.
const TYPE_KEY: &str = "_type";

/// What a value is, as the pass tells it apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// An object.
    Object,
    /// An array.
    Array,
    /// A string.
    String,
    /// A number, a boolean or `null`.
    Other,
}

/// One value of a body, read through either shape.
pub trait Tree<'a>: Copy + 'a {
    /// What the value is.
    fn shape(self) -> Shape;
    /// The member `key` of an object.
    fn get(self, key: &str) -> Option<Self>;
    /// The members of an object, in written order.
    fn members(self) -> Box<dyn Iterator<Item = (&'a str, Self)> + 'a>;
    /// The items of an array, in written order.
    fn items(self) -> Box<dyn Iterator<Item = Self> + 'a>;
    /// Calls `visit` with each member of an object, in written order; the
    /// form a pass over a whole body walks it by, since it boxes no iterator.
    fn each_member(self, visit: &mut dyn FnMut(&'a str, Self)) {
        for (key, value) in self.members() {
            visit(key, value);
        }
    }
    /// Calls `visit` with each item of an array, in written order.
    fn each_item(self, visit: &mut dyn FnMut(Self)) {
        for item in self.items() {
            visit(item);
        }
    }
    /// The text of a string.
    fn text(self) -> Option<Cow<'a, str>>;
    /// Whether the value is `null`.
    fn is_null(self) -> bool;
    /// The value as a canonical value of its own, for a caller that keeps it.
    fn to_value(self) -> Value;

    /// The string member `key` of an object.
    fn get_text(self, key: &str) -> Option<Cow<'a, str>> {
        self.get(key).and_then(Tree::text)
    }

    /// The value an RFC 6901 pointer of member names and array indices names.
    fn pointer(self, pointer: &str) -> Option<Self> {
        let mut at = self;
        for step in pointer.split('/').skip(1) {
            at = match at.shape() {
                Shape::Array => at.items().nth(step.parse().ok()?)?,
                _ => at.get(step)?,
            };
        }
        Some(at)
    }

    /// The string a pointer names.
    fn pointer_text(self, pointer: &str) -> Option<Cow<'a, str>> {
        self.pointer(pointer).and_then(Tree::text)
    }

    /// The value, or `None` for `null`.
    fn present(self) -> Option<Self> {
        (!self.is_null()).then_some(self)
    }
}

impl<'a> Tree<'a> for &'a Value {
    fn shape(self) -> Shape {
        match self {
            Value::Object(_) => Shape::Object,
            Value::Array(_) => Shape::Array,
            Value::String(_) => Shape::String,
            Value::Null | Value::Bool(_) | Value::Number(_) => Shape::Other,
        }
    }

    fn get(self, key: &str) -> Option<Self> {
        self.as_object()?.get(key)
    }

    fn members(self) -> Box<dyn Iterator<Item = (&'a str, Self)> + 'a> {
        match self {
            Value::Object(map) => Box::new(map.iter().map(|(k, v)| (k.as_str(), v))),
            _ => Box::new(std::iter::empty()),
        }
    }

    fn items(self) -> Box<dyn Iterator<Item = Self> + 'a> {
        match self {
            Value::Array(items) => Box::new(items.iter()),
            _ => Box::new(std::iter::empty()),
        }
    }

    fn text(self) -> Option<Cow<'a, str>> {
        self.as_str().map(Cow::Borrowed)
    }

    fn is_null(self) -> bool {
        Value::is_null(self)
    }

    fn to_value(self) -> Value {
        self.clone()
    }
}

/// One value of a rendered body's index.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Indexed<'a> {
    doc: &'a Rendered,
    id: NodeId,
}

impl<'a> Indexed<'a> {
    /// The root value of `doc`.
    pub(crate) fn root(doc: &'a Rendered) -> Self {
        Self {
            doc,
            id: doc.root(),
        }
    }
}

impl<'a> Tree<'a> for Indexed<'a> {
    fn shape(self) -> Shape {
        match self.doc.kind(self.id) {
            Some(Kind::Object) => Shape::Object,
            Some(Kind::Array) => Shape::Array,
            Some(Kind::String | Kind::EscapedString) => Shape::String,
            Some(Kind::Scalar) | None => Shape::Other,
        }
    }

    fn get(self, key: &str) -> Option<Self> {
        self.doc
            .get(self.id, key)
            .map(|id| Self { doc: self.doc, id })
    }

    fn members(self) -> Box<dyn Iterator<Item = (&'a str, Self)> + 'a> {
        let doc = self.doc;
        Box::new(doc.members(self.id).iter().map(move |m| {
            (
                doc.key(m),
                Self {
                    doc,
                    id: Rendered::value(m),
                },
            )
        }))
    }

    fn items(self) -> Box<dyn Iterator<Item = Self> + 'a> {
        let doc = self.doc;
        Box::new(
            doc.items(self.id)
                .iter()
                .map(move |id| Self { doc, id: *id }),
        )
    }

    fn each_member(self, visit: &mut dyn FnMut(&'a str, Self)) {
        let doc = self.doc;
        for member in doc.members(self.id) {
            let id = Rendered::value(member);
            visit(doc.key(member), Self { doc, id });
        }
    }
    fn each_item(self, visit: &mut dyn FnMut(Self)) {
        let doc = self.doc;
        for id in doc.items(self.id) {
            visit(Self { doc, id: *id });
        }
    }
    fn text(self) -> Option<Cow<'a, str>> {
        self.doc.str(self.id)
    }

    fn is_null(self) -> bool {
        self.doc.kind(self.id) == Some(Kind::Scalar) && self.doc.raw(self.id) == b"null"
    }

    #[expect(
        clippy::expect_used,
        reason = "the index spans only the values the JSON serializer wrote, so each span \
                  is one JSON value by construction"
    )]
    fn to_value(self) -> Value {
        serde_json::from_slice(self.doc.raw(self.id))
            .expect("an indexed span should be one JSON value")
    }
}

/// One value of a typed RM body, read through its generated accessor
/// ([`openehr_rm::v1_2::access`]) with nothing rendered (#6038): the shape
/// canonical JSON gives it, `_type` first and then every set attribute in
/// declaration order.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Typed<'a> {
    /// An RM object.
    Object(&'a dyn RmObject),
    /// The container attribute of an object.
    List(&'a dyn RmObject, &'static str),
    /// A primitive attribute.
    Leaf(RmRef<'a>),
    /// A free-form value kept as it was read (`EHR_ACCESS.settings`).
    Open(&'a OpenSubtype),
    /// A value inside a free-form one.
    Json(&'a Value),
}

impl<'a> Typed<'a> {
    /// The attribute `name` of `object`, when it is set.
    fn attribute(object: &'a dyn RmObject, name: &'static str) -> Option<Self> {
        if let Some(value) = object.attribute(name) {
            return Some(match value {
                RmRef::Object(child) => Self::Object(child),
                leaf => Self::Leaf(leaf),
            });
        }
        if !object.is_set(name) {
            return None;
        }
        // NOTE: RM ehr `access_control_settings.adoc` leaves the settings to the
        // scheme, so the accessor carries no view of them and the class is asked.
        if name == "settings"
            && let Some(access) = object.as_any().downcast_ref::<EhrAccess>()
        {
            return access.settings.as_ref().map(Self::Open);
        }
        // A set attribute with no single value is a container.
        Some(Self::List(object, name))
    }
}

impl<'a> Tree<'a> for Typed<'a> {
    fn shape(self) -> Shape {
        match self {
            Self::Object(_) | Self::Open(_) => Shape::Object,
            Self::List(..) => Shape::Array,
            Self::Leaf(RmRef::String(_) | RmRef::Character(_)) => Shape::String,
            Self::Leaf(_) => Shape::Other,
            Self::Json(value) => value.shape(),
        }
    }

    fn get(self, key: &str) -> Option<Self> {
        match self {
            Self::Object(object) if key == TYPE_KEY => {
                Some(Self::Leaf(RmRef::String(object.rm_type())))
            }
            Self::Object(object) => object
                .attributes()
                .iter()
                .find(|name| **name == key)
                .and_then(|name| Self::attribute(object, name)),
            Self::Open(open) if key == TYPE_KEY => {
                Some(Self::Leaf(RmRef::String(open.type_name())))
            }
            Self::Open(open) => open.members().get(key).map(Self::Json),
            Self::Json(value) => value.get(key).map(Self::Json),
            Self::List(..) | Self::Leaf(_) => None,
        }
    }

    fn members(self) -> Box<dyn Iterator<Item = (&'a str, Self)> + 'a> {
        match self {
            Self::Object(object) => Box::new(
                std::iter::once((TYPE_KEY, Self::Leaf(RmRef::String(object.rm_type())))).chain(
                    object.attributes().iter().filter_map(move |name| {
                        Self::attribute(object, name).map(|value| (*name, value))
                    }),
                ),
            ),
            Self::Open(open) => Box::new(
                std::iter::once((TYPE_KEY, Self::Leaf(RmRef::String(open.type_name())))).chain(
                    open.members()
                        .iter()
                        .map(|(name, value)| (name.as_str(), Self::Json(value))),
                ),
            ),
            Self::Json(value) => Box::new(
                value
                    .members()
                    .map(|(name, value)| (name, Self::Json(value))),
            ),
            Self::List(..) | Self::Leaf(_) => Box::new(std::iter::empty()),
        }
    }

    fn items(self) -> Box<dyn Iterator<Item = Self> + 'a> {
        match self {
            Self::List(object, name) => match object.items(name) {
                Some(items) => Box::new(items.map(Self::Object)),
                None => Box::new(std::iter::empty()),
            },
            Self::Json(value) => Box::new(value.items().map(Self::Json)),
            Self::Object(_) | Self::Open(_) | Self::Leaf(_) => Box::new(std::iter::empty()),
        }
    }

    fn each_member(self, visit: &mut dyn FnMut(&'a str, Self)) {
        let Self::Object(object) = self else {
            for (key, value) in self.members() {
                visit(key, value);
            }
            return;
        };
        visit(TYPE_KEY, Self::Leaf(RmRef::String(object.rm_type())));
        for name in object.attributes() {
            if let Some(value) = Self::attribute(object, name) {
                visit(name, value);
            }
        }
    }

    fn each_item(self, visit: &mut dyn FnMut(Self)) {
        match self {
            Self::List(object, name) => {
                for item in object.items(name).into_iter().flatten() {
                    visit(Self::Object(item));
                }
            }
            other => {
                for item in other.items() {
                    visit(item);
                }
            }
        }
    }

    fn text(self) -> Option<Cow<'a, str>> {
        match self {
            Self::Leaf(RmRef::String(text)) => Some(Cow::Borrowed(text)),
            Self::Leaf(RmRef::Character(character)) => Some(Cow::Owned(character.to_string())),
            Self::Json(value) => value.text(),
            _ => None,
        }
    }

    fn is_null(self) -> bool {
        matches!(self, Self::Json(value) if value.is_null())
    }

    fn to_value(self) -> Value {
        match self {
            Self::Object(_) | Self::Open(_) => Value::Object(
                self.members()
                    .map(|(name, value)| (name.to_owned(), value.to_value()))
                    .collect(),
            ),
            Self::List(..) => Value::Array(self.items().map(Tree::to_value).collect()),
            Self::Leaf(RmRef::String(text)) => Value::from(text),
            Self::Leaf(RmRef::Character(character)) => Value::from(character.to_string()),
            Self::Leaf(RmRef::Integer(number)) => Value::from(number),
            Self::Leaf(RmRef::Real(number)) => Value::from(number),
            Self::Leaf(RmRef::Boolean(flag)) => Value::from(flag),
            Self::Leaf(RmRef::Object(object)) => Self::Object(object).to_value(),
            Self::Json(value) => value.clone(),
        }
    }
}

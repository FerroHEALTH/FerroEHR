// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The JSON shape the data-minimisation pass reads a body through.
//!
//! A body is a canonical value or the index of its one rendering
//! ([`crate::storage::render::index`]), so a typed commit is scanned with no
//! canonical value built (#6003). No openEHR spec governs this: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the canonical value is one of the two shapes \
              the pass reads"
)]

use std::borrow::Cow;

use serde_json::Value;

use crate::storage::render::index::{Kind, NodeId, Rendered};

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
    /// The text of a string.
    fn text(self) -> Option<Cow<'a, str>>;
    /// Whether the value is `null`.
    fn is_null(self) -> bool;

    /// The string member `key` of an object.
    fn get_text(self, key: &str) -> Option<Cow<'a, str>> {
        self.get(key).and_then(Tree::text)
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

    fn text(self) -> Option<Cow<'a, str>> {
        self.doc.str(self.id)
    }

    fn is_null(self) -> bool {
        self.doc.kind(self.id) == Some(Kind::Scalar) && self.doc.raw(self.id) == b"null"
    }
}

// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One serialization of a body to compact canonical JSON, observed as it is
//! written: the bytes, and an index of every object, array, string and scalar
//! in them by byte range (#6003).
//!
//! The observer is a `serde_json` [`Formatter`] that writes through
//! [`CompactFormatter`] into its own buffer and records where each value
//! starts and ends, so the text is exactly the bytes `serde_json::to_vec`
//! writes and the index costs no second parse. No openEHR spec governs
//! storage: our own design.

use std::borrow::Cow;
use std::io;
use std::ops::Range;

use serde::Serialize;
use serde_json::ser::{CharEscape, CompactFormatter, Formatter};

/// A value's index in [`Rendered`].
pub type NodeId = u32;

/// What a value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A JSON object; its members are [`Rendered::members`].
    Object,
    /// A JSON array; its items are [`Rendered::items`].
    Array,
    /// A JSON string with no escape in it.
    String,
    /// A JSON string with at least one escape in it.
    EscapedString,
    /// A number, `true`, `false` or `null`.
    Scalar,
}

/// A byte or index range, held in 32 bits: [`render`] refuses a text past
/// `u32::MAX` bytes, so every offset fits.
#[derive(Debug, Clone, Copy, Default)]
struct Span {
    start: u32,
    end: u32,
}

impl Span {
    fn new(start: usize, end: usize) -> Self {
        Self {
            start: u32::try_from(start).unwrap_or(u32::MAX),
            end: u32::try_from(end).unwrap_or(u32::MAX),
        }
    }

    fn range(self) -> Range<usize> {
        // NOTE: u32 widens to usize losslessly on every supported target.
        usize::try_from(self.start).unwrap_or(usize::MAX)
            ..usize::try_from(self.end).unwrap_or(usize::MAX)
    }
}

/// One value of the rendered text.
#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    /// The value's bytes, quotes and brackets included.
    span: Span,
    /// The value's members (an object) or items (an array) in
    /// [`Rendered::members`] or [`Rendered::items`].
    children: Span,
}

/// One member of an object.
#[derive(Debug, Clone)]
pub struct Member {
    /// The member's name, without its quotes (RM attribute names carry no
    /// escape).
    key: Span,
    /// The member's whole bytes, `"name":value`.
    span: Span,
    /// The member's value.
    value: NodeId,
}

/// The compact canonical text of a value and the index of every value in it.
#[derive(Debug, Clone)]
pub struct Rendered {
    text: String,
    nodes: Vec<Node>,
    members: Vec<Member>,
    items: Vec<NodeId>,
    root: NodeId,
}

/// Serializes `value` to compact canonical JSON, indexing it as it goes.
///
/// # Errors
/// The [`serde_json::Error`] of a value that does not serialize (a non-string
/// map key, a `Serialize` impl that fails).
pub fn render<T: Serialize + ?Sized>(value: &T) -> Result<Rendered, serde_json::Error> {
    let mut observer = Observer::default();
    {
        let mut serializer = serde_json::Serializer::with_formatter(io::sink(), &mut observer);
        value.serialize(&mut serializer)?;
    }
    let root = observer.last.ok_or_else(|| {
        <serde_json::Error as serde::ser::Error>::custom("the value wrote no JSON")
    })?;
    if u32::try_from(observer.out.len()).is_err() {
        return Err(<serde_json::Error as serde::ser::Error>::custom(
            "the rendered text is past the 4 GiB an index offset holds",
        ));
    }
    let text = String::from_utf8(observer.out)
        .map_err(<serde_json::Error as serde::ser::Error>::custom)?;
    Ok(Rendered {
        text,
        nodes: observer.nodes,
        members: observer.members,
        items: observer.items,
        root,
    })
}

impl Rendered {
    /// The compact canonical text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The compact canonical text, owned.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }

    /// The root value.
    #[must_use]
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Every value, in the order its writing finished (children first).
    pub fn all(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.nodes.len()).filter_map(|i| NodeId::try_from(i).ok())
    }

    fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(usize::try_from(id).ok()?)
    }

    /// What `id` is; `None` for an id this text does not hold.
    #[must_use]
    pub fn kind(&self, id: NodeId) -> Option<Kind> {
        self.node(id).map(|n| n.kind)
    }

    /// The bytes of `id`.
    #[must_use]
    pub fn raw(&self, id: NodeId) -> &[u8] {
        self.raw_text(id).as_bytes()
    }

    /// The text of `id`.
    #[must_use]
    pub fn raw_text(&self, id: NodeId) -> &str {
        self.node(id)
            .and_then(|n| self.text.get(n.span.range()))
            .unwrap_or_default()
    }

    /// The members of the object `id`, in written order; none for a value
    /// that is no object.
    #[must_use]
    pub fn members(&self, id: NodeId) -> &[Member] {
        match self.node(id) {
            Some(n) if n.kind == Kind::Object => {
                self.members.get(n.children.range()).unwrap_or_default()
            }
            _ => &[],
        }
    }

    /// The items of the array `id`, in written order; none for a value that
    /// is no array.
    #[must_use]
    pub fn items(&self, id: NodeId) -> &[NodeId] {
        match self.node(id) {
            Some(n) if n.kind == Kind::Array => {
                self.items.get(n.children.range()).unwrap_or_default()
            }
            _ => &[],
        }
    }

    /// The name of `member`.
    #[must_use]
    pub fn key(&self, member: &Member) -> &str {
        self.text.get(member.key.range()).unwrap_or_default()
    }

    /// The whole bytes of `member`, `"name":value`.
    #[must_use]
    pub fn member_raw(&self, member: &Member) -> &[u8] {
        self.member_text(member).as_bytes()
    }

    /// The whole text of `member`, `"name":value`.
    #[must_use]
    pub fn member_text(&self, member: &Member) -> &str {
        self.text.get(member.span.range()).unwrap_or_default()
    }

    /// The byte range of `member` in [`Self::text`].
    #[must_use]
    pub fn member_span(member: &Member) -> Range<usize> {
        member.span.range()
    }

    /// The value of `member`.
    #[must_use]
    pub fn value(member: &Member) -> NodeId {
        member.value
    }

    /// The value of the member `key` of the object `id`.
    #[must_use]
    pub fn get(&self, id: NodeId, key: &str) -> Option<NodeId> {
        self.members(id)
            .iter()
            .rev()
            .find(|m| self.key(m) == key)
            .map(|m| m.value)
    }

    /// The text of the string `id`, unescaped; `None` for a value that is no
    /// string.
    #[must_use]
    pub fn str(&self, id: NodeId) -> Option<Cow<'_, str>> {
        let raw = self.raw_text(id);
        match self.kind(id)? {
            Kind::String => raw.get(1..raw.len().checked_sub(1)?).map(Cow::Borrowed),
            Kind::EscapedString => serde_json::from_str::<String>(raw).ok().map(Cow::Owned),
            Kind::Object | Kind::Array | Kind::Scalar => None,
        }
    }

    /// Whether `id` is a JSON string.
    #[must_use]
    pub fn is_string(&self, id: NodeId) -> bool {
        matches!(self.kind(id), Some(Kind::String | Kind::EscapedString))
    }

    /// Whether `id` is a JSON number.
    #[must_use]
    pub fn is_number(&self, id: NodeId) -> bool {
        self.kind(id) == Some(Kind::Scalar)
            && self
                .raw(id)
                .first()
                .is_some_and(|b| b.is_ascii_digit() || *b == b'-')
    }

    /// Whether `id` is `true` or `false`, and which.
    #[must_use]
    pub fn bool(&self, id: NodeId) -> Option<bool> {
        match self.raw(id) {
            b"true" => Some(true),
            b"false" => Some(false),
            _ => None,
        }
    }

    /// The string member `key` of the object `id`.
    #[must_use]
    pub fn get_str(&self, id: NodeId, key: &str) -> Option<Cow<'_, str>> {
        self.get(id, key).and_then(|v| self.str(v))
    }

    /// The `_type` of the object `id`.
    #[must_use]
    pub fn rm_type(&self, id: NodeId) -> Option<Cow<'_, str>> {
        self.get(id, "_type").and_then(|v| self.str(v))
    }

    /// Whether the object `id` has exactly the members `keys`, in any order.
    #[must_use]
    pub fn has_keys(&self, id: NodeId, keys: &[&str]) -> bool {
        let members = self.members(id);
        self.kind(id) == Some(Kind::Object)
            && members.len() == keys.len()
            && keys
                .iter()
                .all(|key| members.iter().any(|m| self.key(m) == *key))
    }

    /// Whether the object `id` is exactly `{"_type": rm_type, "value": value}`,
    /// with `value` a string equal to `expected` when one is given.
    #[must_use]
    pub fn is_typed_string(&self, id: NodeId, rm_type: &str, expected: Option<&str>) -> bool {
        self.has_keys(id, &["_type", "value"])
            && self.rm_type(id).as_deref() == Some(rm_type)
            && self.get(id, "value").is_some_and(|v| {
                self.is_string(v) && expected.is_none_or(|e| self.str(v).as_deref() == Some(e))
            })
    }

    /// Writes the RFC 8785 form of `id` to `out`, read off the rendering: the
    /// members of each object in UTF-16 order of their names, a name given
    /// twice with its last value, each number in its ECMAScript form, and
    /// every string as written, since `serde_json` escapes as RFC 8785
    /// §3.2.2.2 does.
    ///
    /// # Errors
    /// A [`serde_json::Error`] for a number with no finite IEEE 754 double
    /// form (RFC 8785 §3.2.2.3).
    pub fn write_canonical(&self, id: NodeId, out: &mut String) -> Result<(), serde_json::Error> {
        self.canonical_into(id, out, &mut Vec::new())
    }

    /// [`Self::write_canonical`] with `order`, the stack the members of the
    /// open objects are sorted on.
    fn canonical_into<'a>(
        &'a self,
        id: NodeId,
        out: &mut String,
        order: &mut Vec<(Cow<'a, str>, NodeId)>,
    ) -> Result<(), serde_json::Error> {
        use openehr_rm::v1_2::common::change_control::canonical_form_impl::{
            utf16_order, write_number, write_string,
        };
        match self.kind(id) {
            Some(Kind::Object) => {
                let from = order.len();
                order.extend(self.members(id).iter().map(|m| (self.name(m), m.value)));
                if let Some(own) = order.get_mut(from..) {
                    own.sort_by(|(a, _), (b, _)| utf16_order(a, b));
                }
                out.push('{');
                let mut written = false;
                for at in from..order.len() {
                    let (Some((name, value)), next) = (order.get(at), order.get(at + 1)) else {
                        break;
                    };
                    if next.is_some_and(|(next, _)| next == name) {
                        continue;
                    }
                    if written {
                        out.push(',');
                    }
                    written = true;
                    match name {
                        // NOTE: RFC 8785 §3.2.2.2, a name serde_json wrote without a
                        // backslash holds nothing to escape.
                        Cow::Borrowed(plain) => {
                            out.push('"');
                            out.push_str(plain);
                            out.push('"');
                        }
                        Cow::Owned(escaped) => write_string(escaped, out),
                    }
                    out.push(':');
                    let value = *value;
                    self.canonical_into(value, out, order)?;
                }
                order.truncate(from);
                out.push('}');
            }
            Some(Kind::Array) => {
                out.push('[');
                for (at, item) in self.items(id).iter().enumerate() {
                    if at > 0 {
                        out.push(',');
                    }
                    self.canonical_into(*item, out, order)?;
                }
                out.push(']');
            }
            Some(Kind::String | Kind::EscapedString) => out.push_str(self.raw_text(id)),
            Some(Kind::Scalar) => match self.raw_text(id) {
                literal @ ("true" | "false" | "null") => out.push_str(literal),
                number => write_number(&number.parse()?, out)?,
            },
            None => {
                return Err(<serde_json::Error as serde::ser::Error>::custom(
                    "a value this rendering does not hold",
                ));
            }
        }
        Ok(())
    }

    /// The name of `member`, unescaped.
    fn name(&self, member: &Member) -> Cow<'_, str> {
        let key = self.key(member);
        if key.contains('\\') {
            let key = member.key.range();
            let quoted = key
                .start
                .checked_sub(1)
                .and_then(|start| self.text.get(start..key.end.saturating_add(1)));
            if let Some(Ok(name)) = quoted.map(serde_json::from_str::<String>) {
                return Cow::Owned(name);
            }
        }
        Cow::Borrowed(key)
    }
}

/// One object or array being written.
#[derive(Debug, Default)]
struct Frame {
    node: usize,
    key_start: usize,
    key: Span,
    member_start: usize,
    members: Vec<Member>,
    items: Vec<NodeId>,
}

/// The formatter that writes the text and records the index.
#[derive(Debug, Default)]
struct Observer {
    out: Vec<u8>,
    nodes: Vec<Node>,
    members: Vec<Member>,
    items: Vec<NodeId>,
    stack: Vec<Frame>,
    spare: Vec<Frame>,
    in_key: bool,
    string_start: usize,
    escaped: bool,
    last: Option<NodeId>,
}

impl Observer {
    fn push_node(&mut self, kind: Kind, start: usize) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node {
            kind,
            span: Span::new(start, start),
            children: Span::default(),
        });
        index
    }

    fn finish(&mut self, index: usize) {
        let end = self.out.len();
        if let Some(node) = self.nodes.get_mut(index) {
            node.span = Span::new(node.span.range().start, end);
        }
        self.last = NodeId::try_from(index).ok();
    }

    fn scalar(&mut self, write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> io::Result<()> {
        let start = self.out.len();
        write(&mut self.out)?;
        if !self.in_key {
            let index = self.push_node(Kind::Scalar, start);
            self.finish(index);
        }
        Ok(())
    }

    fn open(&mut self, kind: Kind) {
        let start = self.out.len().saturating_sub(1);
        let node = self.push_node(kind, start);
        let mut frame = self.spare.pop().unwrap_or_default();
        frame.node = node;
        self.stack.push(frame);
    }

    fn close(&mut self) {
        let Some(mut frame) = self.stack.pop() else {
            return;
        };
        let children = if self.nodes.get(frame.node).map(|n| n.kind) == Some(Kind::Object) {
            let start = self.members.len();
            self.members.append(&mut frame.members);
            Span::new(start, self.members.len())
        } else {
            let start = self.items.len();
            self.items.append(&mut frame.items);
            Span::new(start, self.items.len())
        };
        if let Some(node) = self.nodes.get_mut(frame.node) {
            node.children = children;
        }
        let node = frame.node;
        frame.items.clear();
        frame.members.clear();
        self.spare.push(frame);
        self.finish(node);
    }
}

macro_rules! scalars {
    ($($method:ident: $t:ty),*) => {$(
        fn $method<W: ?Sized + io::Write>(&mut self, _w: &mut W, value: $t) -> io::Result<()> {
            self.scalar(|out| CompactFormatter.$method(out, value))
        }
    )*};
}

impl Formatter for &mut Observer {
    fn write_null<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.scalar(|out| CompactFormatter.write_null(out))
    }

    scalars!(
        write_bool: bool, write_i8: i8, write_i16: i16, write_i32: i32, write_i64: i64,
        write_i128: i128, write_u8: u8, write_u16: u16, write_u32: u32, write_u64: u64,
        write_u128: u128, write_f32: f32, write_f64: f64
    );

    fn write_number_str<W: ?Sized + io::Write>(
        &mut self,
        _w: &mut W,
        value: &str,
    ) -> io::Result<()> {
        self.scalar(|out| CompactFormatter.write_number_str(out, value))
    }

    fn begin_string<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.string_start = self.out.len();
        self.escaped = false;
        CompactFormatter.begin_string(&mut self.out)
    }

    fn end_string<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        CompactFormatter.end_string(&mut self.out)?;
        if !self.in_key {
            let kind = if self.escaped {
                Kind::EscapedString
            } else {
                Kind::String
            };
            let index = self.push_node(kind, self.string_start);
            self.finish(index);
        }
        Ok(())
    }

    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        _w: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        CompactFormatter.write_string_fragment(&mut self.out, fragment)
    }

    fn write_char_escape<W: ?Sized + io::Write>(
        &mut self,
        _w: &mut W,
        char_escape: CharEscape,
    ) -> io::Result<()> {
        self.escaped = true;
        CompactFormatter.write_char_escape(&mut self.out, char_escape)
    }

    fn write_byte_array<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        value: &[u8],
    ) -> io::Result<()> {
        self.begin_array(w)?;
        let mut first = true;
        for byte in value {
            self.begin_array_value(w, first)?;
            self.write_u8(w, *byte)?;
            self.end_array_value(w)?;
            first = false;
        }
        self.end_array(w)
    }

    fn begin_array<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        CompactFormatter.begin_array(&mut self.out)?;
        self.open(Kind::Array);
        Ok(())
    }

    fn end_array<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        CompactFormatter.end_array(&mut self.out)?;
        self.close();
        Ok(())
    }

    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        _w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        CompactFormatter.begin_array_value(&mut self.out, first)
    }

    fn end_array_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        if let (Some(frame), Some(last)) = (self.stack.last_mut(), self.last.take()) {
            frame.items.push(last);
        }
        Ok(())
    }

    fn begin_object<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        CompactFormatter.begin_object(&mut self.out)?;
        self.open(Kind::Object);
        Ok(())
    }

    fn end_object<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        CompactFormatter.end_object(&mut self.out)?;
        self.close();
        Ok(())
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        _w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        CompactFormatter.begin_object_key(&mut self.out, first)?;
        self.in_key = true;
        let at = self.out.len();
        if let Some(frame) = self.stack.last_mut() {
            frame.member_start = at;
            frame.key_start = at;
        }
        Ok(())
    }

    fn end_object_key<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.in_key = false;
        let at = self.out.len();
        if let Some(frame) = self.stack.last_mut() {
            // The name's quotes are left out of its range.
            frame.key = Span::new(frame.key_start.saturating_add(1), at.saturating_sub(1));
        }
        Ok(())
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        CompactFormatter.begin_object_value(&mut self.out)
    }

    fn end_object_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        let at = self.out.len();
        if let (Some(frame), Some(value)) = (self.stack.last_mut(), self.last.take()) {
            frame.members.push(Member {
                key: frame.key,
                span: Span::new(frame.member_start, at),
                value,
            });
        }
        Ok(())
    }

    fn write_raw_fragment<W: ?Sized + io::Write>(
        &mut self,
        _w: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        self.scalar(|out| CompactFormatter.write_raw_fragment(out, fragment))
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the tests compare the index with the canonical value it renders"
)]
mod tests {
    use serde_json::{Value, json};

    use super::{Kind, NodeId, Rendered, render};

    /// The value the index describes, rebuilt from it.
    fn rebuilt(r: &Rendered, id: NodeId) -> Value {
        match r.kind(id).unwrap() {
            Kind::Object => Value::Object(
                r.members(id)
                    .iter()
                    .map(|m| (r.key(m).to_owned(), rebuilt(r, Rendered::value(m))))
                    .collect(),
            ),
            Kind::Array => Value::Array(r.items(id).iter().map(|i| rebuilt(r, *i)).collect()),
            Kind::String | Kind::EscapedString => Value::from(r.str(id).unwrap().into_owned()),
            Kind::Scalar => serde_json::from_slice(r.raw(id)).unwrap(),
        }
    }

    #[test]
    fn the_index_describes_exactly_the_bytes_serde_json_writes() {
        for doc in [
            json!({ "_type": "COMPOSITION", "name": { "value": "\"ä\"\n\u{1}" }, "n": [1, -2.5, 1e300, true, null, [], {}], "e": {} }),
            json!([]),
            json!("s"),
            json!(1.25),
            json!({ "a": { "b": { "c": [[{"d": "\\"}]] } } }),
        ] {
            let r = render(&doc).unwrap();
            assert_eq!(r.text(), serde_json::to_string(&doc).unwrap(), "{doc}");
            assert_eq!(rebuilt(&r, r.root()), doc);
            for m in r.members(r.root()) {
                assert_eq!(
                    r.member_raw(m),
                    format!(
                        "{}:{}",
                        serde_json::to_string(r.key(m)).unwrap(),
                        serde_json::to_string(&doc[r.key(m)]).unwrap()
                    )
                    .as_bytes()
                );
            }
        }
    }

    /// The RFC 8785 form read off a rendering is the one built from its value.
    #[test]
    fn the_canonical_form_read_off_a_rendering_is_the_values() {
        for doc in [
            json!({ "z": 1, "é": 2, "a": { "y": [3, { "b": 1, "a": 0 }], "x": null }, "\u{10000}": true, "\u{ff61}": false }),
            json!({ "n": [1.0, -0.0, 0.1, 1e21, 1e-7, 5e-324, f64::MAX, -12, 9_007_199_254_740_993_u64, 123_456_789_012_i64] }),
            json!({ "s": "\"q\"\\\n\u{1}\u{1f}\u{7f}/\u{2028}é", "k\"ey": "v", "t\nab": 1 }),
            json!([{ "b": [], "a": {} }, "x", 2.5]),
            json!("only"),
        ] {
            let r = render(&doc).unwrap();
            let mut read = String::new();
            r.write_canonical(r.root(), &mut read).unwrap();
            assert_eq!(
                read,
                openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json(&doc)
                    .unwrap(),
                "{doc}"
            );
        }
    }
}

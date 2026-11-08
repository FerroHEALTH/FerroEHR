// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The locals a bound-dependent default reads.
//!
//! An `Interval` flag the wire omits follows its bound
//! ([`crate::plan::overrides::FIELD_DEFAULTS`]): its default is the expression
//! `__present_<bound>` or `!__present_<bound>`. Each reader binds that local
//! from the bound's slot before it builds the value, because the struct
//! literal moves the bound out of its slot before the flag's default runs.

use crate::plan::{JsonField, XmlField};

/// The local that holds whether field `name` was read.
fn local(name: &str) -> String {
    format!("__present_{name}")
}

/// One `let` per field that some field's default names, each bound to
/// `present(index, name)`.
fn locals<'a>(
    fields: impl Iterator<Item = (&'a str, Option<&'a str>)> + Clone,
    present: impl Fn(usize, &str) -> String,
) -> String {
    let named: Vec<&str> = fields
        .clone()
        .filter_map(|(_, default)| default)
        .map(|default| default.trim_start_matches('!'))
        .collect();
    let mut out = String::new();
    for (index, (name, _)) in fields.enumerate() {
        let local = local(name);
        if named.contains(&local.as_str()) {
            out.push_str(&format!("let {local} = {};\n", present(index, name)));
        }
    }
    out
}

/// The presence locals of a canonical-JSON reader, whose slot `i` is an
/// `Option<Option<T>>` (`null` reads as absent).
pub(crate) fn json_locals(fields: &[JsonField]) -> String {
    let named = fields
        .iter()
        .map(|f| (f.rust_name.as_str(), f.default.as_deref()));
    locals(named, |i, _| {
        format!("__s{i}.as_ref().is_some_and(Option::is_some)")
    })
}

/// The presence locals of a canonical-XML reader, whose accumulator for a
/// field is named by `slot` and holds an `Option<T>`.
pub(crate) fn xml_locals(fields: &[XmlField], slot: fn(&str) -> String) -> String {
    let named = fields
        .iter()
        .map(|f| (f.rust_name.as_str(), f.default.as_deref()));
    locals(named, |_, name| format!("{}.is_some()", slot(name)))
}

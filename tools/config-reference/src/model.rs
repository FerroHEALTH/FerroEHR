// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The configuration tree read from its JSON Schema: one [`Section`] per
//! table, each holding its leaf [`Key`]s, in the order the schema declares
//! them. No specification governs this reading: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "a JSON Schema document is JSON by definition (JSON Schema 2020-12 \
              Core §4.2): this module is the one seam that walks it as values"
)]

use serde_json::{Map, Value};

/// How a section is written in TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// `[path]`: one table.
    Table,
    /// `[[path]]`: an array of tables.
    Array,
    /// `[path.<name>]`: one table per name the operator chooses.
    Named,
}

/// One table of the configuration tree.
#[derive(Debug, Clone)]
pub(crate) struct Section {
    /// The dotted path, `<name>` standing for an operator-chosen key.
    pub(crate) path: String,
    /// How the section is written.
    pub(crate) shape: Shape,
    /// The first paragraph of the table's description.
    pub(crate) description: String,
    /// Whether the table may be left out entirely.
    pub(crate) optional: bool,
    /// The leaf keys, in declaration order.
    pub(crate) keys: Vec<Key>,
}

/// One leaf key.
#[derive(Debug, Clone)]
pub(crate) struct Key {
    /// The key as written.
    pub(crate) name: String,
    /// The default as TOML, `None` when the key has none.
    pub(crate) default: Option<String>,
    /// Whether the default hides a secret (`***`), so it is never written
    /// as a live line.
    pub(crate) masked: bool,
    /// A placeholder value of the key's type, for a commented example line.
    pub(crate) example: String,
    /// The first paragraph of the key's description.
    pub(crate) description: String,
}

/// The sections of the tree `root` describes, every table before the tables
/// it holds.
///
/// # Errors
/// A message naming the path whose `$ref` the schema does not define.
pub(crate) fn sections(root: &Value) -> Result<Vec<Section>, String> {
    let defs = root
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let walker = Walker { defs: &defs };
    let mut out = Vec::new();
    let place = Place {
        path: String::new(),
        shape: Shape::Table,
        optional: false,
        described: None,
        inherited: None,
    };
    walker.table(root, &place, &mut out)?;
    // The root holds no key of its own, only its sections.
    out.retain(|section| !section.path.is_empty() || !section.keys.is_empty());
    Ok(out)
}

/// Where a table sits: its path, its shape, whether it may be left out, the
/// description its holder gives it and the default its holder fills it from.
struct Place<'a> {
    /// The dotted path.
    path: String,
    /// How the table is written.
    shape: Shape,
    /// Whether the table may be left out.
    optional: bool,
    /// The holder's description of the table.
    described: Option<&'a str>,
    /// The holder's default of the table.
    inherited: Option<&'a Value>,
}

/// Reads schemas against the `$defs` of one root.
struct Walker<'a> {
    /// The root's `$defs`.
    defs: &'a Map<String, Value>,
}

impl Walker<'_> {
    /// `node` with its `$ref` followed.
    fn resolve<'b>(&'b self, node: &'b Value) -> Result<&'b Value, String> {
        match node.get("$ref").and_then(Value::as_str) {
            Some(target) => {
                let name = target.trim_start_matches("#/$defs/");
                let def = self
                    .defs
                    .get(name)
                    .ok_or_else(|| format!("the schema defines no {target}"))?;
                self.resolve(def)
            }
            None => Ok(node),
        }
    }

    /// The schema `node` stands for once a `null` alternative is dropped, and
    /// whether it had one.
    fn unwrap<'b>(&'b self, node: &'b Value) -> Result<(&'b Value, bool), String> {
        let node = self.resolve(node)?;
        if let Some(Value::Array(types)) = node.get("type") {
            let nullable = types.iter().any(|t| t == "null");
            return Ok((node, nullable));
        }
        if let Some(Value::Array(branches)) = node.get("anyOf") {
            let (nulls, rest): (Vec<&Value>, Vec<&Value>) = branches
                .iter()
                .partition(|branch| branch.get("type").is_some_and(|t| t == "null"));
            if let ([_, ..], [one]) = (nulls.as_slice(), rest.as_slice()) {
                let (inner, _) = self.unwrap(one)?;
                return Ok((inner, true));
            }
        }
        Ok((node, false))
    }

    /// Whether `node` is a table with named members.
    fn is_table(node: &Value) -> bool {
        node.get("properties")
            .and_then(Value::as_object)
            .is_some_and(|members| !members.is_empty())
    }

    /// The table among the alternatives of `node`, when every alternative is
    /// a table or an array of tables, such as a key written as one table or
    /// as one table per tenant.
    fn table_alternative<'b>(&'b self, node: &'b Value) -> Result<Option<&'b Value>, String> {
        let Some(Value::Array(branches)) = node.get("anyOf").or_else(|| node.get("oneOf")) else {
            return Ok(None);
        };
        let mut table = None;
        for branch in branches {
            let branch = self.resolve(branch)?;
            if branch.get("type").is_some_and(|t| t == "null") {
                continue;
            }
            if Self::is_table(branch) {
                table = table.or(Some(branch));
                continue;
            }
            let items = match branch.get("items") {
                Some(items) => Some(self.resolve(items)?),
                None => None,
            };
            if !items.is_some_and(Self::is_table) {
                return Ok(None);
            }
        }
        Ok(table)
    }

    /// Adds the section at `path` described by `node`, then the sections of
    /// the tables it holds.
    fn table(&self, node: &Value, place: &Place<'_>, out: &mut Vec<Section>) -> Result<(), String> {
        let Place {
            path,
            shape,
            optional,
            described,
            inherited,
        } = place;
        let (path, shape, optional, described, inherited) =
            (path.as_str(), *shape, *optional, *described, *inherited);
        let mut keys = Vec::new();
        let mut children = Vec::new();
        let members = node
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (name, member) in &members {
            let (target, nullable) = self.unwrap(member)?;
            if target.get("deprecated").is_some_and(|d| d == true) {
                continue;
            }
            let child = join(path, name);
            if Self::is_table(target) {
                children.push((child, target, Shape::Table, nullable, member));
                continue;
            }
            if let Some(items) = target.get("items") {
                let (item, _) = self.unwrap(items)?;
                if Self::is_table(item) {
                    children.push((child, item, Shape::Array, true, member));
                    continue;
                }
            }
            if let Some(table) = self.table_alternative(target)? {
                children.push((child, table, Shape::Table, true, member));
                continue;
            }
            if let Some(value) = target.get("additionalProperties").filter(|v| v.is_object()) {
                let (value, _) = self.unwrap(value)?;
                if Self::is_table(value) {
                    children.push((join(&child, "<name>"), value, Shape::Named, true, member));
                    continue;
                }
            }
            // NOTE: a holder's own default wins over its member type's, as serde's
            // container `default` fills a missing table from the holder.
            let default = inherited
                .and_then(|holder| holder.get(name))
                .or_else(|| member.get("default"))
                .filter(|d| !d.is_null());
            let rendered = default.map(toml_value);
            keys.push(Key {
                name: name.clone(),
                masked: rendered.as_deref().is_some_and(|d| d.contains("***")),
                default: rendered,
                example: self.example(member)?,
                description: first_paragraph(member, name),
            });
        }
        out.push(Section {
            path: path.to_owned(),
            shape,
            description: described
                .or_else(|| node.get("description").and_then(Value::as_str))
                .map(|text| {
                    let header = match shape {
                        Shape::Array => format!("[[{path}]]"),
                        Shape::Table | Shape::Named => format!("[{path}]"),
                    };
                    clean(paragraph(text), &header)
                })
                .unwrap_or_default(),
            optional,
            keys,
        });
        for (child, target, shape, optional, member) in children {
            let text = member.get("description").and_then(Value::as_str);
            let name = child.rsplit('.').next().unwrap_or_default();
            // A table the holder's default fills is read from it; an array or
            // a named table has no one default element, so its keys keep
            // their own.
            let default = match shape {
                Shape::Table => inherited
                    .and_then(|holder| holder.get(name))
                    .or_else(|| member.get("default"))
                    .filter(|d| d.is_object()),
                Shape::Array | Shape::Named => None,
            };
            let place = Place {
                path: child,
                shape,
                optional,
                described: text,
                inherited: default,
            };
            self.table(target, &place, out)?;
        }
        Ok(())
    }

    /// A value of `node`'s type for a commented example line.
    fn example(&self, node: &Value) -> Result<String, String> {
        let node = self.resolve(node)?;
        if let Some(first) = node
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|e| e.first())
        {
            return Ok(toml_value(first));
        }
        if let Some(first) = node
            .get("examples")
            .and_then(Value::as_array)
            .and_then(|e| e.first())
        {
            return Ok(toml_value(first));
        }
        for alternatives in ["oneOf", "anyOf"] {
            if let Some(Value::Array(branches)) = node.get(alternatives) {
                for branch in branches {
                    let branch = self.resolve(branch)?;
                    if let Some(value) = branch.get("const") {
                        return Ok(toml_value(value));
                    }
                    if let Some(first) = branch
                        .get("enum")
                        .and_then(Value::as_array)
                        .and_then(|e| e.first())
                    {
                        return Ok(toml_value(first));
                    }
                    if branch.get("type").is_some_and(|t| t == "null") {
                        continue;
                    }
                    return self.example(branch);
                }
            }
        }
        let first_type = match node.get("type") {
            Some(Value::String(one)) => one.as_str(),
            Some(Value::Array(many)) => many
                .iter()
                .filter_map(Value::as_str)
                .find(|t| *t != "null")
                .unwrap_or("string"),
            _ => "string",
        };
        Ok(match first_type {
            "integer" | "number" => "0",
            "boolean" => "false",
            "array" => "[]",
            "object" => "{}",
            _ => "\"\"",
        }
        .to_owned())
    }
}

/// `path` and `name`, dotted.
fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path}.{name}")
    }
}

/// The first paragraph of `member`'s description, cleaned for `name`.
fn first_paragraph(member: &Value, name: &str) -> String {
    member
        .get("description")
        .and_then(Value::as_str)
        .map(|text| clean(paragraph(text), name))
        .unwrap_or_default()
}

/// The first paragraph of `text`, without a closing sentence that only
/// introduces a list the paragraph does not hold.
fn paragraph(text: &str) -> &str {
    let first = text.split("\n\n").next().unwrap_or_default().trim_end();
    if first.ends_with(':')
        && let Some(end) = first.rfind(". ")
    {
        return first.split_at(end + 1).0;
    }
    first
}

/// `text` on one line, with rustdoc's intra-doc links reduced to their text
/// and a leading "`name` —" or "`name`:" dropped, since the key is named
/// beside it.
fn clean(text: &str, name: &str) -> String {
    let mut line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !name.is_empty() {
        for lead in [
            format!("`{name}` — "),
            format!("`{name}`: "),
            format!("`{name}` - "),
        ] {
            if let Some(rest) = line.strip_prefix(&lead) {
                line = capitalised(rest);
                break;
            }
        }
    }
    unlink(&line)
}

/// `text` with its first letter in upper case.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// `text` with every `[label](target)` written as `label`, and every
/// `[label]` whose label is code written as the label alone.
fn unlink(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        let Some(close) = after.find(']') else {
            out.push_str(after);
            return out;
        };
        let (label, tail) = after.split_at(close);
        let label = label.trim_start_matches('[');
        let tail = tail.trim_start_matches(']');
        if let Some(target) = tail.strip_prefix('(')
            && let Some(end) = target.find(')')
        {
            out.push_str(label);
            rest = target.split_at(end).1.trim_start_matches(')');
        } else if label.starts_with('`') && label.ends_with('`') {
            out.push_str(label);
            rest = tail;
        } else {
            out.push('[');
            out.push_str(label);
            out.push(']');
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// `value` as a TOML value: a JSON string is a valid TOML basic string, since
/// every JSON escape is a TOML one (TOML 1.0.0 §String).
pub(crate) fn toml_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => Value::String(s.clone()).to_string(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(toml_value).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(members) => {
            if members.is_empty() {
                return "{}".to_owned();
            }
            format!(
                "{{ {} }}",
                members
                    .iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| format!("{} = {}", bare(k), toml_value(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

/// `key` bare when TOML allows it, quoted otherwise (TOML 1.0.0 §Keys).
pub(crate) fn bare(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        key.to_owned()
    } else {
        Value::String(key.to_owned()).to_string()
    }
}

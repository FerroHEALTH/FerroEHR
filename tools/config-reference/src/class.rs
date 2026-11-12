// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Why each key exists, read from the configuration schema itself (#5455).
//!
//! A key is classified by the `x-ferroehr-class` keyword the configuration
//! types carry through `schemars(extend(..))`: `infrastructure` or `trust`
//! for a key that stays in the file for that reason, `optional` for a key
//! that only tunes a built-in default. The keyword sits on a field, which
//! classifies that key or, for a table of a family crate's type, every key
//! below it; on a struct, which classifies the table's own keys; or on a
//! struct as an object of relative key paths, for the keys a family crate
//! defines under it. `x-ferroehr-must-set` marks the keys a deployment that
//! holds real data sets, the same three ways. A key with no class, an
//! `optional` key with no default, a must-set key that only tunes a default
//! and an entry that names no key are refused, so the file cannot grow a key
//! nobody classified. No specification governs this: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "a JSON Schema document is JSON by definition (JSON Schema 2020-12 \
              Core §4.2): this module reads the keywords the configuration types \
              extend it with"
)]

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;

use serde_json::{Map, Value};

/// The keyword naming a key's class.
pub(crate) const CLASS: &str = "x-ferroehr-class";
/// The keyword marking a key a deployment holding real data sets.
pub(crate) const MUST_SET: &str = "x-ferroehr-must-set";

/// Why a key is in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// What the server needs before it can reach its store: listeners, the
    /// database, the integrations and their keys.
    Infrastructure,
    /// What the server trusts or holds as a floor: the issuers and their
    /// bounds, the administrator authority, the grant ceiling, break-glass,
    /// the security floors, the secrets and the provider identity.
    Trust,
    /// A tuning key with a built-in default, which no deployment needs to
    /// set.
    Optional,
}

impl Class {
    /// The class `token` names.
    fn parse(token: &str) -> Option<Self> {
        match token {
            "infrastructure" => Some(Self::Infrastructure),
            "trust" => Some(Self::Trust),
            "optional" => Some(Self::Optional),
            _ => None,
        }
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Infrastructure => "infrastructure",
            Self::Trust => "trust",
            Self::Optional => "optional",
        })
    }
}

/// The relative entries one struct declares for the keys under it.
#[derive(Debug, Clone)]
pub(crate) struct Rules {
    /// The dotted path of the table the struct is read at.
    base: String,
    /// Relative key or table path to class.
    classes: Map<String, Value>,
    /// Relative key paths a deployment holding real data sets.
    must_set: Vec<String>,
}

/// What a walk has learned: the entries it was given, the entries it used and
/// the problems it found.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    /// Every relative entry, as `<base>|<path>`.
    declared: RefCell<BTreeSet<String>>,
    /// The entries a key or a table matched.
    used: RefCell<BTreeSet<String>>,
    /// The problems, one line each.
    problems: RefCell<Vec<String>>,
}

impl Ledger {
    /// Records a problem.
    pub(crate) fn problem(&self, text: String) {
        self.problems.borrow_mut().push(text);
    }

    /// The relative entries `node` declares for the table at `base`.
    pub(crate) fn rules(&self, node: &Value, base: &str) -> Option<Rules> {
        let classes = node
            .get(CLASS)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let must_set: Vec<String> = node
            .get(MUST_SET)
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if classes.is_empty() && must_set.is_empty() {
            return None;
        }
        let mut declared = self.declared.borrow_mut();
        for entry in classes.keys().chain(must_set.iter()) {
            declared.insert(format!("{base}|{entry}"));
        }
        Some(Rules {
            base: base.to_owned(),
            classes,
            must_set,
        })
    }

    /// The class `node` names as a string, refusing an unknown token.
    pub(crate) fn own(&self, node: &Value, path: &str) -> Option<Class> {
        let token = node.get(CLASS).and_then(Value::as_str)?;
        let class = Class::parse(token);
        if class.is_none() {
            self.problem(format!("{path}: unknown class `{token}`"));
        }
        class
    }

    /// The class the nearest of `rules` gives `path` or a table above it.
    pub(crate) fn lookup(&self, rules: &[Rules], path: &str) -> Option<Class> {
        for rule in rules.iter().rev() {
            let Some(relative) = relative(&rule.base, path) else {
                continue;
            };
            let mut candidate = relative;
            loop {
                if let Some(token) = rule.classes.get(candidate).and_then(Value::as_str) {
                    self.used
                        .borrow_mut()
                        .insert(format!("{}|{candidate}", rule.base));
                    let class = Class::parse(token);
                    if class.is_none() {
                        self.problem(format!("{path}: unknown class `{token}`"));
                    }
                    return class;
                }
                match candidate.rsplit_once('.') {
                    Some((above, _)) => candidate = above,
                    None => break,
                }
            }
        }
        None
    }

    /// The class of the key or table at `path` whose holder schema is
    /// `member`: its own, else the nearest of `rules`, else `fallback`.
    pub(crate) fn classify(
        &self,
        member: &Value,
        path: &str,
        rules: &[Rules],
        fallback: Option<Class>,
    ) -> Option<Class> {
        self.own(member, path)
            .or_else(|| self.lookup(rules, path))
            .or(fallback)
    }

    /// The class of the key at `path` once it is held to the rules: a class
    /// at all, a default when it is `optional`, and a reason to stay when it
    /// must be set; `None`, with the problem recorded, otherwise.
    pub(crate) fn check(
        &self,
        path: &str,
        class: Option<Class>,
        must_set: bool,
        required: bool,
        has_default: bool,
    ) -> Option<Class> {
        let Some(class) = class else {
            self.problem(format!("{path}: no class"));
            return None;
        };
        if class == Class::Optional && (required || !has_default) {
            self.problem(format!("{path}: optional, yet it has no default"));
        }
        if must_set && class == Class::Optional {
            self.problem(format!("{path}: must be set, yet only optional"));
        }
        Some(class)
    }

    /// Whether one of `rules` marks `path` must-set.
    pub(crate) fn must_set(&self, rules: &[Rules], path: &str) -> bool {
        for rule in rules {
            if let Some(relative) = relative(&rule.base, path)
                && rule.must_set.iter().any(|entry| entry == relative)
            {
                self.used
                    .borrow_mut()
                    .insert(format!("{}|{relative}", rule.base));
                return true;
            }
        }
        false
    }

    /// The problems found, the entries no key matched among them.
    pub(crate) fn finish(&self) -> Vec<String> {
        let used = self.used.borrow();
        let mut problems = self.problems.borrow().clone();
        for entry in self.declared.borrow().difference(&used) {
            let (base, path) = entry.split_once('|').unwrap_or((entry, ""));
            problems.push(format!(
                "{}: the entry `{path}` names no key",
                if base.is_empty() { "the root" } else { base }
            ));
        }
        problems
    }
}

/// `path` relative to `base`, when it lies below it.
fn relative<'a>(base: &str, path: &'a str) -> Option<&'a str> {
    if base.is_empty() {
        return Some(path);
    }
    path.strip_prefix(base)?.strip_prefix('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_entry_classifies_a_key_and_the_table_above_it() {
        let ledger = Ledger::default();
        let node = serde_json::json!({
            CLASS: { "issuer": "trust", "realm": "optional" },
            MUST_SET: ["issuer.issuer"],
        });
        let rules = vec![ledger.rules(&node, "auth").expect("entries")];
        assert_eq!(
            ledger.lookup(&rules, "auth.issuer.audience"),
            Some(Class::Trust)
        );
        assert_eq!(ledger.lookup(&rules, "auth.realm"), Some(Class::Optional));
        assert_eq!(ledger.lookup(&rules, "auth.enabled"), None);
        assert!(ledger.must_set(&rules, "auth.issuer.issuer"));
        assert!(!ledger.must_set(&rules, "auth.issuer.kind"));
        assert!(ledger.finish().is_empty(), "{:?}", ledger.finish());
    }

    #[test]
    fn an_entry_no_key_matches_is_a_problem() {
        let ledger = Ledger::default();
        let node = serde_json::json!({ CLASS: { "gone": "trust" } });
        let rules = vec![ledger.rules(&node, "auth").expect("entries")];
        assert_eq!(ledger.lookup(&rules, "auth.here"), None);
        let problems = ledger.finish();
        assert_eq!(problems, ["auth: the entry `gone` names no key"]);
    }

    #[test]
    fn an_optional_key_needs_a_default_and_a_must_set_key_a_reason() {
        let ledger = Ledger::default();
        assert_eq!(
            ledger.check("a.b", None, false, false, true),
            None,
            "no class"
        );
        assert_eq!(
            ledger.check("a.c", Some(Class::Optional), false, true, false),
            Some(Class::Optional)
        );
        assert_eq!(
            ledger.check("a.d", Some(Class::Optional), true, false, true),
            Some(Class::Optional)
        );
        assert_eq!(
            ledger.check("a.e", Some(Class::Trust), true, true, false),
            Some(Class::Trust)
        );
        assert_eq!(
            ledger.finish(),
            [
                "a.b: no class",
                "a.c: optional, yet it has no default",
                "a.d: must be set, yet only optional",
            ]
        );
    }

    #[test]
    fn an_unknown_class_is_a_problem() {
        let ledger = Ledger::default();
        let node = serde_json::json!({ CLASS: "policy" });
        assert_eq!(ledger.own(&node, "authz.x"), None);
        assert_eq!(ledger.finish(), ["authz.x: unknown class `policy`"]);
    }
}

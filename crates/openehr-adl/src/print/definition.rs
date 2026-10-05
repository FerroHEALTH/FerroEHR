// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `definition` section — cADL (`ADL2/master04`, grammar `cadl2.g4` +
//! `cadl2_primitives.g4`): the object/attribute/tuple productions, archetype
//! roots, internal-node proxies and archetype slots, plus every primitive,
//! interval and temporal value rendering the cADL leaf forms need.

use std::fmt::Write;

use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
use openehr_am::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
use openehr_base::prelude::{
    Cardinality, Interval, MultiplicityInterval, PointInterval, ProperInterval,
};

use crate::aom::build::cobject_to_primitive;
use crate::odin::regex_of;
use crate::print::odin::quoted;
use crate::print::rules::assertion_str;
use crate::print::{PrintError, Printer};

impl Printer {
    // ── definition (cADL) ──────────────────────────────────────────────────
    pub(super) fn definition(&mut self, def: &CComplexObject) -> Result<(), PrintError> {
        // The definition root is a `C_COMPLEX_OBJECT` (plain or `C_ARCHETYPE_ROOT`),
        // both dispatched by `object`.
        let obj = CObject::CComplexObject(def.clone());
        self.object(&obj, 1)
    }

    fn object(&mut self, obj: &CObject, depth: usize) -> Result<(), PrintError> {
        let mut head = String::new();
        if let Some(so) = crate::aom::access::sibling_order(obj) {
            head.push_str(&sibling_str(so));
            head.push(' ');
        }
        match obj {
            CObject::CComplexObject(CComplexObject::CComplexObject(d)) => {
                self.complex_object(&mut head, d, depth)?;
            }
            CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
                self.archetype_root(&head, r, depth)?;
            }
            CObject::CComplexObjectProxy(pr) => self.proxy(&head, pr, depth),
            CObject::ArchetypeSlot(s) => self.slot(&head, s, depth)?,
            // Primitives with a real node id are regular primitive objects.
            other => self.primitive_object(&head, other, depth),
        }
        Ok(())
    }

    /// Prints a plain `C_COMPLEX_OBJECT`: its head line, and a `matches` body
    /// when it carries attributes, attribute tuples or a default value.
    ///
    /// # Errors
    /// [`PrintError`] from a nested attribute.
    fn complex_object(
        &mut self,
        head: &mut String,
        d: &openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObjectData,
        depth: usize,
    ) -> Result<(), PrintError> {
        let _ = write!(head, "{}{}", d.rm_type_name, node_bracket(&d.node_id));
        head.push_str(&occ_suffix(d.occurrences.as_ref()));
        let has_body = !d.attributes.as_ref().is_none_or(Vec::is_empty)
            || !d.attribute_tuples.as_ref().is_none_or(Vec::is_empty)
            || d.default_value.is_some();
        if !has_body {
            self.line(depth, head);
            return Ok(());
        }
        self.line(depth, &format!("{head} matches {{"));
        for a in d.attributes.iter().flatten() {
            self.attribute(a, depth + 1)?;
        }
        for t in d.attribute_tuples.iter().flatten() {
            self.attribute_tuple(t, depth + 1);
        }
        if let Some(dv) = &d.default_value {
            self.default_value(dv, depth + 1);
        }
        self.line(depth, "}");
        Ok(())
    }

    /// Prints a primitive object: inline where it carries the placeholder node
    /// id (only reached inside an attribute body), else as a node-identified
    /// object with its `matches` value.
    fn primitive_object(&mut self, head: &str, obj: &CObject, depth: usize) {
        let Some(prim) = cobject_to_primitive(obj) else {
            return;
        };
        let (ty, node_id) = prim_type_and_node(obj);
        if node_id == "Primitive_node_id" {
            self.line(depth, &format!("{head}{}", primitive_inline(&prim)));
            return;
        }
        let value = primitive_inline(&prim);
        if value.is_empty() {
            self.line(depth, &format!("{head}{ty}{}", node_bracket(&node_id)));
        } else {
            self.line(
                depth,
                &format!("{head}{ty}{} matches {{{value}}}", node_bracket(&node_id)),
            );
        }
    }

    fn attribute(&mut self, a: &CAttribute, depth: usize) -> Result<(), PrintError> {
        let name = match &a.differential_path {
            Some(path) => format!("{path}/{}", a.rm_attribute_name),
            None => a.rm_attribute_name.clone(),
        };
        let mut head = name;
        if let Some(ex) = &a.existence {
            let _ = write!(head, " existence matches {}", mult_braces(ex));
        }
        if let Some(card) = &a.cardinality {
            let _ = write!(head, " cardinality matches {}", card_braces(card));
        }
        if a.children.as_ref().is_none_or(Vec::is_empty) {
            self.line(depth, &head);
            return Ok(());
        }
        // A single C_STRING regex child came from the `attr matches {/re/}`
        // contained-regexp shortcut (`cadl2.g4`); re-emit that form.
        if let [CObject::CString(cs)] = a.children.as_deref().unwrap_or_default()
            && let Some(regex) = regex_of(cs.constraint.as_deref().unwrap_or_default())
        {
            let mut body = regex.to_owned();
            if let Some(assumed) = &cs.assumed_value {
                let _ = write!(body, "; {}", quoted(assumed));
            }
            self.line(depth, &format!("{head} matches {{{body}}}"));
            return Ok(());
        }
        // A single inline primitive child prints inline; regular objects nest.
        if let [child] = a.children.as_deref().unwrap_or_default()
            && let Some(prim) = cobject_to_primitive(child)
            && prim_type_and_node(child).1 == "Primitive_node_id"
        {
            self.line(
                depth,
                &format!("{head} matches {{{}}}", primitive_inline(&prim)),
            );
            return Ok(());
        }
        self.line(depth, &format!("{head} matches {{"));
        for child in a.children.iter().flatten() {
            self.object(child, depth + 1)?;
        }
        self.line(depth, "}");
        Ok(())
    }

    fn attribute_tuple(&mut self, t: &CAttributeTuple, depth: usize) {
        let members = t
            .members
            .iter()
            .flatten()
            .map(|m| m.rm_attribute_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        self.line(depth, &format!("[{members}] matches {{"));
        // Tuple rows are comma-separated (`cadl2.g4` `c_primitive_tuple
        // (',' c_primitive_tuple)*`); emit a trailing comma on all but the last.
        let last = t.tuples.as_ref().map_or(0, Vec::len).saturating_sub(1);
        for (idx, row) in t.tuples.iter().flatten().enumerate() {
            self.tuple_row(row, depth + 1, idx != last);
        }
        self.line(depth, "}");
    }

    fn tuple_row(&mut self, row: &CPrimitiveTuple, depth: usize, comma: bool) {
        let items = row
            .members
            .iter()
            .map(|m| format!("{{{}}}", primitive_inline(m)))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if comma { "," } else { "" };
        self.line(depth, &format!("[{items}]{sep}"));
    }

    fn archetype_root(
        &mut self,
        head: &str,
        r: &CArchetypeRoot,
        depth: usize,
    ) -> Result<(), PrintError> {
        let node = if r.node_id.is_empty() {
            format!("[{}]", r.archetype_ref)
        } else {
            format!("[{}, {}]", r.node_id, r.archetype_ref)
        };
        let occ = occ_suffix(r.occurrences.as_ref());
        // An OPT-inlined root carries the flattened filler structure in
        // `attributes`/`attribute_tuples` (OPT2 master03 §Flattening); it prints
        // as a plain object head `TYPE[id, ref] occ matches { … }` (no
        // `use_archetype` keyword), which the cADL parser reads back as a
        // `C_ARCHETYPE_ROOT`. A source-form external reference / slot filler has
        // Void children and prints with the `use_archetype` keyword
        // (`cadl2.g4` c_archetype_root).
        if r.attributes.as_ref().is_none_or(Vec::is_empty)
            && r.attribute_tuples.as_ref().is_none_or(Vec::is_empty)
        {
            self.line(
                depth,
                &format!("{head}use_archetype {}{node}{occ}", r.rm_type_name),
            );
            return Ok(());
        }
        self.line(
            depth,
            &format!("{head}{}{node}{occ} matches {{", r.rm_type_name),
        );
        for a in r.attributes.iter().flatten() {
            self.attribute(a, depth + 1)?;
        }
        for t in r.attribute_tuples.iter().flatten() {
            self.attribute_tuple(t, depth + 1);
        }
        self.line(depth, "}");
        Ok(())
    }

    fn proxy(&mut self, head: &str, pr: &CComplexObjectProxy, depth: usize) {
        let occ = occ_suffix(pr.occurrences.as_ref());
        self.line(
            depth,
            &format!(
                "{head}use_node {}{}{occ} {}",
                pr.rm_type_name,
                node_bracket(&pr.node_id),
                pr.target_path
            ),
        );
    }

    fn slot(&mut self, head: &str, s: &ArchetypeSlot, depth: usize) -> Result<(), PrintError> {
        let base = format!(
            "{head}allow_archetype {}{}",
            s.rm_type_name,
            node_bracket(&s.node_id)
        );
        if s.is_closed {
            self.line(depth, &format!("{base} closed"));
            return Ok(());
        }
        let occ = occ_suffix(s.occurrences.as_ref());
        if s.includes.as_ref().is_none_or(Vec::is_empty)
            && s.excludes.as_ref().is_none_or(Vec::is_empty)
        {
            self.line(depth, &format!("{base}{occ}"));
            return Ok(());
        }
        self.line(depth, &format!("{base}{occ} matches {{"));
        // `c_includes : SYM_INCLUDE assertion+` (`cadl2.g4`): one keyword
        // introduces the whole assertion list.
        if !s.includes.as_ref().is_none_or(Vec::is_empty) {
            self.line(depth + 1, "include");
            for inc in s.includes.iter().flatten() {
                self.line(depth + 2, &assertion_str(inc)?);
            }
        }
        if !s.excludes.as_ref().is_none_or(Vec::is_empty) {
            self.line(depth + 1, "exclude");
            for exc in s.excludes.iter().flatten() {
                self.line(depth + 2, &assertion_str(exc)?);
            }
        }
        self.line(depth, "}");
        Ok(())
    }
}

// ── free helpers ──────────────────────────────────────────────────────────

/// `[node_id]` when the object carries one, otherwise nothing.
fn node_bracket(node_id: &str) -> String {
    if node_id.is_empty() {
        String::new()
    } else {
        format!("[{node_id}]")
    }
}

/// The ` occurrences matches {…}` suffix of an object head, when constrained.
fn occ_suffix(occ: Option<&MultiplicityInterval>) -> String {
    occ.map(|m| format!(" occurrences matches {}", mult_braces(m)))
        .unwrap_or_default()
}

/// A `MULTIPLICITY_INTERVAL` in brace form (`{0..1}`, `{1}`, `{0..*}`).
fn mult_braces(m: &MultiplicityInterval) -> String {
    let lo = m.lower.unwrap_or(0);
    if m.upper_unbounded {
        return format!("{{{lo}..*}}");
    }
    let hi = m.upper.unwrap_or(lo);
    if lo == hi {
        format!("{{{lo}}}")
    } else {
        format!("{{{lo}..{hi}}}")
    }
}

/// A `CARDINALITY` in brace form, with the `unordered`/`unique` modifiers.
fn card_braces(c: &Cardinality) -> String {
    let inner = {
        let m = &c.interval;
        let lo = m.lower.unwrap_or(0);
        if m.upper_unbounded {
            format!("{lo}..*")
        } else {
            let hi = m.upper.unwrap_or(lo);
            if lo == hi {
                format!("{lo}")
            } else {
                format!("{lo}..{hi}")
            }
        }
    };
    let mut s = format!("{{{inner}");
    if !c.is_ordered {
        s.push_str("; unordered");
    }
    if c.is_unique {
        s.push_str("; unique");
    }
    s.push('}');
    s
}

/// The `before[id…]` / `after[id…]` sibling-order prefix of an object head.
fn sibling_str(so: &SiblingOrder) -> String {
    let kw = if so.is_before { "before" } else { "after" };
    format!("{kw}[{}]", so.sibling_node_id)
}

/// The `(rm_type_name, node_id)` pair of a primitive `C_OBJECT` variant
/// (empty for every non-primitive variant).
fn prim_type_and_node(obj: &CObject) -> (String, String) {
    match obj {
        CObject::CBoolean(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CDate(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CDateTime(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CDuration(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CInteger(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CReal(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CString(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CTerminologyCode(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        CObject::CTime(c) => (c.rm_type_name.clone(), c.node_id.clone()),
        _ => (String::new(), String::new()),
    }
}

/// The inline value text of a `C_STRING` constraint: the single delimited
/// regex (`/re/`, `^re^`) or the quoted literal list, with the `; "assumed"`
/// suffix when one is carried (`AOM2/master04.5` §`C_STRING`).
pub(super) fn cstring_inline(c: &CString) -> String {
    let mut s = match regex_of(c.constraint.as_deref().unwrap_or_default()) {
        Some(regex) => regex.to_owned(),
        None => c
            .constraint
            .iter()
            .flatten()
            .map(|v| quoted(v))
            .collect::<Vec<_>>()
            .join(", "),
    };
    if let Some(a) = &c.assumed_value {
        let _ = write!(s, "; {}", quoted(a));
    }
    s
}

/// The inline value text of a primitive constraint (`55`, `|0..100|`,
/// `"x", "y"; "z"`, `yyyy-mm-??`, `[ac1]`, …) — the body a `matches { … }`
/// wraps. Mirrors `cadl2_primitives.g4`.
pub(super) fn primitive_inline(prim: &CPrimitiveObject) -> String {
    match prim {
        CPrimitiveObject::CBoolean(c) => {
            let mut s = c
                .constraint
                .iter()
                .flatten()
                .map(|b| bool_str(*b))
                .collect::<Vec<_>>()
                .join(", ");
            if let Some(a) = c.assumed_value {
                let _ = write!(s, "; {}", bool_str(a));
            }
            s
        }
        CPrimitiveObject::CString(c) => cstring_inline(c),
        CPrimitiveObject::CInteger(c) => {
            let mut s = int_list(c.constraint.as_deref().unwrap_or_default());
            if let Some(a) = c.assumed_value {
                // The integer assumed value is stored as a whole `f64`
                // (`C_INTEGER.assumed_value`); `Display` renders it without a
                // decimal point so it re-lexes as an integer.
                let _ = write!(s, "; {a}");
            }
            s
        }
        CPrimitiveObject::CReal(c) => {
            let mut s = real_list(c.constraint.as_deref().unwrap_or_default());
            if let Some(a) = c.assumed_value {
                let _ = write!(s, "; {}", real_str(a));
            }
            s
        }
        CPrimitiveObject::CDate(c) => temporal(
            c.pattern_constraint.as_deref(),
            c.constraint.as_deref().unwrap_or_default(),
            c.assumed_value.as_ref().map(|v| v.value.as_str()),
        ),
        CPrimitiveObject::CTime(c) => temporal(
            c.pattern_constraint.as_deref(),
            c.constraint.as_deref().unwrap_or_default(),
            c.assumed_value.as_ref().map(|v| v.value.as_str()),
        ),
        CPrimitiveObject::CDateTime(c) => temporal(
            c.pattern_constraint.as_deref(),
            c.constraint.as_deref().unwrap_or_default(),
            c.assumed_value.as_ref().map(|v| v.value.as_str()),
        ),
        CPrimitiveObject::CDuration(c) => temporal(
            c.pattern_constraint.as_deref(),
            c.constraint.as_deref().unwrap_or_default(),
            c.assumed_value.as_ref().map(|v| v.value.as_str()),
        ),
        CPrimitiveObject::CTerminologyCode(c) => terminology_code_inline(c),
    }
}

/// The inline `[ac…]` / `required [at…; at…]` body of a `C_TERMINOLOGY_CODE`.
fn terminology_code_inline(c: &CTerminologyCode) -> String {
    // A fully unconstrained terminology code renders as nothing — the caller
    // prints the bare (any-allowed) node instead of an unparseable `{[]}`.
    if c.constraint.is_empty() && c.assumed_value.is_none() && c.constraint_status.is_none() {
        return String::new();
    }
    let mut s = String::new();
    if let Some(status) = &c.constraint_status {
        s.push_str(strength_keyword(*status));
        s.push(' ');
    }
    let _ = write!(s, "[{}]", c.constraint);
    if let Some(assumed) = &c.assumed_value {
        // A `[ac…; at…]` assumed value re-uses the bracket form.
        let inner = format!("{}; {}", c.constraint, assumed.code_string);
        s = String::new();
        if let Some(status) = &c.constraint_status {
            s.push_str(strength_keyword(*status));
            s.push(' ');
        }
        let _ = write!(s, "[{inner}]");
    }
    s
}

/// The binding-strength keyword of a `CONSTRAINT_STATUS`.
fn strength_keyword(status: ConstraintStatus) -> &'static str {
    match status {
        ConstraintStatus::Extensible => "extensible",
        ConstraintStatus::Preferred => "preferred",
        ConstraintStatus::Example => "example",
        ConstraintStatus::Required | ConstraintStatus::Other(_) => "required",
    }
}

/// A temporal primitive body: a constraint pattern, an interval list, or the
/// mixed `pattern/|interval|` form, with the optional `; assumed` tail.
fn temporal(
    pattern: Option<&str>,
    constraint: &[Interval<impl TemporalValue>],
    assumed: Option<&str>,
) -> String {
    let mut s = String::new();
    match pattern {
        Some(p) => {
            s.push_str(p);
            // `pattern/interval` mixed form (`PWD/|P0W..P50W|`).
            if let Some(first) = constraint.first() {
                let _ = write!(s, "/{}", interval_str(first, temporal_str));
            }
        }
        None => {
            s = constraint
                .iter()
                .map(|iv| interval_str(iv, temporal_str))
                .collect::<Vec<_>>()
                .join(", ");
        }
    }
    if let Some(a) = assumed {
        let _ = write!(s, "; {a}");
    }
    s
}

/// A comma-separated `C_INTEGER` constraint list.
fn int_list(constraint: &[Interval<i32>]) -> String {
    constraint
        .iter()
        .map(|iv| interval_str(iv, ToString::to_string))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A comma-separated `C_REAL` constraint list.
fn real_list(constraint: &[Interval<f64>]) -> String {
    constraint
        .iter()
        .map(|iv| interval_str(iv, |v| real_str(*v)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render one `Interval<T>` in cADL form: a point interval as the bare value, a
/// proper interval as `|lo..hi|` with relational-operator prefixes for
/// exclusivity/unboundedness (`master04.5`).
fn interval_str<T: Clone, F: Fn(&T) -> String>(iv: &Interval<T>, f: F) -> String {
    match iv {
        Interval::PointInterval(PointInterval { lower: Some(v), .. }) => f(v),
        Interval::ProperInterval(ProperInterval::ProperInterval(p)) => proper_str(p, &f),
        // An unbounded point interval, or the `MultiplicityInterval` proper
        // variant — the cADL primitive parser produces neither for a value
        // constraint (the latter is the occurrences/cardinality shape, printed
        // separately) — renders as nothing.
        Interval::PointInterval(_)
        | Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => String::new(),
    }
}

/// The `|…|` body of a proper interval, one- or two-sided.
fn proper_str<T, F: Fn(&T) -> String>(
    p: &openehr_base::prelude::ProperIntervalData<T>,
    f: &F,
) -> String {
    match (&p.lower, &p.upper) {
        (Some(lo), Some(hi)) => {
            let lp = if p.lower_included { "" } else { ">" };
            let hp = if p.upper_included { "" } else { "<" };
            format!("|{lp}{}..{hp}{}|", f(lo), f(hi))
        }
        (Some(lo), None) => {
            let op = if p.lower_included { ">=" } else { ">" };
            format!("|{op}{}|", f(lo))
        }
        (None, Some(hi)) => {
            let op = if p.upper_included { "<=" } else { "<" };
            format!("|{op}{}|", f(hi))
        }
        (None, None) => String::new(),
    }
}

/// The cADL spelling of a boolean literal.
pub(super) fn bool_str(b: bool) -> &'static str {
    if b { "True" } else { "False" }
}

/// Format an `f64` so it always re-lexes as a `Real` (a decimal point present).
fn real_str(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

// ── the temporal value trait ──────────────────────────────────────────────

/// A temporal primitive value whose verbatim ISO-8601 text the printer emits.
trait TemporalValue: Clone {
    fn text(&self) -> &str;
}

/// The verbatim ISO-8601 text of a temporal primitive value.
fn temporal_str<T: TemporalValue>(v: &T) -> String {
    v.text().to_owned()
}

impl TemporalValue for openehr_base::prelude::Iso8601Date {
    fn text(&self) -> &str {
        &self.value
    }
}
impl TemporalValue for openehr_base::prelude::Iso8601Time {
    fn text(&self) -> &str {
        &self.value
    }
}
impl TemporalValue for openehr_base::prelude::Iso8601DateTime {
    fn text(&self) -> &str {
        &self.value
    }
}
impl TemporalValue for openehr_base::prelude::Iso8601Duration {
    fn text(&self) -> &str {
        &self.value
    }
}

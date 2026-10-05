// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Value-expression lowering: the design's **path split** for a data leaf, the
//! typed coercions (QUERY master03 §Comparison operators + §ORDER BY), VERSION /
//! EHR metadata fields, ORDER BY, and paging.
//!
//! No openEHR spec governs the extraction mechanics — this is our own design: a leaf whose anchor is empty reads
//! its `data` fragment off the source node alias; a leaf with structure hops
//! reads through a **correlated scalar subquery** walking the anchor chain
//! (interval containment + promoted-column filters per step). The extracted
//! fragment is coerced per the resolved [`Coercion`]; subqueries return the value
//! or `NULL`, so a missing path compares false and never multiplies rows —
//! keeping `OR`/`NOT`/`EXISTS` correct.

use std::fmt::Write as _;

use sea_query::{Alias, Expr, ExprTrait as _, Order, Query};

use crate::aql::error::{AnalysisError, AqlError, SqlError};
use crate::aql::ir::{Coercion, EhrField, LeafPath, OrderKey, PathTarget, Source, VersionField};
use crate::db::iden::{CommitAudit, Ehr, Node, VersionRow};
use crate::storage::promoted::{PROMOTED_LEAVES, PromotedKind};

use super::expr::{as_text, call, cast, col, derived_col, extract_base, order_coercion};
use super::from::is_vo_root_type;
use super::{Builder, ValueMode};

impl Builder<'_> {
    // ── value expressions (the path split) ──────────────────────────────────

    /// Lower an identified path to a value expression, discriminated by what it
    /// addresses (QUERY master03 §Identified Paths / §Identified expression).
    pub(super) fn value_expr(
        &mut self,
        target: &PathTarget,
        mode: ValueMode,
    ) -> Result<Expr, AqlError> {
        match target {
            PathTarget::Data(leaf) => self.data_leaf_expr(leaf, mode),
            PathTarget::Version { source, field } => {
                let voa = self.version_vo.get(&source.0).cloned().ok_or_else(|| {
                    SqlError::Unsupported("VERSION path without a bound version".to_owned())
                })?;
                let system_id = self.ctx.system_id.clone();
                Ok(version_field_expr(
                    &voa,
                    || self.ensure_audit(&voa),
                    *field,
                    &system_id,
                ))
            }
            PathTarget::Ehr { source, field } => {
                let alias = self.ehr_alias.get(&source.0).cloned().ok_or_else(|| {
                    SqlError::Unsupported("EHR path without a bound EHR".to_owned())
                })?;
                Ok(ehr_field_expr(&alias, *field, &self.ctx.system_id))
            }
            // A scalar leaf under `e/ehr_status` (e.g. `.../subject/...`,
            // `.../is_queryable`): join the EHR_STATUS root, then extract from
            // its node subtree via the same path split as any data leaf. RM 1.2.0
            // `EHR.ehr_status` (`docs/specs/openehr/RM/docs/ehr/`).
            PathTarget::EhrStatus(leaf) => {
                self.ensure_ehr_status_root(leaf.source.0)?;
                self.data_leaf_expr(leaf, mode)
            }
        }
    }

    /// The design's path split for a data leaf: empty anchor → read the source
    /// node's `data` fragment inline; non-empty anchor → a correlated scalar
    /// subquery walking the anchor chain and extracting the fragment.
    pub(super) fn data_leaf_expr(
        &mut self,
        leaf: &LeafPath,
        mode: ValueMode,
    ) -> Result<Expr, AqlError> {
        // The server-assigned OBJECT_VERSION_ID (`uid[/value]`) is synthesized
        // from the joined `version`, not stored in the fragment (uid synthesis).
        if let Some(expr) = self.version_uid_expr(leaf, mode) {
            return Ok(expr);
        }
        // A registered promoted leaf reads its indexed `node` column instead of
        // the correlated subtree extraction (fast path).
        if let Some(expr) = self.promoted_leaf_expr(leaf, mode) {
            return Ok(expr);
        }
        let src = self.source_node(leaf.source.0)?;
        let root_conds = leaf
            .root_predicate
            .as_ref()
            .map(|p| self.node_constraint_conds(&src, p))
            .transpose()?;
        let fragment = self.fragment_path(leaf)?;
        let jp = fragment.as_ref().map(|(jp, _)| jp.clone());
        // NOTE: QUERY master03 §Identified Paths is silent on projecting a
        // list-valued fragment path — our own decision: every match as ONE
        // jsonb array cell; scalar contexts keep the first-match extraction.
        let projected_array =
            matches!(mode, ValueMode::Projection) && jp.is_some() && leaf.fragment_multi_valued();
        let extract = |data: Expr| -> Expr {
            let vars = fragment.as_ref().and_then(|(_, v)| v.clone());
            match (&jp, projected_array) {
                (Some(jp), true) => super::expr::jsonb_path_array(data, jp, vars),
                _ => extract_base(data, jp.as_deref(), vars),
            }
        };

        if leaf.anchor.is_empty() {
            let base = extract(col(&src, Node::Data));
            let value = coerce_value(base, mode, leaf);
            // A root predicate guards the inline read: the value only exists
            // where the source node satisfies it (QUERY master03 §Identified
            // Paths — a node predicate qualifies the object the path reads).
            return Ok(match all_of(root_conds) {
                Some(cond) => sea_query::CaseStatement::new().case(cond, value).into(),
                None => value,
            });
        }

        let (mut sub, last) = self.anchored_walk(leaf, &src)?;
        if let Some(conds) = root_conds {
            for cond in conds {
                sub.and_where(cond);
            }
        }
        let base = extract(col(&last, Node::Data));
        sub.expr(coerce_value(base, mode, leaf));
        sub.limit(1);
        Ok(Expr::from(sub))
    }

    /// The anchored containment walk shared by the scalar extraction (above)
    /// and the existential predicate lowering
    /// ([`data_leaf_exists`](Self::data_leaf_exists)): a correlated
    /// `SELECT`-less subquery joining one `node` alias per anchor step, with
    /// the nested-set containment + type/constraint conditions. Returns the
    /// statement (FROM/WHERE only) and the innermost alias.
    fn anchored_walk(
        &mut self,
        leaf: &LeafPath,
        src: &str,
    ) -> Result<(sea_query::SelectStatement, String), AqlError> {
        let mut sub = Query::select();
        let mut prev = src.to_owned();
        let mut last = String::new();
        for step in &leaf.anchor {
            let alias = format!("s{}", self.next_ctr());
            sub.from_as(Node::Table, Alias::new(alias.as_str()));
            sub.and_where(super::expr::hot(&alias, Node::Tier));
            sub.and_where(col(&alias, Node::VoId).eq(col(&prev, Node::VoId)));
            sub.and_where(col(&alias, Node::SysVersion).eq(col(&prev, Node::SysVersion)));
            sub.and_where(
                col(&alias, Node::Num).between(col(&prev, Node::Num), col(&prev, Node::NumCap)),
            );
            if let Some(cond) = super::expr::type_cond(&alias, &step.node_types) {
                sub.and_where(cond);
            }
            if let Some(pred) = &step.predicate {
                for cond in self.node_constraint_conds(&alias, pred)? {
                    sub.and_where(cond);
                }
            }
            prev.clone_from(&alias);
            last = alias;
        }
        Ok((sub, last))
    }

    /// The EXISTENTIAL lowering of a predicate on an anchored data leaf:
    /// `EXISTS (SELECT 1 <walk> WHERE <cond(extracted value)>)` — the
    /// predicate holds when ANY node the path matches satisfies it.
    ///
    /// NOTE: QUERY master03 §WHERE is silent on predicates over multi-valued
    /// paths, so any-match is our own design decision; it is deterministic where
    /// a scalar `LIMIT 1` pick without an ordering is not, and it lets the
    /// planner cost the filter as a semi-join rather than an opaque scalar
    /// subquery.
    ///
    /// The same any-match rule covers a multi-valued fragment tail: a path
    /// crossing a list-valued fragment attribute (`links`, `participations`,
    /// `identifiers`, `mappings`) yields one item per match through the
    /// set-returning `jsonb_path_query` in the subquery's FROM, where the scalar
    /// `jsonb_path_query_first` would see only the first.
    ///
    /// Returns `Ok(None)` when the leaf is a single-valued inline read, uid
    /// synthesis, or a promoted column — the caller falls back to the scalar
    /// comparison, which is exact there.
    pub(super) fn data_leaf_exists(
        &mut self,
        leaf: &LeafPath,
        mode: ValueMode,
        cond: impl FnOnce(Expr) -> Expr,
    ) -> Result<Option<Expr>, AqlError> {
        if self.version_uid_expr(leaf, mode).is_some()
            || self.promoted_leaf_expr(leaf, mode).is_some()
        {
            return Ok(None);
        }
        let fragment = self.fragment_path(leaf)?;
        let multi = fragment.is_some() && leaf.fragment_multi_valued();
        if leaf.anchor.is_empty() && !multi {
            return Ok(None);
        }
        let src = self.source_node(leaf.source.0)?;
        let (mut sub, base) = if leaf.anchor.is_empty() {
            // `multi` holds here, so the fragment jsonpath exists.
            let Some((jp, vars)) = fragment else {
                return Ok(None);
            };
            let mut sub = Query::select();
            let base = fragment_items(&mut sub, col(&src, Node::Data), &jp, vars, self.next_ctr());
            (sub, base)
        } else {
            let (mut sub, last) = self.anchored_walk(leaf, &src)?;
            let base = match (fragment, multi) {
                (Some((jp, vars)), true) => {
                    fragment_items(&mut sub, col(&last, Node::Data), &jp, vars, self.next_ctr())
                }
                (fragment, _) => {
                    let (jp, vars) = fragment.unzip();
                    extract_base(col(&last, Node::Data), jp.as_deref(), vars.flatten())
                }
            };
            (sub, base)
        };
        // The root predicate correlates against the source node: the value
        // only exists where the source satisfies it.
        if let Some(pred) = &leaf.root_predicate {
            for c in self.node_constraint_conds(&src, pred)? {
                sub.and_where(c);
            }
        }
        sub.expr(Expr::val(1));
        sub.and_where(cond(coerce_value(base, mode, leaf)));
        if self.streaming {
            // The EXISTS must stay CORRELATED: as a bare WHERE sublink the
            // planner may pull it up and decorrelate its inner side into a
            // corpus-wide Materialize. Hosting it in a LATERAL behind the
            // `OFFSET 0` fence pins the per-row SubPlan (a `LIMIT 1` inside the
            // sublink is NOT a fence); semantics are one boolean per outer row.
            let probe = format!("p{}", self.next_ctr());
            let mut wrapper = Query::select();
            wrapper.expr_as(Expr::exists(sub), Alias::new("hit"));
            wrapper.offset(0);
            self.q.join_lateral(
                sea_query::JoinType::Join,
                wrapper,
                Alias::new(probe.as_str()),
                Expr::val(true),
            );
            return Ok(Some(derived_col(&probe, "hit")));
        }
        Ok(Some(Expr::exists(sub)))
    }

    /// The server-assigned `OBJECT_VERSION_ID` for a `uid` / `uid/value` path on
    /// a versioned-object-root variable (uid synthesis). The uid is assigned at commit and
    /// is **not** persisted in the canonical fragment (the REST read path injects
    /// it — `service::ehr::meta::with_uid`), so an AQL jsonb extraction finds
    /// nothing; QUERY master03 §Identified paths lists `COMPOSITION.uid.value` as
    /// normative, so it is synthesized here from the already-joined `version`
    /// via the same law as the wire id (RM common master06 §Version
    /// Identification): `vo_id::creating_system_id::version_tree_id`. `None`
    /// (fall through to the stored fragment) for a contained object's own `uid`,
    /// any predicate on the path, or a source with no versioned-object root.
    fn version_uid_expr(&mut self, leaf: &LeafPath, mode: ValueMode) -> Option<Expr> {
        // Only a direct `uid[/value]` — no structure hop, no path predicate.
        if !leaf.anchor.is_empty()
            || leaf.root_predicate.is_some()
            || leaf.fragment.iter().any(|s| s.predicate.is_some())
        {
            return None;
        }
        // The source must be a versioned-object root: only there does the server
        // assign the OBJECT_VERSION_ID (a contained OBSERVATION's `uid` is its
        // own stored value, if any).
        let Some(Source::Rm(r)) = self.ir.sources.get(leaf.source.0) else {
            return None;
        };
        if r.rm_type.is_empty() || !r.rm_type.names().iter().all(|t| is_vo_root_type(t)) {
            return None;
        }
        let voa = self.vo_alias.get(&leaf.source.0).cloned()?;
        let names: Vec<&str> = leaf.fragment.iter().map(|s| s.name.as_str()).collect();
        let system_id = self.ctx.system_id.clone();
        let uid = version_field_expr(
            &voa,
            || self.ensure_audit(&voa),
            VersionField::Uid,
            &system_id,
        );
        match names.as_slice() {
            // `uid/value` → the OBJECT_VERSION_ID string (projected → JSON string
            // via the caller's `to_jsonb`; compared/ordered as text).
            ["uid", "value"] => Some(uid),
            // `uid` → the whole OBJECT_VERSION_ID object, only as a projected
            // cell; a non-projection use falls through (not a normative shape).
            ["uid"] if matches!(mode, ValueMode::Projection) => Some(call(
                "jsonb_build_object",
                vec![
                    Expr::val("_type"),
                    Expr::val("OBJECT_VERSION_ID"),
                    Expr::val("value"),
                    uid,
                ],
            )),
            _ => None,
        }
    }

    /// The promoted-leaf fast path:
    /// when `leaf` addresses a registered promoted leaf on its versioned-object
    /// root and the requested `mode` matches the column's kind, read
    /// `node.<column>` (indexed) instead of lowering the correlated subtree
    /// extraction + coercion. `None` (fall through to the general lowering) for
    /// every non-promoted leaf, a projection / raw-numeric read, any predicate
    /// along the path (a column cannot honor one), or a coercion mismatch. No
    /// openEHR spec governs the lowering — our own design
    /// (`crate::storage::promoted`).
    fn promoted_leaf_expr(&self, leaf: &LeafPath, mode: ValueMode) -> Option<Expr> {
        let ValueMode::Value(coercion) = mode else {
            return None;
        };
        // The source must be an RM versioned-object root bound to a single
        // rm_type, so its node rows are exactly the `num = 0` root the column
        // is populated on.
        let Some(Source::Rm(r)) = self.ir.sources.get(leaf.source.0) else {
            return None;
        };
        if !r.rm_type.is_singleton() {
            return None;
        }
        let rm_type = r.rm_type.names().first()?.as_str();
        // A promoted column carries no predicate context, so any predicate on the
        // path rules the substitution out.
        if leaf.root_predicate.is_some()
            || leaf.anchor.iter().any(|s| s.predicate.is_some())
            || leaf.fragment.iter().any(|s| s.predicate.is_some())
        {
            return None;
        }
        let path: Vec<&str> = leaf
            .anchor
            .iter()
            .map(|s| s.attribute.as_str())
            .chain(leaf.fragment.iter().map(|s| s.name.as_str()))
            .collect();
        let entry = PROMOTED_LEAVES
            .iter()
            .find(|p| p.rm_type == rm_type && p.path == path.as_slice())?;
        let compatible = match entry.kind {
            PromotedKind::Timestamp => coercion == Coercion::Temporal,
        };
        if !compatible {
            return None;
        }
        let alias = self.node_alias.get(&leaf.source.0)?;
        Some(col(alias, entry.column))
    }

    /// The leaf's fragment jsonpath plus the bound filter variables of any
    /// predicated fragment step, or `None` when the leaf addresses the whole
    /// anchor node.
    ///
    /// A step predicate lowers to a jsonpath filter expression on the step's
    /// member accessor (`$.links ? (@.archetype_node_id == $p0).target.value`
    /// — QUERY master03 §Node predicate / §Standard predicate); every
    /// compared value travels in the `vars` jsonb PARAMETER (`$p0`, `$p1`, …),
    /// never spliced into the path text, and member names are lexer-restricted
    /// identifiers. An archetype constraint on a fragment step is plain
    /// equality on `archetype_node_id` — fragment objects are not `node` rows,
    /// so the subsumption columns do not exist there; an HRID compares as the
    /// written string.
    #[expect(
        clippy::disallowed_types,
        reason = "the jsonpath vars object is SQL/JSON wire material (the bound \
                  third argument of jsonb_path_query*), not an RM value — no \
                  generated type models it"
    )]
    fn fragment_path(&self, leaf: &LeafPath) -> Result<Option<(String, Option<Expr>)>, AqlError> {
        if leaf.fragment.is_empty() {
            return Ok(None);
        }
        let mut jp = String::from("$");
        let mut vars = serde_json::Map::new();
        let var = |v: serde_json::Value, vars: &mut serde_json::Map<String, serde_json::Value>| {
            let name = format!("p{}", vars.len());
            vars.insert(name.clone(), v);
            name
        };
        for step in &leaf.fragment {
            let _ = write!(jp, ".{}", step.name);
            let Some(pred) = &step.predicate else {
                continue;
            };
            let mut conds: Vec<String> = Vec::new();
            if let Some(a) = &pred.archetype {
                let value = match a {
                    crate::aql::ir::ArchetypeConstraint::NodeCode(c)
                    | crate::aql::ir::ArchetypeConstraint::Archetype(c) => c.clone(),
                    crate::aql::ir::ArchetypeConstraint::Param(p) => self.param_str(p)?,
                };
                let v = var(serde_json::Value::String(value), &mut vars);
                conds.push(format!("@.archetype_node_id == ${v}"));
            }
            if let Some(n) = &pred.name {
                match n {
                    crate::aql::ir::NameConstraint::Value(s) => {
                        let v = var(serde_json::Value::String(s.clone()), &mut vars);
                        conds.push(format!("@.name.value == ${v}"));
                    }
                    crate::aql::ir::NameConstraint::Param(p) => {
                        let v = var(serde_json::Value::String(self.param_str(p)?), &mut vars);
                        conds.push(format!("@.name.value == ${v}"));
                    }
                    crate::aql::ir::NameConstraint::TermCode { terminology, code } => {
                        let c = var(serde_json::Value::String(code.clone()), &mut vars);
                        conds.push(format!("@.name.defining_code.code_string == ${c}"));
                        let t = var(serde_json::Value::String(terminology.clone()), &mut vars);
                        conds.push(format!("@.name.defining_code.terminology_id.value == ${t}"));
                    }
                }
            }
            for sp in &pred.standard {
                let value = jsonpath_scalar(self.bind_value(&sp.value)?)?;
                let v = var(value, &mut vars);
                let mut lhs = String::from("@");
                for part in &sp.path {
                    let _ = write!(lhs, ".{part}");
                }
                conds.push(format!("{lhs} {} ${v}", jsonpath_op(sp.op)));
            }
            if !conds.is_empty() {
                let _ = write!(jp, " ? ({})", conds.join(" && "));
            }
        }
        let vars_expr = if vars.is_empty() {
            None
        } else {
            Some(cast(
                Expr::val(serde_json::Value::Object(vars).to_string()),
                "jsonb",
            ))
        };
        Ok(Some((jp, vars_expr)))
    }

    /// Resolve a leaf's source to a node alias present in the FROM.
    pub(super) fn source_node(&self, sid: usize) -> Result<String, AqlError> {
        self.node_alias.get(&sid).cloned().ok_or_else(|| {
            SqlError::Unsupported("data path on a non-node source".to_owned()).into()
        })
    }

    // ── ORDER BY / paging ─────────────────────────────────────────────────────

    /// ORDER BY (QUERY master03 §ORDER BY): multi-key, ASC/DESC, Ordered types
    /// compared by ordered-magnitude via the key's own [`Coercion`].
    pub(super) fn build_order_by(&mut self) -> Result<(), AqlError> {
        for key in self.ir.order_by.clone() {
            let OrderKey { path, ascending } = key;
            let order = if ascending { Order::Asc } else { Order::Desc };
            // PostgreSQL requires every ORDER BY expression of a SELECT
            // DISTINCT to appear in the select list, so a selected sort key
            // orders by its output column and an unselected one is a typed
            // reject — QUERY master03 §DISTINCT defines no semantics for it.
            if self.ir.distinct {
                let selected = self.ir.select.iter().position(
                    |c| matches!(&c.value, crate::aql::ir::SelectValue::Path(p) if *p == path),
                );
                match selected {
                    Some(i) => {
                        self.q.order_by(Alias::new(format!("col{i}")), order);
                        continue;
                    }
                    None => {
                        return Err(AnalysisError::DistinctOrderByUnselected.into());
                    }
                }
            }
            // ORDER BY `e/ehr_id[/value]` sorts by the raw `ehr.id` column: a
            // UUID's canonical text form is fixed-length lowercase hex (BASE
            // base_types master05 §Basic Types — Uuid), so text and binary
            // order coincide and the index serves the sort without a per-row
            // cast. The projection path keeps the text cast.
            if let PathTarget::Ehr {
                source,
                field: EhrField::EhrId | EhrField::Whole,
            } = &path
                && let Some(alias) = self.ehr_alias.get(&source.0).cloned()
            {
                self.q.order_by_expr(col(&alias, Ehr::Id), order);
                continue;
            }
            let coercion = order_coercion(&path);
            let expr = self.value_expr(&path, ValueMode::Value(coercion))?;
            self.q.order_by_expr(expr, order);
        }
        Ok(())
    }

    /// LIMIT/OFFSET (QUERY master03 §LIMIT): the effective window is pre-composed
    /// by the service (AQL clause vs REST `fetch`/`offset`); bounds were checked
    /// at lowering.
    pub(super) fn build_paging(&mut self) {
        if let Some(limit) = self.ctx.limit
            && limit >= 0
        {
            self.q.limit(u64::try_from(limit).unwrap_or(u64::MAX));
        }
        if let Some(offset) = self.ctx.offset
            && offset > 0
        {
            self.q.offset(u64::try_from(offset).unwrap_or(0));
        }
    }
}

// ── coercions ─────────────────────────────────────────────────────────────────

/// Apply the value coercion to a jsonb extraction base (QUERY master03
/// §Comparison operators; `DV_ORDERED` ordered-magnitude via `ext.openehr_magnitude`).
pub(super) fn coerce_value(base: Expr, mode: ValueMode, leaf: &LeafPath) -> Expr {
    match mode {
        ValueMode::Projection => base,
        ValueMode::Value(Coercion::Magnitude) => {
            if leaf.types.names().iter().any(|t| t.starts_with("DV_")) {
                call("openehr_magnitude", vec![base])
            } else {
                cast(as_text(base), "numeric")
            }
        }
        ValueMode::Value(Coercion::Boolean) => cast(as_text(base), "boolean"),
        // NOTE: RM data_types master07 §Partial Date/Times admits reduced
        // precision, so the leaf reads through the total `ext.openehr_timestamp`
        // — floor completion, NULL for garbage.
        ValueMode::Value(Coercion::Temporal) => call("openehr_timestamp", vec![as_text(base)]),
        ValueMode::Value(Coercion::Text | Coercion::Raw) => as_text(base),
        // a mixed-type (`Raw`) leaf being compared/matched against a
        // numeric literal — extract numerically, but guard on the stored jsonb
        // type so a non-number occurrence yields NULL (comparison false) instead
        // of a cast error. "numeric for numbers, text otherwise" (QUERY master03
        // §Comparison operators).
        ValueMode::RawNumeric => raw_numeric(base),
    }
}

/// Cast a bound right-hand-side value to match the comparison coercion
/// (QUERY master03 §Comparison operators).
///
/// A temporal bound routes through the same total `ext.openehr_timestamp` the
/// leaf side uses (floor completion for reduced precision), after a plan-time
/// shape check: a bound that is no ISO-8601 date/time at all is the CALLER's
/// defect and refuses as a typed error — never a silent empty result and
/// never a driver error surfacing as a 500.
///
/// # Errors
/// [`SqlError::UncoercibleTemporal`] when a temporal bound is not an ISO-8601
/// date, time, or date-time (reduced precision is accepted).
pub(super) fn coerce_rhs(value: sea_query::Value, coercion: Coercion) -> Result<Expr, AqlError> {
    Ok(match coercion {
        Coercion::Magnitude => cast(Expr::val(value), "numeric"),
        Coercion::Boolean => cast(Expr::val(value), "boolean"),
        Coercion::Temporal => {
            if let sea_query::Value::String(Some(s)) = &value
                && !is_iso_temporal(s)
            {
                return Err(SqlError::UncoercibleTemporal.into());
            }
            call("openehr_timestamp", vec![cast(Expr::val(value), "text")])
        }
        Coercion::Text | Coercion::Raw => cast(Expr::val(value), "text"),
    })
}

/// Whether `s` is an ISO-8601 date, time, or date-time literal in the shapes
/// the temporal floor accepts — reduced precision and compact forms included
/// (BASE `foundation_types` master06 §Time Types). Purely a plan-time shape
/// gate: the SQL-side completion is the semantics.
fn is_iso_temporal(s: &str) -> bool {
    static ISO_TEMPORAL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        // The separator set includes the space form PostgreSQL's own parser
        // accepts.
        let time = r"\d{2}(:\d{2}(:\d{2}([.,]\d+)?)?|\d{2}(\d{2}([.,]\d+)?)?)?";
        let offset = r"([Zz]|[+-]\d{2}(:?\d{2})?)?";
        let date = r"\d{4}(-\d{2}(-\d{2})?|\d{2}(\d{2})?)?";
        #[expect(
            clippy::expect_used,
            reason = "a hardcoded regex literal should always compile; covered by \
                      the temporal planner tests"
        )]
        regex::Regex::new(&format!(
            "^({date}([Tt ]{time}{offset})?|[Tt]?{time}{offset})$"
        ))
        .expect("hardcoded ISO-8601 shape regex should be valid")
    });
    ISO_TEMPORAL.is_match(s)
}

/// Guarded numeric extraction for a mixed-type (`Raw`) leaf: number-typed jsonb
/// occurrences extract as `numeric`; every non-number occurrence yields `NULL`,
/// so a numeric comparison against them is false — never a cast error and never a
/// silent lexical miscompare (QUERY master03 §Comparison operators). The `$1`/`$2`
/// placeholders both carry the same extraction expression.
fn raw_numeric(base: Expr) -> Expr {
    Expr::cust_with_exprs(
        "CASE WHEN jsonb_typeof($1) = 'number' THEN ($2 #>> '{}'::text[])::numeric END",
        [base.clone(), base],
    )
}

// ── VERSION / EHR fields ────────────────────────────────────────────────────

/// The typed SQL for a VERSION metadata field, off the `version`/`audit`
/// aliases. The `uid` is synthesized as
/// `vo_id::creating_system_id::version_tree_id` from the STORED per-version
/// identity columns (never the live config `system_id` — the creating system
/// is immutable per version, RM common master06 §Distributed versioning) via
/// the typed `PgExpr::concatenate` `||` operator; the tree id renders
/// `trunk[.branch.version]`.
///
/// `audit` summons the audit-table join LAZILY: only the commit-audit
/// fields read it, so a `uid`/`contribution_id`/`lifecycle_state` path
/// never pays the join (the same pay-per-use rule as `ensure_audit`'s
/// doc — the first live plan autopsy found the ward query joining `audit`
/// it never projected).
pub(super) fn version_field_expr(
    voa: &str,
    audit: impl FnOnce() -> String,
    field: VersionField,
    _system_id: &str,
) -> Expr {
    let concat = |parts: Vec<Expr>| -> Expr {
        parts
            .into_iter()
            .reduce(sea_query::extension::postgres::PgExpr::concatenate)
            .unwrap_or_else(|| Expr::val(""))
    };
    match field {
        VersionField::Uid => concat(vec![
            cast(col(voa, VersionRow::VoId), "text"),
            Expr::val("::"),
            col(voa, VersionRow::CreatingSystemId),
            Expr::val("::"),
            cast(col(voa, VersionRow::TrunkVersion), "text"),
            Expr::cust_with_exprs(
                "CASE WHEN $1 > 0 THEN '.' || $2 || '.' || $3 ELSE '' END",
                [
                    col(voa, VersionRow::BranchNumber),
                    cast(col(voa, VersionRow::BranchNumber), "text"),
                    cast(col(voa, VersionRow::BranchVersion), "text"),
                ],
            ),
        ]),
        VersionField::TimeCommitted => col(&audit(), CommitAudit::TimeCommitted),
        VersionField::SystemId => col(&audit(), CommitAudit::SystemId),
        VersionField::ChangeType => col(&audit(), CommitAudit::ChangeType),
        // The rubric renders from the openEHR terminology group at SQL-build
        // time — the bundle is the authority, never a hardcoded rubric.
        VersionField::ChangeTypeRubric => {
            coded_rubric_case(&col(&audit(), CommitAudit::ChangeType), "audit_change_type")
        }
        VersionField::Committer => col(&audit(), CommitAudit::Committer),
        // The stored description is the whole canonical DV_TEXT fragment, so
        // the addressed representation is a jsonb extraction: the bare
        // attribute is the object (as `committer` is), each scalar sub-path a
        // `->>` text read that is NULL when the description is uncoded.
        VersionField::Description => col(&audit(), CommitAudit::Description),
        VersionField::DescriptionValue => sea_query::extension::postgres::PgExpr::cast_json_field(
            col(&audit(), CommitAudit::Description),
            Expr::val("value"),
        ),
        VersionField::DescriptionCode => sea_query::extension::postgres::PgExpr::cast_json_field(
            sea_query::extension::postgres::PgExpr::get_json_field(
                col(&audit(), CommitAudit::Description),
                Expr::val("defining_code"),
            ),
            Expr::val("code_string"),
        ),
        VersionField::DescriptionTerminology => {
            sea_query::extension::postgres::PgExpr::cast_json_field(
                sea_query::extension::postgres::PgExpr::get_json_field(
                    sea_query::extension::postgres::PgExpr::get_json_field(
                        col(&audit(), CommitAudit::Description),
                        Expr::val("defining_code"),
                    ),
                    Expr::val("terminology_id"),
                ),
                Expr::val("value"),
            )
        }
        VersionField::ContributionId => cast(col(voa, VersionRow::ContributionId), "text"),
        VersionField::LifecycleState => col(voa, VersionRow::LifecycleState),
        VersionField::LifecycleStateRubric => coded_rubric_case(
            &col(voa, VersionRow::LifecycleState),
            "version_lifecycle_state",
        ),
        // Both coded version fields belong to the constant `openehr`
        // terminology.
        VersionField::ChangeTypeTerminology | VersionField::LifecycleStateTerminology => {
            Expr::val("openehr")
        }
    }
}

/// `CASE <code_col> WHEN '<code>' THEN '<rubric>' … END` over an openEHR
/// terminology group, generated from the vendored TERM bundle (TERM 3.1.0
/// `openehr_terminology.xml`) — the coded version fields store the numeric
/// group code; their `…/value` sub-path compares against the rubric.
fn coded_rubric_case(code_col: &Expr, group_id: &str) -> Expr {
    let mut case = sea_query::CaseStatement::new();
    for concept in openehr_term::bundle::openehr().concepts_in_group(group_id) {
        case = case.case(
            code_col.clone().eq(Expr::val(concept.id.clone())),
            Expr::val(concept.rubric.clone()),
        );
    }
    case.into()
}

/// The typed SQL for an EHR attribute (`ehr_id/value`, `time_created`,
/// `system_id`). RM 1.2.0 `EHR` (`docs/specs/openehr/RM/docs/ehr/`).
pub(super) fn ehr_field_expr(alias: &str, field: EhrField, system_id: &str) -> Expr {
    match field {
        EhrField::EhrId | EhrField::Whole => cast(col(alias, Ehr::Id), "text"),
        EhrField::TimeCreated => col(alias, Ehr::TimeCreated),
        EhrField::SystemId => Expr::val(system_id.to_owned()),
    }
}

/// AND-combine a lowered condition list, `None` for an absent/empty list.
fn all_of(conds: Option<Vec<Expr>>) -> Option<Expr> {
    conds.and_then(|c| c.into_iter().reduce(sea_query::ExprTrait::and))
}

/// Adds the set-returning `jsonb_path_query(<data>, '<jp>'::jsonpath[, vars])`
/// to the subquery's FROM and returns the per-match item reference.
///
/// The call is implicitly LATERAL (a FROM function may reference columns of
/// preceding FROM items — PostgreSQL docs §7.2.1.5 LATERAL Subqueries), and
/// for a scalar-returning function the table alias doubles as the column name
/// (§7.2.1.4 Table Functions).
fn fragment_items(
    sub: &mut sea_query::SelectStatement,
    data: Expr,
    jp: &str,
    vars: Option<Expr>,
    n: usize,
) -> Expr {
    let alias = format!("f{n}");
    let mut func = sea_query::Func::cust(Alias::new("jsonb_path_query"))
        .arg(data)
        .arg(cast(Expr::val(jp.to_owned()), "jsonpath"));
    if let Some(vars) = vars {
        func = func.arg(vars);
    }
    sub.from_function(func, Alias::new(alias.as_str()));
    derived_col(&alias, &alias)
}

// ── jsonpaths ───────────────────────────────────────────────────────────────

/// The jsonpath filter-expression operator for an AQL comparison operator
/// (PostgreSQL docs §9.16.2 — the SQL/JSON path comparison operators).
fn jsonpath_op(op: openehr_query::lexer::CompOp) -> &'static str {
    match op {
        openehr_query::lexer::CompOp::Eq => "==",
        openehr_query::lexer::CompOp::Ne => "!=",
        openehr_query::lexer::CompOp::Lt => "<",
        openehr_query::lexer::CompOp::Le => "<=",
        openehr_query::lexer::CompOp::Gt => ">",
        openehr_query::lexer::CompOp::Ge => ">=",
    }
}

/// A bound predicate value as the JSON scalar a jsonpath `vars` object
/// carries. A non-scalar bind cannot appear in a filter comparison and is a
/// typed reject.
#[expect(
    clippy::disallowed_types,
    reason = "the jsonpath vars object is SQL/JSON wire material (the bound \
              third argument of jsonb_path_query*), not an RM value — no \
              generated type models it"
)]
fn jsonpath_scalar(value: sea_query::Value) -> Result<serde_json::Value, AqlError> {
    Ok(match value {
        sea_query::Value::String(Some(s)) => serde_json::Value::String(s),
        sea_query::Value::Bool(Some(b)) => serde_json::Value::Bool(b),
        sea_query::Value::BigInt(Some(i)) => serde_json::Value::from(i),
        sea_query::Value::Int(Some(i)) => serde_json::Value::from(i),
        sea_query::Value::Double(Some(f)) => serde_json::Value::from(f),
        other => {
            return Err(SqlError::Unsupported(format!(
                "a fragment-step predicate compares against a non-scalar value ({other:?})"
            ))
            .into());
        }
    })
}

/// Build a jsonpath from a relative object path (`[a, b]` → `$.a.b`) — a node
/// standard-predicate sub-path (QUERY master03 §Standard predicate).
pub(super) fn jsonpath(parts: &[String]) -> String {
    let mut jp = String::from("$");
    for p in parts {
        let _ = write!(jp, ".{p}");
    }
    jp
}

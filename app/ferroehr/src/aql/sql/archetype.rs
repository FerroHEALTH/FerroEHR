// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `archetype_node_id` predicate: which stored nodes an archetype id in a
//! `FROM`/`CONTAINS` constraint selects (QUERY master03 §Archetype predicate,
//! re-grounded on BASE and AM subsumption, see [`archetype_predicate`]).

use std::collections::{BTreeMap, BTreeSet};

use sea_query::extension::postgres::PgFunc;
use sea_query::{Expr, ExprTrait as _};

use super::expr::{call, cast, col};
use crate::aql::lineage::{ArchetypeKey, ArchetypeLineage, ReferenceForm, decompose_hrid};
use crate::db::iden::Node;

/// The minor version of a node's stored `archetype` id: the middle number of a
/// three-part `.vM.m.p` release, a version-status suffix (`-rc.1`) allowed after
/// it. A legacy major-only id, or any id without three parts, has none.
const STORED_MINOR: &str = r"\.v[0-9]+\.([0-9]+)\.[0-9]+(-[^:]*)?$";

/// The patch version of a node's stored `archetype` id, read like
/// [`STORED_MINOR`].
const STORED_PATCH: &str = r"\.v[0-9]+\.[0-9]+\.([0-9]+)(-[^:]*)?$";

/// Builds the `archetype_node_id` predicate condition for a node alias.
///
/// When `value` parses as an archetype id (BASE `base_types` master05
/// §Archetype Identifiers, or the AOM2 physical form), the predicate implements
/// query subsumption: "the data created with any specialised archetype will
/// always be matched by queries based on the parent archetype" (BASE
/// `architecture_overview` master10 §Design-time Relationships), with AM
/// master07 §Supporting Archetype-based Querying making the matching set for a
/// query naming X be X, its older minor and patch variants, its specialisation
/// parents and their older variants, bounded to one major version.
///
/// The queried identity matches data of its own namespace only, an id without
/// a namespace matching only data without one (BASE `artefact_identification`
/// master07 §Referencing: "A namespace is never a wildcard"), and only data
/// whose release is at or after the version the query names: an `ihrid_ref`
/// (`.v1`) every release of the major, an `sihrid_ref` (`.v1.2`) a minor of 2 or
/// more, a `phrid_ref` (`.v1.2.3`) a minor and patch of `(2, 3)` or more. A
/// legacy major-only data id is no abbreviation of a three-part release, so
/// only an `ihrid_ref` matches it.
///
/// Specialisations join the set from two sources. The first is the ADL 1.4
/// naming convention, for an ADL 1.4 id only (no namespace, a major-only
/// version): a child extends the parent's `domain_concept` with a
/// `-`-separated segment, so the parent matches every `arch_concept` that
/// begins with `concept-`, served by `idx_node_arch_subsume`. The second is
/// `lineage`, the stored specialisation graph ([`ArchetypeLineage`]), since
/// "for specialised archetypes, the specialisation lineage can only be obtained
/// from the operational form of the archetype, found in the template used to
/// create the data" (BASE Artefact Identification, Referencing). Every stored descendant
/// joins on its own namespace, entity, major and concept, every release of it.
///
/// For at- and id-codes, arbitrary strings and params resolving to non-HRIDs the
/// predicate is the case-folded equality on `archetype`.
//
// The `-`-prefix rule is exact for an ADL 1.4-form id (lineage encoded in the
// concept) and is applied ONLY to that form: BASE Artefact Identification
// (Source Artefact Identification §Legacy ADL 1.4 Semantics) strips the
// separator of meaning for AOM2-era ids.
// NOTE: QUERY master03 §Archetype predicate equates the predicate to
// `archetype_node_id` string equality; we implement BASE/AM subsumption instead,
// since a parent archetype must retrieve its children (master10).
pub(super) fn archetype_predicate(node: &str, value: &str, lineage: &ArchetypeLineage) -> Expr {
    // Both eras' identifier forms are read here, through the SAME decomposition
    // the lineage index is built with: a form one side accepts and the other
    // declines would silently narrow the answer instead of erroring.
    let Some(queried) = decompose_hrid(value) else {
        return archetype_equality(node, value);
    };
    let key = &queried.key;

    let mut own = col(node, Node::ArchConcept).eq(Expr::val(key.concept.clone()));
    if let Some(release) = release_at_or_after(node, queried.form) {
        own = own.and(release);
    }
    if queried.legacy_form {
        let child_prefix = format!("{}-%", like_escape(&key.concept));
        own = own.or(col(node, Node::ArchConcept).like(child_prefix));
    }
    let mut cond = interface(node, key).and(own);

    // The stored descendants, grouped by the interface boundary each member is
    // scoped to. `BTreeMap`/`BTreeSet` keep the emitted SQL byte-deterministic.
    let mut groups: BTreeMap<ArchetypeKey, BTreeSet<String>> = BTreeMap::new();
    for descendant in lineage.descendants(key, queried.form) {
        let concept = descendant.concept.clone();
        let boundary = ArchetypeKey {
            concept: String::new(),
            ..descendant
        };
        groups.entry(boundary).or_default().insert(concept);
    }
    for (boundary, concepts) in groups {
        cond = cond.or(interface(node, &boundary).and(concept_match(node, &concepts)));
    }
    cond
}

/// The interface boundary of `key`: its namespace, its entity and its major.
fn interface(node: &str, key: &ArchetypeKey) -> Expr {
    namespace_match(node, key.namespace.as_deref())
        .and(col(node, Node::ArchEntity).eq(Expr::val(key.entity.clone())))
        .and(col(node, Node::ArchMajor).eq(Expr::val(key.major)))
}

/// The stored `archetype` id carries exactly `namespace`, or none when it is
/// `None`. The column is lowercased at write, like the key.
fn namespace_match(node: &str, namespace: Option<&str>) -> Expr {
    match namespace {
        Some(ns) => col(node, Node::Archetype).like(format!("{}::%", like_escape(ns))),
        None => col(node, Node::Archetype).not_like("%::%"),
    }
}

/// The stored release is at or after the one `form` names, within the major
/// the interface boundary already fixes; `None` for an `ihrid_ref`, which
/// names every release.
fn release_at_or_after(node: &str, form: ReferenceForm) -> Option<Expr> {
    match form {
        ReferenceForm::Interface => None,
        ReferenceForm::SpecificInterface { minor } => {
            Some(stored_part(node, STORED_MINOR).gte(Expr::val(minor)))
        }
        ReferenceForm::Physical { minor, patch } => Some(
            stored_part(node, STORED_MINOR)
                .gt(Expr::val(minor))
                .or(stored_part(node, STORED_MINOR)
                    .eq(Expr::val(minor))
                    .and(stored_part(node, STORED_PATCH).gte(Expr::val(patch)))),
        ),
    }
}

/// One version number of the stored `archetype` id, `pattern`'s capture group
/// (`substring(text, pattern)`,
/// <https://www.postgresql.org/docs/18/functions-matching.html>), as `numeric`
/// so no stored number overflows the cast; NULL where the id has no such part.
fn stored_part(node: &str, pattern: &'static str) -> Expr {
    cast(
        call(
            "substring",
            vec![col(node, Node::Archetype), Expr::val(pattern)],
        ),
        "numeric",
    )
}

/// `arch_concept` against a matching set: plain equality for the single-member
/// set, else `= ANY(ARRAY[…])`, the form `PostgreSQL` normalizes an `OR`/`IN`
/// list to anyway
/// (<https://www.postgresql.org/docs/18/functions-comparisons.html>). Both
/// shapes are index-servable by `idx_node_arch_subsume`.
fn concept_match(node: &str, concepts: &BTreeSet<String>) -> Expr {
    let members: Vec<String> = concepts.iter().cloned().collect();
    match members.as_slice() {
        [only] => col(node, Node::ArchConcept).eq(Expr::val(only.clone())),
        _ => col(node, Node::ArchConcept).eq(Expr::from(PgFunc::any(Expr::val(members)))),
    }
}

/// Case-insensitive equality on the `archetype` column (the non-HRID
/// fallback: at/id-codes and arbitrary strings). The column is case-folded
/// at write (`storage::codec`, BASE `base_types` master05 §"Composite
/// Identifiers and Case"), so folding the BIND VALUE alone yields plain
/// indexed column equality with honest planner statistics.
fn archetype_equality(node: &str, value: &str) -> Expr {
    col(node, Node::Archetype).eq(Expr::val(value.to_ascii_lowercase()))
}

/// Escapes the SQL `LIKE` metacharacters (`%`, `_`, `\`) in a literal prefix.
/// The default `PostgreSQL` `LIKE` escape character `\` matches.
fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

// ── test-only inline renderer ───────────────────────────────────────────────

/// Renders the `archetype_node_id` predicate for `value` to inline `PostgreSQL`
/// text (values folded in, not parameterized) so tests can assert the emitted
/// condition.
#[cfg(test)]
fn archetype_predicate_sql(value: &str, lineage: &ArchetypeLineage) -> String {
    use sea_query::{PostgresQueryBuilder, Query};

    let cond = archetype_predicate("n", value, lineage);
    let mut q = Query::select();
    q.expr(Expr::val(1)).and_where(cond);
    q.to_string(PostgresQueryBuilder)
}

#[cfg(test)]
mod tests {
    use super::{ArchetypeLineage, archetype_predicate_sql};

    /// The predicate as rendered with no stored ADL2/OPT2 family.
    fn predicate_sql(value: &str) -> String {
        archetype_predicate_sql(value, &ArchetypeLineage::default())
    }

    const NO_NAMESPACE: &str = r#""n"."archetype" NOT LIKE '%::%'"#;
    const MINOR: &str = r#"CAST(substring("n"."archetype", E'\\.v[0-9]+\\.([0-9]+)\\.[0-9]+(-[^:]*)?$') AS numeric)"#;
    const PATCH: &str = r#"CAST(substring("n"."archetype", E'\\.v[0-9]+\\.[0-9]+\\.([0-9]+)(-[^:]*)?$') AS numeric)"#;

    /// The `archetype_node_id` predicate: an ADL 1.4 id lowers to the
    /// subsumption condition (parent + specialisation children, same
    /// namespace, entity and major); an at-code keeps the case-folded
    /// equality. BASE `architecture_overview` master10 §Design-time
    /// Relationships; BASE Artefact Identification, Referencing §Supporting
    /// Archetype-based Querying.
    #[test]
    fn archetype_predicate_subsumption_sql() {
        let parent = predicate_sql("openEHR-EHR-OBSERVATION.laboratory.v1");
        for part in [
            NO_NAMESPACE,
            r#""n"."arch_entity" = 'openehr-ehr-observation'"#,
            r#""n"."arch_major" = 1"#,
            r#""n"."arch_concept" = 'laboratory'"#,
            r#""n"."arch_concept" LIKE 'laboratory-%'"#,
        ] {
            assert!(parent.contains(part), "{part}: {parent}");
        }
        assert!(
            !parent.contains("substring"),
            "an `ihrid_ref` names every release of its major: {parent}"
        );

        // A specialisation child carries its full concept, so it prefix-matches its
        // own further children too.
        let child = predicate_sql("openEHR-EHR-OBSERVATION.laboratory-glucose.v1");
        assert!(
            child.contains(r#""n"."arch_concept" = 'laboratory-glucose'"#),
            "exact specialised concept: {child}"
        );
        assert!(
            child.contains(r#""n"."arch_concept" LIKE 'laboratory-glucose-%'"#),
            "further-specialisation prefix: {child}"
        );

        // An at-code is not a full HRID: plain equality on the write-folded
        // `archetype` column, no subsumption columns.
        let at_code = predicate_sql("AT0001");
        assert!(
            !at_code.contains("LOWER") && at_code.contains(r#""n"."archetype" = 'at0001'"#),
            "at-code compares as plain equality on the write-folded column: {at_code}"
        );
        assert!(
            !at_code.contains("arch_entity"),
            "at-code does not use the subsumption columns: {at_code}"
        );
    }

    /// A namespace is never a wildcard (BASE `artefact_identification`
    /// master07 §Referencing): a namespaced id matches only its namespace, and
    /// an id without one only ids without one.
    #[test]
    fn the_namespace_is_exact() {
        let a = predicate_sql("org.A::openEHR-EHR-OBSERVATION.x.v1.0.0");
        assert!(
            a.contains(r#""n"."archetype" LIKE 'org.a::%'"#),
            "the namespace prefix, case-folded: {a}"
        );
        assert!(!a.contains("NOT LIKE"), "{a}");
        let b = predicate_sql("org.b::openEHR-EHR-OBSERVATION.x.v1.0.0");
        assert!(b.contains(r#""n"."archetype" LIKE 'org.b::%'"#), "{b}");
        let none = predicate_sql("openEHR-EHR-OBSERVATION.x.v1.0.0");
        assert!(none.contains(NO_NAMESPACE), "{none}");
        // `_` is a LIKE metacharacter, escaped inside the namespace prefix.
        let escaped = predicate_sql("org.my_ns::openEHR-EHR-OBSERVATION.x.v1.0.0");
        assert!(
            escaped.contains(r#""n"."archetype" LIKE E'org.my\\_ns::%'"#),
            "{escaped}"
        );
    }

    /// The version a query names bounds the stored release from below (AM
    /// master07 §Supporting Archetype-based Querying, "any previous minor or
    /// patch variant of X"): a minor for an `sihrid_ref`, a minor and patch
    /// for a `phrid_ref`, nothing for an `ihrid_ref`.
    #[test]
    fn the_queried_version_bounds_the_stored_release() {
        let specific = predicate_sql("openEHR-EHR-OBSERVATION.x.v1.2");
        assert!(specific.contains(&format!("{MINOR} >= 2")), "{specific}");
        assert!(!specific.contains("LIKE 'x-%'"), "{specific}");
        assert!(!specific.contains(PATCH), "{specific}");

        let physical = predicate_sql("openEHR-EHR-OBSERVATION.x.v1.2.3");
        assert!(
            physical.contains(&format!("{MINOR} > 2 OR ({MINOR} = 2 AND {PATCH} >= 3)")),
            "{physical}"
        );

        // A version-status suffix on the query is no part of its release.
        let candidate = predicate_sql("openEHR-EHR-OBSERVATION.x.v1.2.3-rc.1");
        assert!(candidate.contains(&format!("{PATCH} >= 3")), "{candidate}");
    }

    /// A value that is no archetype identifier at all falls back to plain
    /// equality on the whole `archetype` column, so the fallback stays
    /// reachable and never widens.
    #[test]
    fn a_non_identifier_falls_back_to_equality() {
        for value in [
            "at0001",
            "id3",
            "not an archetype",
            "openEHR-EHR-OBSERVATION",
        ] {
            let sql = predicate_sql(value);
            assert!(
                sql.contains(r#""n"."archetype" = "#),
                "{value:?} names no interface, so it matches the whole id: {sql}"
            );
            assert!(
                !sql.contains("arch_concept"),
                "{value:?} must not reach the subsumption columns: {sql}"
            );
        }
    }

    /// A stored specialisation family joins as one `= ANY(…)` set per
    /// interface boundary, every release of each child, beside the queried
    /// identity and its own version bound (BASE Artefact Identification, Referencing §Supporting
    /// Archetype-based Querying).
    #[test]
    fn stored_lineage_widens_the_matching_set() {
        let lineage = ArchetypeLineage::from_parent_edges([
            (
                "openEHR-EHR-OBSERVATION.hdl_result.v1.0.0",
                "openEHR-EHR-OBSERVATION.lipid_panel.v1",
            ),
            (
                "openEHR-EHR-OBSERVATION.ldl_result.v1.0.0",
                "openEHR-EHR-OBSERVATION.lipid_panel.v1",
            ),
        ]);
        let sql = archetype_predicate_sql("openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0", &lineage);
        assert!(
            sql.contains(r#""n"."arch_concept" = ANY(ARRAY ['hdl_result','ldl_result'])"#),
            "the stored descendants form one ANY set: {sql}"
        );
        assert!(
            sql.contains(&format!(
                r#""n"."arch_concept" = 'lipid_panel' AND ({MINOR} > 0"#
            )),
            "the queried identity keeps its version bound: {sql}"
        );
        assert!(
            !sql.contains("LIKE E'lipid"),
            "the widening is lineage-derived, never a string heuristic: {sql}"
        );

        // The ADL 1.4-form of the same identity keeps its concept-prefix rule
        // on top of the stored lineage: the two sources are additive.
        let legacy = archetype_predicate_sql("openEHR-EHR-OBSERVATION.lipid_panel.v1", &lineage);
        assert!(
            legacy.contains(r#""n"."arch_concept" LIKE E'lipid\\_panel-%'"#),
            "the 1.4-form prefix rule is unchanged: {legacy}"
        );
        assert!(legacy.contains("'hdl_result','ldl_result'"), "{legacy}");
    }

    /// A descendant that declares a parent in another major line is matched on
    /// its OWN (entity, major) boundary, never folded into the query's.
    #[test]
    fn a_cross_major_descendant_gets_its_own_boundary() {
        let lineage = ArchetypeLineage::from_parent_edges([(
            "openEHR-EHR-EVALUATION.genetic_diagnosis.v2.0.0",
            "openEHR-EHR-EVALUATION.diagnosis.v1",
        )]);
        let sql = archetype_predicate_sql("openEHR-EHR-EVALUATION.diagnosis.v1.29.0", &lineage);
        assert!(
            sql.contains(r#""n"."arch_major" = 1"#)
                && sql.contains(r#""n"."arch_concept" = 'diagnosis'"#),
            "the queried identity keeps its own major: {sql}"
        );
        assert!(
            sql.contains(r#""n"."arch_major" = 2"#)
                && sql.contains(r#""n"."arch_concept" = 'genetic_diagnosis'"#),
            "the descendant is matched at its own major: {sql}"
        );
    }
}

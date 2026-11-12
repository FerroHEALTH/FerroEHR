// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties of a demographic query: what a caller who is no
//! administrator is never served ([`crate::party_binding::policy`]).
//!
//! The query leaves out every version of a party whose root is of a policy
//! archetype or an ADL 2 specialisation of one within its major version, and
//! every version of a party that holds an acts-for relationship inline: the
//! test [`PolicyKind::of`] makes, which the Demographic API read gate applies
//! too (`party_binding::protect::check_read`). Every node a query reaches,
//! through `CONTAINS` or a path, belongs to the version of its root, so the
//! filter on the root row, which the query reads anyway, covers them. It is a
//! predicate of the one statement, so counts, paging and aggregates see only
//! what the caller may read. No openEHR spec governs who reads a party: our
//! own design.

use sea_query::extension::postgres::PgBinOper;
use sea_query::{Expr, ExprTrait, Func, Query};

use crate::db::iden::Node;
use crate::party_binding::policy::PolicyKind;

use super::archetype::legacy_children;
use super::expr::{as_text, col};
use super::value::fragment_items;
use super::{Builder, PolicyParties};

impl Builder<'_> {
    /// Whether this query leaves the policy parties out.
    pub(super) fn hides_policy_parties(&self) -> bool {
        self.ir.domain == crate::aql::ir::QueryDomain::Demographic
            && self.ctx.policy_parties == PolicyParties::Hidden
    }

    /// Leaves the policy parties out of every versioned-object root of a
    /// demographic query, when [`SqlCtx::policy_parties`](super::SqlCtx) says so.
    pub(super) fn hide_policy_parties(&mut self) {
        if !self.hides_policy_parties() {
            return;
        }
        for root in self.group_roots.clone() {
            let n = self.next_ctr();
            self.q.and_where(served(&root, n));
        }
    }
}

/// The root node `root` is no policy party: [`hidden`] is not true of it.
/// `n` names the alias of its relationships walk.
fn served(root: &str, n: usize) -> Expr {
    Expr::from(Func::coalesce([hidden(root, n), Expr::cust("FALSE")])).not()
}

/// The root node `root` is a policy party: its archetype, in the decomposed
/// columns the node codec writes, is a policy archetype or a specialisation
/// of one, or an inline `PARTY_RELATIONSHIP` it holds is of the acts-for
/// archetype or a specialisation of it.
///
/// The root reads only the `arch_entity`, `arch_major` and `arch_concept`
/// columns of its own row, and the relationships only its own `data`, which
/// holds `PARTY.relationships` inline (`crate::storage::structure`).
fn hidden(root: &str, n: usize) -> Expr {
    let mut held = Query::select();
    held.expr(Expr::val(1));
    let id = fragment_items(
        &mut held,
        col(root, Node::Data),
        "$.relationships[*].archetype_node_id",
        None,
        n,
    );
    // NOTE: an archetype the policy test cannot decompose hides every party, failing closed.
    let acts_for = match PolicyKind::ActsFor.id_pattern() {
        Some(pattern) => {
            held.and_where(as_text(id).binary(PgBinOper::Regex, Expr::val(pattern)));
            Expr::exists(held)
        }
        None => Expr::cust("TRUE"),
    };
    PolicyKind::parts()
        .into_iter()
        .map(|(_, entity, concept, major)| {
            col(root, Node::ArchEntity)
                .eq(Expr::val(entity))
                .and(col(root, Node::ArchMajor).eq(Expr::val(major)))
                .and(
                    col(root, Node::ArchConcept)
                        .eq(Expr::val(concept.clone()))
                        .or(legacy_children(root, &concept)),
                )
        })
        .fold(acts_for, ExprTrait::or)
}

#[cfg(test)]
mod tests {
    use sea_query::{PostgresQueryBuilder, Query};

    use super::*;

    /// The filter reads the decomposed archetype columns of the root row and
    /// its own inline relationships, and keeps a row the test cannot decide.
    #[test]
    fn the_filter_reads_the_root_row_only() {
        let mut q = Query::select();
        q.expr(Expr::val(1)).and_where(served("n", 7));
        let sql = q.to_string(PostgresQueryBuilder);
        assert!(sql.contains("WHERE NOT COALESCE("), "{sql}");
        for (_, entity, concept, major) in PolicyKind::parts() {
            assert!(
                sql.contains(&format!(
                    "\"n\".\"arch_entity\" = '{entity}' AND \"n\".\"arch_major\" = {major} \
                     AND (\"n\".\"arch_concept\" = '{concept}' OR"
                )),
                "{sql}"
            );
        }
        assert!(
            sql.contains("FROM jsonb_path_query(\"n\".\"data\", CAST('$.relationships[*].archetype_node_id' AS jsonpath)) AS \"f7\""),
            "{sql}"
        );
        assert!(sql.ends_with(", FALSE)"), "{sql}");
    }
}

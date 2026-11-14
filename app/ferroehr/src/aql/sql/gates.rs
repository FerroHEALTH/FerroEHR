// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access gates of one query, read once when the builder starts: the
//! access evaluator's condition ([`super::access`]), which carries the EHDS
//! Art. 8 restriction, the Art. 10 opt-out and every other step of the
//! decision procedure, and the Art. 3(3) delay of the person's own access
//! (`docs/law/eu/ehds/text.html`), published for the request by
//! `crate::system_log::access_context`. No openEHR spec governs any of them:
//! our own design/extension.
//!
//! A demographic query by a caller who is no administrator also leaves out
//! every version of a party that ever had a policy version
//! ([`crate::party_binding::policy_mark`]), the rule the Demographic API read gate
//! applies (`party_binding::protect::check_read`). The commit that writes a
//! policy version marks the party's head row, nothing clears the mark, and
//! the predicate is one column of the head row every versioned-object source
//! reaches by its primary key ([`super::head::bind_head`]), so every root, an
//! `OR` branch's included, and every node a query reaches through `CONTAINS`
//! or a path, which belongs to its root's version, is covered; counts, paging
//! and aggregates see only what the caller may read. No openEHR spec governs
//! who reads a party, and QUERY master03 §FROM gives semantics to the clinical
//! repository only: our own design.
//!
//! The same head row carries the read rule of the `EHR_ACCESS` settings: an
//! `EHR_ACCESS` object contributes a row only when its EHR is one whose
//! settings the caller may read ([`super::EhrAccessReaders`]), so a
//! restriction stays invisible to a provider (EHDS Art. 8) in every query
//! shape. Who reads the settings is our own design (#5508).

use sea_query::{Expr, ExprTrait as _};

use super::expr::col;
use super::{EhrAccessReaders, PolicyParties};
use crate::access_eval::condition::AccessCondition;
use crate::aql::ir::QueryDomain;
use crate::db::iden::VoHead;

/// Which access gates hold for the query being built.
#[derive(Debug, Clone, Copy)]
pub(super) struct Gates<'a> {
    /// The access evaluator's condition: a row it does not admit contributes
    /// nothing.
    access: &'a AccessCondition,
    /// The Art. 3(3) delay: an object withheld from the person contributes no
    /// row.
    person_withheld: bool,
    /// Whether a demographic query leaves the policy parties out: no party
    /// whose head carries the policy mark contributes a row. A clinical
    /// query reads no party, so it is [`PolicyParties::Served`] there.
    policy_parties: PolicyParties,
    /// The EHRs whose `EHR_ACCESS` objects contribute rows.
    access_settings: &'a EhrAccessReaders,
}

impl<'a> Gates<'a> {
    /// The gates the current request publishes for a query of `domain`
    /// that reads the policy parties as `policy_parties` says, the
    /// `EHR_ACCESS` objects of the EHRs `access_settings` names, and the rows
    /// `access` admits.
    pub(super) fn current(
        domain: QueryDomain,
        policy_parties: PolicyParties,
        access_settings: &'a EhrAccessReaders,
        access: &'a AccessCondition,
    ) -> Self {
        use crate::system_log::access_context::access_delay_enforced;
        Self {
            access,
            person_withheld: access_delay_enforced(),
            policy_parties: if domain == QueryDomain::Demographic {
                policy_parties
            } else {
                PolicyParties::Served
            },
            access_settings,
        }
    }

    /// The predicates on the head row `h` of one versioned object.
    pub(super) fn on_head(self, h: &str) -> Vec<Expr> {
        let mut predicates = Vec::new();
        // NOTE: our own design, only an administrator reads any version of a marked party.
        if self.policy_parties == PolicyParties::Hidden {
            predicates.push(col(h, VoHead::PolicyPartyAt).is_null());
        }
        // NOTE: our own design (#5513), a row the access evaluator does not admit contributes none.
        predicates.extend(super::access::on_head(self.access, h));
        // NOTE: EHDS Art. 3(3), an object withheld from the person contributes no row.
        if self.person_withheld {
            let until = col(h, VoHead::PersonWithheldUntil);
            predicates.push(until.clone().is_null().or(until.lte(Expr::cust("now()"))));
        }
        // NOTE: EHDS Art. 8, an EHR_ACCESS whose settings the caller may not read
        // contributes no row, so a restriction stays invisible (our own design).
        if let EhrAccessReaders::Only(keepers) = self.access_settings {
            let other_kind = col(h, VoHead::Kind).ne(Expr::val("EHR_ACCESS"));
            predicates.push(if keepers.groups.is_empty() {
                other_kind
            } else {
                other_kind.or(super::access::keeper_reads(keepers, h))
            });
        }
        predicates
    }

    /// The access evaluator's condition on the EHR id `target` of a
    /// versioned-object root or an EHR source: an EHR none of whose data the
    /// caller may read contributes no row.
    pub(super) fn on_ehr_id(self, target: &Expr) -> Option<Expr> {
        super::access::on_ehr_id(self.access, target)
    }
}

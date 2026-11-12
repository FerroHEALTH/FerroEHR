// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access gates of one query, read once when the builder starts: the EHDS
//! Art. 8 restriction of access by professionals, the Art. 3(3) delay of the
//! person's own access and the Art. 10 opt-out from access through the access
//! services (`docs/law/eu/ehds/text.html`), each published for the request by
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

use sea_query::{Alias, Expr, ExprTrait as _, Query};

use super::PolicyParties;
use super::expr::col;
use crate::aql::ir::QueryDomain;
use crate::db::iden::{Ehr, VoHead};

/// The alias of the `ehr` row the opt-out probe reads.
const OPT_OUT_PROBE: &str = "opt_out_ehr";

/// Which access gates hold for the query being built.
#[derive(Debug, Clone, Copy)]
pub(super) struct Gates {
    /// The Art. 8 restriction: a restricted object or EHR contributes no row.
    access_restricted: bool,
    /// The Art. 3(3) delay: an object withheld from the person contributes no
    /// row.
    person_withheld: bool,
    /// The Art. 10 opt-out: no object or EHR of an opted-out EHR contributes a
    /// row.
    opted_out: bool,
    /// Whether a demographic query leaves the policy parties out: no party
    /// whose head carries the policy mark contributes a row. A clinical
    /// query reads no party, so it is [`PolicyParties::Served`] there.
    policy_parties: PolicyParties,
}

impl Gates {
    /// The gates the current request publishes for a query of `domain`
    /// that reads the policy parties as `policy_parties` says.
    pub(super) fn current(domain: QueryDomain, policy_parties: PolicyParties) -> Self {
        use crate::system_log::access_context::{
            access_delay_enforced, access_opt_out_enforced, access_restriction_enforced,
        };
        Self {
            access_restricted: access_restriction_enforced(),
            person_withheld: access_delay_enforced(),
            opted_out: access_opt_out_enforced(),
            policy_parties: if domain == QueryDomain::Demographic {
                policy_parties
            } else {
                PolicyParties::Served
            },
        }
    }

    /// The predicates on the head row `h` of one versioned object.
    pub(super) fn on_head(self, h: &str) -> Vec<Expr> {
        let mut predicates = Vec::new();
        // NOTE: our own design, only an administrator reads any version of a marked party.
        if self.policy_parties == PolicyParties::Hidden {
            predicates.push(col(h, VoHead::PolicyPartyAt).is_null());
        }
        // NOTE: EHDS Art. 8, an object under an access restriction contributes no row.
        if self.access_restricted {
            predicates.push(col(h, VoHead::AccessRestrictedAt).is_null());
        }
        // NOTE: EHDS Art. 3(3), an object withheld from the person contributes no row.
        if self.person_withheld {
            let until = col(h, VoHead::PersonWithheldUntil);
            predicates.push(until.clone().is_null().or(until.lte(Expr::cust("now()"))));
        }
        predicates
    }

    /// The predicates on the `ehr` row bound as `alias`.
    pub(super) fn on_ehr(self, alias: &str) -> Vec<Expr> {
        // NOTE: EHDS Art. 8, a whole-EHR access restriction withholds the EHR
        // itself while the request enforces it.
        if self.access_restricted {
            return vec![col(alias, Ehr::AccessRestrictedAt).is_null()];
        }
        Vec::new()
    }

    /// The EHDS Art. 10 opt-out on the EHR id `target` of a versioned-object
    /// root or an EHR source: no opted-out EHR carries that id.
    ///
    /// A `NOT EXISTS`, so a `target` with no EHR (a party object) passes.
    pub(super) fn on_ehr_id(self, target: Expr) -> Option<Expr> {
        if !self.opted_out {
            return None;
        }
        let mut probe = Query::select();
        probe
            .expr(Expr::val(1))
            .from_as(Ehr::Table, Alias::new(OPT_OUT_PROBE))
            .and_where(col(OPT_OUT_PROBE, Ehr::Id).eq(target))
            .and_where(col(OPT_OUT_PROBE, Ehr::OptedOutAt).is_not_null());
        Some(Expr::exists(probe).not())
    }
}

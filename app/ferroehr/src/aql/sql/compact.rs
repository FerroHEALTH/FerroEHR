// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The compact markers in the AQL emitter (#4904): the reads that put back
//! what a node row left out of its fragment under `origin_compact`,
//! `time_compact`, `party_self` and `uid_derived`
//! (`crate::storage::compact`), and the version row a root's `uid` and
//! template come back from.
//!
//! No openEHR spec governs storage: our own design.

use sea_query::{BinOper, Expr, ExprTrait as _};

use crate::aql::ir::Source;
use crate::db::iden::{Node, VersionRow};

use super::Builder;
use super::expr::{cast, col, to_jsonb};
use super::from::is_vo_root_type;

/// The `OBJECT_VERSION_ID` text of the `version` alias `voa`, by the law of
/// the wire id (RM common master06 §Version Identification):
/// `vo_id::creating_system_id::version_tree_id`. The expression
/// `idx_version_hot_uid` indexes, held identical by the storage plans test,
/// and the one `ext.version_uid` renders.
pub(super) fn version_uid_text(voa: &str) -> Expr {
    [
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
    ]
    .into_iter()
    .reduce(sea_query::extension::postgres::PgExpr::concatenate)
    .unwrap_or_else(|| Expr::val(""))
}

/// The version's uid and template a root row's fragment is rebuilt with, off
/// the `version` alias `root` the node `alias` is bound with, and `NULL` on
/// every row but the root (`num = 0`); both `NULL` without a version alias.
pub(super) fn root_columns(alias: &str, root: Option<&str>) -> (Expr, Expr) {
    let Some(voa) = root else {
        return (Expr::cust("NULL::text"), Expr::cust("NULL::text"));
    };
    let at_root = |value: Expr| {
        Expr::cust_with_exprs(
            "CASE WHEN $1 = 0 THEN $2 END",
            [col(alias, Node::Num), value],
        )
    };
    (
        at_root(version_uid_text(voa)),
        at_root(col(voa, VersionRow::TemplateId)),
    )
}

/// `data` with the member a compact marker stands for put back: `origin` and
/// `time` as the `DV_DATE_TIME` of the stored string (`ext.node_date_time`),
/// `subject` as the bare `PARTY_SELF` (`crate::storage::compact`).
pub(super) fn marked_member(alias: &str, data: Expr, member: &str) -> Expr {
    if member == "subject" {
        return Expr::cust_with_exprs(
            "CASE WHEN $1 THEN jsonb_build_object('subject', '{\"_type\": \"PARTY_SELF\"}'::jsonb) \
             ELSE $2 END",
            [col(alias, Node::PartySelf), data],
        );
    }
    let marker = if member == "origin" {
        Node::OriginCompact
    } else {
        Node::TimeCompact
    };
    Expr::cust_with_exprs(
        "CASE WHEN $1 THEN jsonb_build_object($2, node_date_time($3 -> $4)) ELSE $5 END",
        [
            col(alias, marker),
            Expr::val(member),
            data.clone(),
            Expr::val(member),
            data,
        ],
    )
}

/// A scalar the compact markers answer in place of the fragment
/// (`crate::storage::compact`): `origin/value` and `time/value`, which a
/// marked row holds as the string under the member, and the root's
/// `uid/value`, which its version row renders.
pub(super) struct CompactRead {
    /// The marker column that says the row holds the compact form.
    pub(super) marker: Expr,
    /// The scalar, as jsonb, when it does.
    pub(super) value: Expr,
}

/// The compact read of the plain fragment path `names` off `alias`, if the
/// markers answer it; `root` is the version alias of a root-capable node.
pub(super) fn compact_read(alias: &str, names: &[&str], root: Option<&str>) -> Option<CompactRead> {
    let string_member = |member: &str, marker: Node| CompactRead {
        marker: col(alias, marker),
        value: col(alias, Node::Data).binary(BinOper::Custom("->"), Expr::val(member.to_owned())),
    };
    match names {
        ["origin", "value"] => Some(string_member("origin", Node::OriginCompact)),
        ["time", "value"] => Some(string_member("time", Node::TimeCompact)),
        ["uid", "value"] => root.map(|voa| CompactRead {
            marker: col(alias, Node::UidDerived).and(col(alias, Node::Num).eq(Expr::val(0))),
            value: to_jsonb(version_uid_text(voa)),
        }),
        _ => None,
    }
}

impl Builder<'_> {
    /// The `version` alias the node `alias` is bound with when its rows can
    /// be a versioned object's root (`num = 0`): an RM source of versioned
    /// root types, or the `EHR_STATUS` root an EHR source opened. The root
    /// row's `uid` and its header's template come back from that version
    /// row (`crate::storage::compact`); `None` for a walked step, a contained
    /// source or a `VERSION` source, whose rows are never a root.
    pub(super) fn root_version_alias(&self, alias: &str) -> Option<String> {
        let sid = self
            .node_alias
            .iter()
            .find_map(|(sid, node)| (node == alias).then_some(*sid))?;
        match self.ir.sources.get(sid)? {
            Source::Rm(r) => {
                if r.rm_type.is_empty() || !r.rm_type.names().iter().all(|t| is_vo_root_type(t)) {
                    return None;
                }
                self.vo_alias.get(&sid).cloned()
            }
            Source::Ehr(_) => self.ehr_status_vo.get(&sid).cloned(),
            Source::Version(_) => None,
        }
    }
}

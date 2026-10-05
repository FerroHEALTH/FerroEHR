// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `sea-query` identifier vocabulary for the relations the AQL SQL generator
//! builds against (`migrations/clinical/`, and the party domain rendered from
//! the same DDL template).
//!
//! No openEHR spec governs the SQL schema — this is our own PG18-native
//! design.
//!
//! One enum per table, in the official `sea-query` derive shape: the `Table`
//! variant carries an explicit `#[iden = "..."]` and renders the table name,
//! and every other variant renders its `snake_cased` column name. This is the
//! typed name catalog the generator reaches for instead of writing a column as
//! a string, and its tests hold both ends of that: every name here is declared
//! by the clinical migration set, and nothing under `aql/sql/` names a column
//! any other way.
//!
//! It covers the five relations the generator reads and nothing else. A
//! relation reached only through a static, compile-time-checked `sqlx::query!`
//! needs no entry, because that query is checked against the live schema
//! already; an entry it could not use would be a second, unchecked copy of the
//! DDL.

/// `ehr` — one row per EHR.
#[derive(Debug, Clone, Copy, sea_query::Iden)]
pub enum Ehr {
    /// The `ehr` table itself.
    #[iden = "ehr"]
    Table,
    /// `id` — the EHR id (`EHR.ehr_id`), a uuidv7.
    Id,
    /// `system_id` — the system that created this EHR, stored at creation and
    /// never mutated (not the live service configuration).
    SystemId,
    /// `time_created` — the server-computed EHR creation instant.
    TimeCreated,
    /// `subject_id` — denormalized copy of the current
    /// `EHR_STATUS.subject.external_ref.id.value`, backing the
    /// one-EHR-per-subject unique index.
    SubjectId,
    /// `subject_namespace` — denormalized copy of the current
    /// `EHR_STATUS.subject.external_ref.namespace`.
    SubjectNamespace,
    /// `is_queryable` — promoted copy of the current `EHR_STATUS.is_queryable`,
    /// backing the AQL full-population gate.
    IsQueryable,
    /// `is_modifiable` — promoted copy of the current
    /// `EHR_STATUS.is_modifiable`, backing the content-write guard.
    IsModifiable,
    /// `restricted_at` — when restriction of processing was recorded for the
    /// whole EHR (GDPR Art. 18(2)); `NULL` = unrestricted.
    RestrictedAt,
    /// `research_objected_at` — when the subject objected to research
    /// processing (GDPR Art. 21(6)); `NULL` = no objection.
    ResearchObjectedAt,
    /// `research_objection_ground` — the controller's recorded public-interest
    /// ground for overriding the objection; `NULL` while it stands.
    ResearchObjectionGround,
}

/// `commit_audit` — `AUDIT_DETAILS` of every committed change.
#[derive(Debug, Clone, Copy, sea_query::Iden)]
pub enum CommitAudit {
    /// The `commit_audit` table itself.
    #[iden = "commit_audit"]
    Table,
    /// `id` — the audit row id (a uuidv7).
    Id,
    /// `time_committed` — the server-computed commit instant, never
    /// client-supplied.
    TimeCommitted,
    /// `system_id` — `AUDIT_DETAILS.system_id`.
    SystemId,
    /// `change_type` — the `audit_change_type` terminology-group code.
    ChangeType,
    /// `description` — the canonical `DV_TEXT` fragment of
    /// `AUDIT_DETAILS.description` (0..1), as JSONB.
    Description,
    /// `committer` — the canonical `PARTY_PROXY` of the committer, as JSONB.
    Committer,
    /// `attestation` — the `ATTESTATION`-declared attributes as JSONB when the
    /// commit audit is an `ATTESTATION`, else NULL.
    Attestation,
}

/// `version` — one write-once row per version of a versioned object.
#[derive(Debug, Clone, Copy, sea_query::Iden)]
pub enum VersionRow {
    /// The `version` table itself.
    #[iden = "version"]
    Table,
    /// `tier` — the storage tier, and the partition key: `hot` or `cold`.
    Tier,
    /// `vo_id` — the versioned object's id (the `object_id` of every
    /// `OBJECT_VERSION_ID` in its tree).
    VoId,
    /// `kind` — the versioned object's RM type (`COMPOSITION`, `EHR_STATUS`,
    /// `FOLDER`, a demographic party type, …).
    Kind,
    /// `ehr_id` — the owning EHR, or `NULL` for a demographic party.
    EhrId,
    /// `sys_version` — the opaque per-object commit ordinal (1..n across trunk
    /// AND branch commits); the key of `node` / `vo_attestation`, and NOT the
    /// wire version number.
    SysVersion,
    /// `trunk_version` — `VERSION_TREE_ID` first part: the wire version number
    /// on a trunk row, the fork point on a branch row.
    TrunkVersion,
    /// `branch_number` — `VERSION_TREE_ID` second part; `0` = trunk row.
    BranchNumber,
    /// `branch_version` — `VERSION_TREE_ID` third part; `0` = trunk row.
    BranchVersion,
    /// `lifecycle_state` — the `version_lifecycle_state` code (`523` is the
    /// content-less logical delete).
    LifecycleState,
    /// `creating_system_id` — the immutable middle segment of this version's
    /// `OBJECT_VERSION_ID`, reconstructed from storage and never live config.
    CreatingSystemId,
    /// `preceding_version_uid` — `ORIGINAL_VERSION.preceding_version_uid` as a
    /// full `OBJECT_VERSION_ID`; `NULL` for a first version.
    PrecedingVersionUid,
    /// `signature` — `VERSION.signature`, opaque radix-64.
    Signature,
    /// `signature_client_supplied` — whether the signature arrived verbatim
    /// from the committing client rather than being generated here.
    SignatureClientSupplied,
    /// `wrapped_original` — the `IMPORTED_VERSION` discriminator and the
    /// wrapped `ORIGINAL_VERSION`'s own wrapper-level provenance.
    WrappedOriginal,
    /// `other_input_version_uids` — the merge provenance of an
    /// `ORIGINAL_VERSION`; `NULL` when the version is not a merge.
    OtherInputVersionUids,
    /// `origins` — the distinct originating systems of the body.
    Origins,
    /// `contribution_id` — the CONTRIBUTION this version was committed in.
    ContributionId,
    /// `commit_audit_id` — this version's own `AUDIT_DETAILS` row.
    CommitAuditId,
    /// `template_id` — the OPT the content was validated against; `NULL` for
    /// template-less content.
    TemplateId,
    /// `stable_compatible` — whether the released openEHR generation set can
    /// express this version's body.
    StableCompatible,
    /// `committed_at` — the commit instant copied from the commit audit; the
    /// store's whole temporal axis.
    CommittedAt,
    /// `body` — the canonical openEHR JSON bytes, verbatim.
    Body,
}

/// `vo_head` — the one mutable row per versioned object.
#[derive(Debug, Clone, Copy, sea_query::Iden)]
pub enum VoHead {
    /// The `vo_head` table itself.
    #[iden = "vo_head"]
    Table,
    /// `vo_id` — the versioned object, and the primary key.
    VoId,
    /// `kind` — the versioned object's RM type.
    Kind,
    /// `ehr_id` — the owning EHR, or `NULL` for a demographic party.
    EhrId,
    /// `tier` — the tier the object's version and node rows sit in.
    Tier,
    /// `head_sys_version` — the greatest `sys_version` on ANY lineage: the
    /// RM's `latest_version`.
    HeadSysVersion,
    /// `trunk_head_sys_version` — the greatest `sys_version` on the trunk:
    /// `LATEST_VERSION`.
    TrunkHeadSysVersion,
    /// `lifecycle_state` — the lifecycle state of the trunk head.
    LifecycleState,
    /// `template_id` — the template of the trunk head.
    TemplateId,
    /// `committed_at` — the trunk head's commit instant.
    CommittedAt,
    /// `restricted_at` — when restriction of processing was recorded for this
    /// object; `NULL` = unrestricted.
    RestrictedAt,
    /// `retention_hold_at` — when a retention hold was placed; `NULL` = none.
    RetentionHoldAt,
    /// `archived_at` — when the object moved to the cold tier; `NULL` while
    /// hot.
    ArchivedAt,
    /// `archive_reason` — the reason the caller gave for archiving.
    ArchiveReason,
}

/// `node` — the decomposed content: one row per RM structure node, per
/// version (nested-set indexed).
#[derive(Debug, Clone, Copy, sea_query::Iden)]
pub enum Node {
    /// The `node` table itself.
    #[iden = "node"]
    Table,
    /// `tier` — the storage tier, and the partition key: `hot` or `cold`.
    Tier,
    /// `vo_id` — the versioned object this node belongs to.
    VoId,
    /// `sys_version` — the `version` commit ordinal this node belongs to.
    SysVersion,
    /// `num` — the node's pre-order number within the version (root = 0).
    Num,
    /// `num_cap` — the highest `num` in this node's subtree, so the subtree is
    /// the closed interval `num..=num_cap` (this is what AQL CONTAINS joins on).
    NumCap,
    /// `parent_num` — the `num` of the parent structure node (the root points
    /// at itself).
    ParentNum,
    /// `ehr_id` — the owning EHR, denormalized onto every node for
    /// EHR-scoped querying.
    EhrId,
    /// `rm_type` — the node's full RM type name (never an alias).
    RmType,
    /// `archetype` — the node's `archetype_node_id` when it carries one.
    Archetype,
    /// `arch_entity` — the `qualified_rm_entity` of a full archetype HRID,
    /// lowercased; `NULL` on at/id-code nodes.
    ArchEntity,
    /// `arch_concept` — the full `domain_concept` of a full archetype HRID
    /// (specialisation segments included), lowercased, so a parent query
    /// matches a child by prefix.
    ArchConcept,
    /// `arch_major` — the major version of a full archetype HRID; `NULL` on
    /// at/id-code nodes.
    ArchMajor,
    /// `name` — the node's `name.value`.
    Name,
    /// `name_code` — the `code_string` of the node's `name/defining_code` when
    /// its name is coded; `NULL` otherwise.
    NameCode,
    /// `name_terminology` — the `terminology_id` of that defining code.
    NameTerminology,
    /// `path` — the materialized path from the root, whose byte order under
    /// `COLLATE "C"` equals tree order; used for reassembly, never as an AQL
    /// predicate.
    Path,
    /// `data` — the node's canonical openEHR JSON fragment verbatim, with
    /// structure children pruned.
    Data,
    /// `context_start` — the promoted `EVENT_CONTEXT.start_time.value` of the
    /// `COMPOSITION` root row; `NULL` elsewhere and on a context-less
    /// persistent composition.
    ContextStart,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_query::{Expr, ExprTrait as _, Iden as _, PostgresQueryBuilder, Query};

    /// The relations the emitter names, and the clinical migrations that
    /// declare them.
    const DDL: &[(&str, &str)] = &[
        (
            "ehr",
            include_str!("../../migrations/clinical/0002_ehr.sql"),
        ),
        (
            "version",
            include_str!("../../migrations/clinical/0003_change_control.sql"),
        ),
        (
            "vo_head",
            include_str!("../../migrations/clinical/0003_change_control.sql"),
        ),
        (
            "commit_audit",
            include_str!("../../migrations/clinical/0003_change_control.sql"),
        ),
        (
            "node",
            include_str!("../../migrations/clinical/0004_node.sql"),
        ),
    ];

    /// The `CREATE TABLE {table} ( … )` body, comments stripped before the
    /// parentheses are counted: the DDL documents itself in prose, and an
    /// interval written `[a, b)` would otherwise close the table early and
    /// hide every column after it.
    fn create_table_body(table: &str) -> String {
        let head = format!("CREATE TABLE {table} (");
        let body = DDL
            .iter()
            .filter(|(name, _)| *name == table)
            .find_map(|(_, file)| file.split_once(&head))
            .unwrap_or_else(|| panic!("no `{head}` in the clinical migration set"))
            .1;
        let code: String = body
            .lines()
            .map(|line| line.split("--").next().unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        let mut depth = 1usize;
        for (i, ch) in code.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return code.get(..i).unwrap_or_default().to_owned();
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated CREATE TABLE {table} in the clinical migration set");
    }

    /// Whether `body` declares a column named exactly `col` — a line whose
    /// trimmed text is `col` followed by a non-identifier character (so `num`
    /// does not match the `num_cap` declaration).
    fn declares_column(body: &str, col: &str) -> bool {
        body.lines().any(|line| {
            let t = line.trim_start();
            t.strip_prefix(col)
                .is_some_and(|rest| rest.starts_with([' ', '\t']))
        })
    }

    /// Every catalog entry, as `(table, [column, …])`, rendered through the
    /// derive rather than retyped.
    fn catalog() -> Vec<(String, Vec<String>)> {
        vec![
            (
                Ehr::Table.to_string(),
                vec![
                    Ehr::Id.to_string(),
                    Ehr::SystemId.to_string(),
                    Ehr::TimeCreated.to_string(),
                    Ehr::SubjectId.to_string(),
                    Ehr::SubjectNamespace.to_string(),
                    Ehr::IsQueryable.to_string(),
                    Ehr::IsModifiable.to_string(),
                    Ehr::RestrictedAt.to_string(),
                    Ehr::ResearchObjectedAt.to_string(),
                    Ehr::ResearchObjectionGround.to_string(),
                ],
            ),
            (
                CommitAudit::Table.to_string(),
                vec![
                    CommitAudit::Id.to_string(),
                    CommitAudit::TimeCommitted.to_string(),
                    CommitAudit::SystemId.to_string(),
                    CommitAudit::ChangeType.to_string(),
                    CommitAudit::Description.to_string(),
                    CommitAudit::Committer.to_string(),
                    CommitAudit::Attestation.to_string(),
                ],
            ),
            (
                VersionRow::Table.to_string(),
                vec![
                    VersionRow::Tier.to_string(),
                    VersionRow::VoId.to_string(),
                    VersionRow::Kind.to_string(),
                    VersionRow::EhrId.to_string(),
                    VersionRow::SysVersion.to_string(),
                    VersionRow::TrunkVersion.to_string(),
                    VersionRow::BranchNumber.to_string(),
                    VersionRow::BranchVersion.to_string(),
                    VersionRow::LifecycleState.to_string(),
                    VersionRow::CreatingSystemId.to_string(),
                    VersionRow::PrecedingVersionUid.to_string(),
                    VersionRow::Signature.to_string(),
                    VersionRow::SignatureClientSupplied.to_string(),
                    VersionRow::WrappedOriginal.to_string(),
                    VersionRow::OtherInputVersionUids.to_string(),
                    VersionRow::Origins.to_string(),
                    VersionRow::ContributionId.to_string(),
                    VersionRow::CommitAuditId.to_string(),
                    VersionRow::TemplateId.to_string(),
                    VersionRow::StableCompatible.to_string(),
                    VersionRow::CommittedAt.to_string(),
                    VersionRow::Body.to_string(),
                ],
            ),
            (
                VoHead::Table.to_string(),
                vec![
                    VoHead::VoId.to_string(),
                    VoHead::Kind.to_string(),
                    VoHead::EhrId.to_string(),
                    VoHead::Tier.to_string(),
                    VoHead::HeadSysVersion.to_string(),
                    VoHead::TrunkHeadSysVersion.to_string(),
                    VoHead::LifecycleState.to_string(),
                    VoHead::TemplateId.to_string(),
                    VoHead::CommittedAt.to_string(),
                    VoHead::RestrictedAt.to_string(),
                    VoHead::RetentionHoldAt.to_string(),
                    VoHead::ArchivedAt.to_string(),
                    VoHead::ArchiveReason.to_string(),
                ],
            ),
            (
                Node::Table.to_string(),
                vec![
                    Node::Tier.to_string(),
                    Node::VoId.to_string(),
                    Node::SysVersion.to_string(),
                    Node::Num.to_string(),
                    Node::NumCap.to_string(),
                    Node::ParentNum.to_string(),
                    Node::EhrId.to_string(),
                    Node::RmType.to_string(),
                    Node::Archetype.to_string(),
                    Node::ArchEntity.to_string(),
                    Node::ArchConcept.to_string(),
                    Node::ArchMajor.to_string(),
                    Node::Name.to_string(),
                    Node::NameCode.to_string(),
                    Node::NameTerminology.to_string(),
                    Node::Path.to_string(),
                    Node::Data.to_string(),
                    Node::ContextStart.to_string(),
                ],
            ),
        ]
    }

    #[test]
    fn every_catalog_name_is_declared_by_the_schema() {
        for (table, columns) in catalog() {
            let body = create_table_body(&table);
            for column in columns {
                assert!(
                    declares_column(&body, &column),
                    "the catalog names `{table}.{column}`, which `CREATE TABLE {table}` does not \
                     declare in the clinical migration set — schema drift"
                );
            }
        }
    }

    #[test]
    fn the_table_names_render_exactly() {
        assert_eq!(Ehr::Table.to_string(), "ehr");
        assert_eq!(CommitAudit::Table.to_string(), "commit_audit");
        assert_eq!(VersionRow::Table.to_string(), "version");
        assert_eq!(VoHead::Table.to_string(), "vo_head");
        assert_eq!(Node::Table.to_string(), "node");
    }

    #[test]
    fn builds_a_node_query() {
        let (sql, _) = Query::select()
            .column((Node::Table, Node::Num))
            .from(Node::Table)
            .and_where(Expr::col(Node::RmType).eq("OBSERVATION"))
            .build(PostgresQueryBuilder);
        assert_eq!(
            sql,
            r#"SELECT "node"."num" FROM "node" WHERE "rm_type" = $1"#
        );
    }

    /// The emitter names every column through this catalog: no `col(alias,
    /// "literal")`, no `Expr::col(Alias::new("literal"))`, and no
    /// `.column(Alias::new("literal"))` anywhere under `aql/sql/`.
    ///
    /// The catalog exists for the typo protection a string cannot give, and a
    /// column that drops out of the schema has to be a failing test rather
    /// than a runtime SQL error. `derived_col` is the sanctioned exception and
    /// is why the scan is spelled on the `col(` token: a subquery's or a
    /// set-returning function's output column is declared by no relation, so
    /// the catalog cannot name it.
    #[test]
    fn the_emitter_names_no_column_as_a_string() {
        const SOURCES: &[(&str, &str)] = &[
            ("mod.rs", include_str!("../aql/sql/mod.rs")),
            ("expr.rs", include_str!("../aql/sql/expr.rs")),
            ("from.rs", include_str!("../aql/sql/from.rs")),
            ("predicate.rs", include_str!("../aql/sql/predicate.rs")),
            ("select.rs", include_str!("../aql/sql/select.rs")),
            ("value.rs", include_str!("../aql/sql/value.rs")),
        ];
        // `expr.rs` defines `col` and `derived_col` themselves, so its two
        // definition lines are the one place the tokens appear without naming
        // a column.
        const DEFINITIONS: &[&str] = &[
            "pub(super) fn col(alias: &str, column: impl IntoIden) -> Expr {",
            "pub(super) fn derived_col(alias: &str, name: &str) -> Expr {",
        ];
        let mut offenders = Vec::new();
        for (name, source) in SOURCES {
            for (number, line) in source.lines().enumerate() {
                if DEFINITIONS.contains(&line.trim()) {
                    continue;
                }
                let raw_column = is_string_column(line, "col(")
                    || line.contains("Expr::col(Alias::new(\"")
                    || line.contains(".column(Alias::new(\"");
                if raw_column {
                    offenders.push(format!("{name}:{}: {}", number + 1, line.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "the AQL emitter names a column as a string instead of through `crate::db::iden`; \
             use the catalog, or `derived_col` for a column no relation declares:\n{}",
            offenders.join("\n")
        );
    }

    /// Whether `line` calls `token` — as a whole word, so `Expr::col(` and
    /// `derived_col(` do not match `col(` — with a string literal in its
    /// second argument.
    fn is_string_column(line: &str, token: &str) -> bool {
        let mut rest = line;
        while let Some(at) = rest.find(token) {
            let leads = rest
                .get(..at)
                .and_then(|before| before.chars().next_back())
                .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != ':');
            let args = rest.get(at + token.len()..).unwrap_or_default();
            let second = args.find(',').and_then(|comma| args.get(comma + 1..));
            if leads && second.is_some_and(|s| s.trim_start().starts_with('"')) {
                return true;
            }
            rest = args;
        }
        false
    }
}

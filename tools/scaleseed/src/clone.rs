// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The clones: every row the commit path wrote for a prototype, copied out
//! once and written back with `COPY` under new identifiers.
//!
//! A prototype's rows are the `ehr` row (for the prototype EHR), its versions,
//! its head, its nodes, the contributions its versions name, every commit
//! audit those name, and the outbox events of those contributions. A clone
//! replaces each identifier of the prototype's (the EHR, the versioned object,
//! each contribution and each audit) with a fresh one minted by the server's
//! own stamp, as a text substitution over the
//! `COPY` lines: the identifiers are UUIDs, which occur nowhere else in a row,
//! and the stored body carries the versioned object's id in its `uid`. The one
//! value that depends on the identifiers in a way a substitution cannot
//! follow is `version.signature`, the digest of the version's canonical form
//! (RM common master06 §Digital Signature). It is recomputed over the
//! prototype's served version envelope with the identifiers substituted, by
//! the crates' own canonical form and digest signer, so a read of a clone
//! verifies as a read of the prototype does.
//!
//! Each chunk of EHRs is one transaction that also records the chunk, so a
//! seed stopped at any point resumes at the first chunk not recorded.

use std::collections::BTreeMap;

use anyhow::{Context as _, anyhow};
use futures::TryStreamExt as _;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::versioning::signature::signer::Signer;

/// The tables a clone writes, in an order the foreign keys accept, each with
/// the explicit column list its lines carry.
const TABLES: [(&str, &str); 7] = [
    ("ehr", "*"),
    ("commit_audit", "*"),
    ("contribution", "*"),
    ("version", VERSION_COLUMNS),
    ("vo_head", "*"),
    ("node", "*"),
    (
        "event_outbox",
        "contribution_id, ehr_id, envelope, committed_at",
    ),
];

/// The `version` columns a clone writes, `content_key` (generated) left out;
/// `signature` is the twelfth.
const VERSION_COLUMNS: &str = "tier, vo_id, kind, ehr_id, sys_version, trunk_version, \
    branch_number, branch_version, lifecycle_state, creating_system_id, preceding_version_uid, \
    signature, signature_client_supplied, wrapped_original, other_input_version_uids, origins, \
    contribution_id, commit_audit_id, template_id, stable_compatible, committed_at, body, \
    content_digest, content_sys_version";

/// The zero-based position of `signature` in [`VERSION_COLUMNS`].
const SIGNATURE_COLUMN: usize = 11;

/// The zero-based position of `sys_version` in [`VERSION_COLUMNS`].
const SYS_VERSION_COLUMN: usize = 4;

/// One prototype's rows and what a clone substitutes in them.
#[derive(Debug)]
pub(crate) struct Prototype {
    /// The prototype's own identifiers, every one replaced in a clone.
    ids: Vec<Uuid>,
    /// The `COPY` lines per table, keyed by the table's place in [`TABLES`].
    lines: BTreeMap<usize, Vec<String>>,
    /// Each version's served envelope as text, by `sys_version`.
    envelopes: BTreeMap<i32, String>,
    /// The versioned object.
    pub(crate) vo: Uuid,
    /// The versions it holds.
    pub(crate) versions: i64,
}

/// The `WHERE` clause that selects a prototype's rows of each table, with `$1`
/// the versioned object and `$2` its EHR (the EHR row only for an EHR
/// prototype).
fn selection(table: &str, with_ehr: bool) -> Option<&'static str> {
    Some(match table {
        "ehr" if with_ehr => "id = $2",
        "version" | "vo_head" | "node" => "vo_id = $1",
        "contribution" => "id IN (SELECT contribution_id FROM clinical.version WHERE vo_id = $1)",
        "event_outbox" => {
            "contribution_id IN (SELECT contribution_id FROM clinical.version WHERE vo_id = $1)"
        }
        "commit_audit" => {
            "id IN (SELECT commit_audit_id FROM clinical.version WHERE vo_id = $1 \
             UNION SELECT c.commit_audit_id FROM clinical.contribution c \
             JOIN clinical.version v ON v.contribution_id = c.id WHERE v.vo_id = $1)"
        }
        _ => return None,
    })
}

impl Prototype {
    /// Reads the rows of the versioned object `vo` of `ehr`, the EHR row too
    /// when `with_ehr`, and the served envelope of each of its versions.
    ///
    /// # Errors
    /// A failed read, or a version the service does not serve.
    pub(crate) async fn read(
        svc: &FerroEhrService,
        pool: &PgPool,
        ehr: Uuid,
        vo: Uuid,
        with_ehr: bool,
    ) -> anyhow::Result<Self> {
        let mut conn = pool.acquire().await?;
        let mut lines = BTreeMap::new();
        for (place, (table, columns)) in TABLES.iter().enumerate() {
            let Some(filter) = selection(table, with_ehr) else {
                continue;
            };
            // NOTE: COPY takes no bind parameters, so the two identifiers are
            // spelled as UUID literals, which a Uuid's display form always is.
            let filter = filter
                .replace("$1", &format!("'{vo}'::uuid"))
                .replace("$2", &format!("'{ehr}'::uuid"));
            let statement =
                format!("COPY (SELECT {columns} FROM clinical.{table} WHERE {filter}) TO STDOUT");
            let bytes: Vec<u8> = conn
                .copy_out_raw(&statement)
                .await?
                .try_fold(Vec::new(), |mut all, chunk| async move {
                    all.extend_from_slice(&chunk);
                    Ok(all)
                })
                .await?;
            let text = String::from_utf8(bytes)?;
            lines.insert(place, text.lines().map(str::to_owned).collect::<Vec<_>>());
        }
        let ids = identifiers(&mut conn, ehr, vo).await?;
        drop(conn);
        let envelopes = envelopes(svc, &lines, ehr, vo).await?;
        let versions = i64::try_from(envelopes.len())?;
        Ok(Self {
            ids,
            lines,
            envelopes,
            vo,
            versions,
        })
    }

    /// The `COPY` lines of one clone, per table, with `ehr` its EHR and every
    /// other identifier minted fresh; returns the clone's versioned object.
    ///
    /// # Errors
    /// A version line without a parsable `sys_version`, or an envelope whose
    /// canonical form cannot be computed.
    pub(crate) fn clone_into(
        &self,
        svc: &FerroEhrService,
        signer: &Signer,
        proto_ehr: Uuid,
        ehr: Uuid,
        out: &mut BTreeMap<usize, String>,
    ) -> anyhow::Result<Uuid> {
        let mut map: Vec<(String, String)> = vec![(proto_ehr.to_string(), ehr.to_string())];
        let mut vo = self.vo;
        for id in &self.ids {
            if *id == proto_ehr {
                continue;
            }
            let fresh = svc.stamp().mint();
            if *id == self.vo {
                vo = fresh;
            }
            map.push((id.to_string(), fresh.to_string()));
        }
        let swap = |text: &str| {
            map.iter().fold(text.to_owned(), |acc, (from, to)| {
                acc.replace(from.as_str(), to)
            })
        };
        for (place, lines) in &self.lines {
            let target = out.entry(*place).or_default();
            for line in lines {
                let mut swapped = swap(line);
                if TABLES
                    .get(*place)
                    .is_some_and(|(table, _)| *table == "version")
                {
                    swapped = self.resign(&swapped, signer, &swap)?;
                }
                target.push_str(&swapped);
                target.push('\n');
            }
        }
        Ok(vo)
    }

    /// The version line `line` with its signature recomputed over the
    /// prototype's envelope of the same `sys_version`, its identifiers
    /// swapped by `swap`.
    #[expect(
        clippy::disallowed_types,
        reason = "a served version envelope is the canonical form's input, as the read path's own (#1694)"
    )]
    fn resign(
        &self,
        line: &str,
        signer: &Signer,
        swap: &impl Fn(&str) -> String,
    ) -> anyhow::Result<String> {
        let mut columns: Vec<&str> = line.split('\t').collect();
        let sys_version: i32 = columns
            .get(SYS_VERSION_COLUMN)
            .ok_or_else(|| anyhow!("a version line without sys_version"))?
            .parse()?;
        let old = columns
            .get(SIGNATURE_COLUMN)
            .ok_or_else(|| anyhow!("a version line without signature"))?;
        if *old == "\\N" {
            return Ok(line.to_owned());
        }
        let envelope = self
            .envelopes
            .get(&sys_version)
            .ok_or_else(|| anyhow!("no served envelope for sys_version {sys_version}"))?;
        let mut value: serde_json::Value = serde_json::from_str(&swap(envelope))?;
        if let Some(object) = value.as_object_mut() {
            object.remove("signature");
        }
        let canonical =
            openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json(&value)
                .map_err(|e| anyhow!("the canonical form of a cloned version: {e}"))?;
        let signature = signer
            .sign(&canonical)
            .map_err(|e| anyhow!("sign a cloned version: {e}"))?;
        if let Some(slot) = columns.get_mut(SIGNATURE_COLUMN) {
            *slot = &signature;
            return Ok(columns.join("\t"));
        }
        Err(anyhow!("a version line without signature"))
    }
}

/// Every identifier the prototype's rows carry: its EHR, the versioned
/// object, its contributions and its audits.
async fn identifiers(conn: &mut PgConnection, ehr: Uuid, vo: Uuid) -> anyhow::Result<Vec<Uuid>> {
    let mut ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT contribution_id FROM clinical.version WHERE vo_id = $1 \
         UNION SELECT commit_audit_id FROM clinical.version WHERE vo_id = $1 \
         UNION SELECT c.commit_audit_id FROM clinical.contribution c \
           JOIN clinical.version v ON v.contribution_id = c.id WHERE v.vo_id = $1",
    )
    .bind(vo)
    .fetch_all(&mut *conn)
    .await?;
    ids.push(vo);
    ids.push(ehr);
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// The served envelope of each version of `vo`, by `sys_version`, read
/// through the service as a client reads it.
async fn envelopes(
    svc: &FerroEhrService,
    lines: &BTreeMap<usize, Vec<String>>,
    ehr: Uuid,
    vo: Uuid,
) -> anyhow::Result<BTreeMap<i32, String>> {
    let place = TABLES
        .iter()
        .position(|(table, _)| *table == "version")
        .ok_or_else(|| anyhow!("no version table"))?;
    let mut out = BTreeMap::new();
    for line in lines.get(&place).into_iter().flatten() {
        let columns: Vec<&str> = line.split('\t').collect();
        let field = |at: usize| columns.get(at).copied().unwrap_or_default();
        let sys_version: i32 = field(SYS_VERSION_COLUMN).parse()?;
        let kind = field(2);
        let system = field(9);
        let trunk = field(5);
        let branch = field(6);
        let tree = if branch == "0" {
            trunk.to_owned()
        } else {
            format!("{trunk}.{branch}.{}", field(7))
        };
        let body = match kind {
            "COMPOSITION" => {
                let uid = format!("{vo}::{system}::{tree}");
                svc.composition_version_envelope(
                    EhrId(ehr),
                    uid.parse().map_err(|e| anyhow!("{e:?}"))?,
                )
                .await
                .with_context(|| format!("read the envelope of {uid}"))?
                .body
            }
            "EHR_STATUS" => {
                svc.ehr_status_version_envelope(EhrId(ehr), VoId(vo), &tree)
                    .await
                    .with_context(|| format!("read the EHR_STATUS envelope of {vo}"))?
                    .body
            }
            other => return Err(anyhow!("a prototype of kind {other}")),
        };
        out.insert(sys_version, body.to_string());
    }
    Ok(out)
}

/// The tables a chunk writes, in [`TABLES`] order, with their column lists.
pub(crate) fn copy_statements() -> impl Iterator<Item = (usize, String)> {
    TABLES.iter().enumerate().map(|(place, (table, columns))| {
        let list = if *columns == "*" {
            String::new()
        } else {
            format!(" ({columns})")
        };
        (place, format!("COPY clinical.{table}{list} FROM STDIN"))
    })
}

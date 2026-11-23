// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The budget of every hot path (#6007): per request, the representation
//! changes it makes, the allocations of the thread that drives it, its round
//! trips, the statements `PostgreSQL` runs nested in them and the write-ahead
//! log they write, over the assembled router with the access evaluator, the
//! audit drain and the local access log.
//!
//! The paths are the commit of a COMPOSITION in canonical JSON, canonical XML
//! and FLAT, its read in the same three formats, the EHR read, a single-EHR
//! and a population AQL query, a population `matches` over a value list
//! (#6084), a demographic population query (#6087), an ITI-81 page and a
//! party read, all over the
//! IPS composition (128 KB, 402 node rows), and the commit of that composition
//! as a CONTRIBUTION, its read at an instant, the `EHR_STATUS` commit and read,
//! the commit of a PERSON, plain and with ordinal and scale values, the
//! update and the delete of a COMPOSITION, the directory writes, and the
//! update and the delete of a PERSON and the create of an ORGANISATION,
//! GROUP, ROLE and AGENT (#6257).
//!
//! The conversions are the `trace` events under
//! [`openehr_its::conversion::TARGET`]: the codec entry points record each
//! one, and the server records the changes it makes itself
//! (`ferroehr::telemetry::conversion`). The allocations are counted on the
//! thread that polls the request, so the audit runtime's threads do not
//! count; the peak is the most the request held at once. The allocations
//! are judged per request over the measured window
//! ([`Measure::per_request`]), the peak on the lower-quartile request
//! ([`Measure::low`]). The domain pools hold one warm connection each and
//! every request starts past the caches' upkeep interval ([`settle`]), so
//! each request meets the same state.
//!
//! The bytes a request moves over the database wire are counted on their own
//! run, through a relay that splits the protocol messages by statement
//! ([`super::wire`], #6244), and held to [`WIRE_BUDGETS`].
//!
//! Each figure is held to the ceiling [`BUDGETS`] names: the measured end
//! state of the core rewrite (#6001) plus a small margin. A ceiling only ever
//! comes down, on an owner decision. No openEHR spec governs the cost of a
//! request: our own design.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};

use axum::Router;
use axum::body::Body;
use http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt as _;
use tracing::Instrument as _;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

use ferroehr::db::domain::{Domain, DomainPools};

use super::{CHAINER_RELEASE, calls, counted, drained};
use crate::access_evaluator_http::{credential, ehr_with_composition};
use crate::common::{self, BASE, FHIR};

/// The measured requests of each path.
pub(super) const REQUESTS: u64 = 10;

/// The requests of each path sent before the measured ones.
pub(super) const WARM: u64 = 3;

/// The EHRs the paths spread over, each with one IPS composition.
const EHRS: usize = 5;

/// The pages [`World::page`] reads before the records [`Path::Iti81`] serves
/// end, each writing one of them.
const PAGE_OWN: usize = 10;

/// The pause before each measured request, past the 300 ms after which a
/// `moka` cache runs its pending tasks on its next read (moka 0.12
/// `LOG_SYNC_INTERVAL_MILLIS`), so every request meets each cache in that
/// one state.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(400);

/// The span each measured request runs in.
const PATH_SPAN: &str = "hot_path_budget_request";

/// The prefix of the conversions that render the access record, which the
/// audit lane owns and the path's own budget leaves out.
const AUDIT: &str = "audit_";

/// The canonical JSON media type.
const JSON: &str = "application/json";
/// The canonical XML media type.
const XML: &str = "application/xml";
/// The FLAT media type (ITS-REST `Resources.md` §Simplified Formats).
const FLAT: &str = "application/openehr.wt.flat+json";

/// A hot path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Path {
    CommitJson,
    CommitXml,
    CommitFlat,
    ReadJson,
    ReadXml,
    ReadFlat,
    EhrRead,
    AqlSingle,
    AqlPopulation,
    AqlMatches,
    AqlParty,
    Iti81,
    PartyRead,
    CommitContribution,
    ReadAtTime,
    CommitStatus,
    ReadStatus,
    CommitParty,
    CommitPartyScored,
    ReadVersion,
    CommitUpdate,
    CommitDelete,
    DirectoryCreate,
    DirectoryUpdate,
    DirectoryDelete,
    PartyUpdate,
    PartyDelete,
    CommitOrganisation,
    CommitGroup,
    CommitRole,
    CommitAgent,
}

impl Path {
    pub(super) const ALL: [Self; 31] = [
        Self::CommitJson,
        Self::CommitXml,
        Self::CommitFlat,
        Self::ReadJson,
        Self::ReadXml,
        Self::ReadFlat,
        Self::EhrRead,
        Self::AqlSingle,
        Self::AqlPopulation,
        Self::AqlMatches,
        Self::AqlParty,
        Self::Iti81,
        Self::PartyRead,
        Self::CommitContribution,
        Self::ReadAtTime,
        Self::CommitStatus,
        Self::ReadStatus,
        Self::CommitParty,
        Self::CommitPartyScored,
        Self::ReadVersion,
        Self::CommitUpdate,
        Self::CommitDelete,
        Self::DirectoryCreate,
        Self::DirectoryUpdate,
        Self::DirectoryDelete,
        Self::PartyUpdate,
        Self::PartyDelete,
        Self::CommitOrganisation,
        Self::CommitGroup,
        Self::CommitRole,
        Self::CommitAgent,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::CommitJson => "commit JSON",
            Self::CommitXml => "commit XML",
            Self::CommitFlat => "commit FLAT",
            Self::ReadJson => "read JSON",
            Self::ReadXml => "read XML",
            Self::ReadFlat => "read FLAT",
            Self::EhrRead => "EHR read",
            Self::AqlSingle => "single-EHR AQL",
            Self::AqlPopulation => "population AQL",
            Self::AqlMatches => "population AQL matches",
            Self::AqlParty => "demographic AQL",
            Self::Iti81 => "ITI-81 page",
            Self::PartyRead => "party read",
            Self::CommitContribution => "contribution commit",
            Self::ReadAtTime => "read at time",
            Self::CommitStatus => "EHR_STATUS commit",
            Self::ReadStatus => "EHR_STATUS read",
            Self::CommitParty => "party commit",
            Self::CommitPartyScored => "party commit with scores",
            Self::ReadVersion => "read JSON from the version copy",
            Self::CommitUpdate => "update JSON",
            Self::CommitDelete => "delete",
            Self::DirectoryCreate => "directory create",
            Self::DirectoryUpdate => "directory update",
            Self::DirectoryDelete => "directory delete",
            Self::PartyUpdate => "party update",
            Self::PartyDelete => "party delete",
            Self::CommitOrganisation => "ORGANISATION commit",
            Self::CommitGroup => "GROUP commit",
            Self::CommitRole => "ROLE commit",
            Self::CommitAgent => "AGENT commit",
        }
    }
}

/// The ceilings of one path, per request.
struct Budget {
    path: Path,
    conversions: u64,
    allocations: u64,
    peak_bytes: u64,
    round_trips: u64,
    nested: u64,
    /// The `pg_notify` calls, top-level or nested.
    notifies: u64,
    wal: u64,
}

/// The ceilings by path. Before the core rewrite (feat/core-rewrite at
/// 13cda4c7f, debug build) the paths measured, per request, conversions /
/// allocations / round trips / nested statements / WAL bytes: recorded on
/// #6007. The end state (#6001) is the figure measured over four runs at
/// feat/core-rewrite dfed03d9d plus a margin: none on conversions and round
/// trips, 2% on allocations, 10% on the peak, 3% on WAL bytes, and one nested
/// statement on a commit. The nested statements leave out the chainer's
/// rounds, the release of the records the commits held among them. The
/// commits have since taken the `event_time` column and the magnitude index
/// (#6022): 246,564, 247,837, 248,829 and 248,242 WAL bytes measured against
/// 242,177, 243,204, 244,956 and 244,045, 1.8% more, under the same ceilings.
/// The conversions and the allocations of the commits and the raw reads are
/// the figures measured over four runs once a read left the stored audit
/// undecoded and a commit signed its version from typed members (#6037), with
/// the same margins.
/// The commits of a body the data-minimisation pass reads have since lost
/// their second render and the pass's own allocations (#6038): 18,841
/// allocations on the JSON commit against 33,307, measured over three runs.
/// A CONTRIBUTION commit has since read its envelope once from the text and
/// each member from its own text, and judged its committers as typed RM
/// (#6038): 6 conversions and 19,010 allocations against 9 and 51,858.
/// The WAL figure has since been the row records a request writes, read with
/// `pg_walinspect` as `statement_budget::wal` reads them, with the chain
/// heads and page maintenance left out and the lower-quartile request judged
/// (#6035): the sum `pg_stat_statements` gave moved with the chainer's
/// passes, and a read measured 2,248 to 2,325 bytes between runs. Twelve of
/// the sixteen paths now measure the same figure in every one of seven runs,
/// one of them beside 175 other tests, and the other four within 64 bytes,
/// so the ceilings are the largest of the seven plus 1%.
/// The four commits of the IPS composition have since taken the index on the
/// code of an ELEMENT value (#6033): 36 entries of 72 bytes, 2,592 bytes a
/// commit and 1.08%, inside the 3% that issue allows the index. Their WAL
/// ceilings rose by that, to the largest of three runs plus 1%: the one rise
/// of a ceiling, and the trade #6033 states.
/// The harness has since started the `LISTEN` sessions a server runs with,
/// and a request confirms the copies its decision rests on with two indexed
/// reads, one per store (#6048). The round trips are the figures measured
/// then: a read sends 3 (its own, the party store's revisions, the policy
/// heads) where the harness without listeners sent 7 and a server before
/// #6048 sent 1. A party commit moves the binding revision in its
/// transaction, one nested statement and 88 bytes of WAL, the second rise of
/// a ceiling and the price #6048 states for a decision that waits on no
/// notification.
/// The data-minimisation pass has since stopped reading string leaves for
/// the shape of a national identifier (#6081): 1,069 fewer allocations on
/// each of the three composition commits and 1,057 on a CONTRIBUTION commit,
/// measured beside the merge base, and their ceilings fell by as much.
/// The population `matches` over a value list (#6084) is measured last, so
/// its access records stay off the ITI-81 page; its ceilings are the figures
/// of three runs (5,637 to 5,639 allocations, a median peak of 174,788
/// bytes, 13,180 WAL bytes) with the margins above.
/// The demographic population query (#6087) runs as a caller with access
/// groups after the `matches`, for the same reason; its ceilings are the
/// figures of three runs (2,630 to 2,644 allocations, a median peak of
/// 98,022 bytes, 2,062 WAL bytes) with the same margins. It has since judged
/// no `EHR_ACCESS` condition (#6110): 2,619 to 2,621 allocations and a median
/// peak of 97,454 bytes over three runs, and its ceilings fell by as much.
/// The commit of a PERSON holding two ordinals and two scales holds the WAL
/// the index on their numbers costs (#6090): 4 entries of 72 bytes, 288 of
/// the 10,489 bytes it wrote in each of three runs. Its ceilings are the
/// figures of those runs (2,486 allocations, a median peak of at most 120,133
/// bytes, 20 nested statements) with the margins above; no other path moved.
/// It is measured last, so the PERSONs it commits stay out of the population
/// the demographic query reads, which they grew by some 100 allocations.
/// A Basic user's held roles and scopes have since been read once with the
/// grant rows (#6013): 34 to 54 fewer allocations on every path, each of
/// which authenticates a Basic user, measured beside the merge base, and
/// each ceiling fell by its path's figure.
/// The version-tree uniqueness has since been declared per partition without
/// the tier and served the read by version id, and the separate tree index is
/// gone (#6207): every commit that writes a version writes 96 bytes of WAL
/// fewer, measured beside the merge base, and each WAL ceiling fell by it.
/// The read by version id served from the version copy (#5778) is measured
/// last, on EHRs of its own whose settings name an object, so each read is
/// judged on its own row; its median request is a copy hit. The copy has
/// since held the body as shared text (#6205): a hit allocated 564 KiB with a
/// peak of 272,547 bytes before and 431 to 439 KiB with a peak of at most
/// 147,965 bytes after, over three runs, and a fill one copy of the body
/// less. Its ceilings are those figures with the margins above.
/// The token index has since taken the request and resource ids as keys of
/// its own, in place of two hash B-trees on each record of both chains
/// (#6202): 96 to 616 fewer WAL bytes on every path that writes an access
/// record or a system audit record, the read from the version copy among
/// them, measured beside the merge base, and each WAL ceiling fell by its
/// path's figure.
/// A node row has since stored its own path step, and the node key carries no
/// tier (#6204): 14,098, 14,016 and 13,880 bytes fewer on the JSON, XML and
/// FLAT commits of the IPS composition, 13,520 on a CONTRIBUTION commit, and
/// 64 and 136 on the two party commits, measured beside the merge base, and
/// each WAL ceiling fell by its path's figure.
/// A system audit record has since been written once, into `audit_event`,
/// with an id-list intake row and a narrow link the chainer adds, in place of
/// a whole intake row copied into the chain (#6203): 370 to 456 fewer WAL
/// bytes on each path that writes one (the party read and commits and the
/// demographic query), measured beside the merge base, and each of those WAL
/// ceilings fell by its path's figure.
/// A member repeated across a version's node rows has since been stored once
/// and referenced from the later rows (#6229): 42,162, 41,938 and 41,722
/// bytes fewer on the JSON, XML and FLAT commits of the IPS composition,
/// whose entries repeat one participation list, and 41,922 on a CONTRIBUTION
/// commit, measured beside the merge base, and each WAL ceiling fell by its
/// path's figure. No other path moved.
/// The access-log write has since taken no row lock on the shared part an
/// entry names (#6241): 54 bytes fewer for each access-log entry a path
/// writes (64 and 80 on the FLAT and XML commits), measured beside the merge
/// base, and each WAL ceiling fell by its path's figure; the paths that
/// write no entry of their own did not move.
/// A request has since confirmed its policy copies with one primary-key read
/// of `policy_revision` in place of the newest version of every register and
/// of the access policy (#6251): 8 fewer allocations on every path, measured
/// beside the merge base, and each allocation ceiling fell by as much.
/// A read of a COMPOSITION or an EHR has since confirmed its policy copies on
/// its own statement, which reads the policy and definition revisions beside
/// its rows, in place of a read of the policy revision before it (#6251): one
/// round trip fewer on the six read paths those operations serve, measured
/// beside the merge base, and each of those round-trip ceilings fell by one.
/// A COMPOSITION update (a PUT with new content) and a delete are measured
/// last, on the EHRs' own compositions and on compositions committed for
/// them, and a COMPOSITION write judges its pre-checks on one gate read
/// (#6256): an update sends 10 round trips where it sent 12, a delete 11
/// where it sent 16, and a CONTRIBUTION 12 where it sent 13, measured beside
/// the merge base, and the CONTRIBUTION's ceiling fell by one. The new paths'
/// ceilings are the figures of two runs (17,495 and 1,623 allocations, median
/// peaks of 1,386,366 and 134,395 bytes, 186,380 and 5,765 WAL bytes) with
/// the margins above.
/// The access-log entry has since been written once by a plain insert, with
/// no intake row, link, chain head or function around it (#6242): 559 bytes
/// fewer for each entry a read writes and 556 on each commit, measured
/// beside the merge base, and each WAL ceiling fell by its path's figure; the
/// reads and the queries run no nested statement any more, and their nested
/// ceilings fell to 0. A commit runs 4 nested statements where it ran 3, under
/// its ceiling, which does not move.
/// A COMPOSITION commit and a CONTRIBUTION commit have since been decided on
/// the policy revision their gate read returns, in place of a read of it
/// before (#6256): one round trip fewer on the three composition commits, the
/// update, the delete and the CONTRIBUTION commit, measured beside the merge
/// base, and each of those round-trip ceilings fell by one.
/// A COMPOSITION create, update and delete have since committed in one guarded
/// statement with their outbox row, and taken no per-object lock (#6256): 7
/// round trips where the commits sent 8, the update 9 and the delete 10, the
/// delete 8, and 25, 25, 31, 37 and 42 fewer allocations on the JSON, XML and
/// FLAT commits, the update and the delete, measured beside the merge base;
/// each of those ceilings fell by its path's figure.
/// A system audit record has since been written once with the content
/// digest its writer computes, and sealed by window, with no link, intake
/// row, chain head or row digest in the database (#6242): 5 nested
/// statements fewer on the party read, the demographic query and both party
/// commits, and 541, 537, 532 and 532 WAL bytes fewer, measured beside the
/// merge base, and each of those ceilings fell by its path's figure.
/// A CONTRIBUTION of COMPOSITION members has since read its EHR and targets
/// in one gate read after planning and committed in one guarded statement
/// (#6256): 7 round trips where it sent 11, measured beside the merge base,
/// and its round-trip ceiling fell by four; its allocations moved from 17,591
/// to 17,626, under their ceiling, which stays.
/// A COMPOSITION commit and a CONTRIBUTION of COMPOSITION members have since
/// written their access record before their one statement, which reserves it
/// beside their rows, with no transaction of their own (#6256): 4 round trips
/// where the commits, the update and the CONTRIBUTION sent 7 and the delete 5
/// where it sent 8, and 39, 45, 38, 45 and 44 fewer allocations on the JSON,
/// XML and FLAT commits, the update and the delete, measured beside the merge
/// base; each of those ceilings fell by its path's figure.
/// A held write's access-log entries have since gone in with what the
/// gathered writers know the store holds, its shared part left out and its
/// search tokens named by id (#6244): 4 nested statements fewer on the three
/// composition commits, the CONTRIBUTION commit, the `EHR_STATUS` commit, the
/// update and the delete, and 8 to 17 fewer allocations, measured beside the
/// merge base, and each of those ceilings fell by its path's figure.
/// The binding side table's trigger has since moved the binding revision and
/// notified only for a statement that changed a row (#6284): 2 nested
/// statements and 87 WAL bytes fewer on both party commits, measured beside
/// the merge base, and each of those ceilings fell by as much.
/// A COMPOSITION delete has since judged its SMART scope on the template its
/// gate read returns (#6271): 4 round trips where it sent 5, and 1,514
/// allocations where it made 1,533, measured twice beside the merge base, and
/// both ceilings fell by those figures.
/// An `EHR_STATUS` commit has since been judged on one gate read and written,
/// its promoted `ehr` columns with it, by one guarded statement after its
/// access record (#6257): 5 round trips where it sent 14, 1,575 allocations
/// where it made 1,707 and 54 WAL bytes fewer, measured twice beside the merge
/// base, and those ceilings fell by those figures. Its peak rose from 141,897
/// to 148,825 bytes, since the access record is rendered while the prepared
/// version is held; its ceiling is that figure plus 1%, the one rise the
/// orchestrator approved as the price of the records-first order.
/// An `EHR_STATUS` commit has since dropped the EHR's access copy only when it
/// moved the subject, the one part of the status the copy is revised by
/// (#6238): 4 round trips where it sent 5, 1,492 allocations where it made
/// 1,567, a median peak 3 KiB lower and 323 wire bytes fewer, measured beside
/// the merge base, and those ceilings fell by those figures.
/// A directory create, update and delete are measured last, each on an EHR of
/// its own with a composition its FOLDER references, and have since been
/// judged on one gate read and written by one guarded statement after their
/// access record, a new hierarchy's `ehr_folder` row with it (#6257): 4 round
/// trips where they sent 14, 13 and 12, 1,542, 1,518 and 1,388 allocations
/// where they made 1,687, 1,656 and 1,505, and 4,921, 4,731 and 4,088 wire
/// bytes where they moved 5,739, 5,551 and 4,787, measured beside the merge
/// base, with the same WAL. Their ceilings are the merge base's
/// figures with the margins above, lowered by those drops; their median peaks
/// rose from 136,915, 138,815 and 131,027 bytes to 145,604, 143,951 and
/// 133,880, since the access record is rendered while the prepared version is
/// held, inside the margin.
/// Every path has since counted its `pg_notify` calls, top-level or nested,
/// pinned at the counts measured on main 3bf5adead7: 1 on the `EHR_STATUS`
/// commit and on each PERSON commit, update and delete, 0 elsewhere. A PERSON
/// update and delete, each on a PERSON of its own, and an ORGANISATION,
/// GROUP, ROLE and AGENT create are measured last. A party create, update
/// and delete have since been judged on one gate read and committed behind
/// one guarded version statement after their access record (#6257): a PERSON
/// create sends 20 round trips where it sent 29, an update and a delete 20
/// where they sent 33, an ORGANISATION, GROUP, ROLE and AGENT create 12, 11,
/// 21 and 10 where they sent 20, 19, 28 and 17, with 111 to 216 fewer
/// allocations and the same WAL, measured in CI beside the merge base. The
/// new paths' ceilings are the merge base's figures with the margins above,
/// lowered by those drops, with their WAL over the largest of three runs.
/// Their median peaks rose by 6 to 10 KiB, since the access record is
/// rendered while the prepared version is held: the PERSON commits from
/// 115,228 and 130,009 bytes to 124,725 and 139,506, and each peak ceiling of
/// those paths is the figure measured plus 1%, the rise the orchestrator
/// approved as the price of the records-first order.
/// A party commit has since settled its access rows in two statements, every
/// delete and retirement in one and every insert in the next, where it sent
/// one statement per table (#6257): a PERSON create, update and delete send
/// 12 round trips where they sent 20 and run 3 nested statements where they
/// ran 6, with 141, 142, 141 and 144 fewer allocations on the two creates,
/// the update and the delete; an ORGANISATION, GROUP and ROLE create send
/// 10, 10 and 19 where they sent 12, 11 and 21, with 26, 10 and 19 fewer
/// allocations, measured twice beside the merge base, and each of those
/// ceilings fell by its path's figure. The AGENT create, which settles no
/// row, sends 10 as before and made 5 to 12 allocations more, and its
/// ceilings stay. No path's WAL, median peak or notifies moved.
/// A settle has since built no insert step that writes no row, and sends a
/// statement of one step as it is written (#6257): the AGENT create makes 4
/// fewer allocations, the PERSON create, update and delete and the
/// ORGANISATION create 2 fewer, the GROUP and ROLE creates 1, measured twice
/// beside the merge base, and each allocation ceiling fell by that figure.
/// An `EHR_STATUS` read and an AQL query have since confirmed their policy
/// copies on their own statement, the version read or the query's statement,
/// which reads the policy and definition revisions beside its rows, in place
/// of a read of the policy revision before it (#6320): 2 round trips where the
/// `EHR_STATUS` read and the population query sent 3, and 3 where the
/// single-EHR query and the `matches` sent 4, measured beside the merge base,
/// and each of those round-trip ceilings fell by one.
/// The access-log entry has since written its key and uid alone, its search
/// columns copied into the search index by the frontier round (#6319): each
/// path wrote as many WAL bytes fewer as its entries' index records, 614 on a
/// read, 598 on an EHR read and on each commit's own entry, 1,076 on the
/// single-EHR query, 3,300 on the population query, 3,404 on the matches
/// query and 478 on the ITI-81 page, measured beside the merge base, and each
/// WAL ceiling fell by as much. A composition commit's figure moves by a few
/// hundred bytes between runs, so its ceiling fell by its entry's drop, 593,
/// or by the measured drop where that was smaller.
/// A COMPOSITION read, by version id, at an instant or from the version
/// copy, an EHR read and an `EHR_STATUS` read have since read the party
/// store's revisions in their one clinical statement through
/// `party_door.party_revisions(held)`,
/// where the two domains share a database, in place of a read of
/// `binding_grant_revision` on the party pool (#6320): 1 round trip where
/// they sent 2, measured beside the merge base, and each of those round-trip
/// ceilings fell by one. The function runs as its owner, so `PostgreSQL`
/// never inlines it, and its one-row primary-key read counts as a nested
/// statement: those paths' nested ceilings rose from 0 to 1, the trade the
/// orchestrator approved for a round trip and a transaction. A party read
/// takes its revisions the same way on its first party statement: 7 round
/// trips where it sent 8, and its ceiling fell by one. Each step of such a
/// ride runs in an allocation of its own, so no peak or allocation ceiling
/// rose, and a path that does not ride holds 136 bytes less at its peak.
/// The access-log entry has since been stored as its facts and sealed over
/// them (#6322): each path wrote as many WAL bytes fewer as its entries' rows
/// shrank, 592 on a read, 312 on the single-EHR query and on a delete, 1,536
/// on the matches query, and each WAL ceiling fell by its measured drop, a
/// composition commit's by at most its entry's 312. A read allocates 9 times
/// fewer and the ITI-81 page 13 fewer, no longer reading four digest columns
/// nothing used, and those allocation ceilings fell by as much.
// NOTE: allocations carry a 3-allocation margin until #6434 makes them exact.
const BUDGETS: [Budget; 31] = [
    Budget {
        path: Path::CommitJson,
        conversions: 5,
        // NOTE: our own design (#6074), binding the held facts snapshot adds an allocation.
        allocations: 17_021,
        peak_bytes: 1_530_778,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 187_262,
    },
    Budget {
        path: Path::CommitXml,
        conversions: 5,
        allocations: 39_104,
        peak_bytes: 1_533_031,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 187_591,
    },
    Budget {
        path: Path::CommitFlat,
        conversions: 5,
        allocations: 31_318,
        peak_bytes: 1_528_525,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 188_061,
    },
    Budget {
        path: Path::ReadJson,
        conversions: 0,
        allocations: 910,
        peak_bytes: 617_268,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 775,
    },
    Budget {
        path: Path::ReadXml,
        conversions: 2,
        allocations: 10_199,
        peak_bytes: 1_017_140,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 775,
    },
    Budget {
        path: Path::ReadFlat,
        conversions: 2,
        allocations: 14_838,
        peak_bytes: 1_634_407,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 775,
    },
    Budget {
        path: Path::EhrRead,
        conversions: 1,
        allocations: 783,
        peak_bytes: 79_269,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 575,
    },
    Budget {
        path: Path::AqlSingle,
        conversions: 2,
        allocations: 1789,
        peak_bytes: 118_273,
        round_trips: 3,
        nested: 0,
        notifies: 0,
        wal: 1758,
    },
    Budget {
        path: Path::AqlPopulation,
        conversions: 2,
        allocations: 2436,
        peak_bytes: 158_823,
        round_trips: 2,
        nested: 0,
        notifies: 0,
        wal: 4762,
    },
    Budget {
        path: Path::Iti81,
        conversions: 0,
        allocations: 8856,
        peak_bytes: 624_026,
        round_trips: 3,
        nested: 0,
        notifies: 0,
        wal: 567,
    },
    Budget {
        path: Path::PartyRead,
        conversions: 0,
        allocations: 1058,
        peak_bytes: 78_848,
        round_trips: 7,
        nested: 1,
        notifies: 0,
        wal: 1035,
    },
    Budget {
        path: Path::CommitContribution,
        conversions: 6,
        allocations: 17_279,
        peak_bytes: 1_509_372,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 188_700,
    },
    Budget {
        path: Path::ReadAtTime,
        conversions: 0,
        allocations: 1002,
        peak_bytes: 701_748,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 775,
    },
    Budget {
        path: Path::CommitStatus,
        conversions: 4,
        allocations: 1217,
        peak_bytes: 147_242,
        round_trips: 4,
        nested: 0,
        notifies: 1,
        wal: 4725,
    },
    Budget {
        path: Path::ReadStatus,
        conversions: 0,
        allocations: 841,
        peak_bytes: 80_973,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 703,
    },
    Budget {
        path: Path::CommitParty,
        conversions: 8,
        allocations: 1713,
        peak_bytes: 125_972,
        round_trips: 12,
        nested: 3,
        notifies: 1,
        wal: 6511,
    },
    Budget {
        path: Path::AqlMatches,
        conversions: 2,
        allocations: 3867,
        peak_bytes: 192_267,
        round_trips: 3,
        nested: 0,
        notifies: 0,
        wal: 5274,
    },
    Budget {
        path: Path::AqlParty,
        conversions: 2,
        allocations: 2626,
        peak_bytes: 107_199,
        round_trips: 8,
        nested: 0,
        notifies: 0,
        wal: 1027,
    },
    Budget {
        path: Path::CommitPartyScored,
        conversions: 8,
        allocations: 2200,
        peak_bytes: 140_901,
        round_trips: 12,
        nested: 3,
        notifies: 1,
        wal: 9062,
    },
    Budget {
        path: Path::ReadVersion,
        conversions: 0,
        allocations: 997,
        peak_bytes: 162_800,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 775,
    },
    Budget {
        path: Path::CommitUpdate,
        conversions: 5,
        allocations: 17_061,
        peak_bytes: 1_525_458,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 186_756,
    },
    Budget {
        path: Path::CommitDelete,
        conversions: 2,
        allocations: 1166,
        peak_bytes: 135_002,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 4205,
    },
    Budget {
        path: Path::DirectoryCreate,
        conversions: 5,
        allocations: 1267,
        peak_bytes: 150_606,
        round_trips: 4,
        nested: 1,
        notifies: 0,
        wal: 5645,
    },
    Budget {
        path: Path::DirectoryUpdate,
        conversions: 4,
        allocations: 1238,
        peak_bytes: 152_696,
        round_trips: 4,
        nested: 1,
        notifies: 0,
        wal: 4740,
    },
    Budget {
        path: Path::DirectoryDelete,
        conversions: 2,
        allocations: 1116,
        peak_bytes: 144_129,
        round_trips: 4,
        nested: 1,
        notifies: 0,
        wal: 4013,
    },
    Budget {
        path: Path::PartyUpdate,
        conversions: 8,
        allocations: 1965,
        peak_bytes: 129_641,
        round_trips: 12,
        nested: 3,
        notifies: 1,
        wal: 6407,
    },
    Budget {
        path: Path::PartyDelete,
        conversions: 2,
        allocations: 1592,
        peak_bytes: 112_950,
        round_trips: 12,
        nested: 3,
        notifies: 1,
        wal: 4191,
    },
    Budget {
        path: Path::CommitOrganisation,
        conversions: 8,
        allocations: 1574,
        peak_bytes: 126_183,
        round_trips: 10,
        nested: 0,
        notifies: 0,
        wal: 6691,
    },
    Budget {
        path: Path::CommitGroup,
        conversions: 8,
        allocations: 1569,
        peak_bytes: 125_084,
        round_trips: 10,
        nested: 0,
        notifies: 0,
        wal: 6495,
    },
    Budget {
        path: Path::CommitRole,
        conversions: 9,
        allocations: 2141,
        peak_bytes: 126_256,
        round_trips: 19,
        nested: 5,
        notifies: 0,
        wal: 6808,
    },
    Budget {
        path: Path::CommitAgent,
        conversions: 8,
        allocations: 1560,
        peak_bytes: 126_026,
        round_trips: 10,
        nested: 0,
        notifies: 0,
        wal: 6582,
    },
];

/// The paths in the order [`BUDGETS`] measures them, which keeps the access
/// records and parties a later path writes out of what an earlier one reads.
pub(super) fn order() -> impl Iterator<Item = Path> {
    BUDGETS.iter().map(|budget| budget.path)
}

/// The bytes each path may move over the database wire per request, both
/// directions together, as [`every_hot_path_stays_within_its_wire_budget`]
/// counts them (#6244).
// NOTE: our own design (#6074), each path binds the held facts snapshot, 13 bytes.
const WIRE_BUDGETS: [(Path, u64); 31] = [
    (Path::CommitJson, 201_617),
    (Path::CommitXml, 201_617),
    (Path::CommitFlat, 201_613),
    (Path::ReadJson, 130_846),
    (Path::ReadXml, 130_846),
    (Path::ReadFlat, 130_846),
    (Path::EhrRead, 1567),
    (Path::AqlSingle, 4538),
    (Path::AqlPopulation, 6150),
    (Path::Iti81, 26_826),
    (Path::PartyRead, 5303),
    (Path::CommitContribution, 202_074),
    (Path::ReadAtTime, 130_877),
    (Path::CommitStatus, 4511),
    (Path::ReadStatus, 2772),
    (Path::CommitParty, 8264),
    (Path::AqlMatches, 15_649),
    (Path::AqlParty, 11_351),
    (Path::CommitPartyScored, 10_616),
    (Path::ReadVersion, 1696),
    (Path::CommitUpdate, 201_898),
    (Path::CommitDelete, 4089),
    (Path::DirectoryCreate, 4796),
    (Path::DirectoryUpdate, 4607),
    (Path::DirectoryDelete, 3964),
    (Path::PartyUpdate, 9192),
    (Path::PartyDelete, 7743),
    (Path::CommitOrganisation, 7582),
    (Path::CommitGroup, 7411),
    (Path::CommitRole, 11_206),
    (Path::CommitAgent, 7391),
];

/// Counts, inside a [`PATH_SPAN`] span, the `sqlx::query` events (the round
/// trips) and the conversion events by their `op`.
#[derive(Clone, Default)]
struct Counter(Arc<Mutex<Counts>>);

/// What a [`Counter`] saw.
#[derive(Default)]
struct Counts {
    round_trips: u64,
    conversions: BTreeMap<String, u64>,
    /// The round trips by the opening words of their statement.
    sent: BTreeMap<String, u64>,
}

/// Reads the `op` field of a conversion event, and the statement of a
/// `sqlx::query` event.
#[derive(Default)]
struct Op(String, String);

impl Visit for Op {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "op" => value.clone_into(&mut self.0),
            "db.statement" if !value.is_empty() => value.clone_into(&mut self.1),
            "summary" if self.1.is_empty() => value.clone_into(&mut self.1),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "op" && self.0.is_empty() {
            self.0 = format!("{value:?}");
        }
    }
}

/// Whether the report lists each path's round trips by statement: set it to
/// see them, and leave it off for the budget, since reading them allocates on
/// the measured thread.
const LIST_SENT: bool = false;

/// The opening words of `statement`, its whitespace collapsed.
fn opening(statement: &str) -> String {
    statement
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect()
}

impl<S> tracing_subscriber::Layer<S> for Counter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    // NOTE: our own design (#6321), the stage spans are left disabled, so this
    // budget counts what the request allocates with no stage marked.
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _: Context<'_, S>) -> bool {
        metadata.target() != ferroehr::telemetry::stage::TARGET
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let target = event.metadata().target();
        let conversion = target == openehr_its::conversion::TARGET;
        if !conversion && target != "sqlx::query" {
            return;
        }
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };
        let names: Vec<&str> = scope.map(|span| span.name()).collect();
        if !names.contains(&PATH_SPAN) {
            return;
        }
        let mut counts = self.0.lock().unwrap();
        if conversion {
            let mut op = Op::default();
            event.record(&mut op);
            *counts.conversions.entry(op.0).or_default() += 1;
        } else if !names.contains(&ferroehr::db::pool::SESSION_SETUP_SPAN) {
            counts.round_trips += 1;
            // The statements are read only on request: reading them allocates on
            // the measured thread.
            if LIST_SENT {
                let mut op = Op::default();
                event.record(&mut op);
                *counts.sent.entry(opening(&op.1)).or_default() += 1;
            }
        }
    }
}

/// The allocations made while a future was polled on whichever thread
/// polled it, and the most it held at once.
#[derive(Default)]
struct Allocated {
    count: u64,
    bytes: u64,
    held: i64,
    peak: i64,
}

/// A future whose every poll is measured into an [`Allocated`].
struct Measured<'a, F> {
    inner: Pin<Box<F>>,
    into: &'a mut Allocated,
}

impl<F: Future> Future for Measured<'_, F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<F::Output> {
        let this = &mut *self;
        let mut out = Poll::Pending;
        let info = allocation_counter::measure(|| out = this.inner.as_mut().poll(cx));
        this.into.count += info.count_total;
        this.into.bytes += info.bytes_total;
        this.into.peak = this
            .into
            .peak
            .max(this.into.held + i64::try_from(info.bytes_max).unwrap());
        this.into.held += info.bytes_current;
        out
    }
}

/// One request, owned so it can be built again for each send.
pub(super) struct Call {
    method: &'static str,
    uri: String,
    user: &'static str,
    accept: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Option<(&'static str, String)>,
}

impl Call {
    pub(super) fn request(&self) -> Request<Body> {
        let mut builder = Request::builder()
            .method(self.method)
            .uri(&self.uri)
            .header(header::AUTHORIZATION, credential(self.user))
            .header(header::ACCEPT, self.accept);
        for (name, value) in &self.headers {
            builder = builder.header(*name, value);
        }
        match &self.body {
            Some((content_type, body)) => builder
                .header(header::CONTENT_TYPE, *content_type)
                .body(Body::from(body.clone())),
            None => builder.body(Body::empty()),
        }
        .unwrap()
    }
}

/// Sends `request` and reads its whole answer.
pub(super) async fn exchange(
    app: &Router,
    request: Request<Body>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.expect("a response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    (status, headers, body.to_vec())
}

/// Sends `call` unmeasured; its status, headers and body.
async fn plain(app: &Router, call: &Call) -> (StatusCode, HeaderMap, String) {
    let (status, headers, body) = exchange(app, call.request()).await;
    (status, headers, String::from_utf8(body).unwrap())
}

/// The fixtures the paths read and write.
pub(super) struct World {
    ehrs: Vec<(String, String)>,
    xml: String,
    flat: String,
    template_id: String,
    party: String,
    person: String,
    /// The PERSON with two ordinals and two scales in its details.
    scored: String,
    /// An instant after every fixture commit, at microsecond width.
    at: String,
    /// The instant the records [`Path::Iti81`] pages through end at, set
    /// right before its requests ([`Self::page`]).
    page_at: Mutex<String>,
    /// The `EHR_STATUS` a status commit writes, without its `uid`.
    status: String,
    /// The current `EHR_STATUS` version uid of each EHR, which the next status
    /// commit names in `If-Match`.
    status_uids: Mutex<Vec<String>>,
    /// The EHRs whose settings make a read depend on the object, so it is
    /// judged on its own row and served from the version copy, each with the
    /// version uid of its composition; built right before their path.
    judged: Mutex<Vec<(String, String)>>,
    /// The current version uid of each EHR's composition, which the next
    /// update names in `If-Match`.
    update_uids: Mutex<Vec<String>>,
    /// The compositions the deletes remove, each with its EHR and the version
    /// uid the delete names; built right before their path.
    deletable: Mutex<Vec<(String, String)>>,
    /// The EHRs the directory paths write, one per request, each with the
    /// object id of its composition, which its FOLDER references, and its
    /// directory's current version uid; built right before their paths.
    directories: Mutex<Vec<(String, String, String)>>,
    /// The PERSONs the party updates and deletes write, one per request, each
    /// with its current version uid; built right before their paths.
    parties: Mutex<Vec<String>>,
}

/// The `ETag` value of a response, unquoted.
fn etag(headers: &HeaderMap) -> String {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

/// The versioned-object id an `ETag` names.
fn object_id(headers: &HeaderMap) -> String {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("an object id")
        .to_owned()
}

impl World {
    pub(super) async fn build(app: &Router) -> Self {
        let mut ehrs = Vec::new();
        for _ in 0..EHRS {
            ehrs.push(ehr_with_composition(app).await);
        }
        let (ehr, vo) = &ehrs[0];
        let read = |accept| Call {
            method: "GET",
            uri: format!("{BASE}/ehr/{ehr}/composition/{vo}"),
            user: "root",
            accept,
            headers: Vec::new(),
            body: None,
        };
        let (status, _, xml) = plain(app, &read(XML)).await;
        assert_eq!(status, StatusCode::OK, "the XML read: {xml}");
        let (status, _, flat) = plain(app, &read(FLAT)).await;
        assert_eq!(status, StatusCode::OK, "the FLAT read: {flat}");
        let template_id =
            common::ips_canonical_composition()["archetype_details"]["template_id"]["value"]
                .as_str()
                .expect("the IPS template id")
                .to_owned();
        let person = demographic_fixture("person");
        let (status, headers, body) = plain(
            app,
            &Call {
                method: "POST",
                uri: format!("{BASE}/demographic/person"),
                user: "root",
                accept: JSON,
                headers: Vec::new(),
                body: Some((JSON, person.clone())),
            },
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "the person: {body}");
        let mut status_uids = Vec::new();
        let mut status_body = String::new();
        for (ehr, _) in &ehrs {
            let (code, headers, body) = plain(
                app,
                &Call {
                    method: "GET",
                    uri: format!("{BASE}/ehr/{ehr}/ehr_status"),
                    user: "root",
                    accept: JSON,
                    headers: Vec::new(),
                    body: None,
                },
            )
            .await;
            assert_eq!(code, StatusCode::OK, "the EHR_STATUS: {body}");
            status_uids.push(etag(&headers));
            let mut value: Value = serde_json::from_str(&body).expect("the EHR_STATUS");
            value
                .as_object_mut()
                .expect("an object")
                .shift_remove("uid");
            status_body = value.to_string();
        }
        Self {
            xml,
            flat,
            template_id,
            party: object_id(&headers),
            scored: scored(&person),
            person,
            at: ferroehr::system_log::content_digest::instant(jiff::Timestamp::now()),
            update_uids: Mutex::new(
                ehrs.iter()
                    .map(|(_, vo)| format!("{vo}::{}::1", system_id(&status_uids)))
                    .collect(),
            ),
            ehrs,
            status: status_body,
            status_uids: Mutex::new(status_uids),
            judged: Mutex::new(Vec::new()),
            page_at: Mutex::new(String::new()),
            deletable: Mutex::new(Vec::new()),
            directories: Mutex::new(Vec::new()),
            parties: Mutex::new(Vec::new()),
        }
    }

    /// Commits the PERSONs [`Path::PartyUpdate`] updates and
    /// [`Path::PartyDelete`] then deletes, one per request.
    async fn parties(&self, app: &Router) {
        let mut parties = Vec::new();
        for _ in 0..WARM + REQUESTS {
            let (status, headers, body) =
                plain(app, &Self::party_commit(self.person.clone())).await;
            assert_eq!(status, StatusCode::CREATED, "a person to update: {body}");
            parties.push(etag(&headers));
        }
        *self.parties.lock().unwrap() = parties;
    }

    /// The `n`th party update or delete, of the `n`th PERSON
    /// [`Self::parties`] committed, an update naming its `n` in the name so
    /// each one stores new content.
    fn party_write(&self, path: Path, n: usize) -> Call {
        let tip = self.parties.lock().unwrap()[n].clone();
        if path == Path::PartyDelete {
            return Call {
                method: "DELETE",
                uri: format!("{BASE}/demographic/person/{tip}"),
                user: "root",
                accept: JSON,
                headers: Vec::new(),
                body: None,
            };
        }
        let vo = tip.split("::").next().expect("an object id");
        let mut body: Value = serde_json::from_str(&self.person).expect("the person fixture");
        body["name"]["value"] = Value::from(format!("person {n}"));
        Call {
            method: "PUT",
            uri: format!("{BASE}/demographic/person/{vo}"),
            user: "root",
            accept: JSON,
            headers: vec![
                ("Prefer", "return=minimal".to_owned()),
                ("If-Match", format!("\"{tip}\"")),
            ],
            body: Some((JSON, body.to_string())),
        }
    }

    /// The commit of `path`'s party kind from its fixture: an ORGANISATION,
    /// GROUP or AGENT as the corpus holds it, a ROLE performed by the world's
    /// PERSON.
    fn kind_commit(&self, path: Path) -> Call {
        let kind = match path {
            Path::CommitOrganisation => "organisation",
            Path::CommitGroup => "group",
            Path::CommitRole => "role",
            _ => "agent",
        };
        let mut body: Value = serde_json::from_str(&demographic_fixture(kind)).expect("a fixture");
        if kind == "role" {
            body["performer"]["id"]["value"] = Value::from(self.party.clone());
        }
        Call {
            method: "POST",
            uri: format!("{BASE}/demographic/{kind}"),
            user: "root",
            accept: JSON,
            headers: vec![("Prefer", "return=minimal".to_owned())],
            body: Some((JSON, body.to_string())),
        }
    }

    /// Builds what `path` reads or writes besides the shared fixtures, right
    /// before its requests.
    pub(super) async fn prepare(
        &self,
        app: &Router,
        svc: &ferroehr::service::FerroEhrService,
        path: Path,
    ) {
        match path {
            Path::Iti81 => self.page(app).await,
            Path::ReadVersion => self.judge_on_objects(app, svc).await,
            Path::CommitDelete => self.deletable(app).await,
            Path::DirectoryCreate => self.directories(app).await,
            Path::PartyUpdate => self.parties(app).await,
            _ => {}
        }
    }

    /// Sends the [`WARM`] requests of `path` that go before its measured ones.
    pub(super) async fn warm(&self, app: &Router, path: Path) {
        for n in 0..WARM {
            let n = usize::try_from(n).unwrap();
            let (status, headers, body) = plain(app, &self.call(path, n)).await;
            assert!(status.is_success(), "{}: {status} {body}", path.name());
            self.observe(path, n, &headers);
        }
    }

    /// Writes the records of [`PAGE_OWN`] pages, and ends the records
    /// [`Path::Iti81`] serves there: its own ten and the population query's
    /// ten newest, the same twenty on every request.
    async fn page(&self, app: &Router) {
        for _ in 0..PAGE_OWN {
            let (status, _, body) = plain(app, &Self::iti81(None)).await;
            assert_eq!(status, StatusCode::OK, "a page: {body}");
        }
        *self.page_at.lock().unwrap() =
            ferroehr::system_log::content_digest::instant(jiff::Timestamp::now());
    }

    /// An ITI-81 page of the records up to `upto`, or of every record.
    fn iti81(upto: Option<&str>) -> Call {
        let upto = upto.map_or_else(String::new, |at| format!("&date=le{at}"));
        Call {
            method: "GET",
            uri: format!("{FHIR}/AuditEvent?date=ge2026-01-01{upto}&_count=20"),
            user: "root",
            accept: "application/fhir+json",
            headers: Vec::new(),
            body: None,
        }
    }

    /// Builds the EHRs of the directory paths, one per request, each with a
    /// composition its FOLDER references.
    async fn directories(&self, app: &Router) {
        let mut directories = Vec::new();
        for _ in 0..WARM + REQUESTS {
            let (ehr, vo) = ehr_with_composition(app).await;
            directories.push((ehr, vo, String::new()));
        }
        *self.directories.lock().unwrap() = directories;
    }

    /// The `n`th request of a directory path: a create, an update or a delete
    /// of the directory of the `n`th EHR [`Self::directories`] built.
    fn directory(&self, path: Path, n: usize) -> Call {
        let (ehr, vo, tip) = self.directories.lock().unwrap()[n].clone();
        let folder = serde_json::json!({
            "_type": "FOLDER",
            "name": { "_type": "DV_TEXT", "value": format!("directory {n}") },
            "archetype_node_id": "openEHR-EHR-FOLDER.directory.v1",
            "archetype_details": { "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID",
                    "value": "openEHR-EHR-FOLDER.directory.v1" },
                "rm_version": "1.1.0" },
            "items": [{ "_type": "OBJECT_REF", "namespace": "local", "type": "COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": vo } }],
        });
        let mut headers = vec![("Prefer", "return=minimal".to_owned())];
        if path != Path::DirectoryCreate {
            headers.push(("If-Match", format!("\"{tip}\"")));
        }
        let (method, body) = match path {
            Path::DirectoryCreate => ("POST", Some((JSON, folder.to_string()))),
            Path::DirectoryUpdate => ("PUT", Some((JSON, folder.to_string()))),
            _ => ("DELETE", None),
        };
        Call {
            method,
            uri: format!("{BASE}/ehr/{ehr}/directory"),
            user: "root",
            accept: JSON,
            headers,
            body,
        }
    }

    /// Builds the EHRs of [`Path::ReadVersion`]: each restricts an object it
    /// does not hold, so its settings name an object and a read of its
    /// composition by version id is judged on the object's own row (#5959)
    /// and read from the version copy (#5778), which one read of each fills.
    async fn judge_on_objects(&self, app: &Router, svc: &ferroehr::service::FerroEhrService) {
        let mut judged = Vec::new();
        for _ in 0..EHRS {
            let (ehr, vo) = ehr_with_composition(app).await;
            let elsewhere = uuid::Uuid::now_v7().to_string();
            let restriction = common::access_directive("restriction", &[elsewhere.as_str()]);
            common::commit_access_settings(
                svc,
                ehr.parse().expect("an EHR id"),
                common::directive_settings(&[restriction]),
            )
            .await;
            let version = format!("{vo}::{}::1", self.system_id());
            judged.push((ehr, version));
        }
        *self.judged.lock().unwrap() = judged;
        for n in 0..EHRS {
            let (status, _, body) = plain(app, &self.read_version(n)).await;
            assert_eq!(status, StatusCode::OK, "a read that fills the copy: {body}");
        }
    }

    /// The system id the server stamps into the version uids it writes.
    fn system_id(&self) -> String {
        system_id(&self.status_uids.lock().unwrap())
    }

    /// Commits the compositions [`Path::CommitDelete`] removes, one per
    /// request, spread over the EHRs.
    async fn deletable(&self, app: &Router) {
        let mut deletable = Vec::new();
        for n in 0..usize::try_from(WARM + REQUESTS).unwrap() {
            let (ehr, _) = &self.ehrs[n % self.ehrs.len()];
            let (status, headers, body) = plain(
                app,
                &Call {
                    method: "POST",
                    uri: format!("{BASE}/ehr/{ehr}/composition"),
                    user: "root",
                    accept: JSON,
                    headers: vec![("Prefer", "return=minimal".to_owned())],
                    body: Some((JSON, common::ips_canonical_composition().to_string())),
                },
            )
            .await;
            assert_eq!(
                status,
                StatusCode::CREATED,
                "a composition to delete: {body}"
            );
            deletable.push((ehr.clone(), etag(&headers)));
        }
        *self.deletable.lock().unwrap() = deletable;
    }

    /// Records what the answer to the `n`th request of `path` changed.
    pub(super) fn observe(&self, path: Path, n: usize, headers: &HeaderMap) {
        match path {
            Path::CommitStatus => {
                self.status_uids.lock().unwrap()[n % self.ehrs.len()] = etag(headers);
            }
            Path::CommitUpdate => {
                self.update_uids.lock().unwrap()[n % self.ehrs.len()] = etag(headers);
            }
            Path::DirectoryCreate | Path::DirectoryUpdate => {
                self.directories.lock().unwrap()[n].2 = etag(headers);
            }
            Path::PartyUpdate => {
                self.parties.lock().unwrap()[n] = etag(headers);
            }
            _ => {}
        }
    }

    /// The `n`th delete, of a composition [`Self::deletable`] committed.
    fn delete(&self, n: usize) -> Call {
        let (ehr, version) = self.deletable.lock().unwrap()[n].clone();
        Call {
            method: "DELETE",
            uri: format!("{BASE}/ehr/{ehr}/composition/{version}"),
            user: "root",
            accept: JSON,
            headers: Vec::new(),
            body: None,
        }
    }

    /// The `n`th update, of the composition of the EHR `ehr`, with a start
    /// time of its own so each one stores new content.
    fn update(&self, ehr: &str, n: usize) -> Call {
        let tip = self.update_uids.lock().unwrap()[n % self.ehrs.len()].clone();
        let vo = tip.split("::").next().expect("an object id");
        let mut body = common::ips_canonical_composition();
        body.as_object_mut().expect("an object").shift_remove("uid");
        body["context"]["start_time"]["value"] =
            Value::from(format!("2021-12-03T17:34:{:02}.849379+01:00", n % 60));
        Call {
            method: "PUT",
            uri: format!("{BASE}/ehr/{ehr}/composition/{vo}"),
            user: "root",
            accept: JSON,
            headers: vec![
                ("Prefer", "return=minimal".to_owned()),
                ("If-Match", format!("\"{tip}\"")),
            ],
            body: Some((JSON, body.to_string())),
        }
    }

    /// The `n`th status commit, to the EHR `ehr`.
    fn status_commit(&self, ehr: &str, n: usize) -> Call {
        let tip = self.status_uids.lock().unwrap()[n % self.ehrs.len()].clone();
        Call {
            method: "PUT",
            uri: format!("{BASE}/ehr/{ehr}/ehr_status"),
            user: "root",
            accept: JSON,
            headers: vec![
                ("Prefer", "return=minimal".to_owned()),
                ("If-Match", format!("\"{tip}\"")),
            ],
            body: Some((JSON, self.status.clone())),
        }
    }

    /// The `n`th request of `path`.
    pub(super) fn call(&self, path: Path, n: usize) -> Call {
        let (ehr, vo) = &self.ehrs[n % self.ehrs.len()];
        let minimal = vec![("Prefer", "return=minimal".to_owned())];
        let commit = |content_type, body: String, mut headers: Vec<(&'static str, String)>| {
            headers.extend(minimal.clone());
            Call {
                method: "POST",
                uri: format!("{BASE}/ehr/{ehr}/composition"),
                user: "root",
                accept: JSON,
                headers,
                body: Some((content_type, body)),
            }
        };
        let get = |uri: String, user, accept| Call {
            method: "GET",
            uri,
            user,
            accept,
            headers: Vec::new(),
            body: None,
        };
        let composition = format!("{BASE}/ehr/{ehr}/composition/{vo}");
        let aql = |body: Value| Call {
            method: "POST",
            uri: format!("{BASE}/query/aql"),
            user: "nurse",
            accept: JSON,
            headers: Vec::new(),
            body: Some((JSON, body.to_string())),
        };
        match path {
            Path::CommitJson => commit(
                JSON,
                common::ips_canonical_composition().to_string(),
                Vec::new(),
            ),
            Path::CommitXml => commit(XML, self.xml.clone(), Vec::new()),
            Path::CommitFlat => commit(
                FLAT,
                self.flat.clone(),
                vec![("openehr-template-id", self.template_id.clone())],
            ),
            Path::ReadJson => get(composition, "nurse", JSON),
            Path::ReadXml => get(composition, "nurse", XML),
            Path::ReadFlat => get(composition, "nurse", FLAT),
            Path::EhrRead => get(format!("{BASE}/ehr/{ehr}"), "nurse", JSON),
            Path::AqlSingle => aql(serde_json::json!({
                "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr",
                "query_parameters": { "ehr": ehr },
            })),
            Path::AqlPopulation => {
                aql(serde_json::json!({ "q": "SELECT e/ehr_id/value FROM EHR e LIMIT 20" }))
            }
            Path::AqlMatches => aql(serde_json::json!({
                "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
                      CONTAINS ELEMENT el[at0020] WHERE el/value/value \
                      matches {'Medication item 83', $item}",
                "query_parameters": { "item": "Medication item 84" },
            })),
            Path::AqlParty => aql(serde_json::json!({
                "q": "SELECT p/uid/value, p/name/value FROM PERSON p LIMIT 20",
            })),
            Path::Iti81 => Self::iti81(Some(&self.page_at.lock().unwrap())),
            Path::PartyRead => get(
                format!("{BASE}/demographic/person/{}", self.party),
                "root",
                JSON,
            ),
            Path::CommitContribution => Call {
                method: "POST",
                uri: format!("{BASE}/ehr/{ehr}/contribution"),
                user: "root",
                accept: JSON,
                headers: minimal.clone(),
                body: Some((JSON, contribution().to_string())),
            },
            Path::ReadAtTime => get(
                format!("{composition}?version_at_time={}", self.at),
                "nurse",
                JSON,
            ),
            Path::CommitStatus => self.status_commit(ehr, n),
            Path::ReadStatus => get(format!("{BASE}/ehr/{ehr}/ehr_status"), "nurse", JSON),
            Path::CommitUpdate => self.update(ehr, n),
            Path::CommitDelete => self.delete(n),
            Path::DirectoryCreate | Path::DirectoryUpdate | Path::DirectoryDelete => {
                self.directory(path, n)
            }
            Path::ReadVersion => self.read_version(n),
            _ => self.party_call(path, n),
        }
    }

    /// The `n`th request of a party path.
    fn party_call(&self, path: Path, n: usize) -> Call {
        match path {
            Path::CommitParty => Self::party_commit(self.person.clone()),
            Path::CommitPartyScored => Self::party_commit(self.scored.clone()),
            Path::PartyUpdate | Path::PartyDelete => self.party_write(path, n),
            _ => self.kind_commit(path),
        }
    }

    /// The `n`th read by version id, of a composition of the EHRs
    /// [`Self::judge_on_objects`] built.
    fn read_version(&self, n: usize) -> Call {
        let (ehr, version) = self.judged.lock().unwrap()[n % EHRS].clone();
        Call {
            method: "GET",
            uri: format!("{BASE}/ehr/{ehr}/composition/{version}"),
            user: "nurse",
            accept: JSON,
            headers: Vec::new(),
            body: None,
        }
    }

    /// A commit of the `PERSON` `body`.
    fn party_commit(body: String) -> Call {
        Call {
            method: "POST",
            uri: format!("{BASE}/demographic/person"),
            user: "root",
            accept: JSON,
            headers: vec![("Prefer", "return=minimal".to_owned())],
            body: Some((JSON, body)),
        }
    }
}

/// The `v1` demographic fixture of the party `kind` in the corpus.
fn demographic_fixture(kind: &str) -> String {
    std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../corpus/fixtures/demographic/{kind}.v1.json")),
    )
    .expect("the demographic fixture")
}

/// The system id in the version uids `uids`.
fn system_id(uids: &[String]) -> String {
    uids[0].split("::").nth(1).expect("a system id").to_owned()
}

/// `person` with details holding two `DV_ORDINAL` and two `DV_SCALE`
/// values, whose numbers `idx_node_hot_value_number` indexes (#6090).
fn scored(person: &str) -> String {
    let mut body: Value = serde_json::from_str(person).expect("the person fixture");
    let symbol = |code: &str| {
        serde_json::json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": code } })
    };
    let element = |at: &str, value: Value| {
        serde_json::json!({ "_type": "ELEMENT", "archetype_node_id": at,
            "name": { "_type": "DV_TEXT", "value": at }, "value": value })
    };
    body["details"] = serde_json::json!({
        "_type": "ITEM_TREE",
        "archetype_node_id": "at0100",
        "name": { "_type": "DV_TEXT", "value": "details" },
        "items": [
            element("at0101", serde_json::json!({ "_type": "DV_ORDINAL", "value": 1, "symbol": symbol("at0111") })),
            element("at0102", serde_json::json!({ "_type": "DV_ORDINAL", "value": 3, "symbol": symbol("at0113") })),
            element("at0103", serde_json::json!({ "_type": "DV_SCALE", "value": 2.5, "symbol": symbol("at0122") })),
            element("at0104", serde_json::json!({ "_type": "DV_SCALE", "value": 4.5, "symbol": symbol("at0124") })),
        ],
    });
    body.to_string()
}

/// A CONTRIBUTION that creates the IPS composition.
fn contribution() -> Value {
    let coded = |code: &str, value: &str| {
        serde_json::json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": code } })
    };
    let committer = serde_json::json!({ "_type": "PARTY_IDENTIFIED",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": ferroehr::service::DEFAULT_SYSTEM_ID, "id": "root" }] });
    serde_json::json!({
        "_type": "CONTRIBUTION",
        "versions": [ {
            "_type": "ORIGINAL_VERSION",
            "commit_audit": { "change_type": coded("249", "creation"), "committer": committer },
            "lifecycle_state": coded("532", "complete"),
            "data": common::ips_canonical_composition(),
        } ],
        "audit": { "change_type": coded("249", "creation"), "committer": committer },
    })
}

/// What one path measured over [`REQUESTS`] requests.
#[derive(Default)]
struct Measure {
    conversions: BTreeMap<String, u64>,
    /// The allocations, the bytes allocated and the peak of each request.
    each: Vec<Allocated>,
    round_trips: u64,
    nested: u64,
    notifies: u64,
    /// The row records of the lower-quartile request.
    wal: u64,
    /// The row records of each request and of the window by relation, with
    /// the chain heads' the figure leaves out.
    wal_ledger: String,
    /// The statements `PostgreSQL` ran nested, and the chainer's rounds the
    /// count leaves out.
    statements: String,
    /// The bytes each request moved over the database wire.
    wire_each: Vec<u64>,
    /// The bytes the window moved over the database wire, by statement.
    wire: super::wire::Ledger,
}

impl Measure {
    /// The window's `figure` per request: a buffer or block one request
    /// grows serves the next, so the window's sum is what each request owes.
    fn per_request(&self, figure: impl Fn(&Allocated) -> u64) -> u64 {
        self.each
            .iter()
            .map(figure)
            .sum::<u64>()
            .div_euclid(REQUESTS)
    }

    /// The lower-quartile request's `figure`, the third smallest of ten.
    ///
    /// A request handed a pool connection whose buffers have not grown to
    /// the path's messages yet, or that opens one, grows them inside its
    /// window, 8 KiB a step, with the allocations around it: on CI the party
    /// read made 974 allocations with a peak of 72,870 bytes on four requests
    /// of ten and up to 1,234 and 101,773 on the others, and the demographic
    /// query 2,543 against up to 2,682, so the median crossed a ceiling
    /// whenever half a window met such connections. Which requests do depends
    /// on what the pool hands out; what a request itself allocates more,
    /// every request allocates more, so the lower quartile still moves with
    /// it (#6238).
    fn low(&self, figure: impl Fn(&Allocated) -> u64) -> u64 {
        // NOTE: our own design (#6007), a connection the pool opens inside a request is the
        // pool's work, as its session setup is for the round trips (#5899).
        self.ranked(figure, 4)
    }

    /// The `figure` of the request at `1 / of` of the window, smallest first.
    fn ranked(&self, figure: impl Fn(&Allocated) -> u64, of: usize) -> u64 {
        let mut figures: Vec<u64> = self.each.iter().map(figure).collect();
        figures.sort_unstable();
        figures
            .get(figures.len().div_euclid(of))
            .copied()
            .unwrap_or(0)
    }

    /// The allocations and the peak of each request, for the report.
    fn requests(&self) -> Vec<(u64, i64, u64)> {
        self.each
            .iter()
            .map(|a| (a.count, a.peak, a.bytes))
            .collect()
    }

    /// The conversions of the request's own body and answer, per request.
    fn conversions(&self) -> u64 {
        self.of(|op| !op.starts_with(AUDIT))
    }

    /// The renders of the request's access record, per request.
    fn audit(&self) -> u64 {
        self.of(|op| op.starts_with(AUDIT))
    }

    fn of(&self, keep: impl Fn(&str) -> bool) -> u64 {
        self.conversions
            .iter()
            .filter(|(op, _)| keep(op))
            .map(|(_, n)| n)
            .sum::<u64>()
            .div_euclid(REQUESTS)
    }
}

/// Whether `relation` is a seal or owing state of either log, which a
/// chainer pass writes once whatever it seals, so how often depends on how
/// the passes fall among the requests.
fn head_relation(relation: &str) -> bool {
    super::wal::pass_relation(relation)
}

/// The row records of the lower-quartile request, and the ledger of the
/// window: `marks` are the WAL positions before the first request and after
/// each one, taken once the chainer had moved that request's entries.
///
/// The figure is what [`super::wal`] counts for an access-log entry: the
/// records a statement writes for each row of the test's own database, read
/// with `pg_walinspect`. Page maintenance and the records of the chain heads
/// are left out, since both fall on a request by timing (#6035).
///
/// A low request is judged, the third smallest of ten, because timing still
/// moves single requests. An update that finds no room on its row's page
/// writes the whole row, a lock record and an entry in each index, and
/// whether it does depends on what pruning could clear, so on the oldest
/// transaction open on the shared server: beside the other budget tests a
/// window of ten `EHR_STATUS` commits wrote 1,550 bytes more. An index insert
/// that splits its page is logged as the split and leaves the count. What a
/// request itself writes more, every request writes more.
async fn row_records(pool: &PgPool, marks: &[String]) -> (u64, String) {
    let (Some(from), Some(to)) = (marks.first(), marks.last()) else {
        return (0, String::new());
    };
    super::wal::flushed(pool, to).await;
    let mut each = Vec::new();
    for pair in marks.windows(2) {
        let [before, after] = pair else { continue };
        each.push(
            super::wal::by_relation(pool, before, after)
                .await
                .into_iter()
                .filter(|(relation, _, _)| !head_relation(relation))
                .map(|(_, bytes, _)| bytes)
                .sum::<u64>(),
        );
    }
    let mut ledger = format!("  each request: {each:?}\n");
    for (relation, bytes, records) in super::wal::by_relation(pool, from, to).await {
        let head = head_relation(&relation);
        writeln!(
            ledger,
            "  {bytes:>8} B {records:>5}  {relation}{}",
            if head {
                "  (a chain head, left out)"
            } else {
                ""
            }
        )
        .unwrap();
    }
    writeln!(
        ledger,
        "  {} rows updated in place",
        updates_in_place(pool, from, to).await
    )
    .unwrap();
    each.sort_unstable();
    let judged = each.get(each.len().div_euclid(4)).copied().unwrap_or(0);
    (judged, ledger)
}

/// The rows the window updated in place, outside the chain heads and the
/// catalogs.
///
/// An update is the one row record whose size is not the statement's alone:
/// it is short while the new version fits the old one's page, and where it
/// does not it carries the whole row, a lock record before it and an entry in
/// each index. Which it is depends on the room pruning has left on the page,
/// and so on how long the oldest transaction of the server has been open. The
/// report prints the count so a path's ceiling can be read against it.
async fn updates_in_place(pool: &PgPool, from: &str, to: &str) -> u64 {
    let updates: i64 = sqlx::query_scalar(
        "SELECT /* statement_budget_own */ count(DISTINCT b.start_lsn) \
           FROM pg_get_wal_block_info($1::pg_lsn, $2::pg_lsn) b \
           JOIN pg_class c ON pg_relation_filenode(c.oid) = b.relfilenode \
          WHERE b.reldatabase = (SELECT oid FROM pg_database WHERE datname = current_database()) \
            AND b.resource_manager = 'Heap' \
            AND split_part(b.record_type, '+', 1) IN ('UPDATE', 'HOT_UPDATE') \
            AND c.relkind = 'r' \
            AND c.relname NOT IN ('access_log_seal_state', 'audit_event_seal_state', \
                                  'delivery_frontier') \
            AND c.relnamespace NOT IN \
                ('pg_catalog'::regnamespace, 'information_schema'::regnamespace)",
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .expect("pg_walinspect reads");
    u64::try_from(updates).expect("a count")
}

/// Waits [`SETTLE`], then takes and returns the connection of each domain
/// pool, so the measured request meets it idle for less than the pool's
/// idle probe (`ferroehr::db::pool::IDLE_PROBE`) and sends no probe of its
/// own.
async fn settle(pools: &DomainPools) {
    tokio::time::sleep(SETTLE).await;
    for domain in Domain::ALL {
        drop(
            ferroehr::db::acquire::acquire(pools.get(domain))
                .await
                .expect("the kept connection"),
        );
    }
}

/// Measures [`REQUESTS`] requests of `path`.
async fn measure(
    app: &Router,
    pool: &PgPool,
    pools: &DomainPools,
    wire: Option<&super::wire::Wire>,
    world: &World,
    counter: &Counter,
    path: Path,
) -> Measure {
    world.warm(app, path).await;
    drained(pool).await;
    let evicted = testkit::statements::statement_window(pool)
        .await
        .expect("the statement counts reset");
    *counter.0.lock().unwrap() = Counts::default();
    let mut marks = vec![super::wal::insert_lsn(pool).await];
    let mut out = Measure::default();
    if let Some(wire) = wire {
        drop(wire.take());
    }
    for n in 0..REQUESTS {
        let n = usize::try_from(WARM + n).unwrap();
        let call = world.call(path, n);
        settle(pools).await;
        let mut allocated = Allocated::default();
        let (status, headers, body) = Measured {
            inner: Box::pin(
                exchange(app, call.request()).instrument(tracing::info_span!(PATH_SPAN)),
            ),
            into: &mut allocated,
        }
        .await;
        assert!(
            status.is_success(),
            "{}: {status} {}",
            path.name(),
            String::from_utf8_lossy(&body)
        );
        world.observe(path, n, &headers);
        out.each.push(allocated);
        drained(pool).await;
        marks.push(super::wal::insert_lsn(pool).await);
        if let Some(wire) = wire {
            let moved = wire.take();
            out.wire_each.push(moved.total().bytes());
            out.wire.merge(&moved);
        }
    }
    let statements = counted(pool).await;
    let nested: u64 = statements
        .iter()
        .filter(|s| !s.toplevel)
        .map(|s| s.calls)
        .sum();
    let notifies: u64 = statements
        .iter()
        .filter(|s| s.query.contains("pg_notify"))
        .map(|s| s.calls)
        .sum();
    let chainer = calls(&statements, "SELECT audit.audit_intake_release") * CHAINER_RELEASE;
    let counts = std::mem::take(&mut *counter.0.lock().unwrap());
    out.conversions = counts.conversions;
    out.round_trips = counts.round_trips.div_euclid(REQUESTS);
    out.nested = nested.saturating_sub(chainer).div_euclid(REQUESTS);
    out.notifies = notifies.div_euclid(REQUESTS);
    out.statements = sent_ledger(&counts.sent) + &nested_ledger(&statements, nested, chainer);
    (out.wal, out.wal_ledger) = row_records(pool, &marks).await;
    testkit::statements::assert_statements_kept(pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    out
}

/// The round trips of a window by their statement, per request.
fn sent_ledger(sent: &BTreeMap<String, u64>) -> String {
    let mut out = format!("round trips by statement over {REQUESTS} requests:\n");
    for (statement, n) in sent {
        writeln!(out, "  {n:>5}  {statement}").unwrap();
    }
    out
}

/// The nested statements of a window, and what the chainer's rounds take
/// out of their count.
fn nested_ledger(statements: &[super::Counted], nested: u64, chainer: u64) -> String {
    let mut out = format!("{nested} nested, {chainer} of them the chainer's\n");
    for s in statements {
        let chain = s.toplevel && s.query.contains("_chain_");
        if !s.toplevel || chain {
            let level = if s.toplevel { "top" } else { "nested" };
            let text: String = s.query.chars().take(140).collect();
            writeln!(out, "  {level:>6} {:>4}  {text}", s.calls).unwrap();
        }
    }
    out
}

/// The median of `figures`.
fn median(figures: &[u64]) -> u64 {
    let mut figures = figures.to_vec();
    figures.sort_unstable();
    figures
        .get(figures.len().div_euclid(2))
        .copied()
        .unwrap_or(0)
}

/// The database wire of a window by statement, per request: calls, bytes
/// sent and bytes received.
fn wire_ledger(m: &Measure) -> String {
    let total = m.wire.total();
    let mut out = format!(
        "database wire per request (each request {:?}): {} sent, {} received; \
         by statement, the calls over {REQUESTS} requests and the bytes per request:\n",
        m.wire_each,
        total.sent.div_euclid(REQUESTS),
        total.received.div_euclid(REQUESTS),
    );
    let mut lines: Vec<_> = m
        .wire
        .own
        .iter()
        .map(|line| ("", line))
        .chain(m.wire.background.iter().map(|line| ("(background) ", line)))
        .collect();
    lines.sort_by_key(|(_, (_, tally))| std::cmp::Reverse(tally.bytes()));
    for (apart, (statement, tally)) in lines {
        let text: String = apart.chars().chain(statement.chars()).take(160).collect();
        writeln!(
            out,
            "  {:>5} calls {:>8} B sent {:>8} B received  {text}",
            tally.calls,
            tally.sent.div_euclid(REQUESTS),
            tally.received.div_euclid(REQUESTS),
        )
        .unwrap();
    }
    out
}

/// Whether `measured` passes `ceiling`, and the cell that reports it.
fn judged(measured: u64, ceiling: u64) -> (bool, String) {
    (measured <= ceiling, format!("{measured} (<= {ceiling})"))
}

/// Whether the allocation and peak ceilings gate this build: they are the
/// figures an uninstrumented Linux build measures, and another allocator or
/// the coverage instrumentation (`--cfg coverage`) counts and holds more.
// NOTE: #6338 and #6387 gate allocations on uninstrumented Linux; other builds print them.
pub(super) const ALLOCATIONS_GATED: bool = cfg!(all(target_os = "linux", not(coverage)));

/// [`judged`] for an allocation count or a peak, which passes where
/// [`ALLOCATIONS_GATED`] is false and reports the measured figure beside its
/// ceiling there.
fn judged_allocations(measured: u64, ceiling: u64) -> (bool, String) {
    if ALLOCATIONS_GATED {
        judged(measured, ceiling)
    } else {
        (
            true,
            format!("{measured} (ceiling {ceiling}, gated on uninstrumented Linux)"),
        )
    }
}

/// Measures every path of [`Path::ALL`] in the order of [`BUDGETS`] over the
/// router `app`, counting the database wire through `wire` when one is given.
async fn measure_all(
    app: &Router,
    pool: &PgPool,
    pools: &DomainPools,
    svc: &ferroehr::service::FerroEhrService,
    wire: Option<&super::wire::Wire>,
    counter: &Counter,
) -> Vec<(&'static Budget, Measure)> {
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_walinspect")
        .execute(pool)
        .await
        .expect("pg_walinspect: a contrib module the server ships");
    super::wal::kept(pool).await;
    let world = World::build(app).await;
    let mut measured = Vec::with_capacity(BUDGETS.len());
    for budget in &BUDGETS {
        let path = budget.path;
        world.prepare(app, svc, path).await;
        let m = Box::pin(measure(app, pool, pools, wire, &world, counter, path)).await;
        measured.push((budget, m));
    }
    measured
}

/// Every hot path stays within its budget, and the report names what each
/// one measured.
#[tokio::test]
async fn every_hot_path_stays_within_its_budget() {
    let counter = Counter::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(counter.clone()));
    // The budgets count what a release build does, without the debug reference cells.
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, svc, app, pools) = Box::pin(super::steady()).await;
    let measured = Box::pin(measure_all(&app, &pool, &pools, &svc, None, &counter)).await;
    let mut report = String::from(
        "path | conversions | audit renders | allocations | alloc KiB | peak KiB | round trips | nested | notifies | WAL B\n",
    );
    let mut ledger = String::new();
    let mut statements = String::new();
    let mut over = Vec::new();
    for (budget, m) in &measured {
        let path = budget.path;
        let allocations = m.per_request(|a| a.count);
        let kib = m.per_request(|a| a.bytes).div_euclid(1024);
        let peak = m.low(|a| u64::try_from(a.peak.max(0)).unwrap());
        let cells = [
            ("conversions", judged(m.conversions(), budget.conversions)),
            (
                "allocations",
                judged_allocations(allocations, budget.allocations),
            ),
            ("peak bytes", judged_allocations(peak, budget.peak_bytes)),
            ("round trips", judged(m.round_trips, budget.round_trips)),
            ("nested statements", judged(m.nested, budget.nested)),
            ("notifies", judged(m.notifies, budget.notifies)),
            ("WAL bytes", judged(m.wal, budget.wal)),
        ];
        for (what, (ok, cell)) in &cells {
            if !ok {
                over.push(format!("{}: {what} {cell}", path.name()));
            }
        }
        writeln!(
            report,
            "{} | {} | {} | {} | {kib} | {} | {} | {} | {} | {}",
            path.name(),
            cells[0].1.1,
            m.audit(),
            cells[1].1.1,
            peak.div_euclid(1024),
            cells[3].1.1,
            cells[4].1.1,
            cells[5].1.1,
            cells[6].1.1,
        )
        .unwrap();
        let ops: Vec<String> = m
            .conversions
            .iter()
            .map(|(op, n)| format!("{op}={}", n.div_euclid(REQUESTS)))
            .collect();
        writeln!(ledger, "{}: {}", path.name(), ops.join(", ")).unwrap();
        writeln!(
            statements,
            "{}: allocations, peak and bytes of each request {:?}\n{}row records by relation:\n{}",
            path.name(),
            m.requests(),
            m.statements,
            m.wal_ledger
        )
        .unwrap();
    }
    eprintln!(
        "{report}\nconversions per request by op:\n{ledger}\nnested statements over {REQUESTS} \
         requests:\n{statements}"
    );
    assert!(
        over.is_empty(),
        "a hot path exceeds its budget: {over:?}\n{report}\n{ledger}\n{statements}"
    );
    assert_eq!(BUDGETS.len(), Path::ALL.len(), "every path has its budget");
}

/// Every hot path stays within the bytes its budget allows it over the
/// database wire, both directions together (#6244), and the report names
/// what each statement moved.
///
/// The server reaches the database through a [`super::wire::Wire`] relay,
/// which delays every exchange a little and so moves how a request's buffers
/// grow: its other figures are judged by
/// [`every_hot_path_stays_within_its_budget`], over a direct connection.
#[tokio::test]
async fn every_hot_path_stays_within_its_wire_budget() {
    let counter = Counter::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(counter.clone()));
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, svc, app, pools, wire) = Box::pin(super::wired()).await;
    let measured = Box::pin(measure_all(
        &app,
        &pool,
        &pools,
        &svc,
        Some(&wire),
        &counter,
    ))
    .await;
    let mut report = String::from("path | wire B | sent B | received B\n");
    let mut ledgers = String::new();
    let mut over = Vec::new();
    for (budget, m) in &measured {
        let path = budget.path;
        let ceiling = WIRE_BUDGETS
            .iter()
            .find(|(of, _)| *of == path)
            .map(|(_, ceiling)| *ceiling)
            .expect("every path has its wire budget");
        let (ok, cell) = judged(median(&m.wire_each), ceiling);
        if !ok {
            over.push(format!("{}: wire bytes {cell}", path.name()));
        }
        let total = m.wire.total();
        writeln!(
            report,
            "{} | {cell} | {} | {}",
            path.name(),
            total.sent.div_euclid(REQUESTS),
            total.received.div_euclid(REQUESTS),
        )
        .unwrap();
        writeln!(ledgers, "{}: {}", path.name(), wire_ledger(m)).unwrap();
    }
    eprintln!("{report}\n{ledgers}");
    assert!(
        over.is_empty(),
        "a hot path exceeds its wire budget: {over:?}\n{report}\n{ledgers}"
    );
    assert_eq!(
        WIRE_BUDGETS.len(),
        Path::ALL.len(),
        "every path has its wire budget"
    );
}

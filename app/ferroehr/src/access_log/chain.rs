// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The verification of a span of an access log against the checkpoint that
//! seals it: the one verifier of a log held apart from the database that
//! wrote it, such as an export or an imported log (#5129).
//!
//! The digests are the draft's (`docs/plans/access-log.md` §Integrity), the
//! same the insert trigger of `audit.access_log_record` computes: the
//! content digest is the SHA-256 of a record's canonical form, the link
//! digest the SHA-256 of the previous digest, the sequence and the content
//! digest, one per line, each in lower-case hexadecimal. A tombstone keeps
//! the content digest of the entry it replaced, so its content digest is
//! taken as recorded and only its link is recomputed, which is how a log
//! verifies after retention removed content in its middle.
//!
//! The chain of the local database is verified where it is stored, by
//! `audit.verify_audit_chain()`, since it interleaves the records of
//! `audit_event`, whose digest only the database recomputes. No openEHR spec
//! governs log integrity: our own design.

use crate::access_log::store::{AccessLogError, canonical_form};
use crate::access_log_draft::access_log::access_log_checkpoint::AccessLogCheckpoint;
use crate::access_log_draft::access_log::access_log_link::AccessLogLink;
use crate::access_log_draft::access_log::access_log_record::AccessLogRecord;
use crate::digest::sha256_hex;

/// The one digest algorithm of the chain, as a link names it.
pub const ALGORITHM: &str = "SHA-256";

/// `ACCESS_LOG_LINK.digest` of a link at `sequence` over `content_digest`,
/// following the link whose digest is `previous`.
#[must_use]
pub fn link_digest(previous: &str, sequence: i64, content_digest: &str) -> String {
    let line = format!("{previous}\n{sequence}\n{content_digest}");
    sha256_hex(&line)
}

/// One thing a span of the log fails, at the position it is about.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainFinding {
    /// The span holds no link.
    #[error("the span holds no link")]
    NoLinks,
    /// A link does not follow the one before it in sequence.
    #[error("the link at {sequence} does not follow the link at {previous}")]
    NotAdjacent {
        /// The link's sequence.
        sequence: i64,
        /// The sequence of the link before it in the span.
        previous: i64,
    },
    /// A link names an algorithm the chain does not use.
    #[error("the link at {sequence} names the algorithm {algorithm:?}")]
    Algorithm {
        /// The link's sequence.
        sequence: i64,
        /// The algorithm it names.
        algorithm: String,
    },
    /// A link's digest is not the digest of its own attributes.
    #[error("the digest of the link at {sequence} does not recompute")]
    LinkDigest {
        /// The link's sequence.
        sequence: i64,
    },
    /// A link's previous digest is not the digest of the link before it.
    #[error("the link at {sequence} does not link to the link before it")]
    PreviousDigest {
        /// The link's sequence.
        sequence: i64,
    },
    /// A link's signature does not verify.
    #[error("the signature of the link at {sequence} does not verify")]
    LinkSignature {
        /// The link's sequence.
        sequence: i64,
    },
    /// The span does not end at the checkpoint's sequence.
    #[error("the span ends at {sequence}, not at the checkpoint's {checkpoint}")]
    CheckpointPosition {
        /// The sequence of the last link of the span.
        sequence: i64,
        /// The checkpoint's sequence.
        checkpoint: i64,
    },
    /// The last link's digest is not the digest the checkpoint seals.
    #[error("the link at {sequence} is not the link the checkpoint seals")]
    CheckpointDigest {
        /// The checkpoint's sequence.
        sequence: i64,
    },
    /// The checkpoint's proof does not verify against a trusted key.
    #[error("the checkpoint at {sequence} is not signed by a trusted key")]
    CheckpointProof {
        /// The checkpoint's sequence.
        sequence: i64,
    },
    /// A record belongs to another system than the checkpoint.
    #[error("the record at {sequence} belongs to the system {system_id:?}")]
    OtherSystem {
        /// The record's sequence.
        sequence: i64,
        /// The system it names.
        system_id: String,
    },
    /// A record's link is not the span's link at its sequence.
    #[error("the record at {sequence} carries a link the span does not hold")]
    RecordLink {
        /// The record's sequence.
        sequence: i64,
    },
    /// An entry's content is not the content its link digested.
    #[error("the content of the entry at {sequence} was modified after it was written")]
    ContentDigest {
        /// The entry's sequence.
        sequence: i64,
    },
}

/// A span of one system's log: its records, every link from the first
/// record's to the checkpoint's, and the checkpoint.
#[derive(Debug, Clone, Copy)]
pub struct Span<'a> {
    /// The records, entries and tombstones, each with its link.
    pub records: &'a [AccessLogRecord],
    /// The links, consecutive, the last at the checkpoint's sequence.
    pub links: &'a [AccessLogLink],
    /// The checkpoint the span verifies against.
    pub checkpoint: &'a AccessLogCheckpoint,
}

/// Verify `span` against its checkpoint.
///
/// Each link recomputes and links to the one before it, the last is the one
/// the checkpoint seals, the checkpoint and each signed link carry a
/// signature `signed` accepts, and each record carries the span's link at
/// its sequence over its own content.
///
/// A checkpoint's proof signs its canonical form, the proof left out (RM
/// common `master04-generic_package.adoc` §Attestation); a link's signature
/// signs its digest.
///
/// `signed(data, signature)` says whether `signature` is a trusted key's
/// signature over `data`. An empty result means the span verifies.
///
/// # Errors
/// [`AccessLogError::Canonical`] when a record or the checkpoint has no
/// canonical form.
pub fn verify(
    span: Span<'_>,
    signed: &dyn Fn(&[u8], &str) -> bool,
) -> Result<Vec<ChainFinding>, AccessLogError> {
    let mut findings = Vec::new();
    verify_links(span.links, signed, &mut findings);
    verify_checkpoint(span, signed, &mut findings)?;
    for record in span.records {
        verify_record(span, record, &mut findings)?;
    }
    Ok(findings)
}

/// The findings of the links alone.
fn verify_links(
    links: &[AccessLogLink],
    signed: &dyn Fn(&[u8], &str) -> bool,
    findings: &mut Vec<ChainFinding>,
) {
    let mut before: Option<&AccessLogLink> = None;
    for link in links {
        let sequence = link.sequence;
        if link.algorithm != ALGORITHM {
            findings.push(ChainFinding::Algorithm {
                sequence,
                algorithm: link.algorithm.clone(),
            });
        }
        if link.digest != link_digest(&link.previous_digest, sequence, &link.content_digest) {
            findings.push(ChainFinding::LinkDigest { sequence });
        }
        if let Some(previous) = before {
            if previous.sequence.checked_add(1) != Some(sequence) {
                findings.push(ChainFinding::NotAdjacent {
                    sequence,
                    previous: previous.sequence,
                });
            } else if link.previous_digest != previous.digest {
                findings.push(ChainFinding::PreviousDigest { sequence });
            }
        }
        if let Some(signature) = &link.signature
            && !signed(link.digest.as_bytes(), signature)
        {
            findings.push(ChainFinding::LinkSignature { sequence });
        }
        before = Some(link);
    }
}

/// The findings of the checkpoint against the last link.
fn verify_checkpoint(
    span: Span<'_>,
    signed: &dyn Fn(&[u8], &str) -> bool,
    findings: &mut Vec<ChainFinding>,
) -> Result<(), AccessLogError> {
    let checkpoint = span.checkpoint;
    let sequence = checkpoint.sequence;
    match span.links.last() {
        None => findings.push(ChainFinding::NoLinks),
        Some(last) if last.sequence != sequence => {
            findings.push(ChainFinding::CheckpointPosition {
                sequence: last.sequence,
                checkpoint: sequence,
            });
        }
        Some(last) if last.digest != checkpoint.digest => {
            findings.push(ChainFinding::CheckpointDigest { sequence });
        }
        Some(_) => {}
    }
    if checkpoint.algorithm != ALGORITHM {
        findings.push(ChainFinding::Algorithm {
            sequence,
            algorithm: checkpoint.algorithm.clone(),
        });
    }
    let form = canonical_form(checkpoint)?;
    if !signed(form.as_bytes(), &checkpoint.proof) {
        findings.push(ChainFinding::CheckpointProof { sequence });
    }
    Ok(())
}

/// The findings of one record against the span.
fn verify_record(
    span: Span<'_>,
    record: &AccessLogRecord,
    findings: &mut Vec<ChainFinding>,
) -> Result<(), AccessLogError> {
    let (system_id, link, entry) = match record {
        AccessLogRecord::AccessLogEntry(entry) => (&entry.system_id, &entry.link, true),
        AccessLogRecord::AccessLogTombstone(tombstone) => {
            (&tombstone.system_id, &tombstone.link, false)
        }
    };
    let sequence = link.sequence;
    if *system_id != span.checkpoint.system_id {
        findings.push(ChainFinding::OtherSystem {
            sequence,
            system_id: system_id.clone(),
        });
    }
    let held = span
        .links
        .binary_search_by_key(&sequence, |held| held.sequence)
        .ok()
        .and_then(|index| span.links.get(index));
    if held != Some(link) {
        findings.push(ChainFinding::RecordLink { sequence });
    }
    if entry && link.content_digest != sha256_hex(&canonical_form(record)?) {
        findings.push(ChainFinding::ContentDigest { sequence });
    }
    Ok(())
}

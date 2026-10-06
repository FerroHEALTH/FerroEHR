// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FerroEHR` application library — the platform crate.
//!
//! Modern idiomatic Rust on top of the generated `openehr-*` crates. Two
//! layers of module, mirroring the specification structure:
//!
//! **Spec-governed areas** (each maps to its openEHR oracle):
//! - [`versioning`] — change control + integrity (RM common
//!   `master06-change_control_package` + `master04-generic_package`; BASE
//!   `base_types/master05-identification_package`; digital signature per RM
//!   common master06 §Digital Signature lives inside it as
//!   `versioning::signature`).
//! - [`service`] — the SM Platform Service Model realization, one folder per
//!   SM chapter (`SM/docs/openehr_platform/`).
//! - [`aql`] — the AQL 1.1 execution engine (QUERY `master03-syntax`; the
//!   lowering internals are our own design).
//! - [`validation`] — archetype/template artefact validity along the AOM
//!   constraint model (AM AOM 1.4 + AOM 2).
//! - [`templates`] — OPT ingestion, the template store, and derived runtime
//!   artefacts (BASE `resource/master02-resource_package`; AM OPT).
//! - [`system_log`] — the SM System Log component ("IHE ATNA-compliant
//!   system log", SM `master02-overview`; rendered per DICOM PS3.15 §A.5 over
//!   RFC 5424/5425 syslog).
//!
//! **Spec-silent internals and extensions** (no openEHR spec governs these —
//! our own design, each flagged in its module docs):
//! - [`storage`] — the decomposed node model, node codec, and row I/O.
//! - [`db`] — pool, settings, migrators.
//! - [`telemetry`] — observability infrastructure.
//! - [`extensions`] — quarantined enterprise extensions (eventing, FHIR
//!   connector, multimedia offload), each off by default behind its
//!   own config gate.
//! - [`manufacturer`] — the manufacturer every surface names, and [`report`]
//!   — the redacted deployment report `ferroehr report` writes.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
pub mod aql;
pub mod banner;
pub mod config;
pub mod db;
pub mod extensions;
pub mod ids;
pub mod licence;
pub mod manufacturer;
pub mod privacy;
pub mod report;
pub mod service;
pub mod storage;
pub mod system_log;
pub mod telemetry;
pub mod templates;
pub mod usage_report;
pub mod validation;
pub mod versioning;

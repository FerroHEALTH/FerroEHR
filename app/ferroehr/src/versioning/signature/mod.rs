// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR `VERSION.signature` signing + verification.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Digital Signature —
//! a digital signature over the version's canonical serialized form, and BASE
//! arch-overview `master07-security.adoc` §Integrity, which groups "Versioning"
//! and "Digital Signature" as the two faces of one integrity mechanism. The
//! spec places the signature **inside** the change-control model, so this lives
//! as a submodule of `versioning`, not a standalone sibling.
//!
//! Two spec-blessed modes, both first-class:
//! - **digest** — `sha256:` + radix-64(SHA-256(canonical_form)): a
//!   data-integrity check that needs no key management. master07 §Digital
//!   Signature: "the encryption step might be omitted, resulting in a digest
//!   only", i.e. a pure integrity check. The default.
//! - **pgp** — an `OpenPGP` RFC 4880 detached signature (rPGP), ASCII-armored,
//!   with a server-held private key: authentication + non-repudiation
//!   (master06 §Digital Signature: the signature "is generated according to the
//!   openPGP standard (IETF RFC 4880)").
//!
//! The bytes signed are the `VERSION.canonical_form()` produced by `openehr-rm`
//! (canonical openEHR JSON per RFC 8785, signature attribute Void during
//! serialization — master06 §Digital Signature). NOTE
//! (master06 §Digital Signature `[.tbd]`): the exact canonical serialization is
//! openEHR-TBD (ODIN preferred, XML libraries differ); we use canonical openEHR
//! JSON, which is deterministic and signature-independent. This module is
//! agnostic to how that string is produced — it only signs/verifies it.

pub mod config;
pub mod key;
pub mod signer;
pub mod verify;

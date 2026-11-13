// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The spec-homeless surface: deliberate local extensions.
//!
//! No openEHR specification governs anything in this module tree; every
//! submodule is our own design/extension. These are the features the `ferroehr`
//! binary offers that no openEHR spec requires or describes, consolidated in one
//! place so the spec-homeless surface stays visible and auditable.
//!
//! The no-spec-home finding is verified rather than assumed. The three openEHR
//! chapters that touch integration, deployment and security-of-access were
//! checked as the last possible spec home: master14 governs
//! archetype-to-archetype data conversion (`GENERIC_ENTRY` + `FEEDER_AUDIT`) and
//! not change feeds or outbound emission; master13 is
//! informative deployment guidance prescribing no eventing or
//! blob offload; master07 governs the `EHR_ACCESS` object and
//! authn-at-deployment. The one place a submodule touches spec-defined data is
//! the multimedia offload's `DV_MULTIMEDIA` rewrite, which carries its RM
//! citation in [`multimedia`].
//!
//! Every extension keeps two non-negotiables: each submodule doc comment carries
//! the explicit flag "no openEHR spec governs this — our own design/extension",
//! and the feature does nothing unless its gate enables it, so with its gate
//! off the commit and read paths are byte-identical to the no-extension
//! behaviour.
//!
//! | Submodule | Gate (default off) |
//! |---|---|
//! | [`multimedia`] | `[record.multimedia] enabled` |
//! | [`outbox`] | a `read_change_feed` grant in force (none) |

pub mod multimedia;
pub mod outbox;

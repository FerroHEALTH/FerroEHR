// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! **Native canonical-JSON codec** (ITS-JSON, both directions).
//!
//! The per-type work is EMITTED (`openehr-codegen -- emit-json`) as manual
//! `serde::Serialize`/`serde::Deserialize` impls that live in each spec crate's
//! own `json_serde` module — the impls have to live where the types are defined
//! (orphan rule), and being in-crate is also what lets them read a validated
//! class's `pub(crate)` fields and construct through its hand-written door.
//! Their shared runtime is `openehr_base::serde_support`; the named entry
//! points are [`crate::json`].
//!
//! What remains here is the part that cannot live in any single spec crate:
//! [`generated::structural`], the `_type`-keyed dispatch from a wire class name
//! to that class's `Deserialize` (and the matching declared-key table), which
//! spans every spec crate at once.

pub mod generated;

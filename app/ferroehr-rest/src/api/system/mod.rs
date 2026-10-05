// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **System API** (Release-1.1.0, STABLE) —
//! `docs/specs/openehr/ITS-REST/specifications/docs/system/` + the
//! `system-*.openapi.yaml` OAS group.
//!
//! One operation: `OPTIONS /` — the System Options and Conformance manifest.
//!
//! The System API is not part of the generated ITS-REST contract (the
//! `emit-rest` groups are `ehr`/`query`/`definition`/`admin`/`demographic`
//! only, `crates/openehr-its/src/rest/generated/`), so its single operation is
//! hand-written in [`options`]. [`options::route`] builds the `OPTIONS`
//! handler; the wiring layer (`crate::router::router`) constructs the
//! [`options::SystemManifest`] from config + the live mounted-group set and
//! mounts it at the API base-path root (and a bare-`/` compatibility alias),
//! above the CORS layer.

pub mod options;

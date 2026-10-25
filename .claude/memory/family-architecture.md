---
name: family-architecture
description: The FerroHEALTH family is separate servers that talk only over published protocols, plus shared crates for cross-cutting work; no product hand-writes what another product or a shared crate provides (owner, 2026-10-25)
metadata:
  type: project
---

<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

The FerroHEALTH family is five products, each its own server, image and
repository, and each reaches the others only over a published protocol.
No product is a compile-time dependency of another.

| Product | What it is | How the others reach it |
|---|---|---|
| FerroEHR | the openEHR CDR | openEHR ITS-REST and AQL |
| FerroBRIDGE | openEHR to FHIR (FHIRconnect) and to the OMOP CDM (OMOCL) | it calls the CDR over ITS-REST; clients call its FHIR R4 face |
| FerroFED | the federation gateway (Federation Tier with AQL, EHDS cross-border, MyHealth@EU) | it calls each CDR over ITS-REST; clients call its ITS-REST and FHIR faces |
| FerroPIX | the master patient index (IHE PIX, PIXm, PDQ, PDQm, PMIR, HL7 v2) | IHE PIXm, PDQm and PMIR, and HL7 v2 over MLLP |
| FerroTERM | the FHIR terminology server | FHIR terminology operations |

The rules that follow from it:

- **A product stays in its own lane.** FerroEHR implements no FHIR, OMOP,
  federation or IHE profile: FHIR and OMOP are FerroBRIDGE's, federation is
  FerroFED's, identity and the IHE profiles are FerroPIX's. When a product
  needs another's function, it calls that server over its protocol, as a
  client.
- **Shared crates carry the cross-cutting work**, never a product's domain:
  - FerroAUTH (renamed from FerroSMART on 2026-10-25): `ferrohealth-smart` (authentication, OAuth 2.0 and SMART,
    the authenticated subject with its scopes and roles) and
    `ferrohealth-authz` (the policy decision: ABAC, Cedar, a remote PDP);
  - FerroAUDIT: `ihe-atna` (the whole audit trail: sending, forwarding and
    retrieving) and `ehds-logging` (the EHDS access record);
  - FerroUI: `ferrohealth-ui` (every console);
  - protocol clients and models: `fhir-types` and `ferrobridge-term` from
    FerroBRIDGE, `ihe-iti` and `nl-generic-functions` from FerroPIX, the
    `openehr-*` crates from FerroEHR.
- **Nothing is hand-written that a shared crate or another product
  provides.** When the shared crate lacks something, it is added in the
  owning repository and released; a local copy is never the workaround.
- **Every console is reactive.** A link, a form or an action never reloads
  the whole document; WebAssembly is the normal path and the
  no-WebAssembly page only the fallback (FerroUI #410).

Out of scope until the owner says otherwise: FerroFORMS (paused), the
FerroHEALTH umbrella repository (paused), and Veredictum (being rewritten,
so its verdicts are not authority). FerroPULSE receives the products'
usage report, which release compose files keep on by default with a
documented off switch.

**Why:** the owner corrected the orchestrator on 2026-10-25 after briefs
put FerroBRIDGE's FHIR crates into FerroEHR, called FerroEHR an IHE actor
and kept hand-written copies of shared work. A stable family needs each
product to own one job and every shared concern to live in one crate.

**How to apply:** before filing an issue, writing a brief or adding code,
check it against the table and the rules above. A change that puts another
product's domain into this repository, or copies what a shared crate does,
is wrong by construction.
